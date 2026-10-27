// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The frame builder's draw of a fragment shader that discards (#1384 S6,
//! docs/notes/mali-discard.md): what a `can_discard` program moves in the
//! draw, as Mesa 26.2.1 builds it on v10 (`csf_emit_draw_state`,
//! pan_csf.c:1433-1673, with `pan_earlyzs_analyze`, pan_earlyzs.c:31-115,
//! and `panfrost_fs_required`, pan_cmdstream.h:199-235):
//!
//! * DCD0's `Allow Forward Pixel To Kill` is off (`can_fpk` is false for a
//!   discard, pan_compiler.c:269-271) and `Shader Modifies Coverage` on
//!   (pan_csf.c:1607-1612) in every discarding draw;
//! * the ZS update is `Force Late` only when the draw writes depth or
//!   stencil or an occlusion query counts it (`late_coverage &&
//!   writes_zs_or_oq`, pan_earlyzs.c:61-76), the kill unchanged (:78-81);
//! * a draw with no colour channel written still runs the shader (`sidefx`,
//!   pan_cmdstream.h:205-208: femtovg's colour-masked stencil pass), with
//!   the render target mask of its output and the blend descriptor of a
//!   target not written.
//!
//! The one board recording of a discarding draw, gl_test case 213 discloop
//! (colour on, no depth or stencil, tools/mali/board-dump/
//! discloop-2026-10-23.txt, cut from 0dce654a8:tools/mali/board-ref/
//! 2026-10-23-s6-discard/mesa), pins the first row; the other rows are the
//! source's predictions (section 6 of the note), no recording yet.

use super::arena::Arena;
use super::cs::{Instr, Stream};
use super::frame::*;
use super::genxml::v10::*;
use super::trace;
use std::collections::BTreeMap;

const DISCLOOP: &str = include_str!("../../tools/mali/board-dump/discloop-2026-10-23.txt");
const DESC: u64 = 0x7fff_fffd_0000;
const TARGET: u64 = 0x7fff_fffe_4000;
const ZS_BUF: u64 = 0x7fff_fffe_0000;
const SAMPLES: u64 = 0x7fff_ffff_f000;
const HEAP_DESC: u64 = 0x7fff_ffff_b000;
const GEOMETRY: u64 = 0x7fff_fffe_b000;

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

fn meta(stage: ShaderStage, can_discard: bool) -> ShaderMeta<'static> {
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
        can_discard,
        writes_depth: false,
        writes_stencil: false,
        writes_coverage: false,
        writes_global: false,
        ubo: &[],
    }
}

/// One draw's state.
#[derive(Clone, Copy)]
struct State {
    discard: bool,
    zs: DepthStencilState,
    colour_mask: u8,
    occlusion_query: Option<u64>,
}

const PLAIN: State = State { discard: false, zs: DepthStencilState::OFF, colour_mask: 0xf, occlusion_query: None };

/// A frame of one draw of six vertices into a 64x64 RGBA8 target with a
/// D24S8 attachment: the descriptors and the registers its RUN_IDVS reads.
fn build(s: State) -> (Vec<u8>, BTreeMap<u32, u32>) {
    let (mut desc_m, mut code_m, mut spd_m, mut ctx_m) =
        (vec![0u8; 0x4000], vec![0u8; 0x800], vec![0u8; 0x100], vec![0u8; 0x200]);
    let mut words = vec![0u64; 512];
    let n;
    {
        let mut ctx_a = Arena::new("context", HEAP_DESC, &mut ctx_m).unwrap();
        let heap = tiler_heap_descriptor(&mut ctx_a, 0x8000_00a0_0000, 2 << 20).unwrap();
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
        let vs = upload_program(&mut code, &mut spds, &bin, meta(ShaderStage::VERTEX, false)).unwrap();
        let fs = upload_program(&mut code, &mut spds, &bin, meta(ShaderStage::FRAGMENT, s.discard)).unwrap();
        let attributes = [VertexAttribute {
            buffer: VertexBuffer { gpu: 0x7fff_ffec_c000, size: 144 },
            offset: 0,
            stride: 24,
            components: 4,
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
            flip_y: true,
            blend_color: [0.0; 4],
            blend: None,
            polygon_offset: None,
            sample_mask: 0xf,
            vs_uniforms: &[],
            fs_uniforms: &[],
            zs: s.zs,
            colour_mask: s.colour_mask,
            front_cw: false,
            cull: Cull::None,
            texture: None,
            vs_texture: None,
            occlusion_query: s.occlusion_query,
            alpha_to_coverage: false,
            blend_shader: None,
        };
        let target =
            Target { gpu: TARGET, width: 64, height: 64, row_stride: 256, format: TargetFormat::Rgba8888, samples: 1 };
        let attachment = ZsAttachment { gpu: ZS_BUF, row_stride: 256, clear_depth: 1.0, clear_stencil: 0 };
        let mut cs = Stream::new(&mut words, 92);
        let mut open = begin_frame(&mut desc, &mut cs).unwrap();
        add_draw(&mut open, &mut desc, &mut cs, &ctx, &target, true, &draw).unwrap();
        end_frame(&open, &mut desc, &mut cs, &ctx, &target, [1.0, 0.0, 1.0, 1.0], Some(attachment)).unwrap();
        n = cs.words().len();
    }
    words.truncate(n);
    let regs = regs_at_idvs(&words);
    assert_eq!(regs.len(), 1, "one RUN_IDVS");
    (desc_m, regs.into_iter().next().unwrap())
}

fn dcd0(r: &BTreeMap<u32, u32>) -> DcdFlags0 {
    DcdFlags0::unpack(&r[&57].to_le_bytes()).unwrap()
}

/// The early-z and coverage bits of a draw's DCD0 (forward pixel kill, the
/// kill, the update, shader modifies coverage).
fn modes(r: &BTreeMap<u32, u32>) -> (bool, PixelKill, PixelKill, bool) {
    let f = dcd0(r);
    (f.allow_forward_pixel_to_kill, f.pixel_kill_operation, f.zs_update_operation, f.shader_modifies_coverage)
}

/// femtovg's stencil-stroke pass 1 (gl_test case 38): stencil ALWAYS,
/// INCR on pass, every bit written; the depth test off.
fn stencil_incr() -> DepthStencilState {
    let face = StencilFace {
        func: Func::ALWAYS,
        fail: StencilOp::KEEP,
        depth_fail: StencilOp::KEEP,
        depth_pass: StencilOp::INCR_WRAP,
        value_mask: 0xff,
        write_mask: 0xff,
        reference: 0,
    };
    DepthStencilState { stencil_test: true, faces: [face; 2], ..DepthStencilState::OFF }
}

/// Pass 2 of the same: EQUAL 0, KEEP everywhere (the stencil read, not
/// written).
fn stencil_equal() -> DepthStencilState {
    let face = StencilFace { func: Func::EQUAL, ..StencilFace::off(0) };
    DepthStencilState {
        stencil_test: true,
        faces: [StencilFace { value_mask: 0xff, ..face }; 2],
        ..DepthStencilState::OFF
    }
}

/// `util_writes_depth_stencil` as the frame reads the state: a depth
/// write with a test that is not NEVER, a stencil face with a write mask
/// and an op other than KEEP.
#[test]
fn writes_zs_is_util_writes_depth_stencil() {
    assert!(!DepthStencilState::OFF.writes_zs());
    assert!(DepthStencilState { depth_func: Func::LESS, depth_write: true, ..DepthStencilState::OFF }.writes_zs());
    assert!(!DepthStencilState { depth_func: Func::LESS, depth_write: false, ..DepthStencilState::OFF }.writes_zs());
    assert!(!DepthStencilState { depth_func: Func::NEVER, depth_write: true, ..DepthStencilState::OFF }.writes_zs());
    assert!(stencil_incr().writes_zs());
    assert!(!stencil_equal().writes_zs());
    let masked =
        DepthStencilState { faces: [StencilFace { write_mask: 0, ..stencil_incr().faces[0] }; 2], ..stencil_incr() };
    assert!(!masked.writes_zs());
}

/// Colour on, no depth or stencil (case 213's draw): forward pixel kill
/// off and the coverage marked modified, the kill and the update weak
/// early as for the plain draw; those two bits are all that moves in DCD0,
/// and DCD1 and the tiler flags do not move.
#[test]
fn a_discarding_draw_turns_forward_pixel_kill_off_and_marks_the_coverage() {
    let (_, plain) = build(PLAIN);
    let (_, ours) = build(State { discard: true, ..PLAIN });
    assert_eq!(modes(&plain), (true, PixelKill::WEAK_EARLY, PixelKill::WEAK_EARLY, false));
    assert_eq!(modes(&ours), (false, PixelKill::WEAK_EARLY, PixelKill::WEAK_EARLY, true));
    assert_eq!(plain[&57] ^ ours[&57], 1 | 1 << 20, "r57 plain {:#x} ours {:#x}", plain[&57], ours[&57]);
    assert_eq!((plain[&56], plain[&58]), (ours[&56], ours[&58]));
    assert!(!dcd0(&ours).alpha_to_coverage);
}

/// The board's Mesa 25.0.7 drew case 213 with these flags (one draw, colour
/// on, depth and stencil off, into a pbuffer): its DCD0 is ours to the bit,
/// and its DCD1 and tiler flags too.
#[test]
fn the_discloop_draw_s_flags_are_the_board_s() {
    let words: Vec<u64> = trace::cs_words(DISCLOOP, "mesa-discloop").iter().map(|w| w.word).collect();
    let theirs = regs_at_idvs(&words);
    assert_eq!(theirs.len(), 1, "case 213 is one draw");
    let (_, ours) = build(State { discard: true, ..PLAIN });
    for reg in [56, 57, 58] {
        assert_eq!(ours[&reg], theirs[0][&reg], "r{reg}: Mesa {:#x}, ours {:#x}", theirs[0][&reg], ours[&reg]);
    }
    assert_eq!(modes(&theirs[0]), (false, PixelKill::WEAK_EARLY, PixelKill::WEAK_EARLY, true));
}

/// The table of docs/notes/mali-discard.md section 2.2: the ZS update is
/// Force Late when the draw writes depth or stencil (femtovg's stencil
/// INCR pass, a depth write) or an occlusion query counts it, else the
/// kill's mode; the kill never moves (Weak Early where the test always
/// passes, Force Early otherwise).
#[test]
fn the_zs_update_is_late_only_where_the_draw_writes_zs_or_an_oq_counts() {
    let depth = DepthStencilState { depth_func: Func::LESS, depth_write: true, ..DepthStencilState::OFF };
    let depth_read = DepthStencilState { depth_func: Func::LESS, depth_write: false, ..DepthStencilState::OFF };
    let rows: [(State, (bool, PixelKill, PixelKill, bool)); 6] = [
        /* no write, always passes */
        (State { discard: true, ..PLAIN }, (false, PixelKill::WEAK_EARLY, PixelKill::WEAK_EARLY, true)),
        /* no write, a test: stencil EQUAL with KEEP, a depth test without the write */
        (
            State { discard: true, zs: stencil_equal(), ..PLAIN },
            (false, PixelKill::FORCE_EARLY, PixelKill::FORCE_EARLY, true),
        ),
        (
            State { discard: true, zs: depth_read, ..PLAIN },
            (false, PixelKill::FORCE_EARLY, PixelKill::FORCE_EARLY, true),
        ),
        /* written, always passes: the stencil INCR pass */
        (
            State { discard: true, zs: stencil_incr(), ..PLAIN },
            (false, PixelKill::WEAK_EARLY, PixelKill::FORCE_LATE, true),
        ),
        /* written, a test */
        (State { discard: true, zs: depth, ..PLAIN }, (false, PixelKill::FORCE_EARLY, PixelKill::FORCE_LATE, true)),
        /* an occlusion query counting */
        (
            State { discard: true, occlusion_query: Some(0x7fff_fffc_7000), ..PLAIN },
            (false, PixelKill::WEAK_EARLY, PixelKill::FORCE_LATE, true),
        ),
    ];
    for (k, (s, want)) in rows.iter().enumerate() {
        let (_, ours) = build(*s);
        assert_eq!(modes(&ours), *want, "row {k}");
        /* the same state without the discard: the plain modes */
        let (_, plain) = build(State { discard: false, ..*s });
        let (_, kill, update, _) = modes(&plain);
        assert_eq!(kill, want.1, "row {k}: the kill does not move");
        assert_eq!(update, kill, "row {k}: the plain update is the kill's mode");
        assert!(dcd0(&plain).allow_forward_pixel_to_kill && !dcd0(&plain).shader_modifies_coverage, "row {k}");
    }
}

/// No colour channel written (femtovg's pass 3 of case 38, a stencil
/// clear under the stroke): the discarding draw still runs the fragment
/// shader (its SPD set, the render target mask of its output), with the
/// blend descriptor of a target not written (the same bytes the draw
/// without a discard has), forward pixel kill off; the draw without a
/// discard runs no shader (SPD 0, RT mask 0, forward pixel kill on).
#[test]
fn a_discarding_draw_with_no_colour_written_runs_the_shader() {
    let mask0 = State { colour_mask: 0, ..PLAIN };
    let (desc, ours) = build(State { discard: true, ..mask0 });
    let (none_desc, none) = build(mask0);
    let (_, colour) = build(State { discard: true, ..PLAIN });
    let spd = |r: &BTreeMap<u32, u32>| u64::from(r[&20]) | u64::from(r[&21]) << 32;
    assert_ne!(spd(&ours), 0, "the shader runs");
    assert_eq!(spd(&ours), spd(&colour), "the same program as with colour");
    assert_eq!(spd(&none), 0, "no shader without the discard");
    let rt = |r: &BTreeMap<u32, u32>| DcdFlags1::unpack(&r[&58].to_le_bytes()).unwrap().render_target_mask;
    assert_eq!((rt(&ours), rt(&none)), (1, 0));
    assert_eq!(modes(&ours), (false, PixelKill::WEAK_EARLY, PixelKill::WEAK_EARLY, true));
    assert_eq!(modes(&none), (true, PixelKill::FORCE_EARLY, PixelKill::FORCE_EARLY, false));
    /* the blend descriptor: the disabled one, as the mask-0 draw's */
    let blend = |r: &BTreeMap<u32, u32>| ((u64::from(r[&50]) | u64::from(r[&51]) << 32) & !63) - DESC;
    let (at, none_at) = (blend(&ours) as usize, blend(&none) as usize);
    assert_eq!(desc[at..at + Blend::LENGTH], none_desc[none_at..none_at + Blend::LENGTH]);
    /* and not the colour draw's, which is enabled */
    let (colour_desc, colour) = build(State { discard: true, ..PLAIN });
    let colour_at = blend(&colour) as usize;
    assert_ne!(desc[at..at + Blend::LENGTH], colour_desc[colour_at..colour_at + Blend::LENGTH]);
}

/// The frame no longer refuses a discarding program; it still refuses one
/// that writes depth, stencil, coverage or memory.
#[test]
fn check_draw_takes_a_discarding_program_and_refuses_the_other_effects() {
    let (_, ours) = build(State { discard: true, ..PLAIN });
    assert!(ours.contains_key(&57));
    assert!(NOT_BUILT_FS_EFFECTS.contains("writes depth, stencil, coverage or memory"));
    assert!(!NOT_BUILT_FS_EFFECTS.contains("discard"));
}
