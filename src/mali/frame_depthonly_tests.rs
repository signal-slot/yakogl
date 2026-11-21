// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! A framebuffer with no colour attachment (cause D-DEPTHONLY of
//! docs/notes/mali-glmark2-forecast.md: glmark2's shadow pass renders into
//! a GL_DEPTH_COMPONENT texture alone), drawn as Mesa 26.2.1's panfrost
//! draws a batch of `nr_cbufs` 0 (docs/notes/mali-depth-only.md).
//!
//! No board or drm-shim record holds such a frame (tools/mali/board-dump
//! and tools/mali/testdata have none; every render target there has a
//! surface), so the words that differ from a frame with a colour target
//! are pinned here to the Mesa lines they follow:
//!
//! - the framebuffer descriptor: `Render Target Count` 1
//!   (`MAX2(fb->rt_count, 1)`, src/panfrost/lib/pan_desc.c:1410), the tile
//!   size and colour buffer allocation of one RGBA8 target ("dummy/
//!   non-existent render-targets use RGBA8 UNORM", `pan_cbuf_bytes_per_pixel`,
//!   pan_desc.c:569-592, through `pan_select_tile_size`, :607-648), the
//!   FBD pointer's tag one render target (:1509-1512);
//! - the one render target: Mesa's dummy (`pan_emit_rt` for `!rt`,
//!   pan_desc.c:984-1001): word 0 the internal format R8G8B8A8 (bits
//!   26-31) at internal buffer offset 0, word 1 the writeback block format
//!   Tiled U-Interleaved (bits 40-43) and dithering (bit 47), every other
//!   word 0: no Write Enable, no clean tile write (`pan_get_clean_tile_info`
//!   loops over `fb->rt_count`, 0, :1270-1279), no clear (`rt_clear`,
//!   :964-976), no writeback buffer;
//! - each draw: the target "unbacked", so its blend off ("Disable blending
//!   for unbacked render targets", src/gallium/drivers/panfrost/
//!   pan_cmdstream.c:298-307), no fragment shader (`panfrost_fs_required`,
//!   pan_cmdstream.h:200-235, loops over `nr_cbufs`), DCD 1's render target
//!   mask `outputs_written & fb_rt_mask` with `fb_rt_mask` 0
//!   (pan_context.c:457-462, pan_csf.c:1641-1645).
//!
//! The draw's words are the board's: Mesa 25.0.7's depth-only draw into an
//! RGBA8 FBO with glColorMask(none) (tools/mali/board-dump/
//! zs-depth-2026-10-19.txt, source "nocolour") takes the same branches
//! (`fs_required` false, the target's blend off, `fb_rt_mask` cut by no
//! written output), so its ZSD, Blend, tiler flags and DCD words are
//! compared byte for byte with ours, as is the framebuffer descriptor's
//! parameters block (the board's frame has a colour target at the same tile
//! size and allocation), every difference named.

use super::arena::Arena;
use super::backend::tests::{
    draw_flush, fbd_at_fragment, reg_at_idvs, regs_at_each_idvs, target, triangle, with, with_programs, Rig, TRI_FS,
    TRI_VS, VERTICES,
};
use super::backend::{mali_depth_resolve, mali_draw, mali_flush, mali_init};
use super::cs::{Instr, Stream};
use super::frame::*;
use super::genxml::v10::*;
use super::trace;
use crate::backend::{self, GlesDraw};
use crate::ds_retention;
use std::collections::BTreeMap;

const ZS: &str = include_str!("../../tools/mali/board-dump/zs-depth-2026-10-19.txt");

/* the trace's addresses (frame_zs_tests.rs): the descriptor buffer with
 * the FBD at its start, the ZS buffer, the sample positions, the heap */
const DESC: u64 = 0x7fff_fffd_0000;
const ZS_BUF: u64 = 0x7fff_fffe_0000;
const SAMPLES: u64 = 0x7fff_ffff_f000;
const HEAP_DESC: u64 = 0x7fff_ffff_b000;
const GEOMETRY: u64 = 0x7fff_fffe_b000;
/// The tiler context Mesa's FBD names, after its OOM context and
/// incremental-rendering FBDs (not built).
const TRACE_TILER: u64 = DESC + 0x300;

/// Mesa's dummy render target, the 16 words (`pan_emit_rt`, `!rt`).
const DUMMY_RT: [u32; 16] = {
    let mut w = [0u32; 16];
    /* Internal Format R8G8B8A8 (1) at bits 26-31 */
    w[0] = 1 << 26;
    /* Writeback Block Format Tiled U-Interleaved (1) at bits 40-43,
     * Dithering Enable bit 47 */
    w[1] = 1 << 8 | 1 << 15;
    w
};

fn words_of(bytes: &[u8]) -> Vec<u32> {
    bytes.as_chunks::<4>().0.iter().map(|&c| u32::from_le_bytes(c)).collect()
}

/// The registers each RUN_IDVS of a stream reads, in order.
fn regs_at_idvs(words: &[u64]) -> Vec<BTreeMap<u32, u32>> {
    let mut r = BTreeMap::new();
    let mut out = Vec::new();
    for &w in words {
        match Instr::decode(w) {
            Ok(Instr::Move32(m)) => {
                r.insert(m.destination, m.immediate);
            }
            Ok(Instr::Move48(m)) => {
                r.insert(m.destination, m.immediate as u32);
                r.insert(m.destination + 1, (m.immediate >> 32) as u32);
            }
            Ok(Instr::RunIdvs(_)) => out.push(r.clone()),
            _ => {}
        }
    }
    out
}

/// A render target's fields (its writeback format read as the integer
/// member of the union).
fn rt_of(bytes: &[u8]) -> RgbRenderTarget {
    RgbRenderTarget::unpack_as(bytes.try_into().unwrap(), &["Writeback Format"]).unwrap()
}

fn reg64(r: &BTreeMap<u32, u32>, n: u32) -> u64 {
    u64::from(r[&n]) | u64::from(r[&(n + 1)]) << 32
}

fn meta(stage: ShaderStage) -> ShaderMeta<'static> {
    ShaderMeta {
        stage,
        register_allocation: ShaderRegisterAllocation::_32_PER_THREAD,
        preload: 0,
        sysvals: &[],
        push: &[],
        varyings: 0,
        varying_bytes: 0,
        secondary: None,
        no_psiz_offset: None,
        colour_outputs: u32::from(stage == ShaderStage::FRAGMENT),
        samples_texture: false,
        requires_helper_threads: false,
        can_discard: false,
        writes_depth: false,
        writes_stencil: false,
        writes_coverage: false,
        writes_global: false,
        ubo: &[],
    }
}

struct Built {
    desc: Vec<u8>,
    words: Vec<u64>,
    frame: Frame,
}

/// A 64x64 frame with the D24S8 at the trace's address, cleared to depth
/// 1.0 and stencil 0x5a, and one depth-tested draw (LESS, writes on) whose
/// colour mask is `colour_mask`: with no colour surface (`no_colour`,
/// [`begin_frame_depth_only`]) or into an RGBA8 target.
fn build(no_colour: bool, colour_mask: u8) -> Built {
    let (mut desc_m, mut code_m, mut spd_m, mut ctx_m) =
        (vec![0u8; 0x4000], vec![0u8; 0x800], vec![0u8; 0x100], vec![0u8; 0x200]);
    let mut words = vec![0u64; 512];
    let frame;
    {
        let mut ctx_a = Arena::new("context", HEAP_DESC, &mut ctx_m).unwrap();
        let heap = tiler_heap_descriptor(&mut ctx_a, 0x8000_00a0_0000, 2 << 20).unwrap();
        let ctx = Context {
            heap: heap.gpu,
            geometry_buffer: GEOMETRY,
            geometry_buffer_size: GEOMETRY_BUFFER_BYTES,
            sample_positions: SAMPLES,
            tiler_max_levels: 8,
            tiler_bin_budget: (2 << 20) - 64,
            tile_budget: TileBudget::G610,
        };
        let mut desc = Arena::new("desc", DESC, &mut desc_m).unwrap();
        let mut code = Arena::new("code", 0x7fff_ffff_c000, &mut code_m).unwrap();
        let mut spds = Arena::new("spds", 0x7fff_ffff_d000, &mut spd_m).unwrap();
        let bin = [0u8; 128];
        let vs = upload_program(&mut code, &mut spds, &bin, meta(ShaderStage::VERTEX)).unwrap();
        let fs = upload_program(&mut code, &mut spds, &bin, meta(ShaderStage::FRAGMENT)).unwrap();
        let attributes = [VertexAttribute {
            buffer: VertexBuffer { gpu: 0x7fff_ffec_c000, size: 72 },
            offset: 0,
            stride: 12,
            components: 3,
            instance_divisor: 0,
        }];
        let draw = DrawCall {
            vs: &vs,
            fs: &fs,
            primitive: Topology::Triangles,
            first_vertex: 0,
            vertex_count: 6,
            instance_count: 1,
            indices: None,
            attributes: &attributes,
            viewport: Viewport { x: 0, y: 0, width: 64, height: 64, near: 0.0, far: 1.0 },
            scissor: None,
            line_width: 1.0,
            flip_y: false,
            blend_color: [0.0; 4],
            blend: None,
            polygon_offset: None,
            sample_mask: 0xf,
            vs_uniforms: &[],
            fs_uniforms: &[],
            zs: DepthStencilState { depth_func: Func::LESS, depth_write: true, ..DepthStencilState::OFF },
            colour_mask,
            front_cw: false,
            cull: Cull::None,
            textures: &[],
            vs_texture: None,
            occlusion_query: None,
            alpha_to_coverage: false,
            blend_shader: None,
        };
        /* with no colour surface only the size and the samples are read */
        let gpu = if no_colour { 0 } else { 0x7fff_fffe_4000 };
        let target = Target { gpu, width: 64, height: 64, row_stride: 256, format: TargetFormat::Rgba8888, samples: 1 };
        let attachment = ZsAttachment { gpu: ZS_BUF, row_stride: 256, clear_depth: 1.0, clear_stencil: 0x5a };
        let mut cs = Stream::new(&mut words, 92);
        let mut open = if no_colour {
            begin_frame_depth_only(&mut desc, &mut cs).unwrap()
        } else {
            begin_frame(&mut desc, &mut cs).unwrap()
        };
        assert_eq!(open.no_colour, no_colour);
        add_draw(&mut open, &mut desc, &mut cs, &ctx, &target, true, &draw).unwrap();
        assert!(open.uses_zs);
        frame = end_frame(&open, &mut desc, &mut cs, &ctx, &target, [0.0, 0.0, 1.0, 1.0], Some(attachment)).unwrap();
        let n = cs.words().len();
        words.truncate(n);
    }
    Built { desc: desc_m, words, frame }
}

impl Built {
    fn at(&self, gpu: u64, len: usize) -> Vec<u8> {
        self.desc[(gpu - DESC) as usize..(gpu - DESC) as usize + len].to_vec()
    }
}

/// The frame of no colour surface: the parameters block the board's (one
/// RGBA8 target's tile size and allocation, one render target, the ZS/CRC
/// extension, depth written back) but for the tiler pointer; the ZS/CRC
/// extension the colour frame's; the render target Mesa's dummy, word for
/// word; the FBD pointer tagged with the extension and one render target.
#[test]
fn a_depth_only_frame_is_mesa_s_frame_of_no_colour_buffer() {
    let b = build(true, 0);
    let m = trace::memory_of(ZS, "nocolour");
    /* the parameters block: equal to the board's frame of one RGBA8 target
     * but at word 14-15, the tiler context (Mesa's comes after its OOM
     * context and incremental-rendering FBDs, frame_zs_tests.rs) */
    let ours = b.at(DESC, 128);
    let mut dump = trace::bytes(&m, DESC, 128);
    assert_eq!(dump[0x38..0x40], TRACE_TILER.to_le_bytes(), "the dump's tiler pointer");
    let tiler = b.frame.layout.tiler.expect("a tiler context").gpu;
    dump[0x38..0x40].copy_from_slice(&tiler.to_le_bytes());
    assert_eq!(ours, dump, "the framebuffer parameters");
    let p = FramebufferParameters::unpack(&ours[..64].try_into().unwrap()).unwrap();
    assert_eq!((p.render_target_count, p.effective_tile_size, p.color_buffer_allocation), (1, 1024, 4096));
    assert!(p.has_zs_crc_extension && p.z_write_enable);
    assert_eq!((p.z_clear, p.s_clear), (1.0, 0x5a));
    /* the ZS/CRC extension: the linear D24S8 at the attachment */
    let ext = ZsTarget::unpack(&b.at(DESC + 128, 64).try_into().unwrap()).unwrap();
    assert_eq!((ext.base, ext.row_stride, ext.write_format), (ZS_BUF, 256, ZsFormat::D24S8));
    assert!(ext.clean_tile_write_enable, "the tiles start from the clear values");
    /* the render target: Mesa's dummy */
    let rt = b.at(DESC + 192, 64);
    assert_eq!(words_of(&rt), DUMMY_RT, "the dummy render target's words");
    let u = rt_of(&rt);
    assert!(!u.write_enable && !u.clean_tile_write_enable && u.dithering_enable);
    assert_eq!(
        (u.internal_format, u.writeback_block_format),
        (ColorBufferInternalFormat::R8G8B8A8, BlockFormat::TILED_U_INTERLEAVED)
    );
    assert_eq!((u.writeback_buffer.base, u.clear.color_0), (0, 0));
    /* the FBD pointer: the extension (bit 0) and one render target */
    assert_eq!(b.frame.fbd_pointer, DESC | 3);
}

/// Its draw: the board's depth-only draw (glColorMask(none) into an RGBA8
/// FBO, "nocolour"), whose branches are the same: no fragment shader
/// (d4, d12, d20 0), the ZSD (LESS, writes on), the tiler flags and both
/// DCD words (force early, forward pixel kill, render target mask 0) byte
/// for byte, the blend off with the one named difference of
/// frame_zs_tests.rs (Mesa's equation word is all zero, unnamed in
/// v10.xml; ours "replace" with no channel, not read with mode Off). A
/// colour mask on the draw changes none of it: there is no colour to write.
#[test]
fn a_depth_only_draw_is_the_board_s_colourless_draw_whatever_its_mask() {
    let m = trace::memory_of(ZS, "nocolour");
    let mesa = regs_at_idvs(&trace::cs_words(ZS, "nocolour").iter().map(|w| w.word).collect::<Vec<_>>());
    for mask in [0, 0xf] {
        let b = build(true, mask);
        let ours = regs_at_idvs(&b.words);
        assert_eq!(ours.len(), 1);
        let (mr, or) = (&mesa[0], &ours[0]);
        for reg in [4, 12, 20] {
            assert_eq!((reg64(mr, reg), reg64(or, reg)), (0, 0), "mask {mask:#x} d{reg}: no fragment shader");
        }
        for reg in [56, 57, 58] {
            assert_eq!(or[&reg], mr[&reg], "mask {mask:#x} r{reg}");
        }
        let dcd1 = DcdFlags1::unpack(&or[&58].to_le_bytes()).unwrap();
        assert_eq!((dcd1.render_target_mask, dcd1.sample_mask), (0, 0xffff));
        assert_eq!(
            b.at(reg64(or, 52), DepthStencil::LENGTH),
            trace::bytes(&m, reg64(mr, 52), DepthStencil::LENGTH),
            "the ZSD"
        );
        let (dump, ours) =
            (trace::bytes(&m, reg64(mr, 50) & !63, Blend::LENGTH), b.at(reg64(or, 50) & !63, Blend::LENGTH));
        assert_eq!(reg64(or, 50) & 63, 1, "one blend descriptor, MAX2(nr_cbufs, 1)");
        assert_eq!(dump[4..8], [0; 4], "the dump's equation word is zero");
        assert_eq!(
            (ours[..4].to_vec(), ours[8..].to_vec()),
            (dump[..4].to_vec(), dump[8..].to_vec()),
            "mask {mask:#x}"
        );
        /* Enable (word 0 bit 9) off, the internal word's mode (bits 0-1) Off */
        assert_eq!((ours[1] & 2, ours[8] & 3), (0, BlendMode::OFF.0 as u8));
    }
}

/// A frame with a colour target is what it was: its render target the
/// target (Write Enable, the target's address, cleared blue), and its draw
/// with the colour mask on runs the fragment shader into target 0.
#[test]
fn a_frame_with_a_colour_target_is_unchanged() {
    let b = build(false, 0xf);
    let rt = rt_of(&b.at(DESC + 192, 64));
    assert!(rt.write_enable && rt.clean_tile_write_enable);
    assert_eq!((rt.writeback_buffer.base, rt.writeback_block_format), (0x7fff_fffe_4000, BlockFormat::LINEAR));
    assert_ne!(words_of(&b.at(DESC + 192, 64)), DUMMY_RT);
    let r = &regs_at_idvs(&b.words)[0];
    assert_ne!(reg64(r, 20), 0, "the fragment shader runs");
    assert_eq!(DcdFlags1::unpack(&r[&58].to_le_bytes()).unwrap().render_target_mask, 1);
}

/// What a frame of no colour surface cannot have, refused as the caller's
/// defect: a colour preload, a sample store, more colour targets.
#[test]
fn a_depth_only_frame_with_colour_state_is_refused() {
    let mut desc_m = vec![0u8; 0x1000];
    let mut words = vec![0u64; 64];
    let mut desc = Arena::new("desc", DESC, &mut desc_m).unwrap();
    let mut cs = Stream::new(&mut words, 92);
    let open = begin_frame_depth_only(&mut desc, &mut cs).unwrap();
    let ctx = Context {
        heap: HEAP_DESC,
        geometry_buffer: GEOMETRY,
        geometry_buffer_size: GEOMETRY_BUFFER_BYTES,
        sample_positions: SAMPLES,
        tiler_max_levels: 8,
        tiler_bin_budget: (2 << 20) - 64,
        tile_budget: TileBudget::G610,
    };
    let t = Target { gpu: 0, width: 64, height: 64, row_stride: 0, format: TargetFormat::Rgba8888, samples: 1 };
    let zs = Some(ZsAttachment { gpu: ZS_BUF, row_stride: 256, clear_depth: 1.0, clear_stencil: 0 });
    let shader = FrameShader { binary: 0x7fff_ffff_c000, preload: 0 };
    let store = SampleStore { gpu: 0x7fff_fff0_0000, surface_stride: 0x4000, reload: false, resolve: false };
    let more = Target { gpu: 0x7fff_fff8_0000, ..t };
    let cases: [(&str, OpenFrame); 3] = [
        ("a colour preload", OpenFrame { colour_preload: Some(shader), ..open }),
        ("a sample store", OpenFrame { sample_store: Some(store), ..open }),
        ("more targets", {
            let mut o = open;
            o.more_targets[0] = Some(more);
            o
        }),
    ];
    for (what, o) in cases {
        let e = end_frame(&o, &mut desc, &mut cs, &ctx, &t, [0.0; 4], zs).map(|_| ());
        assert_eq!(e, Err(FrameError::Invalid(INVALID_DEPTH_ONLY)), "{what}");
    }
    /* the frame as begun ends: a depth/stencil frame of no draw */
    end_frame(&open, &mut desc, &mut cs, &ctx, &t, [0.0; 4], zs).unwrap();
}

/* ---- the backend: the GL core's depth-only draws ------------------------ */

/// The depth texture's GL object (bit 30: a texture) and its attachment
/// pair, as the GL core names a depth-only FBO's.
const DEPTH_TEX: u32 = 0x4000_0000 | 9;
const PAIR: u32 = 0x77;

/// A draw as the GL core hands one into a framebuffer whose only attachment
/// is a 64x64 depth texture (glmark2's shadow pass, src/gl/draw.rs
/// `a_depth_texture_alone_is_a_target_where_the_backend_draws_without_colour`):
/// no colour target (`target_gpu` 0) at the depth texture's size, no
/// colour clear, colour mask 0; the depth test LESS with writes.
fn depth_only_draw() -> GlesDraw {
    let mut d = triangle(0, &VERTICES);
    (d.do_clear, d.color_mask) = (false, 0);
    (d.ds_id, d.depth_id, d.stencil_id, d.ds_w, d.ds_h) = (PAIR, DEPTH_TEX, 0, 64, 64);
    (d.depth_test, d.depth_write, d.depth_func) = (true, true, backend::GLES_CMP_LESS);
    d
}

fn last_frame() -> Vec<u64> {
    with(|m| m.submits.last().unwrap().1.clone())
}

/// glClear(GL_DEPTH_BUFFER_BIT) and a draw into the depth texture alone:
/// the clear is the depth plane's value (no kick), the draw opens one frame
/// of no colour surface, submitted at the flush: Mesa's dummy render
/// target, the ZS/CRC extension naming a D24S8 buffer of the backend's at
/// the depth texture's size with the cleared depth, the depth written
/// back; the draw runs no fragment shader and names no render target. A
/// second draw with a colour mask on goes into the same frame and changes
/// none of that.
#[test]
fn a_depth_only_clear_and_draw_are_one_frame_of_no_colour() {
    let _rig = Rig::new(true);
    assert_eq!(mali_init(), 0);
    with_programs(TRI_VS, TRI_FS, |vs, fs| {
        let mut d = depth_only_draw();
        (d.vs, d.fs) = (vs, fs);
        let mut clear = d;
        (clear.index_count, clear.do_clear_depth, clear.clear_depth) = (0, true, 0.75);
        let kicks = with(|m| m.submits.len());
        assert_eq!(draw_flush(&clear), 0);
        assert_eq!(with(|m| m.submits.len()), kicks, "a whole depth clear with no frame is the plane's value");
        let mut masked = d;
        (masked.color_mask, masked.color_write) = (0xf, true);
        // SAFETY: valid draws whose records keep holds.
        assert_eq!(unsafe { mali_draw(&d) }, 0);
        // SAFETY: as above.
        assert_eq!(unsafe { mali_draw(&masked) }, 0);
        assert_eq!(with(|m| m.submits.len()), kicks, "both draws in the open frame");
        assert_eq!(mali_flush(), 0);
        assert_eq!(with(|m| m.submits.len()), kicks + 1, "one frame");
        let words = last_frame();
        let fbd = fbd_at_fragment(&words).expect("a fragment job");
        let params = FramebufferParameters::unpack(&with(|m| m.read(fbd, 64)).try_into().unwrap()).unwrap();
        assert_eq!((params.width, params.height, params.render_target_count), (64, 64, 1));
        assert!(params.has_zs_crc_extension && params.z_write_enable);
        assert_eq!(params.z_clear, 0.75, "the parked clear is the frame's start");
        assert_eq!(params.pre_frame_0, PrePostFrameShaderMode::NEVER, "no colour preload");
        let ext = ZsTarget::unpack(&with(|m| m.read(fbd + 128, 64)).try_into().unwrap()).unwrap();
        assert_eq!(ext.row_stride, 4 * 64);
        assert!(with(|m| m.find(ext.base, 64 * 64 * 4).is_some()), "the D24S8 is an OSAL buffer");
        assert_eq!(words_of(&with(|m| m.read(fbd + 192, 64))), DUMMY_RT, "Mesa's dummy render target");
        let regs = regs_at_each_idvs(&words);
        assert_eq!(regs.len(), 2);
        for r in &regs {
            assert_eq!(r.get(&20).copied().unwrap_or(0), 0, "no fragment shader");
            assert_eq!(DcdFlags1::unpack(&r[&58].to_le_bytes()).unwrap().render_target_mask, 0);
        }
    });
}

/// The depth pass, then the pass that samples its texture: after the
/// frame of no colour surface completed, `mali_depth_resolve` (which the
/// GL core calls before a draw samples the depth texture) reads the depth
/// the frame wrote back into its D24S8 buffer, `unorm24_depth` of each
/// word, row for row; -EBUSY while the frame is still open. A colour pass
/// into another target then opens a frame of its own, with its render
/// target and no ZS/CRC extension, leaving the depth texture's plane as it
/// was.
#[test]
fn a_depth_only_pass_s_texture_resolves_for_the_pass_that_samples_it() {
    let _rig = Rig::new(true);
    assert_eq!(mali_init(), 0);
    let t = target();
    with_programs(TRI_VS, TRI_FS, |vs, fs| {
        let mut d = depth_only_draw();
        (d.vs, d.fs) = (vs, fs);
        d.do_clear_depth = true;
        d.clear_depth = 1.0;
        let (w, h) = (64u32, 64u32);
        let mut texels = vec![-1.0f32; (w * h) as usize];
        // SAFETY: `texels` holds `h` rows of `w` floats.
        let resolve = |texels: &mut [f32]| unsafe { mali_depth_resolve(DEPTH_TEX, texels.as_mut_ptr(), w, h, w) };
        assert_eq!(resolve(&mut texels), 1, "nothing drawn yet");
        // SAFETY: a valid draw.
        assert_eq!(unsafe { mali_draw(&d) }, 0);
        assert_eq!(resolve(&mut texels), -crate::util::EBUSY, "the open frame draws into it");
        assert_eq!(mali_flush(), 0);
        let fbd = fbd_at_fragment(&last_frame()).expect("a fragment job");
        let ext = ZsTarget::unpack(&with(|m| m.read(fbd + 128, 64)).try_into().unwrap()).unwrap();
        /* what the GPU wrote back: a depth ramp in the low 24 bits */
        let word = |x: u32, y: u32| (x * 0x1000 + y * 0x40_000) & 0xff_ffff;
        let mut buf = Vec::with_capacity((4 * w * h) as usize);
        for y in 0..h {
            for x in 0..w {
                buf.extend_from_slice(&word(x, y).to_le_bytes());
            }
        }
        with(|m| m.write(ext.base, &buf));
        assert_eq!(resolve(&mut texels), 0);
        for y in 0..h {
            for x in 0..w {
                assert_eq!(texels[(y * w + x) as usize], ds_retention::unorm24_depth(word(x, y)), "({x}, {y})");
            }
        }
        /* the colour pass into another target: its own frame, the target
         * its render target, no depth attachment */
        let mut colour = triangle(t, &VERTICES);
        (colour.vs, colour.fs) = (vs, fs);
        assert_eq!(draw_flush(&colour), 0);
        let words = last_frame();
        let fbd = fbd_at_fragment(&words).expect("a fragment job");
        let params = FramebufferParameters::unpack(&with(|m| m.read(fbd, 64)).try_into().unwrap()).unwrap();
        assert!(!params.has_zs_crc_extension);
        let rt = rt_of(&with(|m| m.read(fbd + 128, 64)));
        assert!(rt.write_enable);
        assert_eq!(rt.writeback_buffer.base, t);
        assert_ne!(reg_at_idvs(&words, 20).unwrap_or(0), 0, "the colour pass runs its fragment shader");
        assert_eq!(resolve(&mut texels), 0, "the depth texture's plane is still held");
        assert_eq!(texels[1], ds_retention::unorm24_depth(word(1, 0)));
    });
}

/// A draw with neither a colour target nor a depth or stencil object is
/// still the GL core's defect (`-EINVAL`), and one with a colour target and
/// a depth attachment is drawn into its target as before.
#[test]
fn a_draw_with_no_attachment_at_all_is_still_refused() {
    let _rig = Rig::new(true);
    assert_eq!(mali_init(), 0);
    let t = target();
    with_programs(TRI_VS, TRI_FS, |vs, fs| {
        let mut none = depth_only_draw();
        (none.vs, none.fs) = (vs, fs);
        (none.ds_id, none.depth_id) = (0, 0);
        // SAFETY: a valid draw.
        assert_eq!(unsafe { mali_draw(&none) }, -crate::util::EINVAL);
        let mut both = depth_only_draw();
        (both.vs, both.fs) = (vs, fs);
        (both.target_gpu, both.do_clear, both.color_mask) = (t, true, 0xf);
        both.do_clear_depth = true;
        assert_eq!(draw_flush(&both), 0);
        let fbd = fbd_at_fragment(&last_frame()).expect("a fragment job");
        let rt = rt_of(&with(|m| m.read(fbd + 192, 64)));
        assert!(rt.write_enable);
        assert_eq!(rt.writeback_buffer.base, t, "the render target after the extension is the target");
    });
}
