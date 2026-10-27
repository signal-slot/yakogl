// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The frame builder's depth/stencil against the board: the frame Mesa
//! 25.0.7 drew on the NanoPC-T6 for two depth-tested draws into a 64x64
//! RGBA8 FBO with a D24S8 renderbuffer (tools/mali/board-dump/
//! zs-depth-2026-10-19.txt, cut from 0dce654a8:tools/mali/board-ref/
//! 2026-10-19-frame-depth/mesa), built again with the framebuffer
//! descriptor's arena at the trace's own address.
//!
//! What is compared: the framebuffer descriptor (parameters, ZS/CRC
//! extension, render target), each draw's Depth/stencil and Blend
//! descriptors, the DCD flags and the tiler flags each draw's RUN_IDVS
//! reads, the frame's one VERTEX_TILER_STARTED and the FBD pointer's tag.
//! Every byte where ours may differ from the dump is listed with its cause,
//! the dump is checked to hold what the cause says, and every other byte is
//! equal. The draws' other blocks (programs, attributes, uniforms, resource
//! tables) are the colour-only frame's, which `frame_tests.rs` compares
//! with the L1 trace; Mesa emits them again only when they change, the
//! builder for every draw, so their places differ and are not compared
//! here.

use super::arena::Arena;
use super::cs::{Instr, Stream};
use super::frame::*;
use super::genxml::v10::*;
use super::trace;
use std::collections::BTreeMap;

const ZS: &str = include_str!("../../tools/mali/board-dump/zs-depth-2026-10-19.txt");

/* the trace's addresses: the descriptor buffer (the FBD at its start,
 * "Framebuffer @7ffffffd0000"), the render target ("RGB: Base"), the ZS
 * buffer ("ZS Writeback Base"), the sample positions and the heap */
const DESC: u64 = 0x7fff_fffd_0000;
const TARGET: u64 = 0x7fff_fffe_4000;
const ZS_BUF: u64 = 0x7fff_fffe_0000;
const SAMPLES: u64 = 0x7fff_ffff_f000;
const HEAP_DESC: u64 = 0x7fff_ffff_b000;
const GEOMETRY: u64 = 0x7fff_fffe_b000;
/// The tiler context Mesa's FBD names ("Tiler: 0x7ffffffd0300"): after its
/// OOM context and incremental-rendering FBDs, which the builder does not
/// build.
const TRACE_TILER: u64 = DESC + 0x300;

/// The registers each RUN_IDVS of a stream reads, in order: every MOVE32
/// and MOVE48 before it, per register (a MOVE48 writes the pair).
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

/// The state of zs.c's draws: LESS with writes for "depth", off for
/// "nodepth" (st/mesa's ZSA: the test off is ALWAYS, no write).
fn zs_of(source: &str) -> DepthStencilState {
    match source {
        "depth" => DepthStencilState { depth_func: Func::LESS, depth_write: true, ..DepthStencilState::OFF },
        _ => DepthStencilState::OFF,
    }
}

struct Built {
    desc: Vec<u8>,
    words: Vec<u64>,
    frame: Frame,
}

/// zs.c's frame: cleared blue, depth 1.0, stencil 0x5a, two draws of six
/// vertices with `zs` and `colour_mask`, built into a descriptor arena at the trace's
/// address.
fn build(zs: DepthStencilState, colour_mask: u8) -> Built {
    let (mut desc_m, mut code_m, mut spd_m, mut ctx_m) =
        (vec![0u8; 0x4000], vec![0u8; 0x800], vec![0u8; 0x100], vec![0u8; 0x200]);
    let mut words = vec![0u64; 512];
    let frame;
    {
        let mut ctx_a = Arena::new("context", HEAP_DESC, &mut ctx_m).unwrap();
        let heap = tiler_heap_descriptor(&mut ctx_a, 0x8000_00a0_0000, 2 << 20).unwrap();
        assert_eq!(heap.gpu, HEAP_DESC);
        let ctx = Context {
            heap: heap.gpu,
            geometry_buffer: GEOMETRY,
            geometry_buffer_size: GEOMETRY_BUFFER_BYTES,
            sample_positions: SAMPLES,
            tiler_max_levels: 8,
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
            zs,
            colour_mask,
            front_cw: false,
            cull: Cull::None,
            texture: None,
            vs_texture: None,
            occlusion_query: None,
            alpha_to_coverage: false,
            blend_shader: None,
        };
        let target =
            Target { gpu: TARGET, width: 64, height: 64, row_stride: 256, format: TargetFormat::Rgba8888, samples: 1 };
        let attachment = ZsAttachment { gpu: ZS_BUF, row_stride: 256, clear_depth: 1.0, clear_stencil: 0x5a };
        let mut cs = Stream::new(&mut words, 92);
        let mut open = begin_frame(&mut desc, &mut cs).unwrap();
        for _ in 0..2 {
            add_draw(&mut open, &mut desc, &mut cs, &ctx, &target, true, &draw).unwrap();
        }
        assert_eq!(open.draws, 2);
        assert_eq!(open.uses_zs, zs.uses_attachment());
        frame = end_frame(&open, &mut desc, &mut cs, &ctx, &target, [0.0, 0.0, 1.0, 1.0], Some(attachment)).unwrap();
        let n = cs.words().len();
        words.truncate(n);
    }
    Built { desc: desc_m, words, frame }
}

/// One place where our bytes may differ from the dump's.
struct Difference {
    block: &'static str,
    at: usize,
    dump: Vec<u8>,
    ours: Vec<u8>,
    cause: &'static str,
}

fn compare(name: &'static str, ours: &[u8], dump: &[u8], diffs: &[Difference]) -> Result<(), String> {
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

fn check_source(source: &'static str) {
    let b = build(zs_of(source), 0xf);
    let m = trace::memory_of(ZS, source);
    let ours = |gpu: u64, len: usize| b.desc[(gpu - DESC) as usize..(gpu - DESC) as usize + len].to_vec();
    let tiler = b.frame.layout.tiler.expect("the frame's tiler context").gpu;
    let diffs = [
        Difference {
            block: "Framebuffer Parameters",
            at: 0x38,
            dump: TRACE_TILER.to_le_bytes().to_vec(),
            ours: tiler.to_le_bytes().to_vec(),
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
    let mut failures = Vec::new();
    let mut blocks = vec![
        ("Framebuffer Parameters", DESC, 128),
        ("ZS CRC Extension", DESC + 0x80, 64),
        ("Render Target", DESC + 0xc0, 64),
    ];
    /* each draw's ZSD and blend: Mesa's two draws share one of each (the
     * state did not change), ours has its own per draw */
    let mesa_regs = regs_at_idvs(&trace::cs_words(ZS, source).iter().map(|w| w.word).collect::<Vec<_>>());
    let our_regs = regs_at_idvs(&b.words);
    assert_eq!((mesa_regs.len(), our_regs.len()), (2, 2), "two RUN_IDVS each");
    let reg64 = |r: &BTreeMap<u32, u32>, n: u32| u64::from(r[&n]) | u64::from(r[&(n + 1)]) << 32;
    for (k, (mr, or)) in mesa_regs.iter().zip(&our_regs).enumerate() {
        /* d52 is the ZSD's address; d50 the blend array's, with its count
         * in the low 6 bits */
        for (what, reg, len, tag) in [("Depth/stencil", 52, DepthStencil::LENGTH, 0), ("Blend", 50, Blend::LENGTH, 63)]
        {
            let (mg, og) = (reg64(mr, reg) & !tag, reg64(or, reg) & !tag);
            let mesa = trace::bytes(&m, mg, len);
            if let Err(e) = compare("draw", &ours(og, len), &mesa, &[]) {
                failures.push(format!("draw {k} {what}: {e}"));
            }
        }
        /* the draw flags: the tiler flags (r56), DCD 0 and 1 (r57, r58) */
        for reg in [56, 57, 58] {
            if mr[&reg] != or[&reg] {
                failures.push(format!("draw {k} r{reg}: Mesa {:#x}, ours {:#x}", mr[&reg], or[&reg]));
            }
        }
    }
    for (name, va, len) in blocks.drain(..) {
        if let Err(e) = compare(name, &ours(va, len), &trace::bytes(&m, va, len), &diffs) {
            failures.push(e);
        }
    }
    /* one VERTEX_TILER_STARTED for the frame, before its first draw */
    let started = |words: &[u64]| {
        words
            .iter()
            .filter(|&&w| {
                matches!(Instr::decode(w), Ok(Instr::HeapOperation(h))
                    if h.operation == CsHeapOperationEnum::VERTEX_TILER_STARTED)
            })
            .count()
    };
    let mesa_words: Vec<u64> = trace::cs_words(ZS, source).iter().map(|w| w.word).collect();
    assert_eq!((started(&mesa_words), started(&b.words)), (1, 1));
    /* the FBD pointer's tag: the ZS/CRC extension and one render target
     * ("MOVE d40, #0x7FFFFFFD0003" before RUN_FRAGMENT) */
    assert_eq!(b.frame.fbd_pointer, DESC | 3);
    assert!(trace::cs_words(ZS, source).iter().any(|w| w.mnemonic == "MOVE d40, #0x7FFFFFFD0003"));
    eprintln!("mali zs frame vs the board ({source}): {} differences named", diffs.len());
    for d in &diffs {
        eprintln!("  {} +{:#x}: {}", d.block, d.at, d.cause);
    }
    assert!(failures.is_empty(), "{source}:\n{}", failures.join("\n"));
}

/// The two-draw frame with the depth test on (LESS, writes on): the board's
/// bytes, every difference named.
#[test]
fn the_depth_tested_frame_is_the_board_s() {
    check_source("depth");
}

/// The same frame with the depth test off and the D24S8 attached: the
/// descriptors of a test that always passes (ALWAYS, no write, weak early).
#[test]
fn the_frame_with_the_depth_test_off_is_the_board_s() {
    check_source("nodepth");
}

/// What decides the DCD's early-ZS modes and forward pixel kill (Mesa's
/// `pan_earlyzs` and `pan_allow_forward_pixel_to_kill`): a test that can
/// fail is force early, one that always passes weak early, and a colour
/// mask that leaves a channel out turns forward pixel kill off and reads
/// the destination (the board's masked clear, mesa-mask.pandecode lines
/// 475-508 and 551-555).
#[test]
fn a_test_that_can_fail_is_force_early_and_a_mask_reads_the_destination() {
    let early = |zs: DepthStencilState| {
        let words = build(zs, 0xf).words;
        let r = &regs_at_idvs(&words)[0];
        let f = DcdFlags0::unpack(&r[&57].to_le_bytes()).unwrap();
        (f.pixel_kill_operation, f.zs_update_operation)
    };
    let force = (PixelKill::FORCE_EARLY, PixelKill::FORCE_EARLY);
    let weak = (PixelKill::WEAK_EARLY, PixelKill::WEAK_EARLY);
    assert_eq!(early(zs_of("depth")), force);
    assert_eq!(early(DepthStencilState::OFF), weak);
    /* a stencil test that always passes still writes: weak early */
    let face =
        StencilFace { func: Func::ALWAYS, depth_pass: StencilOp::REPLACE, write_mask: 0xff, ..StencilFace::off(7) };
    let replace = DepthStencilState { stencil_test: true, faces: [face, face], ..DepthStencilState::OFF };
    assert_eq!(early(replace), weak);
    let equal = DepthStencilState { faces: [StencilFace { func: Func::EQUAL, ..face }; 2], ..replace };
    assert_eq!(early(equal), force);
}

/// Mesa's masked clear quad (glColorMask(R only), the board's
/// mesa-mask.pandecode): the Blend descriptor and DCD 0 of a draw whose
/// colour mask is R, depth and stencil off. Mesa's quad runs its own clear
/// shader; neither descriptor depends on the program.
#[test]
fn a_masked_draw_s_blend_and_dcd_are_the_board_s_masked_clear() {
    let b = build(DepthStencilState::OFF, 0x1);
    let m = trace::memory_of(ZS, "mask");
    let mesa = regs_at_idvs(&trace::cs_words(ZS, "mask").iter().map(|w| w.word).collect::<Vec<_>>());
    let ours = regs_at_idvs(&b.words);
    let reg64 = |r: &BTreeMap<u32, u32>, n: u32| u64::from(r[&n]) | u64::from(r[&(n + 1)]) << 32;
    let (mg, og) = (reg64(&mesa[0], 50) & !63, reg64(&ours[0], 50) & !63);
    let at = (og - DESC) as usize;
    assert_eq!(b.desc[at..at + Blend::LENGTH], trace::bytes(&m, mg, Blend::LENGTH)[..], "the Blend descriptor");
    assert_eq!(ours[0][&57], mesa[0][&57], "DCD 0: no forward pixel kill, weak early");
    assert_eq!(mesa[0][&57], 0x21aa, "the trace's word (mesa-mask.pandecode line 72)");
}

/// A draw that does not fit the open frame is undone: the frame's first
/// draw gets the error as it is (the stream is too small for one draw), a
/// later one [`NOT_BUILT_FRAME_ROOM`] (the caller goes on in a second
/// frame); either way the stream and the arena are as before the draw,
/// and the frame still ends.
#[test]
fn a_draw_that_does_not_fit_leaves_the_open_frame_as_it_was() {
    let (mut desc_m, mut code_m, mut spd_m) = (vec![0u8; 0x4000], vec![0u8; 0x800], vec![0u8; 0x100]);
    let ctx = Context {
        heap: HEAP_DESC,
        geometry_buffer: GEOMETRY,
        geometry_buffer_size: GEOMETRY_BUFFER_BYTES,
        sample_positions: SAMPLES,
        tiler_max_levels: 8,
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
        zs: DepthStencilState::OFF,
        colour_mask: 0xf,
        front_cw: false,
        cull: Cull::None,
        texture: None,
        vs_texture: None,
        occlusion_query: None,
        alpha_to_coverage: false,
        blend_shader: None,
    };
    let target =
        Target { gpu: TARGET, width: 64, height: 64, row_stride: 256, format: TargetFormat::Rgba8888, samples: 1 };
    /* room for the frame's start, one draw and its end, not two draws */
    let mut probe = vec![0u64; 512];
    let one = {
        let mut cs = Stream::new(&mut probe, 92);
        let mut o = begin_frame(&mut desc, &mut cs).unwrap();
        add_draw(&mut o, &mut desc, &mut cs, &ctx, &target, false, &draw).unwrap();
        cs.words().len()
    };
    desc.reset();
    let mut words = vec![0u64; one + 24 + 10];
    let mut cs = Stream::new(&mut words, 92);
    let mut open = begin_frame(&mut desc, &mut cs).unwrap();
    add_draw(&mut open, &mut desc, &mut cs, &ctx, &target, false, &draw).unwrap();
    let (top, len, layout) = (desc.top(), cs.words().len(), open.layout);
    assert_eq!(
        add_draw(&mut open, &mut desc, &mut cs, &ctx, &target, false, &draw),
        Err(FrameError::NotBuilt(NOT_BUILT_FRAME_ROOM))
    );
    assert_eq!((desc.top(), cs.words().len(), open.draws), (top, len, 1), "undone");
    assert_eq!(open.layout.depth_stencil, layout.depth_stencil);
    end_frame(&open, &mut desc, &mut cs, &ctx, &target, [0.0; 4], None).unwrap();
    /* a stream too small for even the first draw: the stream's own error */
    let mut tiny = vec![0u64; 40];
    let mut cs = Stream::new(&mut tiny, 92);
    desc.reset();
    let mut open = begin_frame(&mut desc, &mut cs).unwrap();
    let e = add_draw(&mut open, &mut desc, &mut cs, &ctx, &target, false, &draw);
    assert!(matches!(e, Err(FrameError::Cs(super::cs::CsError::StreamFull { .. }))), "{e:?}");
    assert_eq!((cs.words().len(), open.draws), (2, 0));
}

/// A draw with the colour mask off runs no fragment shader, as Mesa's
/// (the board's mesa-nocolour.pandecode, a depth-only draw LESS with
/// writes): the fragment SRT, FAU and SPD registers 0, no render target in
/// DCD 1, force early with forward pixel kill in DCD 0, the target's blend
/// off. One named difference: the disabled blend's equation word.
#[test]
fn a_draw_with_no_colour_is_the_board_s_depth_only_draw() {
    let b = build(zs_of("depth"), 0);
    let m = trace::memory_of(ZS, "nocolour");
    let mesa = regs_at_idvs(&trace::cs_words(ZS, "nocolour").iter().map(|w| w.word).collect::<Vec<_>>());
    let ours = regs_at_idvs(&b.words);
    let reg64 = |r: &BTreeMap<u32, u32>, n: u32| u64::from(r[&n]) | u64::from(r[&(n + 1)]) << 32;
    let (mr, or) = (&mesa[0], &ours[0]);
    for reg in [4, 12, 20] {
        assert_eq!((reg64(mr, reg), reg64(or, reg)), (0, 0), "d{reg}: no fragment shader");
    }
    assert!(!mr.contains_key(&18) && !or.contains_key(&18), "no varying program");
    for reg in [56, 57, 58] {
        assert_eq!(or[&reg], mr[&reg], "r{reg}");
    }
    let ours_at = |gpu: u64, len: usize| b.desc[(gpu - DESC) as usize..(gpu - DESC) as usize + len].to_vec();
    let zsd = reg64(mr, 52);
    assert_eq!(ours_at(reg64(or, 52), DepthStencil::LENGTH), trace::bytes(&m, zsd, DepthStencil::LENGTH));
    let (mg, og) = (reg64(mr, 50) & !63, reg64(or, 50) & !63);
    let (dump, ours) = (trace::bytes(&m, mg, Blend::LENGTH), ours_at(og, Blend::LENGTH));
    /* the equation (word 1): Mesa's is all zero, operand A a value v10.xml
     * does not name; ours is "replace" with no channel. Not read: the
     * blend is off (mode Off). Every other byte equal. */
    assert_eq!(dump[4..8], [0; 4], "the dump's equation word is zero");
    assert_eq!(ours[..4], dump[..4]);
    assert_eq!(ours[8..], dump[8..]);
    let eq = BlendEquation::unpack(&ours[4..8].try_into().unwrap()).unwrap();
    assert_eq!((eq.color_mask, eq.rgb.a, eq.rgb.c), (0, BlendOperandA::SRC, BlendOperandC::ZERO));
}
