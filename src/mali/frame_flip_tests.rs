// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The frame builder's top-down (window) target against the board: the
//! draws Mesa 25.0.7 made on the NanoPC-T6 for four modes, each into a
//! 64x48 EGL pbuffer (a window framebuffer to Mesa: FlipY, Y_0_TOP) and
//! into a 64x48 FBO (tools/mali/board-dump/flip-2026-10-19.txt, cut from
//! 0dce654a8:tools/mali/board-ref/2026-10-19-flip/mesa).
//!
//! What is compared, per mode and target: the draw's box (r42/r43, the
//! viewport cut by the scissor, mirrored for the pbuffer), the vertex
//! shader's push words (the viewport scale and offset: y negated and
//! mirrored for the pbuffer), and the front-face bit of DCD0 (r57 bit 16,
//! which the pbuffer inverts and nothing else in r57 moves). The draw's
//! other blocks do not move with the flip in the dump (the framebuffer and
//! render target descriptors differ only in the pbuffer's BGRA format and
//! the addresses), and are the FBO frame's, which `frame_tests.rs`
//! compares with the L1 trace.

use super::arena::Arena;
use super::cs::{Instr, Stream};
use super::frame::*;
use super::genxml::v10::*;
use super::trace;
use std::collections::BTreeMap;

const FLIP: &str = include_str!("../../tools/mali/board-dump/flip-2026-10-19.txt");
const DESC: u64 = 0x7fff_fffd_0000;
const W: u32 = 64;
const H: u32 = 48;
/// DCD0's "Front face CCW" (r57 bit 16).
const FRONT_FACE_CCW: u32 = 1 << 16;

/// The registers the one RUN_IDVS of a stream reads.
fn regs_at_idvs(words: &[u64]) -> BTreeMap<u32, u32> {
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
            Ok(Instr::RunIdvs(_)) => return r,
            _ => {}
        }
    }
    panic!("no RUN_IDVS in the stream")
}

/// The push block the vertex stage's FAU register (d8) names: its address
/// in the low 56 bits, its 64-bit slot count above.
fn vs_push(regs: &BTreeMap<u32, u32>) -> (u64, usize) {
    let fau = u64::from(regs[&8]) | u64::from(regs[&9]) << 32;
    (fau & ((1 << 56) - 1), (fau >> 56) as usize)
}

fn meta(stage: ShaderStage, sysvals: &'static [Sysval], push: &'static [PushWord]) -> ShaderMeta<'static> {
    ShaderMeta {
        stage,
        register_allocation: ShaderRegisterAllocation::_32_PER_THREAD,
        preload: 0,
        sysvals,
        push,
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

/// flip.c's vertex shader's push words, in the dump's order (scale.x,
/// offset.x, scale.y, offset.y, scale.z: 16, 24, 8, 12, 0.5 for the
/// viewport (8, 4, 32, 16) into the FBO).
const VS_SYSVALS: [Sysval; 2] = [Sysval::ViewportScale, Sysval::ViewportOffset];
const VS_PUSH: [PushWord; 5] = [
    PushWord::Sysval { sysval: 0, component: 0 },
    PushWord::Sysval { sysval: 1, component: 0 },
    PushWord::Sysval { sysval: 0, component: 1 },
    PushWord::Sysval { sysval: 1, component: 1 },
    PushWord::Sysval { sysval: 0, component: 2 },
];

/// The registers the draw's RUN_IDVS reads and the vertex push words.
struct Built {
    regs: BTreeMap<u32, u32>,
    push: Vec<u32>,
}

/// flip.c's frame in `mode`, cleared blue with one draw, into a top-down
/// target when `flip_y`.
fn build(mode: &str, flip_y: bool) -> Built {
    let (mut desc_m, mut code_m, mut spd_m, mut ctx_m) =
        (vec![0u8; 0x4000], vec![0u8; 0x800], vec![0u8; 0x100], vec![0u8; 0x200]);
    let mut words = vec![0u64; 512];
    let n;
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
        let vs_meta = meta(ShaderStage::VERTEX, &VS_SYSVALS, &VS_PUSH);
        let vs = upload_program(&mut code, &mut spds, &bin, vs_meta).unwrap();
        let fs = upload_program(&mut code, &mut spds, &bin, meta(ShaderStage::FRAGMENT, &[], &[])).unwrap();
        let attributes = [VertexAttribute {
            buffer: VertexBuffer { gpu: 0x7fff_ffec_c000, size: 48 },
            offset: 0,
            stride: 8,
            components: 2,
            instance_divisor: 0,
        }];
        let (viewport, scissor) = match mode {
            "viewport" => ((8, 4, 32, 16), None),
            "scissor_draw" => ((0, 0, W as i32, H as i32), Some([8, 4, 16, 20])),
            _ => ((0, 0, W as i32, H as i32), None),
        };
        let input = FrameInput {
            zs: None,
            target: Target {
                gpu: 0x7fff_fffe_4000,
                width: W,
                height: H,
                row_stride: 4 * W,
                format: TargetFormat::Rgba8888,
                samples: 1,
            },
            clear_rgba: Some([0.0, 0.0, 1.0, 1.0]),
            draw: Some(DrawCall {
                zs: DepthStencilState::OFF,
                colour_mask: 0xf,
                front_cw: mode == "frontcw",
                cull: Cull::None,
                texture: None,
                vs_texture: None,
                occlusion_query: None,
                alpha_to_coverage: false,
                blend_shader: None,
                vs: &vs,
                fs: &fs,
                primitive: Topology::Triangles,
                first_vertex: 0,
                vertex_count: if mode == "tri" || mode == "frontcw" { 3 } else { 6 },
                instance_count: 1,
                indices: None,
                attributes: &attributes,
                viewport: Viewport {
                    x: viewport.0,
                    y: viewport.1,
                    width: viewport.2,
                    height: viewport.3,
                    near: 0.0,
                    far: 1.0,
                },
                scissor,
                line_width: 1.0,
                flip_y,
                blend_color: [0.0; 4],
                blend: None,
                polygon_offset: None,
                sample_mask: 0xf,
                vs_uniforms: &[],
                fs_uniforms: &[],
            }),
        };
        let mut cs = Stream::new(&mut words, 92);
        build_frame(&mut desc, &mut cs, &ctx, &input).expect("flip.c's frame builds");
        n = cs.words().len();
    }
    let regs = regs_at_idvs(&words[..n]);
    let (at, slots) = vs_push(&regs);
    let off = (at - DESC) as usize;
    let push = desc_m[off..off + 8 * slots].chunks(4).map(|b| u32::from_le_bytes(b.try_into().unwrap())).collect();
    Built { regs, push }
}

/// The dump's registers at the draw and its vertex push words.
fn mesa(mode: &str, target: &str) -> Built {
    let source = format!("mesa-{mode}-{target}");
    let words: Vec<u64> = trace::cs_words(FLIP, &source).iter().map(|w| w.word).collect();
    let regs = regs_at_idvs(&words);
    let (at, slots) = vs_push(&regs);
    let m = trace::memory_of(FLIP, &source);
    let push = trace::bytes(&m, at, 8 * slots).chunks(4).map(|b| u32::from_le_bytes(b.try_into().unwrap())).collect();
    Built { regs, push }
}

/// Each mode into the top-down target and into the FBO: the board's box,
/// viewport push words and front face; DCD0 moves only in its front-face
/// bit, in the dump and in ours.
#[test]
fn a_top_down_target_s_draw_is_the_board_s_pbuffer_draw() {
    for mode in ["tri", "frontcw", "viewport", "scissor_draw"] {
        for (target, flip_y) in [("fbo", false), ("pb", true)] {
            let (ours, theirs) = (build(mode, flip_y), mesa(mode, target));
            for r in [42, 43] {
                assert_eq!(ours.regs[&r], theirs.regs[&r], "{mode} {target}: r{r} (the box)");
            }
            assert_eq!(ours.push, theirs.push, "{mode} {target}: the vertex push words");
            assert_eq!(
                ours.regs[&57] & FRONT_FACE_CCW,
                theirs.regs[&57] & FRONT_FACE_CCW,
                "{mode} {target}: front face"
            );
        }
        let (fbo, pb) = (build(mode, false), build(mode, true));
        let (mfbo, mpb) = (mesa(mode, "fbo"), mesa(mode, "pb"));
        assert_eq!(mfbo.regs[&57] ^ mpb.regs[&57], FRONT_FACE_CCW, "{mode}: the dump's DCD0 moved elsewhere");
        assert_eq!(fbo.regs[&57] ^ pb.regs[&57], FRONT_FACE_CCW, "{mode}: our DCD0 moved elsewhere");
    }
    /* the dump holds what the test reads: viewport (8, 4, 32, 16) is y
     * [4, 20) in the FBO and [28, 44) in the pbuffer; scale y 8 and offset
     * y 12 in the FBO, -8 and 36 (48 - 12) in the pbuffer */
    let w = |s| trace::cs_words(FLIP, s);
    assert!(w("mesa-viewport-fbo").iter().any(|w| w.mnemonic == "MOVE32 r42, #0x40008"));
    assert!(w("mesa-viewport-pb").iter().any(|w| w.mnemonic == "MOVE32 r42, #0x1C0008"));
    assert!(w("mesa-viewport-pb").iter().any(|w| w.mnemonic == "MOVE32 r43, #0x2B0027"));
    let pb = mesa("viewport", "pb");
    assert_eq!(pb.push[2..4], [(-8.0f32).to_bits(), 36.0f32.to_bits()]);
}
