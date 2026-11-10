// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The value a V3D fragment program writes for `gl_FragDepth`, on the QPU
//! interpreter (`sim::run`): what the host model can vouch for of dEQP-GLES3
//! `shaders.fragdepth.write.*` and `.compare.*` beyond the build and the code
//! shape, which tests/es3_fragdepth_v3d.rs pins for the eight dEQP writers.
//!
//! The model is the TLB write itself: the interpreter records each TLBU
//! write as its configuration word and one value per lane ([`sim::TlbWrite`]),
//! so a shader's depth is observable as the f32 bits of the write whose
//! configuration is the per-pixel depth specifier (`select::TLB_Z_PER_PIXEL`,
//! 0xffffff8c). Pinned here, lane by lane: a constant depth is its exact f32
//! (0.25 is 0x3e800000, the first reference draw of the V3D words report);
//! a depth computed from the payload Z, a varying and a uniform is the
//! expression's value; a depth outside [0, 1] reaches the TLB as written,
//! because neither Mesa's compiler (nir_to_vir.c:2097-2108, Mesa 26.2.1) nor
//! this one clamps it -- GL's clamp of the written depth to [0, 1] is past the
//! QPU, in the TLB, and is the board's to measure (select.rs); a clamp the
//! shader itself writes is honoured; a depth written in one arm of an `if`
//! only is the arm's value where the arm ran (elsewhere undefined, GLSL ES
//! 3.00 7.2, and not pinned); and a lane that discards still carries its
//! depth, with its multisample flag 0 (`Outcome::msf`), the one form of
//! "fragment rejected" the model has.
//!
//! What the model lacks: the interpreters (sim.rs, sim71.rs) have no depth
//! buffer. `FsInputs` carries the payload Z the FEP hands the shader, not a
//! TLB Z to compare with, `Env` has no depth function, and `Outcome` has the
//! TLB writes and the multisample flags only; the back end's host rig
//! (src/v3d/backend_tests.rs) sees jobs, shader records and config words,
//! not pixels. So the depth test itself (GL_LESS keeping or rejecting a
//! fragment by the written value, `fragdepth.compare.*`) is not modelled
//! here, and no test below pretends to.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_fragdepth_v3d_sim
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-v3d", not(feature = "report-es2")))]

use std::sync::Mutex;
use yakogl::shaderc::online::arena::Arena;
use yakogl::shaderc::online::reflect::Reflection;
use yakogl::shaderc::online::v3d::chain::{compile_program, Prim, Target, V3dProgram};
use yakogl::shaderc::online::v3d::listing::write_instr;
use yakogl::shaderc::online::v3d::select::TLB_Z_PER_PIXEL;
use yakogl::shaderc::online::v3d::sim::{self, Env, FsInputs, Inputs, Outcome, Varying, LANES};
use yakogl::shaderc::online::{lower, sema};

/// The compiler keeps its output in one arena that every compile resets.
static COMPILING: Mutex<()> = Mutex::new(());

const TARGET: Target = Target { vpm_size: 65536 };

const VS_COORD: &str = "#version 300 es\n\
in highp vec4 a_position;\nin highp vec2 a_coord;\nout highp vec2 v_coord;\n\
void main (void)\n{\n    gl_Position = a_position;\n    v_coord = a_coord;\n}\n";

/// A fragment shader with the uniforms the tests set, `v_coord` from the
/// vertex shader above and a colour output, round `body` (its main).
fn fs(body: &str) -> String {
    format!(
        "#version 300 es\nuniform highp vec4 u_scale;\nuniform highp float u_depth;\n\
         in highp vec2 v_coord;\nlayout(location = 0) out mediump vec4 o_color;\n\
         void main (void)\n{{\n    o_color = vec4(1.0, 0.0, 0.0, 1.0);\n{body}\n}}\n"
    )
}

fn listing(p: &V3dProgram<'_>) -> Vec<String> {
    p.fs.code
        .instrs
        .iter()
        .map(|i| {
            let mut s = String::new();
            write_instr(&mut s, i).unwrap();
            s
        })
        .collect()
}

/// `f` on the compiled program of `VS_COORD` + `fs`, with the fragment
/// stage's reflection (the uniform block's layout).
fn with_program<R>(fs: &str, f: impl FnOnce(&V3dProgram<'_>, &Reflection<'_>) -> R) -> R {
    let _g = COMPILING.lock().unwrap_or_else(|e| e.into_inner());
    let mut mem = vec![0u8; 16 << 20];
    let a = Arena::new(&mut mem);
    let vs = a.alloc_str(VS_COORD).expect("arena");
    let fs_ = a.alloc_str(fs).expect("arena");
    let v = lower::compile_to_ir(&a, sema::Stage::Vertex, vs).map_err(|e| e.msg.to_string()).expect("the VS lowers");
    let f_ = lower::compile_to_ir(&a, sema::Stage::Fragment, fs_)
        .unwrap_or_else(|e| panic!("the FS lowers: {}\n{fs}", e.msg));
    let p = compile_program(&v.module, &f_.module, Prim::Triangles, TARGET, &a)
        .unwrap_or_else(|e| panic!("the program compiles: {e:?}\n{fs}"));
    f(&p, &f_.reflection)
}

/// The run of `fs` for one quad of 16 pixels whose `v_coord` is `coords`
/// (lane n: `coords[n]`) and whose payload Z (the FEP's, `gl_FragCoord.z`)
/// is `z`, with the uniforms `uniforms` (name, dwords) and W = 1.
fn run_fs(fs: &str, uniforms: &[(&str, &[u32])], coords: [[f32; 2]; LANES], z: f32) -> Outcome {
    with_program(fs, |p, refl| {
        let ops = listing(p);
        assert!(p.fs.writes_z && p.fs.disable_ez, "a depth writer: writes_z and disable_ez\n{ops:#?}");
        /* The uniform block, laid out as the reflection says (dword
         * offsets): QUNIFORM_UNIFORM (contents 1) reads dword `data` of it;
         * a constant (contents 0) is its data. */
        let mut block = vec![0u32; usize::from(refl.uniform_dwords)];
        for (name, words) in uniforms {
            let Some(row) = refl.uniforms.iter().find(|r| r.name == *name) else { continue };
            for (k, w) in words.iter().enumerate().take(usize::from(row.dwords)) {
                block[usize::from(row.dword_off) + k] = *w;
            }
        }
        let stream: Vec<u32> =
            p.fs.code
                .uniforms
                .iter()
                .map(|u| match u.contents {
                    0 => u.data,
                    1 => *block.get(u.data as usize).unwrap_or_else(|| panic!("uniform dword {} of the block", u.data)),
                    c => panic!("uniform contents {c} not modelled by this test\n{ops:#?}"),
                })
                .collect();
        let one = [1.0f32.to_bits(); LANES];
        let zero = [0u32; LANES];
        /* v_coord is the one varying (VARYING_SLOT_VAR0 + location 0), its
         * components in input order; W is 1 so the smooth value is C +
         * vary * W with C = 0 and vary = the value. */
        assert_eq!(p.fs.num_inputs as usize, p.fs.input_slots.len());
        let varyings =
            p.fs.input_slots
                .iter()
                .map(|&(slot, c)| {
                    assert!(slot == 32 && c < 2, "v_coord's components only: {:?}\n{ops:#?}", p.fs.input_slots);
                    Varying { vary: core::array::from_fn(|l| coords[l][usize::from(c)].to_bits()), c: zero }
                })
                .collect();
        let inputs = FsInputs {
            w: one,
            w_centroid: one,
            z: [z.to_bits(); LANES],
            x: zero,
            y: zero,
            back_facing: [false; LANES],
            varyings,
        };
        let env = Env { threads: p.fs.threads, ..Env::default() };
        sim::run(p.fs.code.words, &stream, &Inputs::Fragment(Box::new(inputs)), &env)
            .unwrap_or_else(|e| panic!("the fragment program runs: {e}\n{ops:#?}"))
    })
}

/// The depth write's lanes: the one TLBU write with the per-pixel depth
/// specifier, before the colour's words (the colour goes through TLB, no
/// configuration); every lane defined.
fn depth_bits(o: &Outcome) -> [u32; LANES] {
    let depth: Vec<&sim::TlbWrite> = o.tlb.iter().filter(|w| w.config == Some(TLB_Z_PER_PIXEL)).collect();
    assert_eq!(depth.len(), 1, "one per-pixel depth write: {:#x?}", o.tlb);
    assert_eq!(o.tlb[0].config, Some(TLB_Z_PER_PIXEL), "the depth before the colour: {:#x?}", o.tlb);
    assert!(o.tlb[1..].iter().all(|w| w.config.is_none()), "the colour through TLB: {:#x?}", o.tlb);
    core::array::from_fn(|l| depth[0].value[l].unwrap_or_else(|| panic!("lane {l}: an undefined depth {:#x?}", o.tlb)))
}

fn depth_of(o: &Outcome) -> [f32; LANES] {
    depth_bits(o).map(f32::from_bits)
}

fn coords(f: impl Fn(usize) -> [f32; 2]) -> [[f32; 2]; LANES] {
    core::array::from_fn(f)
}

const U_SCALE: [u32; 4] = [0.5f32.to_bits(), 0.25f32.to_bits(), 0.125f32.to_bits(), 1.0f32.to_bits()];

// ---- a constant, and a computed, depth -----------------------------------------

/// `gl_FragDepth = 0.25`: every lane's TLB Z is 0.25's f32, 0x3e800000, and
/// not the payload Z (0.75) the FEP handed the shader.
#[test]
fn a_constant_depth_is_written_as_its_f32_bits() {
    let o = run_fs(&fs("    gl_FragDepth = 0.25;"), &[], coords(|_| [0.0, 0.0]), 0.75);
    assert_eq!(depth_bits(&o), [0x3e80_0000; LANES]);
    assert_eq!(o.msf, [1; LANES], "no lane discarded");
}

/// `gl_FragDepth = gl_FragCoord.z * u_scale.x + (v_coord.x - v_coord.y) *
/// u_scale.y + u_scale.z` with Z 0.375, `u_scale` (0.5, 0.25, 0.125) and
/// `v_coord` (n/16, n/32) on lane n: the expression's value per lane, exact
/// (every operand and partial sum is a short dyadic rational, so the order
/// of the adds does not matter), and different on every lane.
#[test]
fn a_computed_depth_is_the_expression_per_lane() {
    let c = coords(|l| [l as f32 / 16.0, l as f32 / 32.0]);
    let z = 0.375f32;
    let want: [f32; LANES] = core::array::from_fn(|l| z * 0.5 + (c[l][0] - c[l][1]) * 0.25 + 0.125);
    let o = run_fs(
        &fs("    gl_FragDepth = gl_FragCoord.z * u_scale.x + (v_coord.x - v_coord.y) * u_scale.y + u_scale.z;"),
        &[("u_scale", &U_SCALE)],
        c,
        z,
    );
    assert_eq!(depth_of(&o), want);
    assert_eq!(want[0], 0.3125);
    assert_eq!(want[LANES - 1], 0.3125 + 15.0 / 128.0);
    assert!(o.approximate.is_empty(), "no SFU operation in the expression: {:?}", o.approximate);
}

// ---- the clamp ----------------------------------------------------------------

/// A depth outside [0, 1] reaches the TLB as written: a constant 1.5 or
/// -0.5, a uniform 1.75 and a varying expression from -1 to 2.75. Neither
/// Mesa's `emit_frag_end` nor select.rs clamps (no fsat before the `mov
/// tlbu`): GL's clamp of the written depth to [0, 1] is the TLB's, past the
/// QPU, and the board's to measure (dEQP `fbo.depth.write_clamp.*`). A
/// compiler that started clamping would move this pin on purpose.
#[test]
fn a_depth_outside_zero_one_reaches_the_tlb_unclamped() {
    let flat = coords(|_| [0.0, 0.0]);
    let o = run_fs(&fs("    gl_FragDepth = 1.5;"), &[], flat, 0.5);
    assert_eq!(depth_bits(&o), [0x3fc0_0000; LANES], "1.5 as written");
    let o = run_fs(&fs("    gl_FragDepth = -0.5;"), &[], flat, 0.5);
    assert_eq!(depth_bits(&o), [0xbf00_0000; LANES], "-0.5 as written");
    let o = run_fs(&fs("    gl_FragDepth = u_depth;"), &[("u_depth", &[1.75f32.to_bits()])], flat, 0.5);
    assert_eq!(depth_of(&o), [1.75; LANES], "a uniform 1.75 as written");
    /* v_coord.x * 4 - 1 on lane n with x = n/16: -1, -0.75, ..., 2.75 */
    let c = coords(|l| [l as f32 / 16.0, 0.0]);
    let want: [f32; LANES] = core::array::from_fn(|l| c[l][0] * 4.0 - 1.0);
    let o = run_fs(&fs("    gl_FragDepth = v_coord.x * 4.0 - 1.0;"), &[], c, 0.5);
    assert_eq!(depth_of(&o), want);
    assert_eq!((want[0], want[LANES - 1]), (-1.0, 2.75));
}

/// A clamp the shader writes itself is honoured: `clamp(v_coord.x * 4.0 -
/// 1.0, 0.0, 1.0)` is 0 on the lanes below 0, 1 on those above 1, the value
/// between (the compiler's fmin/fmax, which the interpreter models exactly).
#[test]
fn the_shaders_own_clamp_is_honoured() {
    let c = coords(|l| [l as f32 / 16.0, 0.0]);
    let want: [f32; LANES] = core::array::from_fn(|l| (c[l][0] * 4.0 - 1.0).clamp(0.0, 1.0));
    let o = run_fs(&fs("    gl_FragDepth = clamp(v_coord.x * 4.0 - 1.0, 0.0, 1.0);"), &[], c, 0.5);
    assert_eq!(depth_of(&o), want);
    assert_eq!(want[..4], [0.0; 4], "the lanes below 0");
    assert_eq!(want[4..9], [0.0, 0.25, 0.5, 0.75, 1.0], "the lanes between");
    assert_eq!(want[9..], [1.0; LANES - 9], "the lanes above 1");
}

// ---- a conditional write, and a discard ----------------------------------------

/// `if (v_coord.x < 0.5) gl_FragDepth = 0.25;` with no else: the lanes the
/// arm ran on carry 0.25. The others are undefined by the language (GLSL
/// ES 3.00 7.2: a path that does not set gl_FragDepth) and are not pinned,
/// only defined (the compiler's merge takes the arm's value).
#[test]
fn a_depth_written_in_one_arm_only_is_the_arms_value_where_it_ran() {
    let c = coords(|l| [if l % 3 == 0 { 0.25 } else { 0.75 }, 0.0]);
    let o = run_fs(&fs("    if (v_coord.x < 0.5)\n        gl_FragDepth = 0.25;"), &[], c, 0.5);
    let got = depth_of(&o);
    for (l, d) in got.iter().enumerate() {
        if c[l][0] < 0.5 {
            assert_eq!(*d, 0.25, "lane {l}: the arm ran\n{got:?}");
        }
    }
    /* and an else arm with its own value: each lane its arm's (as
     * es3_fragdepth_v3d.rs pins for the dEQP dynamic_conditional_write) */
    let o = run_fs(
        &fs("    if (v_coord.x < 0.5)\n        gl_FragDepth = 0.25;\n    else\n        gl_FragDepth = 0.875;"),
        &[],
        c,
        0.5,
    );
    let want: [f32; LANES] = core::array::from_fn(|l| if c[l][0] < 0.5 { 0.25 } else { 0.875 });
    assert_eq!(depth_of(&o), want);
}

/// `if (v_coord.x > 0.5) discard; gl_FragDepth = v_coord.y;`: the discarded
/// lanes end with multisample flag 0 (the fragment is rejected, the form of
/// rejection the model has; the TLB ignores their writes) and the kept
/// lanes with 1, and the depth write carries every lane's `v_coord.y`.
#[test]
fn a_discarded_lane_is_flagged_and_still_carries_its_depth() {
    let c = coords(|l| [if l % 4 == 1 { 1.0 } else { 0.0 }, l as f32 / 16.0]);
    let o = run_fs(&fs("    if (v_coord.x > 0.5)\n        discard;\n    gl_FragDepth = v_coord.y;"), &[], c, 0.5);
    let want_msf: [u32; LANES] = core::array::from_fn(|l| u32::from(c[l][0] <= 0.5));
    assert_eq!(o.msf, want_msf, "lanes 1, 5, 9, 13 discarded");
    let want: [f32; LANES] = core::array::from_fn(|l| c[l][1]);
    assert_eq!(depth_of(&o), want);
}
