// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The frame builder's 4x target against the board: gl_test case 16's 4x
//! draw as Mesa 25.0.7 made it on the NanoPC-T6 into a 64x64 RGBA8 texture
//! attached with EXT_multisampled_render_to_texture (4x tile buffer
//! averaged into single-sampled memory, the backend contract's 4x), and the
//! same draw single-sampled (tools/mali/board-dump/msaa-2026-10-19.txt, cut
//! from 0dce654a8:tools/mali/board-ref/2026-10-19-msaa/mesa).
//!
//! What is compared, for both: the sample-position block the frame names,
//! the framebuffer parameters (sample count, pattern and locations, tile
//! size and colour buffer allocation), the render target (writeback MSAA),
//! the tiler context (sample pattern) and DCD0 (multisample enable,
//! aligned line ends). The pointers that only name where each stack put
//! its own blocks (the tiler context's, the polygon list) are set equal
//! before the comparison; the others are built at the trace's addresses.
//!
//! The 4x frame with depth/stencil: gl_test case 207 msaazs's depth frame
//! and stencil frame as Mesa made them with a D24S8 texture attached the
//! same way (tools/mali/board-dump/msaa-zs-2026-10-20.txt, cut from
//! 0dce654a8:tools/mali/board-ref/2026-10-20-msaa-zs/mesa): the framebuffer
//! descriptor, ZS/CRC extension, render target, tiler context and sample
//! positions, and each draw's depth/stencil and blend descriptors and DCD
//! registers, byte for byte, every difference named with its cause.
//!
//! Alpha-to-coverage: gl_test case 198 a2cconst's draw as Mesa made it on
//! its own 4x pbuffer (tools/mali/board-dump/a2c-2026-10-23.txt, cut from
//! 0dce654a8:tools/mali/board-ref/2026-10-23-a2c/mesa): DCD 0 and 1, the tiler flags,
//! the depth/stencil and blend descriptors.
//!
//! A 4x blend shader: gl_test case 211 msaablend's blended draw as Mesa made
//! it on the backend contract's 4x target
//! (tools/mali/board-dump/blendshader-2026-10-24.txt, cut from
//! 0dce654a8:tools/mali/board-ref/2026-10-24-mesa-refs4/mesa): its blend descriptor
//! and DCD0.

use super::arena::Arena;
use super::cs::{Instr, Stream};
use super::frame::*;
use super::genxml::v10::*;
use super::trace;
use std::collections::BTreeMap;

const MSAA: &str = include_str!("../../tools/mali/board-dump/msaa-2026-10-19.txt");
const MSAA_ZS: &str = include_str!("../../tools/mali/board-dump/msaa-zs-2026-10-20.txt");
const A2C: &str = include_str!("../../tools/mali/board-dump/a2c-2026-10-23.txt");
const DESC: u64 = 0x7fff_fffd_0000;
/// The trace's sample-position buffer, heap descriptor and geometry buffer.
const SAMPLES: u64 = 0x7fff_ffff_f000;
const HEAP: u64 = 0x7fff_ffff_b000;
const GEOMETRY: u64 = 0x7fff_fffe_b000;

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
        requires_helper_threads: false,
        samples_texture: false,
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
        can_discard: false,
        writes_depth: false,
        writes_stencil: false,
        writes_coverage: false,
        writes_global: false,
        ubo: &[],
    }
}

/// Ours: case 16's draw into a 64x64 target at `gpu` with `samples`; the
/// descriptor and sample arenas, the RUN_IDVS registers and the frame.
fn build(gpu: u64, samples: u32, sample_mask: u8) -> (Vec<u8>, Vec<u8>, BTreeMap<u32, u32>, Frame) {
    build_with(gpu, samples, TargetFormat::Rgba8888, |d| d.sample_mask = sample_mask)
}

/// [`build`] into a target of `format`, with the draw's state as `f`
/// leaves it.
fn build_with(
    gpu: u64,
    samples: u32,
    format: TargetFormat,
    f: impl Fn(&mut DrawCall),
) -> (Vec<u8>, Vec<u8>, BTreeMap<u32, u32>, Frame) {
    let (mut desc_m, mut code_m, mut spd_m, mut samples_m) =
        (vec![0u8; 0x4000], vec![0u8; 0x800], vec![0u8; 0x100], vec![0u8; 0x300]);
    let mut words = vec![0u64; 512];
    let (frame, n);
    {
        let mut sa = Arena::new("samples", SAMPLES, &mut samples_m).unwrap();
        let positions = sample_positions(&mut sa).unwrap();
        let ctx = Context {
            heap: HEAP,
            geometry_buffer: GEOMETRY,
            geometry_buffer_size: GEOMETRY_BUFFER_BYTES,
            sample_positions: positions.gpu,
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
        let mut draw = DrawCall {
            texture: None,
            vs_texture: None,
            occlusion_query: None,
            alpha_to_coverage: false,
            blend_shader: None,
            zs: DepthStencilState::OFF,
            colour_mask: 0xf,
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
        };
        f(&mut draw);
        let input = FrameInput {
            zs: None,
            target: Target { gpu, width: 64, height: 64, row_stride: 256, format, samples },
            clear_rgba: Some([0.0, 0.0, 0.0, 1.0]),
            draw: Some(draw),
        };
        let mut cs = Stream::new(&mut words, 92);
        frame = build_frame(&mut desc, &mut cs, &ctx, &input).expect("msaa.c's frame builds");
        n = cs.words().len();
    }
    let (regs, _) = regs_at_idvs(&words[..n]);
    (desc_m, samples_m, regs, frame)
}

/// The FBD pointer of a trace: the d40 move the fragment run reads (its
/// last before RUN_FRAGMENT), tag cleared.
fn their_fbd(source: &str) -> u64 {
    let words = trace::cs_words(MSAA, source);
    let mut fbd = 0;
    for w in &words {
        match Instr::decode(w.word) {
            Ok(Instr::Move48(m)) if m.destination == 40 => fbd = m.immediate,
            Ok(Instr::RunFragment(_)) => return fbd & !63,
            _ => {}
        }
    }
    panic!("{source}: no RUN_FRAGMENT")
}

fn arr<const N: usize>(v: &[u8]) -> [u8; N] {
    v[..N].try_into().unwrap()
}

/// The 4x frame and its single-sampled control are the board's.
#[test]
fn a_4x_target_is_the_board_s_multisampled_render_to_texture() {
    for (source, samples) in [("mesa-msrtt", 4), ("mesa-fbo1", 1)] {
        let m = trace::memory_of(MSAA, source);
        let fbd = their_fbd(source);
        let theirs_rt = trace::bytes(&m, fbd + 0x80, 64);
        let base = u64::from_le_bytes(theirs_rt[32..40].try_into().unwrap());
        let (desc, sample_mem, regs, frame) = build(base, samples, 0xf);
        let ours_fbd = ((frame.fbd_pointer & !63) - DESC) as usize;
        /* the framebuffer parameters, the tiler context's address set equal */
        let mut ours_p = FramebufferParameters::unpack(&arr(&desc[ours_fbd..])).unwrap();
        let mut theirs_p = FramebufferParameters::unpack(&arr(&trace::bytes(&m, fbd, 64))).unwrap();
        let (our_tiler, their_tiler) = (ours_p.tiler, theirs_p.tiler);
        (ours_p.tiler, theirs_p.tiler) = (0, 0);
        assert_eq!(format!("{ours_p:?}"), format!("{theirs_p:?}"), "{source}: the framebuffer parameters");
        /* the sample-position block the FBD names, byte for byte */
        let at = (theirs_p.sample_locations - SAMPLES) as usize;
        assert_eq!(
            sample_mem[at..at + 256],
            trace::bytes(&m, theirs_p.sample_locations, 256)[..],
            "{source}: sample positions"
        );
        /* the render target, byte for byte (built at the trace's base) */
        assert_eq!(desc[ours_fbd + 0x80..ours_fbd + 0xc0], theirs_rt[..], "{source}: the render target");
        /* the tiler context, its polygon list set equal, and its private
         * state, which the GPU writes while the job runs (the dump is the
         * memory after it ran: the trace's word 0 is 1, the builder's 0) */
        let tiler = |v: &[u8]| {
            let mut t = TilerContext::unpack(&arr(v)).unwrap();
            t.polygon_list = 0;
            t.private_state = TilerState::default();
            format!("{t:?}")
        };
        let ours_t = &desc[(our_tiler - DESC) as usize..];
        assert_eq!(tiler(ours_t), tiler(&trace::bytes(&m, their_tiler, 128)), "{source}: the tiler context");
        /* DCD0 */
        let words: Vec<u64> = trace::cs_words(MSAA, source).iter().map(|w| w.word).collect();
        let (theirs, _) = regs_at_idvs(&words);
        assert_eq!(regs[&57], theirs[&57], "{source}: DCD0");
    }
    /* the dump holds what the test reads: 4 samples, the rotated grid at
     * +0x200, multisample on (r57 0x801AB against 0x21AB) */
    let m = trace::memory_of(MSAA, "mesa-msrtt");
    let p = FramebufferParameters::unpack(&arr(&trace::bytes(&m, their_fbd("mesa-msrtt"), 64))).unwrap();
    assert_eq!(
        (p.sample_count, p.sample_pattern, p.sample_locations),
        (4, SamplePattern::ROTATED_4X_GRID, SAMPLES + 0x200)
    );
}

/// glSampleCoverage(0.5) writes samples 0 and 1, inverted samples 2 and 3
/// (DCD1's sample mask, r58's low 16 bits): the inverted mask is the
/// board's bytes; the plain one has the bits above the 4 samples set where
/// Mesa leaves them clear (0xFFF3 for its 0x0003), the same samples.
#[test]
fn sample_coverage_is_the_board_s_sample_mask() {
    let r58 = |source: &str| {
        let words: Vec<u64> = trace::cs_words(MSAA, source).iter().map(|w| w.word).collect();
        regs_at_idvs(&words).0[&58]
    };
    let ours = |mask| build(0x7fff_fffe_4000, 4, mask).2[&58];
    assert_eq!(ours(0xc), r58("mesa-cov50inv"), "inverted");
    assert_eq!((r58("mesa-cov50"), ours(0x3)), (0x1_0003, 0x1_fff3), "plain: the named difference");
    assert_eq!(ours(0x3) & 0xf, r58("mesa-cov50") & 0xf, "the same 4 samples");
    assert_eq!(build(0x7fff_fffe_4000, 1, 0x3).2[&58] & 0xffff, 0xffff, "a 1x target ignores the mask");
}

/// Case 198's draw (alpha to coverage on, alpha masked, depth test off,
/// blending off) on the 4x target of a pbuffer (`flip_y`, gallium's
/// front_ccw for a window surface) against the first draw of Mesa's second
/// picture (tools/mali/board-dump/a2c-2026-10-23.txt): DCD 0 and 1 and the
/// tiler flags word for word, the depth/stencil and blend descriptors byte
/// for byte, and what alpha-to-coverage moves in DCD 0. Mesa's target is
/// its own 4x pbuffer (a 4x image, `Writeback MSAA: Layered`) where ours is
/// the backend contract's 4x tile buffer over single-sampled memory, so its
/// framebuffer descriptor is not compared here (the 4x one is,
/// `a_4x_target_is_the_board_s_multisampled_render_to_texture`). The
/// config Mesa's eglChooseConfig gave for `EGL_SAMPLES 4` is RGB565 (the
/// dump's `Internal Format: R5G6B5A0`, the blend's conversion R5G6B5 AU), so
/// ours is built into an RGB565 target: then the blend is the dump's byte
/// for byte (into RGBA8 it differs at +0xd only, the conversion's RGBA8 TB).
#[test]
fn alpha_to_coverage_is_the_board_s() {
    const SOURCE: &str = "mesa-a2cconst";
    let case_198 = |d: &mut DrawCall| (d.colour_mask, d.flip_y) = (0x7, true);
    let (desc, _, ours, _) = build_with(0x7fff_fffe_4000, 4, TargetFormat::Rgb565, |d| {
        case_198(d);
        d.alpha_to_coverage = true;
    });
    let words: Vec<u64> = trace::cs_words(A2C, SOURCE).iter().map(|w| w.word).collect();
    let (theirs, _) = regs_at_idvs(&words);
    let m = trace::memory_of(A2C, SOURCE);
    for reg in [56, 57, 58] {
        assert_eq!(ours[&reg], theirs[&reg], "r{reg}: Mesa {:#x}, ours {:#x}", theirs[&reg], ours[&reg]);
    }
    let reg64 = |r: &BTreeMap<u32, u32>, n: u32| u64::from(r[&n]) | u64::from(r[&(n + 1)]) << 32;
    for (what, reg, len, tag) in [("Depth/stencil", 52, DepthStencil::LENGTH, 0), ("Blend", 50, Blend::LENGTH, 63)] {
        let (og, mg) = (reg64(&ours, reg) & !tag, reg64(&theirs, reg) & !tag);
        let at = (og - DESC) as usize;
        compare(what, &desc[at..at + len], &trace::bytes(&m, mg, len), &[]).unwrap();
    }
    /* what the dump says: forward pixel kill off, the update force late
     * with the kill weak early, coverage modified by alpha-to-coverage,
     * not inverted (r57 0x5901BA) */
    let flags = DcdFlags0::unpack(&theirs[&57].to_le_bytes()).unwrap();
    assert_eq!(
        (
            flags.allow_forward_pixel_to_kill,
            flags.zs_update_operation,
            flags.pixel_kill_operation,
            flags.shader_modifies_coverage,
            flags.alpha_to_coverage,
            flags.alpha_to_coverage_invert,
            flags.multisample_enable,
        ),
        (false, PixelKill::FORCE_LATE, PixelKill::WEAK_EARLY, true, true, false, true)
    );
    /* the same draw without it: only those bits move (bits 4-5 force late
     * against weak early, 20, 22; forward pixel kill is already off, the
     * mask leaves alpha out) */
    let (_, _, plain, _) = build_with(0x7fff_fffe_4000, 4, TargetFormat::Rgb565, case_198);
    assert_eq!(plain[&57] ^ theirs[&57], 0x10 | 1 << 20 | 1 << 22, "r57 without alpha-to-coverage {:#x}", plain[&57]);
    assert_eq!((plain[&56], plain[&58]), (theirs[&56], theirs[&58]));
}

/// Alpha-to-coverage with no channel written still runs the fragment
/// shader (`panfrost_fs_required`, Mesa 26.2.1 pan_cmdstream.h:210-213):
/// DCD 0 as with a colour, forward pixel kill off, the render target mask
/// of the shader's output, the blend descriptor that of a target not
/// written (enable off, mode Off). A draw without it writes nothing and runs
/// no shader.
#[test]
fn alpha_to_coverage_without_colour_writes_runs_the_shader() {
    let (desc, _, regs, _) = build_with(0x7fff_fffe_4000, 4, TargetFormat::Rgba8888, |d| {
        (d.colour_mask, d.alpha_to_coverage) = (0, true);
    });
    let flags = DcdFlags0::unpack(&regs[&57].to_le_bytes()).unwrap();
    assert_eq!(
        (
            flags.allow_forward_pixel_to_kill,
            flags.zs_update_operation,
            flags.pixel_kill_operation,
            flags.shader_modifies_coverage,
            flags.alpha_to_coverage,
        ),
        (false, PixelKill::FORCE_LATE, PixelKill::WEAK_EARLY, true, true)
    );
    assert_eq!(DcdFlags1::unpack(&regs[&58].to_le_bytes()).unwrap().render_target_mask, 1);
    let (none_desc, _, none, _) = build_with(0x7fff_fffe_4000, 4, TargetFormat::Rgba8888, |d| d.colour_mask = 0);
    /* the blend descriptor of the draw that writes nothing, which is the
     * disabled one (frame_zs_tests.rs names its words) */
    let blend = |r: &BTreeMap<u32, u32>| ((u64::from(r[&50]) | u64::from(r[&51]) << 32) & !63) - DESC;
    let (at, none_at) = (blend(&regs) as usize, blend(&none) as usize);
    assert_eq!(desc[at..at + Blend::LENGTH], none_desc[none_at..none_at + Blend::LENGTH]);
    let flags = DcdFlags0::unpack(&none[&57].to_le_bytes()).unwrap();
    assert_eq!((flags.allow_forward_pixel_to_kill, flags.zs_update_operation), (true, PixelKill::FORCE_EARLY));
    assert!(!flags.alpha_to_coverage);
    assert_eq!(DcdFlags1::unpack(&none[&58].to_le_bytes()).unwrap().render_target_mask, 0);
}

/// gl_test case 211 msaablend's blended draw (glBlendColor 0.25, 0.5, 0.75,
/// 1.0 with CONSTANT_COLOR, ONE_MINUS_CONSTANT_COLOR) into the backend
/// contract's 4x target, against the board's Mesa
/// (tools/mali/board-dump/blendshader-2026-10-24.txt, the second draw of
/// 0dce654a8:tools/mali/board-ref/2026-10-24-mesa-refs4 `mesa-msaablend-rtt`): the
/// blend descriptor byte for byte with the frame given Mesa's shader address
/// (mode Shader, `PC: 4294750208`, the destination loaded, no equation nor
/// constant: the 1x shape of case 8), and DCD0 0x809AA: the blended 4x draw
/// evaluated per sample (bit 11, `valhall_has_blend_shader`), forward pixel
/// kill off (bit 0). The words at that address are the board's 25.0.7 (19,
/// ending in ST_TILE); ours are the drm-shim's 26.2.1 (22, ending in BLEND:
/// docs/notes/mali-blend-shaders.md section 4, frame_blend_tests.rs
/// `the_reference_blend_shaders_are_the_drm_shim_s_words`).
#[test]
fn case_211_s_4x_blend_shader_is_the_board_s() {
    use super::blend::{Base, Blending, Channel, Factor, Func};
    const BLENDSHADER: &str = include_str!("../../tools/mali/board-dump/blendshader-2026-10-24.txt");
    const SOURCE: &str = "mesa-msaablend-rtt";
    const PC: u64 = 0x7fff_fffc_b000;
    let cc = Factor { base: Base::ConstColor, invert: false };
    let c = Channel { func: Func::Add, src: cc, dst: Factor { invert: true, ..cc } };
    let case_211 = |d: &mut DrawCall| {
        d.blend = Some(Blending { rgb: c, alpha: c });
        d.blend_color = [0.25, 0.5, 0.75, 1.0];
        d.blend_shader = Some(PC);
    };
    let (desc, _, ours, _) = build_with(0x7fff_fffe_4000, 4, TargetFormat::Rgba8888, case_211);
    let words: Vec<u64> = trace::cs_words(BLENDSHADER, SOURCE).iter().map(|w| w.word).collect();
    let (theirs, _) = regs_at_idvs(&words);
    let m = trace::memory_of(BLENDSHADER, SOURCE);
    let reg64 = |r: &BTreeMap<u32, u32>, n: u32| u64::from(r[&n]) | u64::from(r[&(n + 1)]) << 32;
    let at = ((reg64(&ours, 50) & !63) - DESC) as usize;
    let their_blend = trace::bytes(&m, reg64(&theirs, 50) & !15, Blend::LENGTH);
    compare("Blend", &desc[at..at + Blend::LENGTH], &their_blend, &[]).unwrap();
    assert_eq!(ours[&57], theirs[&57], "DCD0: Mesa {:#x}, ours {:#x}", theirs[&57], ours[&57]);
    /* what the dump says: load destination and enable, mode Shader at
     * 0xFFFCB000 (`mesa-msaablend-rtt.pandecode:1266-1269`), per-sample
     * evaluation and multisampling on (:1329, :1335) */
    assert_eq!(their_blend, [1, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x00, 0xb0, 0xfc, 0xff]);
    assert_eq!(theirs[&57], 0x809aa);
    let flags = DcdFlags0::unpack(&theirs[&57].to_le_bytes()).unwrap();
    assert!(flags.evaluate_per_sample && flags.multisample_enable && !flags.allow_forward_pixel_to_kill);
    /* the shader Mesa ran is in the excerpt where the descriptor points:
     * 25.0.7's first word, the MOV of r61 the drm-shim's starts with too */
    assert_eq!(trace::bytes(&m, PC, 8), 0x0091_c200_0000_007du64.to_le_bytes());
    /* a fixed-function blend that reads the destination, 4x: the same DCD0
     * but per-sample evaluation, which only a blend shader turns on */
    let over = Channel { func: Func::Add, src: Factor::SRC_ALPHA, dst: Factor::INV_SRC_ALPHA };
    let (_, _, ff, _) = build_with(0x7fff_fffe_4000, 4, TargetFormat::Rgba8888, |d| {
        d.blend = Some(Blending { rgb: over, alpha: over })
    });
    assert_eq!(ff[&57] ^ ours[&57], 1 << 11);
}

/* msaazs.c's addresses: the render target ("RGB: Base") and the ZS buffer
 * ("ZS Writeback Base") */
const ZS_TARGET: u64 = 0x7fff_fffe_4000;
const ZS_BUF: u64 = 0x7fff_fffe_0000;
/// The tiler context Mesa's FBD names ("Tiler: 0x7ffffffd0340").
const ZS_TRACE_TILER: u64 = DESC + 0x340;

/// One draw of case 207: its depth/stencil state, first vertex and count.
type ZsDraw = (DepthStencilState, u32, u32);

/// The registers each RUN_IDVS of a stream reads, in order.
fn regs_at_each_idvs(words: &[u64]) -> Vec<BTreeMap<u32, u32>> {
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

/// Ours: case 207's frame into the 4x target with its D24S8 at the trace's
/// addresses (cleared black, depth 1.0, stencil 0): the descriptor and
/// sample arenas, the stream and the frame.
fn build_207(draws: &[ZsDraw]) -> (Vec<u8>, Vec<u8>, Vec<u64>, Frame) {
    let (mut desc_m, mut code_m, mut spd_m, mut samples_m, mut ctx_m) =
        (vec![0u8; 0x4000], vec![0u8; 0x800], vec![0u8; 0x100], vec![0u8; 0x300], vec![0u8; 0x200]);
    let mut words = vec![0u64; 1024];
    let frame;
    {
        let mut sa = Arena::new("samples", SAMPLES, &mut samples_m).unwrap();
        let positions = sample_positions(&mut sa).unwrap();
        let mut ctx_a = Arena::new("context", HEAP, &mut ctx_m).unwrap();
        let heap = tiler_heap_descriptor(&mut ctx_a, 0x8000_00a0_0000, 2 << 20).unwrap();
        let ctx = Context {
            heap: heap.gpu,
            geometry_buffer: GEOMETRY,
            geometry_buffer_size: GEOMETRY_BUFFER_BYTES,
            sample_positions: positions.gpu,
            tiler_max_levels: 8,
        };
        let mut desc = Arena::new("desc", DESC, &mut desc_m).unwrap();
        let mut code = Arena::new("code", 0x7fff_ffff_c000, &mut code_m).unwrap();
        let mut spds = Arena::new("spds", 0x7fff_ffff_d000, &mut spd_m).unwrap();
        let bin = [0u8; 128];
        let vs = upload_program(&mut code, &mut spds, &bin, meta(ShaderStage::VERTEX)).unwrap();
        let fs = upload_program(&mut code, &mut spds, &bin, meta(ShaderStage::FRAGMENT)).unwrap();
        let attributes = [VertexAttribute {
            buffer: VertexBuffer { gpu: 0x7fff_ffec_c000, size: 21 * 40 },
            offset: 0,
            stride: 40,
            components: 4,
            instance_divisor: 0,
        }];
        let target = Target {
            gpu: ZS_TARGET,
            width: 64,
            height: 64,
            row_stride: 256,
            format: TargetFormat::Rgba8888,
            samples: 4,
        };
        let attachment = ZsAttachment { gpu: ZS_BUF, row_stride: 256, clear_depth: 1.0, clear_stencil: 0 };
        let mut cs = Stream::new(&mut words, 92);
        let mut open = begin_frame(&mut desc, &mut cs).unwrap();
        for &(zs, first_vertex, vertex_count) in draws {
            let draw = DrawCall {
                texture: None,
                vs_texture: None,
                occlusion_query: None,
                alpha_to_coverage: false,
                blend_shader: None,
                zs,
                colour_mask: 0xf,
                front_cw: false,
                cull: Cull::None,
                vs: &vs,
                fs: &fs,
                primitive: Topology::Triangles,
                first_vertex,
                vertex_count,
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
            };
            add_draw(&mut open, &mut desc, &mut cs, &ctx, &target, true, &draw).expect("a 4x draw with depth/stencil");
        }
        frame = end_frame(&open, &mut desc, &mut cs, &ctx, &target, [0.0, 0.0, 0.0, 1.0], Some(attachment))
            .expect("a 4x frame with depth/stencil");
        let n = cs.words().len();
        words.truncate(n);
    }
    (desc_m, samples_m, words, frame)
}

/// One place where our bytes may differ from the dump's.
struct Difference {
    block: &'static str,
    at: usize,
    dump: Vec<u8>,
    ours: Vec<u8>,
    cause: &'static str,
}

fn compare(name: &str, ours: &[u8], dump: &[u8], diffs: &[Difference]) -> Result<(), String> {
    let mut expected = dump.to_vec();
    for d in diffs.iter().filter(|d| d.block == name) {
        let span = d.at..d.at + d.dump.len();
        assert_eq!(dump[span.clone()], d.dump[..], "{name} +{:#x}: the dump is not what the cause says", d.at);
        expected[span].copy_from_slice(&d.ours);
    }
    if expected == ours {
        Ok(())
    } else {
        let at: Vec<usize> = (0..ours.len()).filter(|&i| expected[i] != ours[i]).collect();
        Err(format!("{name}: bytes {at:x?}\n  dump  {dump:02x?}\n  ours  {ours:02x?}"))
    }
}

/// The dump's ZS/CRC extension with its S target base (a named difference,
/// bits our ZS Target holds reserved) cleared.
fn zs_target_of(m: &BTreeMap<u64, [u8; 16]>) -> ZsTarget {
    let mut b = trace::bytes(m, DESC + 0x80, 64);
    assert_eq!(b[0x30..0x38], ZS_BUF.to_le_bytes(), "the S target base the difference names");
    b[0x30..0x38].fill(0);
    ZsTarget::unpack(&arr(&b)).unwrap()
}

/// Case 207's frame against the board's, `source` in the excerpt, and the
/// differences only that source has.
fn check_207(source: &str, draws: &[ZsDraw], extra: Vec<Difference>) {
    let (desc, sample_mem, words, frame) = build_207(draws);
    let m = trace::memory_of(MSAA_ZS, source);
    let ours = |gpu: u64, len: usize| desc[(gpu - DESC) as usize..(gpu - DESC) as usize + len].to_vec();
    let our_tiler = frame.layout.tiler.expect("the frame's tiler context").gpu;
    let mut diffs = vec![
        Difference {
            block: "Framebuffer Parameters",
            at: 0x38,
            dump: ZS_TRACE_TILER.to_le_bytes().to_vec(),
            ours: our_tiler.to_le_bytes().to_vec(),
            cause: "the tiler context: Mesa's comes after its OOM context and incremental-rendering FBDs (not built)",
        },
        Difference {
            block: "ZS CRC Extension",
            at: 0x30,
            dump: ZS_BUF.to_le_bytes().to_vec(),
            ours: vec![0; 8],
            cause: "S target base: Mesa 25.0.7 also writes the ZS base there with the S block format No Write \
                    (unused); Mesa 26.2.1 merges no S part for a D24S8 view (pan_job.c:596-612, pan_desc.c:540-550)",
        },
    ];
    diffs.extend(extra);
    let mut failures = Vec::new();
    for (name, va, len) in [
        ("Framebuffer Parameters", DESC, 128),
        ("ZS CRC Extension", DESC + 0x80, 64),
        ("Render Target", DESC + 0xc0, 64),
    ] {
        if let Err(e) = compare(name, &ours(va, len), &trace::bytes(&m, va, len), &diffs) {
            failures.push(e);
        }
    }
    /* the sample-position block the FBD names, byte for byte */
    let p = FramebufferParameters::unpack(&arr(&trace::bytes(&m, DESC, 64))).unwrap();
    let at = (p.sample_locations - SAMPLES) as usize;
    if let Err(e) =
        compare("Sample positions", &sample_mem[at..at + 256], &trace::bytes(&m, p.sample_locations, 256), &[])
    {
        failures.push(e);
    }
    /* the tiler context, its polygon list and private state set equal (as
     * in the colour-only 4x frame's proof) */
    let tiler = |v: &[u8]| {
        let mut t = TilerContext::unpack(&arr(v)).unwrap();
        t.polygon_list = 0;
        t.private_state = TilerState::default();
        format!("{t:?}")
    };
    let (ours_t, theirs_t) = (tiler(&ours(our_tiler, 128)), tiler(&trace::bytes(&m, ZS_TRACE_TILER, 128)));
    if ours_t != theirs_t {
        failures.push(format!("the tiler context:\n  dump  {theirs_t}\n  ours  {ours_t}"));
    }
    /* each draw's ZSD and blend (d52; d50 with its count in the low 6 bits)
     * and its tiler flags and DCD 0 and 1 (r56, r57, r58) */
    let mesa_words: Vec<u64> = trace::cs_words(MSAA_ZS, source).iter().map(|w| w.word).collect();
    let (mesa_regs, our_regs) = (regs_at_each_idvs(&mesa_words), regs_at_each_idvs(&words));
    assert_eq!((mesa_regs.len(), our_regs.len()), (draws.len(), draws.len()), "{source}: one RUN_IDVS a draw");
    let reg64 = |r: &BTreeMap<u32, u32>, n: u32| u64::from(r[&n]) | u64::from(r[&(n + 1)]) << 32;
    for (k, (mr, or)) in mesa_regs.iter().zip(&our_regs).enumerate() {
        for (what, reg, len, tag) in [("Depth/stencil", 52, DepthStencil::LENGTH, 0), ("Blend", 50, Blend::LENGTH, 63)]
        {
            let (mg, og) = (reg64(mr, reg) & !tag, reg64(or, reg) & !tag);
            if let Err(e) = compare(what, &ours(og, len), &trace::bytes(&m, mg, len), &[]) {
                failures.push(format!("draw {k}: {e}"));
            }
        }
        for reg in [56, 57, 58] {
            if mr[&reg] != or[&reg] {
                failures.push(format!("draw {k} r{reg}: Mesa {:#x}, ours {:#x}", mr[&reg], or[&reg]));
            }
        }
    }
    /* the FBD pointer's tag: the ZS/CRC extension and one render target */
    assert_eq!(frame.fbd_pointer, DESC | 3);
    assert!(trace::cs_words(MSAA_ZS, source).iter().any(|w| w.mnemonic == "MOVE d40, #0x7FFFFFFD0003"));
    eprintln!("mali 4x zs frame vs the board ({source}): {} differences named", diffs.len());
    for d in &diffs {
        eprintln!("  {} +{:#x}: {}", d.block, d.at, d.cause);
    }
    assert!(failures.is_empty(), "{source}:\n{}", failures.join("\n"));
}

/// Case 207's depth frame: one draw of 12 vertices, LESS with writes. What
/// the 4x depth/stencil frame moves against the 1x one
/// (tools/mali/board-dump/zs-depth-2026-10-19.txt), held by the dump: 4
/// samples on the rotated grid, the colour buffer allocation 16384 with the
/// tile kept at 1024, the ZS target's and the render target's MSAA Average
/// (the D24S8 is a view of 4 samples on an image of 1, `mali_sampling_mode`,
/// pan_desc.c:26-48), and DCD0's multisample enable.
#[test]
fn a_4x_depth_frame_is_the_board_s() {
    let less = DepthStencilState { depth_func: Func::LESS, depth_write: true, ..DepthStencilState::OFF };
    check_207("mesa-msrtt", &[(less, 0, 12)], vec![]);
    let m = trace::memory_of(MSAA_ZS, "mesa-msrtt");
    let p = FramebufferParameters::unpack(&arr(&trace::bytes(&m, DESC, 64))).unwrap();
    assert_eq!(
        (p.sample_count, p.sample_pattern, p.effective_tile_size, p.color_buffer_allocation, p.z_clear),
        (4, SamplePattern::ROTATED_4X_GRID, 1024, 16384, 1.0)
    );
    let ext = zs_target_of(&m);
    assert_eq!((ext.msaa, ext.write_format, ext.base), (Msaa::AVERAGE, ZsFormat::D24S8, ZS_BUF));
    let words: Vec<u64> = trace::cs_words(MSAA_ZS, "mesa-msrtt").iter().map(|w| w.word).collect();
    assert!(DcdFlags0::unpack(&regs_at_each_idvs(&words)[0][&57].to_le_bytes()).unwrap().multisample_enable);
}

/// Case 207's stencil frame: 3 vertices writing stencil 1 (ALWAYS,
/// REPLACE), then 6 where the stencil is 0 (EQUAL, KEEP), the depth test
/// off; the ZS target Average as in the depth frame. Mesa's frame has its
/// D24S8 on STENCIL only (Mesa 25.0.7 calls it on both points with 4
/// samples incomplete, msaazs.c), so its glClear clears no depth: Z Clear
/// 0.0 where case 207's surface, with both, clears 1.0 (the depth frame's
/// dump, which has the depth, holds 1.0).
#[test]
fn a_4x_stencil_frame_is_the_board_s() {
    let face = |func, depth_pass, reference| StencilFace {
        func,
        fail: StencilOp::KEEP,
        depth_fail: StencilOp::KEEP,
        depth_pass,
        value_mask: 0xff,
        write_mask: 0xff,
        reference,
    };
    let stencil = |func, depth_pass, reference| DepthStencilState {
        stencil_test: true,
        faces: [face(func, depth_pass, reference); 2],
        ..DepthStencilState::OFF
    };
    check_207(
        "mesa-msrtts",
        &[(stencil(Func::ALWAYS, StencilOp::REPLACE, 1), 12, 3), (stencil(Func::EQUAL, StencilOp::KEEP, 0), 15, 6)],
        vec![Difference {
            block: "Framebuffer Parameters",
            at: 0x34,
            dump: 0.0f32.to_le_bytes().to_vec(),
            ours: 1.0f32.to_le_bytes().to_vec(),
            cause: "Z Clear: Mesa's D24S8 is on STENCIL only, so its glClear clears no depth (msaazs.c msrtts)",
        }],
    );
    let m = trace::memory_of(MSAA_ZS, "mesa-msrtts");
    assert_eq!(zs_target_of(&m).msaa, Msaa::AVERAGE);
}
