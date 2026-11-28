// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Fragment-shader `discard` with four colour outputs on the V3D online
//! compiler (RC14, plan #368, item discard4-v3d): a discarded pixel must
//! leave all four targets at what they held.
//!
//! This rig cannot observe framebuffer pixels: the GL-level rigs
//! (es3_mrt4_v3d.rs, es3_mrt4_gl_v3d.rs) hand jobs to a stand-in that never
//! runs a fragment shader. What it can see is the QPU interpreter's outcome
//! for the compiled program (the rig of es3_mrt4_out_v3d.rs and
//! es3_fragdepth_v3d_sim.rs): per lane the multisample flag at the end
//! (`Outcome::msf`, 0 = the lane is discarded; the TLB masks a lane whose
//! flag is 0 for every render target, Mesa 26.2.1 nir_to_vir.c:3771-3792
//! `setmsf`), and the TLB writes of the four colour groups. A pixel keeps its
//! clear colour exactly when its lane's flag is 0, so "cleared colour in the
//! discarded pixels, written colour elsewhere" is derived from the flags and
//! the writes together, on each of the four targets. Pinned:
//!
//! 1. an unconditional `discard` leaves every lane's flag at 0, the four
//!    colour groups still emitted (RT 0..3, in order);
//! 2. a conditional `discard` on `gl_FragCoord.x` clears the flag of exactly
//!    the lanes it names, and the other lanes carry the written colour of
//!    every one of the four targets;
//! 3. the compiled program's discard state (`uses_discard`, `writes_z`,
//!    `disable_ez`: the late-Z answer of Mesa's `can_earlyz_with_discard`) and
//!    the `rt_mask` are the same shape with four outputs as with one
//!    (`rt_mask` 0xf against 0x1); the TLB config words carry RT 0..3.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_mrt4_discard_v3d
//! ```
#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-v3d", not(feature = "report-es2")))]

use std::sync::Mutex;
use yakogl::shaderc::online::arena::Arena;
use yakogl::shaderc::online::v3d::chain::{compile_program, Prim, Target};
use yakogl::shaderc::online::v3d::sim::{self, Env, FsInputs, Inputs, LANES};
use yakogl::shaderc::online::{lower, sema};

/// The compiler keeps its output in one arena that every compile resets.
static COMPILING: Mutex<()> = Mutex::new(());

const VS: &str = "#version 300 es\nin vec4 p;\nvoid main() { gl_Position = p; }\n";

/// What one run of a fragment shader shows.
struct Run {
    rt_mask: u8,
    uses_discard: bool,
    writes_z: bool,
    disable_ez: bool,
    msf: [u32; LANES],
    /// The TLB writes: the TLBU config (`None` for TLB) and lane 0's word.
    writes: Vec<(Option<u32>, u32)>,
}

/// Lane `l` runs the pixel at x = `l` (FXCD, the pixel centre as a float), y = 0.
fn run_fs(fs: &str) -> Run {
    let _g = COMPILING.lock().unwrap_or_else(|e| e.into_inner());
    let mut mem = vec![0u8; 16 << 20];
    let a = Arena::new(&mut mem);
    let vs = a.alloc_str(VS).expect("arena");
    let fs_s = a.alloc_str(fs).expect("arena");
    let v = lower::compile_to_ir(&a, sema::Stage::Vertex, vs).unwrap_or_else(|e| panic!("VS: {}", e.msg));
    let f = lower::compile_to_ir(&a, sema::Stage::Fragment, fs_s).unwrap_or_else(|e| panic!("FS: {}\n{fs}", e.msg));
    let p = compile_program(&v.module, &f.module, Prim::Triangles, Target { vpm_size: 65536 }, &a)
        .unwrap_or_else(|e| panic!("compile: {e:?}\n{fs}"));
    let uniforms: Vec<u32> =
        p.fs.code
            .uniforms
            .iter()
            .map(|u| match u.contents {
                0 => u.data,
                c => panic!("uniform contents {c} not modelled by this test"),
            })
            .collect();
    assert_eq!(p.fs.num_inputs, 0, "the shaders here read no varying");
    let inputs = FsInputs {
        w: [1.0f32.to_bits(); LANES],
        w_centroid: [1.0f32.to_bits(); LANES],
        z: [0; LANES],
        x: core::array::from_fn(|l| (l as f32 + 0.5).to_bits()),
        y: [0; LANES],
        back_facing: [false; LANES],
        varyings: Vec::new(),
    };
    let env = Env { threads: p.fs.threads, ..Env::default() };
    let o = sim::run(p.fs.code.words, &uniforms, &Inputs::Fragment(Box::new(inputs)), &env)
        .unwrap_or_else(|e| panic!("run: {e}\n{fs}"));
    Run {
        rt_mask: p.fs.rt_mask,
        uses_discard: p.fs.uses_discard,
        writes_z: p.fs.writes_z,
        disable_ez: p.fs.disable_ez,
        msf: o.msf,
        writes: o.tlb.iter().map(|w| (w.config, w.value[0].unwrap_or(0xdead_beef))).collect(),
    }
}

/// A shader of `n` vec4 outputs (locations 0..n), output `i` written with
/// (i+1)/8 in every component, after `pre` (the discard).
fn fs_of(n: usize, pre: &str) -> String {
    let mut s = String::from("#version 300 es\nprecision highp float;\n");
    for i in 0..n {
        s += &format!("layout(location = {i}) out vec4 o{i};\n");
    }
    s += "void main() {\n";
    s += pre;
    for i in 0..n {
        s += &format!("o{i} = vec4({}.0 / 8.0);\n", i + 1);
    }
    s + "}\n"
}

/// The TLB config words of the colour groups, in write order (a group's first
/// write goes through TLBU). The pass-through Z write a discarding program
/// makes (a TLBU write whose config selects slot 0 of `(7 - rt) << 3`, not a colour target) is left out.
fn configs(r: &Run) -> Vec<u32> {
    r.writes.iter().filter_map(|w| w.0).filter(|c| (c >> 3) & 7 != 0).collect()
}

/// RT of a config word (`(7 - rt) << 3`).
fn rt_of(conf: u32) -> u32 {
    7 - ((conf >> 3) & 7)
}

const UNCONDITIONAL: &str = "if (gl_FragCoord.x > -1.0) discard;\n";
/// Lanes 0..7 (x + 0.5 < 8.0) are discarded, lanes 8..15 draw.
const CONDITIONAL: &str = "if (gl_FragCoord.x < 8.0) discard;\n";

#[test]
fn unconditional_discard_clears_every_lane_flag_and_keeps_four_groups() {
    for pre in [UNCONDITIONAL, "discard;\n"] {
        let r = run_fs(&fs_of(4, pre));
        assert_eq!(r.msf, [0; LANES], "every lane discarded: {pre}");
        // the pixels keep their clear colour on all four targets: nothing
        // survives the flag, whatever the writes are
        let rts: Vec<u32> = configs(&r).into_iter().map(rt_of).collect();
        assert!(rts.contains(&3), "RT 3 is part of the discarding program: {rts:?}");
    }
}

#[test]
fn conditional_discard_keeps_clear_in_discarded_pixels_on_all_four_targets() {
    let r = run_fs(&fs_of(4, CONDITIONAL));
    let want: [u32; LANES] = core::array::from_fn(|l| u32::from(l >= 8));
    assert_eq!(r.msf, want, "lanes 0..7 discarded, 8..15 kept");
    // one group per target, RT 0..3 in order; RT 0's f16 vec4 is the one
    // config the compiler leaves out (`~0`), so its group has no TLBU word
    let rts: Vec<u32> = configs(&r).into_iter().map(rt_of).collect();
    assert_eq!(rts, [1, 2, 3], "{:#x?}", r.writes);
    // the colour the kept lanes write on each target: o<i> = (i+1)/8, f16
    // pairs (r,g) and (b,a)
    let half = |i: u32| -> u32 { [0x3000, 0x3400, 0x3600, 0x3800][i as usize] };
    let colour = r.writes.iter().skip_while(|w| w.0.is_some_and(|c| (c >> 3) & 7 == 0));
    let data: Vec<u32> = colour.map(|w| w.1).collect();
    // two words a target
    assert_eq!(data.len(), 8, "{:#x?}", r.writes);
    for i in 0..4u32 {
        let h = half(i);
        assert_eq!(data[2 * i as usize], h | h << 16, "RT {i} r,g");
        assert_eq!(data[2 * i as usize + 1], h | h << 16, "RT {i} b,a");
    }
    // pixel result per target: written where the flag is set, cleared else
    for rt in 0..4 {
        for l in 0..LANES {
            let drawn = r.msf[l] != 0;
            assert_eq!(drawn, l >= 8, "RT {rt} lane {l}");
        }
    }
}

#[test]
fn discard_job_state_is_the_same_with_four_outputs_as_with_one() {
    for pre in [CONDITIONAL, UNCONDITIONAL] {
        let one = run_fs(&fs_of(1, pre));
        let four = run_fs(&fs_of(4, pre));
        assert_eq!(one.rt_mask, 0x1);
        assert_eq!(four.rt_mask, 0xf);
        assert!(one.uses_discard && four.uses_discard, "{pre}");
        assert_eq!(
            (one.writes_z, one.disable_ez),
            (four.writes_z, four.disable_ez),
            "late-Z / Z-write answer moves with the output count: {pre}"
        );
        assert!(four.writes_z && four.disable_ez, "a discard writes the pass-through Z, tests late");
        assert_eq!(one.msf, four.msf, "the same lanes are discarded: {pre}");
    }
    // and without a discard neither shape disables early Z
    let one = run_fs(&fs_of(1, ""));
    let four = run_fs(&fs_of(4, ""));
    assert!(!one.uses_discard && !four.uses_discard);
    assert_eq!((one.writes_z, one.disable_ez), (four.writes_z, four.disable_ez));
    assert_eq!(four.msf, [1; LANES]);
}
