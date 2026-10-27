// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The frame builder's occlusion query against the board: gl_test case 196
//! `occl` as Mesa 25.0.7 drew it on the NanoPC-T6 into a 64x64 RGBA8
//! pbuffer with a D24S8, with its queries and without (the control), cut
//! into tools/mali/board-dump/query-2026-10-20.txt.
//!
//! Between the control and the case, Mesa's dumps differ by two things and
//! by addresses only (the record's README lists them): a counted draw
//! writes `OQ` (d46) with its query's buffer, and DCD0 (r57) has Occlusion
//! Query = Predicate, 0x12183 to 0x16183. The case's first two draws are
//! the occluder, outside every query, and q0's hidden quad. Ours are built
//! with q0's buffer address: the registers each RUN_IDVS reads are Mesa's
//! for both, and our stream with the query differs from ours without it by
//! exactly those two words. The buffers after each frame are what the
//! hardware wrote: 1 in the first qword of a query some sample passed, the
//! rest 0 (what `mali_query_result` sums, backend_tests.rs).

use super::arena::Arena;
use super::cs::{Instr, Stream};
use super::frame::*;
use super::genxml::v10::*;
use super::trace;
use std::collections::BTreeMap;

const QUERY: &str = include_str!("../../tools/mali/board-dump/query-2026-10-20.txt");
const DESC: u64 = 0x7fff_fffd_0000;
const TARGET: u64 = 0x7fff_fffe_4000;
const ZS_BUF: u64 = 0x7fff_fffe_0000;
const SAMPLES: u64 = 0x7fff_ffff_f000;
const HEAP_DESC: u64 = 0x7fff_ffff_b000;
const GEOMETRY: u64 = 0x7fff_fffe_b000;
/// The buffers of q0 .. q4 in the dump (`Occlusion: 7ffffffc7000`, ...).
const Q: [u64; 5] = [0x7fff_fffc_7000, 0x7fff_fffc_6000, 0x7fff_fffc_5000, 0x7fff_fffc_4000, 0x7fff_fffc_3000];

/// The registers each RUN_IDVS of a stream reads, in order, and which
/// registers the words since the RUN_IDVS before it wrote.
fn regs_at_idvs(words: &[u64]) -> Vec<(BTreeMap<u32, u32>, Vec<u32>)> {
    let mut r = BTreeMap::new();
    let mut written = Vec::new();
    let mut out = Vec::new();
    for &w in words {
        match Instr::decode(w) {
            Ok(Instr::Move32(m)) => {
                r.insert(m.destination, m.immediate);
                written.push(m.destination);
            }
            Ok(Instr::Move48(m)) => {
                r.insert(m.destination, m.immediate as u32);
                r.insert(m.destination + 1, (m.immediate >> 32) as u32);
                written.extend([m.destination, m.destination + 1]);
            }
            Ok(Instr::RunIdvs(_)) => out.push((r.clone(), core::mem::take(&mut written))),
            _ => {}
        }
    }
    out
}

fn theirs(source: &str) -> Vec<(BTreeMap<u32, u32>, Vec<u32>)> {
    let words: Vec<u64> = trace::cs_words(QUERY, source).iter().map(|w| w.word).collect();
    regs_at_idvs(&words)
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

/// The case's first two draws, depth LESS with writes into a pbuffer (a
/// window-order target, `flip_y`): the occluder uncounted, then the hidden
/// quad counted on `counter`. The stream's words, or the counted draw's
/// refusal.
fn try_build(counter: Option<u64>) -> Result<Vec<u64>, FrameError> {
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
        let vs = upload_program(&mut code, &mut spds, &bin, meta(ShaderStage::VERTEX)).unwrap();
        let fs = upload_program(&mut code, &mut spds, &bin, meta(ShaderStage::FRAGMENT)).unwrap();
        let attributes = [VertexAttribute {
            buffer: VertexBuffer { gpu: 0x7fff_ffec_c000, size: 240 },
            offset: 0,
            stride: 40,
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
            zs: DepthStencilState { depth_func: Func::LESS, depth_write: true, ..DepthStencilState::OFF },
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
        let attachment = ZsAttachment { gpu: ZS_BUF, row_stride: 256, clear_depth: 1.0, clear_stencil: 0 };
        let mut cs = Stream::new(&mut words, 92);
        let mut open = begin_frame(&mut desc, &mut cs).unwrap();
        add_draw(&mut open, &mut desc, &mut cs, &ctx, &target, true, &draw).unwrap();
        add_draw(&mut open, &mut desc, &mut cs, &ctx, &target, true, &DrawCall { occlusion_query: counter, ..draw })?;
        end_frame(&open, &mut desc, &mut cs, &ctx, &target, [0.0, 0.0, 0.0, 1.0], Some(attachment)).unwrap();
        n = cs.words().len();
    }
    words.truncate(n);
    Ok(words)
}

fn build(counter: Option<u64>) -> Vec<u64> {
    try_build(counter).expect("case 196's draws build")
}

/// The dumps hold what the test reads: the control's two draws and the
/// case's occluder leave OQ unwritten and the flag off; q0's draw writes
/// its buffer and the flag Predicate, nothing else of DCD0 moves.
#[test]
fn the_dump_names_the_counter_and_predicate_for_the_counted_draw_only() {
    let (occl, nooq) = (theirs("mesa-occl"), theirs("mesa-nooq"));
    assert_eq!((occl.len(), nooq.len()), (2, 2), "two draws each");
    for (what, (regs, written)) in [("control 0", &nooq[0]), ("control 1", &nooq[1]), ("occluder", &occl[0])] {
        assert!(!written.contains(&46) && !regs.contains_key(&46), "{what}: no OQ");
        assert_eq!(regs[&57], 0x12183, "{what}: DCD0");
    }
    let (regs, written) = &occl[1];
    assert!(written.contains(&46) && written.contains(&47), "q0's draw writes OQ");
    assert_eq!(u64::from(regs[&46]) | u64::from(regs[&47]) << 32, Q[0], "q0's buffer");
    assert_eq!(regs[&57], 0x16183, "q0's DCD0");
    let flags = |v: u32| DcdFlags0::unpack(&v.to_le_bytes()).unwrap();
    assert_eq!(flags(regs[&57]).occlusion_query, OcclusionMode::PREDICATE);
    assert_eq!(
        format!("{:?}", DcdFlags0 { occlusion_query: OcclusionMode::DISABLED, ..flags(regs[&57]) }),
        format!("{:?}", flags(0x12183)),
        "only the occlusion mode moves"
    );
}

/// Ours with q0's buffer: each draw's DCD0 is Mesa's, q0's draw names the
/// buffer Mesa names, the occluder writes no OQ.
#[test]
fn a_counted_draw_is_the_board_s() {
    let ours = regs_at_idvs(&build(Some(Q[0])));
    let occl = theirs("mesa-occl");
    assert_eq!(ours.len(), 2);
    for (i, what) in ["the occluder", "q0's draw"].into_iter().enumerate() {
        assert_eq!(ours[i].0[&57], occl[i].0[&57], "{what}: DCD0");
        assert_eq!(ours[i].1.contains(&46), occl[i].1.contains(&46), "{what}: OQ written");
    }
    assert_eq!((ours[1].0[&46], ours[1].0[&47]), (occl[1].0[&46], occl[1].0[&47]), "q0's OQ");
}

/// The query moves our stream by exactly what it moves Mesa's: one MOVE
/// d46 after HIGH_DEPTH_CLAMP (r45) and before VARY_SIZE (r48), where
/// Mesa writes it (the dump's order), and DCD0's bit 14; every other word
/// is the same.
#[test]
fn a_counted_draw_moves_two_words() {
    let (with, without) = (build(Some(Q[0])), build(None));
    assert_eq!(with.len(), without.len() + 1, "one word more");
    let at = with.iter().position(|&w| matches!(Instr::decode(w), Ok(Instr::Move48(m)) if m.destination == 46));
    let at = at.expect("a MOVE d46");
    let mut rest = with.clone();
    rest.remove(at);
    let differ: Vec<usize> = (0..rest.len()).filter(|&i| rest[i] != without[i]).collect();
    assert_eq!(differ.len(), 1, "one word moves besides the MOVE d46: {differ:?}");
    match (Instr::decode(rest[differ[0]]), Instr::decode(without[differ[0]])) {
        (Ok(Instr::Move32(a)), Ok(Instr::Move32(b))) => {
            assert_eq!((a.destination, a.immediate ^ b.immediate), (57, 1 << 14), "DCD0's Occlusion Query")
        }
        other => panic!("not two MOVE32s: {other:?}"),
    }
    let dest = |w: u64| match Instr::decode(w) {
        Ok(Instr::Move32(m)) => Some(m.destination),
        Ok(Instr::Move48(m)) => Some(m.destination),
        _ => None,
    };
    assert_eq!((dest(with[at - 1]), dest(with[at + 1])), (Some(45), Some(48)), "between r45 and r48");
    /* the same in the dump */
    let words: Vec<u64> = trace::cs_words(QUERY, "mesa-occl").iter().map(|w| w.word).collect();
    let d46 = words.iter().rposition(|&w| dest(w) == Some(46)).unwrap();
    assert_eq!((dest(words[d46 - 1]), dest(words[d46 + 1])), (Some(45), Some(48)), "the dump's order");
}

/// A counter not on a qword is the caller's defect.
#[test]
fn a_counter_off_a_qword_is_refused() {
    assert_eq!(try_build(Some(Q[0] + 4)), Err(FrameError::Invalid(INVALID_OQ_ALIGN)));
}

/// What the hardware wrote, after each frame (the counter of each query,
/// PREDICATE mode): 1 in qword 0 of q1 (visible), of q3 after its second
/// kick (the hidden quad in the first, the cyan one in the second: a later
/// frame writes into the counter as it is, nothing resets it), of q4 (counted, then
/// covered by a later draw of the same frame); 0 in q0 and q2. No other
/// qword of the 512 bytes is written.
#[test]
fn the_board_wrote_one_qword_a_passing_query() {
    let qwords = |source: &str, va: u64| {
        let m = trace::memory_of(QUERY, source);
        let b = trace::bytes(&m, va, 512);
        b.chunks(8).map(|c| u64::from_le_bytes(c.try_into().unwrap())).collect::<Vec<u64>>()
    };
    let first = |v: &[u64]| (v[0], v[1..].iter().any(|&w| w != 0));
    let after1: Vec<_> = Q[..4].iter().map(|&va| first(&qwords("mesa-occl-frame1", va))).collect();
    assert_eq!(after1, [(0, false), (1, false), (0, false), (0, false)], "after frame 1 (q0..q3)");
    let after2: Vec<_> = Q.iter().map(|&va| first(&qwords("mesa-occl-frame2", va))).collect();
    assert_eq!(after2, [(0, false), (1, false), (0, false), (1, false), (1, false)], "after frame 2 (q0..q4)");
}
