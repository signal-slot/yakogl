// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Milestone L1 of driver/docs/nanopc-t6-mali.md without the device: the
//! triangle frame YakoGL's frame builder (src/mali/frame.rs) builds, in
//! memory the caller gives as (CPU slice, GPU VA) pairs, and the read-back
//! and framebuffer helpers `panthor-frame` uses. Everything here runs on the
//! host too; `panthor-frame` adds the panthor objects and the submit.
//!
//! The frame is the board trace's (tools/mali/board-dump/
//! tri-frame-2026-10-18.txt): a 64x64 linear RGBA8 target cleared to blue,
//! two red triangles over its left half, Mesa 25.0.7's two programs
//! ([`crate::tri_shaders`]).

use yakogl::mali::arena::Arena;
use yakogl::mali::cs::Stream;
use yakogl::mali::frame::{self, FrameError};
use yakogl::mali::genxml::v10::ShaderStage;

use crate::tri_shaders;

/// The target's size and layout.
pub const WIDTH: u32 = 64;
pub const HEIGHT: u32 = 64;
pub const ROW_STRIDE: u32 = WIDTH * 4;
/// Mesa's geometry buffer (src/mali/frame.rs).
pub use yakogl::mali::frame::GEOMETRY_BUFFER_BYTES;
/// The six vertices of the trace's program (tri.c: two triangles over the
/// left half of clip space), two floats each.
pub const VERTICES: [f32; 12] = [-1.0, -1.0, 0.0, -1.0, -1.0, 1.0, 0.0, -1.0, 0.0, 1.0, -1.0, 1.0];
/// glClearColor(0, 0, 1, 1).
pub const CLEAR: [f32; 4] = [0.0, 0.0, 1.0, 1.0];

/// The per-context objects and the user registers, in src/mali since L2.
pub use yakogl::mali::frame::{build_context, user_regs};

/// The arenas of one frame.
pub struct FrameArenas<'a, 'm> {
    /// Descriptors.
    pub desc: &'a mut Arena<'m>,
    /// Shader binaries (bound executable).
    pub code: &'a mut Arena<'m>,
    /// Shader Program descriptors.
    pub spds: &'a mut Arena<'m>,
    /// Vertex data.
    pub data: &'a mut Arena<'m>,
}

/// Build the triangle frame (or, with `draw` false, the clear alone) into
/// `a` and its command stream into `cs`, for the target at `target_gpu`.
pub fn build_triangle_frame(
    a: FrameArenas,
    cs: &mut Stream,
    ctx: &frame::Context,
    target_gpu: u64,
    draw: bool,
) -> Result<frame::Frame, FrameError> {
    let target = frame::Target {
        gpu: target_gpu,
        width: WIDTH,
        height: HEIGHT,
        row_stride: ROW_STRIDE,
        format: frame::TargetFormat::Rgba8888,
        samples: 1,
    };
    if !draw {
        let input = frame::FrameInput { target, clear_rgba: Some(CLEAR), zs: None, draw: None };
        return frame::build_frame(a.desc, cs, ctx, &input);
    }
    let vs_bin = tri_shaders::bytes(&tri_shaders::VS_WORDS);
    let fs_bin = tri_shaders::bytes(&tri_shaders::FS_WORDS);
    let vs = frame::upload_program(a.code, a.spds, &vs_bin, tri_shaders::meta(ShaderStage::VERTEX))?;
    let fs = frame::upload_program(a.code, a.spds, &fs_bin, tri_shaders::meta(ShaderStage::FRAGMENT))?;
    let vbytes: Vec<u8> = VERTICES.iter().flat_map(|v| v.to_le_bytes()).collect();
    let vb = a.data.upload(&vbytes, 64)?;
    let attributes = [frame::VertexAttribute {
        buffer: frame::VertexBuffer { gpu: vb.gpu, size: vb.len as u32 },
        offset: 0,
        stride: 8,
        components: 2,
        instance_divisor: 0,
    }];
    let input = frame::FrameInput {
        target,
        clear_rgba: Some(CLEAR),
        zs: None,
        draw: Some(frame::DrawCall {
            vs: &vs,
            fs: &fs,
            primitive: frame::Topology::Triangles,
            first_vertex: 0,
            vertex_count: 6,
            instance_count: 1,
            indices: None,
            attributes: &attributes,
            viewport: frame::Viewport { x: 0, y: 0, width: WIDTH as i32, height: HEIGHT as i32, near: 0.0, far: 1.0 },
            scissor: None,
            line_width: 1.0,
            flip_y: false,
            blend_color: [0.0; 4],
            blend: None,
            polygon_offset: None,
            sample_mask: 0xf,
            /* the board's tri programs read no uniform */
            vs_uniforms: &[],
            fs_uniforms: &[],
            zs: frame::DepthStencilState::OFF,
            colour_mask: 0xf,
            front_cw: false,
            cull: frame::Cull::None,
            textures: &[],
            vs_texture: None,
            occlusion_query: None,
            alpha_to_coverage: false,
            blend_shader: None,
        }),
    };
    frame::build_frame(a.desc, cs, ctx, &input)
}

/// What a read-back holds, pixel by pixel.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Tally {
    /// Opaque red (ff 00 00 ff).
    pub red: usize,
    /// Opaque blue (00 00 ff ff).
    pub blue: usize,
    /// Neither.
    pub other: usize,
    /// Pixels equal to the fill pattern (a subset of `other`).
    pub fill: usize,
    /// Pixels that are not what the frame should put there: red in the
    /// left half and blue in the right with the draw, blue everywhere
    /// without it.
    pub misplaced: usize,
}

/// The fill pattern the target holds before the frame (not red, not blue,
/// not 0, not ff, so neither a clear nor an untouched page reads as it).
pub const FILL: [u8; 4] = [0x5a, 0xa5, 0x3c, 0xc3];

/// Count `bytes` as a WIDTH x HEIGHT RGBA8 image (row 0 is GL's bottom; the
/// triangles' left half is x < WIDTH / 2 on every row).
pub fn tally(bytes: &[u8], draw: bool) -> Result<Tally, String> {
    let need = (ROW_STRIDE * HEIGHT) as usize;
    if bytes.len() < need {
        return Err(format!("{} bytes hold no {WIDTH}x{HEIGHT} RGBA8 image ({need})", bytes.len()));
    }
    let mut t = Tally::default();
    for y in 0..HEIGHT as usize {
        for x in 0..WIDTH as usize {
            let o = y * ROW_STRIDE as usize + x * 4;
            let p = &bytes[o..o + 4];
            let red = p == [0xff, 0, 0, 0xff];
            let blue = p == [0, 0, 0xff, 0xff];
            match (red, blue) {
                (true, _) => t.red += 1,
                (_, true) => t.blue += 1,
                _ => t.other += 1,
            }
            if p == FILL {
                t.fill += 1;
            }
            let want_red = draw && x < WIDTH as usize / 2;
            if (want_red && !red) || (!want_red && !blue) {
                t.misplaced += 1;
            }
        }
    }
    Ok(t)
}

/// One row of framebuffer bytes to write: its byte offset and its bytes.
pub type FbRow = (usize, Vec<u8>);

/// The rectangle a blit writes, in framebuffer pixels: [x0, x1) x [y0, y1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x0: u32,
    pub y0: u32,
    pub x1: u32,
    pub y1: u32,
}

/// The rows that put the RGBA8 target, scaled by `scale`, at the top-left
/// of a 32-bpp XRGB8888 framebuffer of `fb_w` x `fb_h` pixels and
/// `fb_stride` bytes a row, inside a 1-pixel white border. The target's
/// row 0 (GL's bottom) goes to the bottom of the picture, so the picture is
/// what GL drew. Nothing outside the rectangle is written; a rectangle that
/// does not fit is refused.
pub fn fb_rows(target: &[u8], scale: u32, fb_w: u32, fb_h: u32, fb_stride: u32) -> Result<(Rect, Vec<FbRow>), String> {
    if scale == 0 {
        return Err("scale 0".into());
    }
    let side = WIDTH * scale + 2;
    let tall = HEIGHT * scale + 2;
    if side > fb_w || tall > fb_h || fb_stride < fb_w * 4 {
        return Err(format!(
            "a {side}x{tall} rectangle does not fit a {fb_w}x{fb_h} framebuffer of stride {fb_stride}"
        ));
    }
    if target.len() < (ROW_STRIDE * HEIGHT) as usize {
        return Err(format!("target of {} bytes", target.len()));
    }
    let white = [0xffu8, 0xff, 0xff, 0xff];
    let mut rows = Vec::with_capacity(tall as usize);
    for fy in 0..tall {
        let mut row = Vec::with_capacity(side as usize * 4);
        for fx in 0..side {
            if fy == 0 || fy == tall - 1 || fx == 0 || fx == side - 1 {
                row.extend_from_slice(&white);
                continue;
            }
            let tx = (fx - 1) / scale;
            let ty = HEIGHT - 1 - (fy - 1) / scale;
            let o = (ty * ROW_STRIDE + tx * 4) as usize;
            let (r, g, b) = (target[o], target[o + 1], target[o + 2]);
            /* XRGB8888, little-endian: B G R X */
            row.extend_from_slice(&[b, g, r, 0xff]);
        }
        rows.push(((fy * fb_stride) as usize, row));
    }
    Ok((Rect { x0: 0, y0: 0, x1: side, y1: tall }, rows))
}

#[cfg(test)]
mod tests {
    use super::*;
    use yakogl::mali::cs::Instr;

    /// Every arena's bytes and the stream's words of one built frame.
    #[derive(Debug, PartialEq)]
    struct Bytes {
        ctx: Vec<u8>,
        desc: Vec<u8>,
        code: Vec<u8>,
        spds: Vec<u8>,
        data: Vec<u8>,
        words: Vec<u64>,
    }

    struct Mem {
        ctx: Vec<u8>,
        desc: Vec<u8>,
        code: Vec<u8>,
        spds: Vec<u8>,
        data: Vec<u8>,
    }

    impl Mem {
        fn new() -> Mem {
            Mem {
                ctx: vec![0; 0x1000],
                desc: vec![0; 0x4000],
                code: vec![0; 0x1000],
                spds: vec![0; 0x1000],
                data: vec![0; 0x1000],
            }
        }
        fn ptrs(&self) -> [u64; 5] {
            [&self.ctx, &self.desc, &self.code, &self.spds, &self.data].map(|v| v.as_ptr() as u64)
        }
    }

    /// Build the frame into `m` with the arenas based at `bases` (ctx,
    /// desc, code, spds, data), the heap and geometry buffer and target at
    /// fixed VAs.
    fn build(m: &mut Mem, bases: [u64; 5], draw: bool) -> Bytes {
        let mut words = vec![0u64; 256];
        let n;
        {
            let mut ctx = Arena::new("context", bases[0], &mut m.ctx).expect("arena");
            let mut desc = Arena::new("desc", bases[1], &mut m.desc).expect("arena");
            let mut code = Arena::new("code", bases[2], &mut m.code).expect("arena");
            let mut spds = Arena::new("spds", bases[3], &mut m.spds).expect("arena");
            let mut data = Arena::new("data", bases[4], &mut m.data).expect("arena");
            let c = build_context(&mut ctx, 0x8000_00a0_0000, 2 << 20, 0x0200_0000, 8).expect("context");
            let mut cs = Stream::new(&mut words, 92);
            let a = FrameArenas { desc: &mut desc, code: &mut code, spds: &mut spds, data: &mut data };
            build_triangle_frame(a, &mut cs, &c, 0x0300_0000, draw).expect("the frame builds");
            n = cs.words().len();
        }
        words.truncate(n);
        Bytes {
            ctx: m.ctx.clone(),
            desc: m.desc.clone(),
            code: m.code.clone(),
            spds: m.spds.clone(),
            data: m.data.clone(),
            words,
        }
    }

    /// The pin of the GPU-VA path: a frame built with every arena based at
    /// its memory's CPU address (the only path backend.rs has) and one
    /// built into other memory whose arenas are based at those same
    /// addresses, which are not its own, hold the same bytes and words. So
    /// what `panthor-frame` writes at a BO's VA depends on the VA only, not
    /// on where the CPU mapping is.
    #[test]
    fn a_frame_built_at_given_vas_is_the_cpu_address_frame_when_the_bases_are_equal() {
        for draw in [true, false] {
            let mut cpu = Mem::new();
            let bases = cpu.ptrs();
            let a = build(&mut cpu, bases, draw);
            let mut other = Mem::new();
            assert!(other.ptrs().iter().zip(&bases).all(|(p, b)| p != b), "the second memory is elsewhere");
            let b = build(&mut other, bases, draw);
            assert_eq!(a, b, "draw {draw}");
            /* and the VA is what moves the words: another base changes them */
            let mut third = Mem::new();
            let moved = bases.map(|b| b + 0x10_0000);
            let c = build(&mut third, moved, draw);
            assert_ne!(a.words, c.words, "draw {draw}: the stream names the arenas' VAs");
            assert_eq!(a.words.len(), if draw { 51 } else { 13 }, "draw {draw}");
        }
    }

    /// The stream's first word and the FBD it names come from the desc
    /// arena's base, so a frame built for a BO at its VA points into it.
    #[test]
    fn the_fragment_job_names_the_desc_arena() {
        let mut m = Mem::new();
        let bases = [0x0100_0000, 0x0101_0000, 0x0102_0000, 0x0103_0000, 0x0104_0000];
        let b = build(&mut m, bases, true);
        let fbd = b
            .words
            .iter()
            .filter_map(|&w| match Instr::decode(w) {
                Ok(Instr::Move48(i)) if i.destination == 40 => Some(i.immediate),
                _ => None,
            })
            .collect::<Vec<_>>();
        /* d40 is the tiler context (IDVS) and then the FBD pointer
         * (fragment): both inside the desc arena */
        assert_eq!(fbd.len(), 2, "{fbd:x?}");
        for v in fbd {
            assert!((0x0101_0000..0x0101_4000).contains(&(v & !63)), "{v:#x}");
        }
    }

    #[test]
    fn the_user_registers_follow_the_csif_query() {
        assert_eq!(user_regs(96, 4), Some(92));
        assert_eq!(user_regs(96, 0), Some(92));
        assert_eq!(user_regs(96, 8), Some(88));
        assert_eq!(user_regs(4, 4), None);
    }

    #[test]
    fn the_tally_counts_what_the_frame_should_put_where() {
        let mut img = vec![0u8; (ROW_STRIDE * HEIGHT) as usize];
        for p in img.chunks_mut(4) {
            p.copy_from_slice(&FILL);
        }
        let t = tally(&img, true).unwrap();
        assert_eq!(t, Tally { red: 0, blue: 0, other: 4096, fill: 4096, misplaced: 4096 });
        for (i, p) in img.chunks_mut(4).enumerate() {
            let x = i % WIDTH as usize;
            p.copy_from_slice(if x < 32 { &[0xff, 0, 0, 0xff] } else { &[0, 0, 0xff, 0xff] });
        }
        assert_eq!(tally(&img, true).unwrap(), Tally { red: 2048, blue: 2048, other: 0, fill: 0, misplaced: 0 });
        assert_eq!(tally(&img, false).unwrap().misplaced, 2048);
        assert!(tally(&img[1..], true).is_err());
    }

    #[test]
    fn the_blit_writes_the_scaled_picture_inside_a_white_border_only() {
        let mut img = vec![0u8; (ROW_STRIDE * HEIGHT) as usize];
        /* GL's bottom-left pixel red, every other one blue */
        for p in img.chunks_mut(4) {
            p.copy_from_slice(&[0, 0, 0xff, 0xff]);
        }
        img[..4].copy_from_slice(&[0xff, 0, 0, 0xff]);
        let (r, rows) = fb_rows(&img, 8, 3840, 2160, 3840 * 4).unwrap();
        assert_eq!(r, Rect { x0: 0, y0: 0, x1: 514, y1: 514 });
        assert_eq!(rows.len(), 514);
        assert!(rows.iter().enumerate().all(|(y, (off, b))| *off == y * 3840 * 4 && b.len() == 514 * 4));
        let px = |x: usize, y: usize| -> [u8; 4] { rows[y].1[x * 4..x * 4 + 4].try_into().unwrap() };
        assert_eq!(px(0, 0), [0xff; 4]);
        assert_eq!(px(513, 200), [0xff; 4]);
        /* the red pixel is at the picture's bottom-left, 8x8 framebuffer pixels */
        assert_eq!(px(1, 512), [0, 0, 0xff, 0xff]);
        assert_eq!(px(8, 505), [0, 0, 0xff, 0xff]);
        assert_eq!(px(9, 512), [0xff, 0, 0, 0xff]);
        assert_eq!(px(1, 504), [0xff, 0, 0, 0xff]);
        assert_eq!(px(1, 1), [0xff, 0, 0, 0xff]);
        assert!(fb_rows(&img, 8, 500, 2160, 2000).is_err());
        assert!(fb_rows(&img, 0, 3840, 2160, 3840 * 4).is_err());
    }
}
