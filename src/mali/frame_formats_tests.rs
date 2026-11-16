// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The frame builder's colour target formats against the board: the
//! frames Mesa 25.0.7 drew on the NanoPC-T6 into a 64x48 pbuffer of an
//! XRGB8888 and of an RGB565 config (tools/mali/board-dump/
//! formats-2026-10-19.txt, cut from 0dce654a8:tools/mali/board-ref/
//! 2026-10-19-formats/mesa), built again with the target at the trace's
//! address and row stride.
//!
//! What is compared: the render target descriptor (internal and writeback
//! format, swizzle, dithering, the row stride, the clear colour in the
//! tile buffer's layout) and the draw's blend descriptor (its conversion
//! format), byte for byte. RGBA8888 is the L1 frame's, which
//! `frame_tests.rs` compares (Mesa's own RGBA pbuffer is B, G, R, A, not
//! this layout).
//!
//! The f16rt section: gl_test case 54's `GL_RGBA16F` cube faces (128x128
//! and 4x4) and the same faces as `GL_RGBA`, cleared blue and drawn, as the
//! board's Mesa 25.0.7 drew them (tools/mali/board-dump/
//! f16rt-2026-10-24.txt, cut from 0dce654a8:tools/mali/board-ref/2026-10-24-mesa-refs5/
//! mesa/mesa-f16rt.pandecode; docs/notes/mali-frame-formats.md sections 4
//! and 6). Compared as above, and the Framebuffer descriptor's size, tile
//! and colour buffer allocation words.

use super::arena::Arena;
use super::cs::{Instr, Stream};
use super::frame::*;
use super::genxml::v10::*;
use super::trace;
use std::collections::BTreeMap;

const FMT: &str = include_str!("../../tools/mali/board-dump/formats-2026-10-19.txt");
const F16RT: &str = include_str!("../../tools/mali/board-dump/f16rt-2026-10-24.txt");
const DESC: u64 = 0x7fff_fffd_0000;

/// The registers the one RUN_IDVS of a stream reads, and its flags
/// override.
fn regs_at_idvs(words: &[u64]) -> (BTreeMap<u32, u32>, u32) {
    let mut r = BTreeMap::new();
    for &w in words {
        match Instr::decode(w) {
            Ok(Instr::Move32(m)) => {
                r.insert(m.destination, m.immediate);
            }
            Ok(Instr::Move48(m)) => {
                r.insert(m.destination, m.immediate as u32);
                r.insert(m.destination + 1, (m.immediate >> 32) as u32);
            }
            Ok(Instr::RunIdvs(i)) => return (r, i.flags_override),
            _ => {}
        }
    }
    panic!("no RUN_IDVS in the stream")
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

/// fmt.c's frame: cleared blue, the triangle, into `target` (the viewport
/// the whole target); the descriptor arena's bytes, the RUN_IDVS registers
/// and the frame.
fn build(target: Target) -> (Vec<u8>, BTreeMap<u32, u32>, Frame) {
    build_with(target, [0.0, 0.0, 1.0, 1.0], None, DepthStencilState::OFF, 0xf)
}

/// [`build`]'s frame with the clear colour `clear`, the depth/stencil
/// attachment `zs`, the draw's depth/stencil state and colour mask.
fn build_with(
    target: Target,
    clear: [f32; 4],
    zs: Option<ZsAttachment>,
    zs_state: DepthStencilState,
    colour_mask: u8,
) -> (Vec<u8>, BTreeMap<u32, u32>, Frame) {
    let (mut desc_m, mut code_m, mut spd_m, mut ctx_m) =
        (vec![0u8; 0x4000], vec![0u8; 0x800], vec![0u8; 0x100], vec![0u8; 0x200]);
    let mut words = vec![0u64; 512];
    let (frame, n);
    {
        let mut ctx_a = Arena::new("context", 0x7fff_ffff_b000, &mut ctx_m).unwrap();
        let heap = tiler_heap_descriptor(&mut ctx_a, 0x8000_00a0_0000, 2 << 20).unwrap();
        let ctx = Context {
            heap: heap.gpu,
            geometry_buffer: 0x7fff_fffe_b000,
            geometry_buffer_size: GEOMETRY_BUFFER_BYTES,
            sample_positions: 0x7fff_ffff_f000,
            tiler_max_levels: 8,
        };
        let mut desc = Arena::new("desc", DESC, &mut desc_m).unwrap();
        let mut code = Arena::new("code", 0x7fff_ffff_c000, &mut code_m).unwrap();
        let mut spds = Arena::new("spds", 0x7fff_ffff_d000, &mut spd_m).unwrap();
        let bin = [0u8; 128];
        let vs = upload_program(&mut code, &mut spds, &bin, meta(ShaderStage::VERTEX)).unwrap();
        let fs = upload_program(&mut code, &mut spds, &bin, meta(ShaderStage::FRAGMENT)).unwrap();
        let attributes = [VertexAttribute {
            buffer: VertexBuffer { gpu: 0x7fff_ffec_c000, size: 24 },
            offset: 0,
            stride: 8,
            components: 2,
            instance_divisor: 0,
        }];
        let input = FrameInput {
            zs,
            target,
            clear_rgba: Some(clear),
            draw: Some(DrawCall {
                zs: zs_state,
                colour_mask,
                front_cw: false,
                cull: Cull::None,
                vs: &vs,
                fs: &fs,
                primitive: Topology::Triangles,
                first_vertex: 0,
                vertex_count: 3,
                instance_count: 1,
                indices: None,
                attributes: &attributes,
                viewport: Viewport {
                    x: 0,
                    y: 0,
                    width: target.width as i32,
                    height: target.height as i32,
                    near: 0.0,
                    far: 1.0,
                },
                scissor: None,
                line_width: 1.0,
                flip_y: true,
                blend_color: [0.0; 4],
                blend: None,
                polygon_offset: None,
                sample_mask: 0xf,
                vs_uniforms: &[],
                fs_uniforms: &[],
                textures: &[],
                vs_texture: None,
                occlusion_query: None,
                alpha_to_coverage: false,
                blend_shader: None,
            }),
        };
        let mut cs = Stream::new(&mut words, 92);
        frame = build_frame(&mut desc, &mut cs, &ctx, &input).expect("fmt.c's frame builds");
        n = cs.words().len();
    }
    let (regs, _) = regs_at_idvs(&words[..n]);
    (desc_m, regs, frame)
}

/// Each format's render target and blend descriptor are the board's bytes.
#[test]
fn an_xrgb8888_and_an_rgb565_target_are_the_board_s() {
    for (source, format, row_stride) in
        [("mesa-xrgb8888-mid", TargetFormat::Xrgb8888, 256), ("mesa-rgb565-mid", TargetFormat::Rgb565, 128)]
    {
        let m = trace::memory_of(FMT, source);
        let words: Vec<u64> = trace::cs_words(FMT, source).iter().map(|w| w.word).collect();
        let (theirs, _) = regs_at_idvs(&words);
        let their_blend = (u64::from(theirs[&50]) | u64::from(theirs[&51]) << 32) & !63;
        let their_desc = their_blend & !0xfff;
        let their_rt = trace::bytes(&m, their_desc + 0x80, 64);
        /* the trace's target: its writeback base (the RT's bytes 32-39) */
        let base = u64::from_le_bytes(their_rt[32..40].try_into().unwrap());
        assert_eq!(
            u32::from_le_bytes(their_rt[40..44].try_into().unwrap()),
            row_stride,
            "{source}: the dump's row stride"
        );
        let target = Target { gpu: base, width: 64, height: 48, row_stride, format, samples: 1 };
        let (desc, regs, frame) = build(target);
        /* ours: the render target after the FBD (no ZS/CRC extension, the
         * FBD pointer's tag), the blend descriptor that d50 names */
        let fbd = ((frame.fbd_pointer & !63) - DESC) as usize;
        assert_eq!(desc[fbd + 0x80..fbd + 0xc0], their_rt[..], "{source}: the render target");
        let blend = ((u64::from(regs[&50]) | u64::from(regs[&51]) << 32) & !63) - DESC;
        assert_eq!(
            desc[blend as usize..blend as usize + Blend::LENGTH],
            trace::bytes(&m, their_blend, Blend::LENGTH)[..],
            "{source}: the blend descriptor"
        );
    }
    /* the dump holds what the test reads: RGB565's blue clear in the tile
     * layout (B 5.5 bits from bit 20: 0x3E000000) and its blend conversion
     * R5G6B5_AU with the RGB1 order (0xe2 << 12 | 0x10, bytes 12-15) */
    let m = trace::memory_of(FMT, "mesa-rgb565-mid");
    let at = |va: u64, n| trace::bytes(&m, va, n);
    assert_eq!(at(0x7fff_fffd_60b0, 4), 0x3e00_0000u32.to_le_bytes());
    assert_eq!(at(0x7fff_fffd_614c, 4), (0xe2u32 << 12 | 0x10).to_le_bytes());
}

/// The f16rt frames: each face's render target, blend descriptor and
/// Framebuffer words are the board's bytes. The 16F faces: `Internal
/// Format` and `Writeback Format` RAW64, swizzle RGBA, the blue clear as
/// halves, blend `RGBA16F RGBA`, tile 1024 and allocation 8192; the RGBA8
/// faces of the same session as the control (allocation 4096).
#[test]
fn an_rgba16f_target_is_the_board_s() {
    for (source, format, size) in [
        ("f16rt1", TargetFormat::Rgba16f, 128),
        ("f16rt2", TargetFormat::Rgba16f, 4),
        ("f16rt3", TargetFormat::Rgba8888, 128),
        ("f16rt4", TargetFormat::Rgba8888, 4),
    ] {
        let m = trace::memory_of(F16RT, source);
        let words: Vec<u64> = trace::cs_words(F16RT, source).iter().map(|w| w.word).collect();
        let (theirs, _) = regs_at_idvs(&words);
        let their_blend = (u64::from(theirs[&50]) | u64::from(theirs[&51]) << 32) & !63;
        let their_desc = their_blend & !0xfff;
        let their_rt = trace::bytes(&m, their_desc + 0x80, 64);
        let base = u64::from_le_bytes(their_rt[32..40].try_into().unwrap());
        let row_stride = u32::from_le_bytes(their_rt[40..44].try_into().unwrap());
        let target = Target { gpu: base, width: size, height: size, row_stride, format, samples: 1 };
        let (desc, regs, frame) = build(target);
        let fbd = ((frame.fbd_pointer & !63) - DESC) as usize;
        assert_eq!(desc[fbd + 0x80..fbd + 0xc0], their_rt[..], "{source}: the render target");
        let blend = ((u64::from(regs[&50]) | u64::from(regs[&51]) << 32) & !63) - DESC;
        assert_eq!(
            desc[blend as usize..blend as usize + Blend::LENGTH],
            trace::bytes(&m, their_blend, Blend::LENGTH)[..],
            "{source}: the blend descriptor"
        );
        /* the Framebuffer parameters from the size to the Z clear: width,
         * height, the bound, the sample count and pattern, the effective
         * tile size, the colour buffer allocation, the ZS words */
        assert_eq!(desc[fbd + 0x20..fbd + 0x38], trace::bytes(&m, their_desc + 0x20, 0x18)[..], "{source}: the FBD");
    }
    /* the dump holds what the test reads: the 16F face's RAW64 pair (bits
     * 26..31 of word 0, 35..39 of the RT), the blue halves, the blend's
     * RGBA16F RGBA (0xdf << 12) and the 8192-byte allocation (0x08 in bits
     * 24..31 of the tile word) */
    let m = trace::memory_of(F16RT, "f16rt1");
    let at = |va: u64, n| trace::bytes(&m, va, n);
    let rt = at(0x7fff_fff1_0080, 8);
    assert_eq!(u32::from_le_bytes(rt[0..4].try_into().unwrap()) >> 26, 0x23);
    assert_eq!(rt[4] >> 3 & 0x1f, 5);
    assert_eq!(at(0x7fff_fff1_00b0, 16), [0, 0, 0, 0, 0, 0x3c, 0, 0x3c, 0, 0, 0, 0, 0, 0x3c, 0, 0x3c]);
    assert_eq!(at(0x7fff_fff1_014c, 4), (0xdfu32 << 12).to_le_bytes());
    assert_eq!(at(0x7fff_fff1_002c, 4), 0x0800_1480u32.to_le_bytes());
}

/// zs-refs' Mesa 26.2.1 trace of dEQP-GLES2
/// fbo.render.stencil_clear.rbo_rgb565_stencil_index8
/// (tools/mali/pan_zs_ref.sh). Its sibling
/// `rbo_rgb565_depth_component16_stencil_index8` has no frame there: Mesa's
/// context is ES 3, which calls separate depth and stencil renderbuffers
/// GL_FRAMEBUFFER_UNSUPPORTED (the trace's `Frames: 0`), where our
/// `report-es2` build draws it. Its draws are the same (no depth test), so
/// they take this trace's descriptors.
const STENCIL_CLEAR_565: &str =
    include_str!("../../tools/mali/testdata/pan_zs_ref/stencil_clear__rbo_rgb565_stencil_index8.txt");
const STENCIL_CLEAR_565_D16: &str =
    include_str!("../../tools/mali/testdata/pan_zs_ref/stencil_clear__rbo_rgb565_depth_component16_stencil_index8.txt");

/// pandecode's field `key` in block `block` of a zs-refs trace (`## <block>
/// lines a-b` headed).
fn ref_field<'a>(trace: &'a str, block: &str, key: &str) -> &'a str {
    let mut inside = false;
    for line in trace.lines() {
        if let Some(h) = line.strip_prefix("## ") {
            inside = h.starts_with(&format!("{block} lines "));
            continue;
        }
        if let Some(v) = line.trim().strip_prefix(key).and_then(|r| r.strip_prefix(": ")).filter(|_| inside) {
            return v;
        }
    }
    panic!("{block}: no field {key}")
}

/// The Depth/stencil descriptor pandecode printed in `block`, as ours
/// unpacks (the fields Mesa's frames of these cases set).
fn ref_depth_stencil(trace: &str, block: &str) -> DepthStencil {
    let f = |k: &str| ref_field(trace, block, k);
    let func = |k: &str| match f(k) {
        "Never" => Func::NEVER,
        "Equal" => Func::EQUAL,
        "Always" => Func::ALWAYS,
        v => panic!("{block}: {k} {v}"),
    };
    let op = |k: &str| match f(k) {
        "Keep" => StencilOp::KEEP,
        "Replace" => StencilOp::REPLACE,
        v => panic!("{block}: {k} {v}"),
    };
    let hex = |k: &str| u32::from_str_radix(f(k).trim_start_matches("0x"), 16).unwrap();
    let flag = |k: &str| f(k) == "true";
    assert_eq!(
        (f("Depth clamp mode"), f("Depth source"), f("Depth units"), f("Depth factor"), f("Depth bias clamp")),
        ("[0, 1]", "Fixed function", "0.000000", "0.000000", "0.000000"),
        "{block}"
    );
    DepthStencil {
        front_compare_function: func("Front compare function"),
        front_stencil_fail: op("Front stencil fail"),
        front_depth_fail: op("Front depth fail"),
        front_depth_pass: op("Front depth pass"),
        back_compare_function: func("Back compare function"),
        back_stencil_fail: op("Back stencil fail"),
        back_depth_fail: op("Back depth fail"),
        back_depth_pass: op("Back depth pass"),
        stencil_from_shader: flag("Stencil from shader"),
        stencil_test_enable: flag("Stencil test enable"),
        front_write_mask: hex("Front write mask"),
        back_write_mask: hex("Back write mask"),
        front_value_mask: hex("Front value mask"),
        back_value_mask: hex("Back value mask"),
        front_reference_value: hex("Front reference value"),
        back_reference_value: hex("Back reference value"),
        shader_read_only_z_s: flag("Shader read only Z/S"),
        depth_cull_enable: flag("Depth cull enable"),
        depth_clamp_mode: DepthClampMode::_0_1,
        depth_source: DepthSource::FIXED_FUNCTION,
        depth_write_enable: flag("Depth write enable"),
        depth_bias_enable: flag("Depth bias enable"),
        depth_function: func("Depth function"),
        ..Default::default()
    }
}

/// fbo.render.stencil_clear.rbo_rgb565_* (plan #373 cause 8, inv-misc8):
/// the GL core keeps a `GL_RGB565` renderbuffer in RGBA8 storage
/// (`GLES_SURF_RGBA8888`, `target_alpha_bits` 0), and the case's textures
/// carry an alpha other than 1, which the draws wrote: dEQP's fuzzy compare
/// reads alpha 1 for a buffer without alpha (ES 2.0 4.3.1) and failed.
/// Mesa draws the case into an R5G6B5 target that stores no alpha
/// (`Internal Format: R5G6B5A0`, `Writeback Format: R5G6B5`); ours stores
/// RGBA8 and writes R, G and B only ([`TargetFormat::draw_mask`]), so the
/// clear's alpha 1 stays. An XRGB8888 or RGB565 layout keeps GL's mask.
///
/// The case's four draws (Mesa's two scissored stencil clears, ALWAYS and
/// REPLACE with references 1 and 2 and no colour, then the two textured
/// quads, EQUAL with references 1 and 2) pack the Depth/stencil
/// descriptor Mesa's frame holds, field for field, into the case's 128x128
/// target with a D24S8 plane.
#[test]
fn an_rgb565_renderbuffer_s_stencil_clear_is_mesa_s() {
    assert_eq!(TargetFormat::Rgba8888.draw_mask(0xf, 0), 0x7, "RGBA8 storage of a format without alpha");
    assert_eq!(TargetFormat::Rgba8888.draw_mask(0x9, 0), 0x1);
    for bits in [1, 4, 8] {
        assert_eq!(TargetFormat::Rgba8888.draw_mask(0xf, bits), 0xf, "{bits} alpha bits");
    }
    assert_eq!(TargetFormat::Rgba16f.draw_mask(0xf, 0), 0x7, "RGBA16F storage of GL_RGB16F");
    for f in [TargetFormat::Xrgb8888, TargetFormat::Rgb565] {
        assert_eq!(f.draw_mask(0xf, 0), 0xf, "{f:?}: no alpha stored, GL's mask as Mesa's");
    }

    let target = Target {
        gpu: 0x7fff_fffb_c000,
        width: 128,
        height: 128,
        row_stride: 128 * 4,
        format: TargetFormat::Rgba8888,
        samples: 1,
    };
    let zs = ZsAttachment { gpu: 0x7fff_fffa_c000, row_stride: 128 * 4, clear_depth: 0.0, clear_stencil: 0 };
    let face = |func, op, reference| StencilFace {
        func,
        fail: op,
        depth_fail: op,
        depth_pass: op,
        value_mask: 0xff,
        write_mask: 0xff,
        reference,
    };
    assert!(
        STENCIL_CLEAR_565_D16.contains("GL_FRAMEBUFFER_UNSUPPORTED") && STENCIL_CLEAR_565_D16.contains("# Frames: 0")
    );
    let (case, trace) = ("rbo_rgb565_stencil_index8", STENCIL_CLEAR_565);
    assert_eq!(ref_field(trace, "frame1-fbd", "Internal Format"), "R5G6B5A0", "{case}: Mesa stores no alpha");
    assert_eq!(ref_field(trace, "frame1-fbd", "Writeback Format"), "R5G6B5", "{case}");
    for (block, func, op, reference, mask) in [
        ("frame1-draw1", Func::ALWAYS, StencilOp::REPLACE, 1, 0),
        ("frame1-draw2", Func::ALWAYS, StencilOp::REPLACE, 2, 0),
        ("frame1-draw3", Func::EQUAL, StencilOp::KEEP, 1, 0xf),
        ("frame1-draw4", Func::EQUAL, StencilOp::KEEP, 2, 0xf),
    ] {
        let state = DepthStencilState {
            depth_func: Func::ALWAYS,
            depth_write: false,
            stencil_test: true,
            faces: [face(func, op, reference); 2],
        };
        let colour_mask = target.format.draw_mask(mask, 0);
        let (desc, _, frame) = build_with(target, [0.0, 0.0, 0.0, 1.0], Some(zs), state, colour_mask);
        let at = frame.layout.depth_stencil.expect("a depth/stencil descriptor").offset;
        let ours = DepthStencil::unpack(&desc[at..at + DepthStencil::LENGTH].try_into().unwrap()).unwrap();
        assert_eq!(ours, ref_depth_stencil(trace, block), "{case} {block}: the Depth/stencil descriptor");
        if mask != 0 {
            let at = frame.layout.blend.expect("a blend descriptor").offset;
            /* the Blend's `Equation`, bits 32..=63 */
            let equation = BlendEquation::unpack(&desc[at + 4..at + 8].try_into().unwrap()).unwrap();
            assert_eq!(equation.color_mask, 0x7, "{case} {block}: no alpha written");
        }
    }
}
