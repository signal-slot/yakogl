// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The V3D compiler at the hardware's varying limits (plan #359, item
//! `v3d-varyings-compiler`, docs/notes/v3d-varyings.md "Changes of
//! `v3d-varyings-compiler`"): 64 fragment shader inputs, the flat and
//! centroid sets as three 24-bit words (input i is bit i % 24 of word
//! i / 24, Mesa `v3d_set_fs_prog_data_inputs`, vir.c:992-1003), and a
//! vertex shader of 4 + 1 + 64 = 69 VPM output rows (the fixed-function
//! rows, gl_PointSize, 16 vec4 varyings).
//!
//! What runs: the programs are compiled and linked through `SHC_ONLINE`, as
//! glCompileShader and glLinkProgram reach it, and the linked record's
//! `ShcV3dFs` / `ShcV3dVs` are read: word 0 of each set in the old fields
//! (`flat_shade_flags`, `centroid_flags`), words 1 and 2 in the `*_hi`
//! fields. A fragment shader of 65 scalar inputs is refused by name
//! (`TooManyFragmentInputs`). The 64-input fragment program and the
//! 69-row vertex program also run on the QPU interpreter (`sim::run`):
//! flat inputs above 24 and 48 read the C coefficient alone, centroid ones
//! the centroid W, and every one of the 69 rows is written.
//!
//! Not here: the draw side (the job builder's flag packets and its
//! 24-input cap, items `v3d-draw-flag-words` / `v3d-varyings-raise`) and
//! the reported GL limits, which stay as they are.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d_varyings_compile
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-v3d", not(feature = "report-es2")))]

use core::ffi::c_char;
use std::sync::{Mutex, MutexGuard};
use yakogl::shaderc::online::arena::Arena;
use yakogl::shaderc::online::v3d::chain::{compile_program, flag_words, Prim, Target, V3dError, V3dProgram};
use yakogl::shaderc::online::v3d::sim::{self, Env, FsInputs, Inputs, Varying, VsInputs, LANES};
use yakogl::shaderc::online::{lower, sema};
use yakogl::shaderc::{ShcShader, ShcV3d, SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX};

/// The compiler keeps its output in one arena that every compile resets.
static COMPILING: Mutex<()> = Mutex::new(());

fn lock() -> MutexGuard<'static, ()> {
    COMPILING.lock().unwrap_or_else(|e| e.into_inner())
}

/// The Pi 4's VPM (IDENT1.VPM_SIZE 8) and the Pi 5's (hw_mock_test.c, 7.1).
const TARGET: Target = Target { vpm_size: 65536 };

/// The qualifier of varying `k` in the programs below: flat for 1, 7 and 13
/// (inputs 4-7, 28-31 and 52-55: words 0, 1 and 2), centroid for 9 and 14
/// (inputs 36-39 and 56-59: words 1 and 2), smooth otherwise.
fn qual(k: usize) -> &'static str {
    match k {
        1 | 7 | 13 => "flat ",
        9 | 14 => "centroid ",
        _ => "",
    }
}

/// A vertex shader writing `n` vec4 varyings `v0..`, with `qual`'s
/// qualifiers, plus `extra` scalar float varyings `e0..`, and gl_PointSize
/// when `psiz`. Varying k is `u_v[k]` (and `e{k}` is `u_e[k]`): uniforms,
/// which the chain loads again at each VPM write under register pressure
/// (`Pressure::Remat`). 64 values computed in the shader do not fit 2
/// threads' registers today: `select_vertex` writes every varying after the
/// body, and there is no spilling yet.
fn vs(n: usize, extra: usize, psiz: bool) -> String {
    let mut s = format!("#version 300 es\nin highp vec4 a_position;\nuniform highp vec4 u_v[{n}];\n");
    if extra > 0 {
        s += &format!("uniform highp float u_e[{extra}];\n");
    }
    for k in 0..n {
        s += &format!("{}out highp vec4 v{k};\n", qual(k));
    }
    for k in 0..extra {
        s += &format!("out highp float e{k};\n");
    }
    s += "void main (void)\n{\n    gl_Position = a_position;\n";
    if psiz {
        s += "    gl_PointSize = 1.0;\n";
    }
    for k in 0..n {
        s += &format!("    v{k} = u_v[{k}];\n");
    }
    for k in 0..extra {
        s += &format!("    e{k} = u_e[{k}];\n");
    }
    s + "}\n"
}

/// A computed varying per vec4 (`a_position * (k + 1)`), as dEQP's
/// varying shaders write them.
fn vs_computed(n: usize) -> String {
    let mut s = String::from("#version 300 es\nin highp vec4 a_position;\n");
    for k in 0..n {
        s += &format!("{}out highp vec4 v{k};\n", qual(k));
    }
    s += "void main (void)\n{\n    gl_Position = a_position;\n";
    for k in 0..n {
        s += &format!("    v{k} = a_position * {}.0;\n", k + 1);
    }
    s + "}\n"
}

/// 16 vec4 varyings as one array and a float: 65 scalar inputs in two
/// declarations (`ShcShader::varyings` holds 16 declarations).
const VS_65: &str = "#version 300 es\nin highp vec4 a_position;\nout highp vec4 va[16];\nout highp float e;\n\
void main (void)\n{\n    gl_Position = a_position;\n    for (int i = 0; i < 16; i++) va[i] = a_position * float(i);\n    e = a_position.x;\n}\n";
const FS_65: &str = "#version 300 es\nprecision highp float;\nin highp vec4 va[16];\nin highp float e;\n\
layout(location = 0) out highp vec4 o_color;\nvoid main (void)\n{\n    vec4 c = vec4(e);\n    for (int i = 0; i < 16; i++) c += va[i];\n    o_color = c;\n}\n";

/// The fragment shader reading every component of what [`vs`] writes: the
/// colour is the sum of the vec4s (and of the scalars in `x`).
fn fs(n: usize, extra: usize) -> String {
    let mut s = String::from("#version 300 es\nprecision highp float;\n");
    for k in 0..n {
        s += &format!("{}in highp vec4 v{k};\n", qual(k));
    }
    for k in 0..extra {
        s += &format!("in highp float e{k};\n");
    }
    s += "layout(location = 0) out highp vec4 o_color;\nvoid main (void)\n{\n    o_color = vec4(0.0)";
    for k in 0..n {
        s += &format!(" + v{k}");
    }
    for k in 0..extra {
        s += &format!(" + vec4(e{k}, 0.0, 0.0, 0.0)");
    }
    s + ";\n}\n"
}

fn text_of(log: &[c_char]) -> String {
    log.iter().take_while(|c| **c != 0).map(|c| *c as u8 as char).collect()
}

/// One stage through `SHC_ONLINE`, or the compile's log.
fn compile(stage: u32, src: &str) -> Result<ShcShader, String> {
    let mut out = ShcShader::ZERO;
    let mut log = [0 as c_char; 1024];
    let text = format!("{src}\0");
    let compile = yakogl::shaderc::online::SHC_ONLINE.compile.expect("the online back end compiles");
    // SAFETY: a NUL-terminated source, a real output, and a log of the size announced.
    let rc = unsafe { compile(stage, text.as_ptr() as *const c_char, &mut out, log.as_mut_ptr(), log.len()) };
    if rc != 0 {
        return Err(format!("compile: {}", text_of(&log)));
    }
    Ok(out)
}

/// The link of `vs` and `fs` through `SHC_ONLINE`: the linked record, or
/// the compile's or the link's log.
fn link(vs: &str, fs: &str) -> Result<ShcV3d, String> {
    let _g = lock();
    yakogl::shaderc::online::backend::forget_compiled();
    let vso = compile(SHC_STAGE_VERTEX, vs)?;
    let mut fso = compile(SHC_STAGE_FRAGMENT, fs)?;
    let link = yakogl::shaderc::online::SHC_ONLINE.link.expect("the online back end links");
    let mut log = [0 as c_char; 1024];
    // SAFETY: two distinct compiled shaders and a log of the size announced.
    let rc = unsafe { link(&vso, &mut fso, log.as_mut_ptr(), log.len()) };
    if rc != 0 {
        return Err(text_of(&log));
    }
    assert_eq!(fso.v3d.linked, 1, "the V3D record is the linked program's");
    Ok(fso.v3d)
}

/// `f` on the chain's program for `vs` + `fs` (the per-input slots and the
/// code the interpreter runs), or the chain's refusal.
fn with_program<R>(vs: &str, fs: &str, target: Target, f: impl FnOnce(Result<&V3dProgram<'_>, V3dError>) -> R) -> R {
    let _g = lock();
    let mut mem = vec![0u8; 32 << 20];
    let a = Arena::new(&mut mem);
    let vs_ = a.alloc_str(vs).expect("arena");
    let fs_ = a.alloc_str(fs).expect("arena");
    let v =
        lower::compile_to_ir(&a, sema::Stage::Vertex, vs_).unwrap_or_else(|e| panic!("the VS lowers: {}\n{vs}", e.msg));
    let fr = lower::compile_to_ir(&a, sema::Stage::Fragment, fs_)
        .unwrap_or_else(|e| panic!("the FS lowers: {}\n{fs}", e.msg));
    match compile_program(&v.module, &fr.module, Prim::Triangles, target, &a) {
        Ok(p) => f(Ok(&p)),
        Err(e) => f(Err(e)),
    }
}

/// The sets of inputs (bit i: input i) whose varying is flat, and centroid,
/// from the program's input slots (slot 32 + k is varying `v{k}`).
fn expected_sets(slots: &[(u8, u8)]) -> (u64, u64) {
    let (mut flat, mut centroid) = (0u64, 0u64);
    for (i, &(slot, _)) in slots.iter().enumerate() {
        match qual(usize::from(slot - 32)) {
            "flat " => flat |= 1 << i,
            "centroid " => centroid |= 1 << i,
            _ => {}
        }
    }
    (flat, centroid)
}

// ---- the flag words through the public API ---------------------------------------

/// The three words of a 64-bit set, Mesa's packing: bits 0-23, 24-47, 48-63.
#[test]
fn flag_words_pack_24_inputs_to_a_word() {
    assert_eq!(flag_words(0), [0, 0, 0]);
    assert_eq!(flag_words(0xf0), [0xf0, 0, 0]);
    assert_eq!(flag_words(1 << 24), [0, 1, 0]);
    assert_eq!(flag_words(1 << 47), [0, 0x80_0000, 0]);
    assert_eq!(flag_words(1 << 48), [0, 0, 1]);
    assert_eq!(flag_words(u64::MAX), [0xff_ffff, 0xff_ffff, 0xffff]);
}

/// 16 vec4 varyings (64 inputs), flat 1/7/13 and centroid 9/14: the linked
/// record has 64 inputs and the flat and centroid bits in words 0, 1 and 2,
/// inputs in declaration order (varying k is inputs 4k..4k+3).
#[test]
fn sixty_four_inputs_carry_three_flag_words() {
    let r = link(&vs(16, 0, true), &fs(16, 0)).unwrap_or_else(|e| panic!("16 vec4 varyings link: {e}"));
    let f = r.fs;
    assert_eq!(f.num_inputs, 64);
    /* flat: v1 = inputs 4-7 (word 0), v7 = 28-31 (word 1 bits 4-7),
     * v13 = 52-55 (word 2 bits 4-7) */
    assert_eq!(f.flat_shade_flags, 0xf0, "flat word 0");
    assert_eq!(f.flat_shade_flags_hi, [0xf0, 0xf0], "flat words 1, 2");
    /* centroid: v9 = inputs 36-39 (word 1 bits 12-15), v14 = 56-59 (word 2
     * bits 8-11) */
    assert_eq!(f.centroid_flags, 0, "centroid word 0");
    assert_eq!(f.centroid_flags_hi, [0xf000, 0xf00], "centroid words 1, 2");
    assert_eq!((f.noperspective_flags, f.noperspective_flags_hi), (0, [0, 0]));
    /* The vertex shader's 69 rows: 9 sectors of 8 rows, the 64 KiB VPM
     * holds 7 batches, so the VCM cache is the most (vir.c:931-937). */
    assert_eq!(r.vs.writes_psiz, 1);
    assert_eq!((r.vs.vpm_output_size, r.vs.vcm_cache_size), (9, 4), "69 VPM rows");
}

/// 15 vec4 varyings (60 inputs, ES 3.0's MAX_FRAGMENT_INPUT_COMPONENTS)
/// without gl_PointSize: 64 rows, still 8 sectors.
#[test]
fn sixty_inputs_link() {
    let r = link(&vs(15, 0, false), &fs(15, 0)).unwrap_or_else(|e| panic!("15 vec4 varyings link: {e}"));
    assert_eq!(r.fs.num_inputs, 60);
    assert_eq!(r.fs.flat_shade_flags, 0xf0);
    assert_eq!(r.fs.flat_shade_flags_hi, [0xf0, 0xf0]);
    assert_eq!(r.fs.centroid_flags_hi, [0xf000, 0xf00]);
    assert_eq!(r.vs.writes_psiz, 0);
    assert_eq!(r.vs.vpm_output_size, 8, "4 + 60 rows");
}

/// Fewer than 25 inputs leave words 1 and 2 zero: what the job builder
/// emits today (word 0 only) stays what it was.
#[test]
fn twenty_four_inputs_leave_the_high_words_zero() {
    let r = link(&vs(6, 0, false), &fs(6, 0)).unwrap_or_else(|e| panic!("6 vec4 varyings link: {e}"));
    assert_eq!(r.fs.num_inputs, 24);
    assert_eq!(r.fs.flat_shade_flags, 0xf0);
    assert_eq!(
        (r.fs.flat_shade_flags_hi, r.fs.noperspective_flags_hi, r.fs.centroid_flags_hi),
        ([0, 0], [0, 0], [0, 0])
    );
}

/// 65 scalar inputs (16 vec4 and a float) are past what the hardware
/// interpolates: the chain refuses them by name, not as a gap of this
/// compiler. Through the GL path the same program needs 17 varying
/// declarations (an array counts one per element), which the stage's
/// varying table (`ShcShader::varyings`, 16) refuses at compile, by name as
/// well; so no linked record past 64 inputs exists.
#[test]
fn sixty_five_inputs_are_refused_by_name() {
    with_program(VS_65, FS_65, TARGET, |p| assert_eq!(p.err(), Some(V3dError::TooManyFragmentInputs)));
    with_program(&vs(16, 1, false), &fs(16, 1), TARGET, |p| assert_eq!(p.err(), Some(V3dError::TooManyFragmentInputs)));
    for (vs, fs) in [(VS_65.to_string(), FS_65.to_string()), (vs(16, 1, false), fs(16, 1))] {
        let e = link(&vs, &fs).map(|r| r.fs.num_inputs).expect_err("65 inputs do not link");
        assert!(e.contains("TooManyVaryings { got: 17 }"), "the refusal names the limit: {e}");
        assert!(!e.contains("NotYet"), "not a gap: {e}");
    }
}

/// 16 vec4 varyings the vertex shader computes (dEQP's form) do not fit 2
/// threads' registers: `select_vertex` writes every varying after the
/// body, so 64 values are live at once, and the chain does not spill yet.
/// 15 do. Not this item's (the row count is accepted, see
/// [`the_vertex_shader_writes_69_rows`]); run with `--ignored` once the
/// vertex shader writes its varyings where they are made or spills.
#[test]
fn computed_varyings_compile_up_to_15_vec4() {
    with_program(&vs_computed(15), &fs(15, 0), TARGET, |p| assert!(p.is_ok(), "{:?}", p.err()));
}

#[test]
#[ignore = "64 computed varyings: no spilling and no early VPM writes yet"]
fn sixteen_computed_vec4_varyings_compile() {
    with_program(&vs_computed(16), &fs(16, 0), TARGET, |p| assert!(p.is_ok(), "{:?}", p.err()));
}

/// The chain's words agree with the input slots, whatever order the inputs
/// come in: each flat or centroid varying's inputs, and only those.
#[test]
fn the_flag_words_follow_the_input_slots() {
    with_program(&vs(16, 0, true), &fs(16, 0), TARGET, |p| {
        let p = p.expect("the program compiles");
        assert_eq!(p.fs.input_slots.len(), 64);
        let (flat, centroid) = expected_sets(p.fs.input_slots);
        let fw = flag_words(flat);
        let cw = flag_words(centroid);
        assert_eq!((p.fs.flat_shade_flags, p.fs.flat_shade_flags_hi), (fw[0], [fw[1], fw[2]]));
        assert_eq!((p.fs.centroid_flags, p.fs.centroid_flags_hi), (cw[0], [cw[1], cw[2]]));
        assert_eq!(flat.count_ones(), 12);
        assert_eq!(centroid.count_ones(), 8);
    });
}

/// A 16 KiB VPM (VPM_SIZE 2, the hw_mock part) cannot hold two batches of
/// 69 rows (9 sectors against 16 a half): refused as Mesa asserts
/// (vir.c:936), while 64 rows (8 sectors) still fit.
#[test]
fn a_16k_vpm_takes_64_rows_not_69() {
    let small = Target { vpm_size: 16384 };
    with_program(&vs(16, 0, true), &fs(16, 0), small, |p| assert_eq!(p.err(), Some(V3dError::VpmTooSmall)));
    with_program(&vs(15, 0, false), &fs(15, 0), small, |p| {
        assert_eq!(p.map(|p| p.vs.vpm_output_size).ok(), Some(8));
    });
}

// ---- the programs on the interpreter ---------------------------------------------

/// The 64-input fragment program on the interpreter. Varying k's inputs
/// carry vary = 2(k + 1) and C = k + 1 with W 1 and centroid W 0.5: a
/// smooth input is vary * W + C = 3(k + 1), a centroid one vary * 0.5 + C =
/// 2(k + 1), a flat one C alone = k + 1. Each channel sums one input of
/// every varying, so a flag lost, or moved to another varying, changes it;
/// every partial sum is an integer below 2048, exact in f32 and in the f16
/// the colour reaches the TLB as, in any order.
#[test]
fn flat_and_centroid_inputs_above_32_read_their_own_planes() {
    with_program(&vs(16, 0, true), &fs(16, 0), TARGET, |p| {
        let p = p.expect("the program compiles");
        let (flat, centroid) = expected_sets(p.fs.input_slots);
        let stream: Vec<u32> =
            p.fs.code
                .uniforms
                .iter()
                .map(|u| match u.contents {
                    0 => u.data,
                    c => panic!("uniform contents {c} not modelled by this test"),
                })
                .collect();
        let varyings =
            p.fs.input_slots
                .iter()
                .map(|&(slot, _)| {
                    let k = f32::from(slot - 32);
                    Varying { vary: [(2.0 * (k + 1.0)).to_bits(); LANES], c: [(k + 1.0).to_bits(); LANES] }
                })
                .collect();
        let inputs = FsInputs {
            w: [1.0f32.to_bits(); LANES],
            w_centroid: [0.5f32.to_bits(); LANES],
            z: [0; LANES],
            x: [0; LANES],
            y: [0; LANES],
            back_facing: [false; LANES],
            varyings,
        };
        let env = Env { threads: p.fs.threads, ..Env::default() };
        let o = sim::run(p.fs.code.words, &stream, &Inputs::Fragment(Box::new(inputs)), &env)
            .unwrap_or_else(|e| panic!("the fragment program runs: {e}"));
        let mut want = [0f32; 4];
        for (i, &(slot, comp)) in p.fs.input_slots.iter().enumerate() {
            let k = f32::from(slot - 32);
            want[usize::from(comp)] += if flat & (1 << i) != 0 {
                k + 1.0
            } else if centroid & (1 << i) != 0 {
                2.0 * (k + 1.0)
            } else {
                3.0 * (k + 1.0)
            };
        }
        /* the colour: two TLB writes of packed f16 pairs (r g, b a), the
         * first component in the low half */
        let colour: Vec<f32> = o
            .tlb
            .iter()
            .filter(|w| w.config.is_none())
            .flat_map(|w| {
                let v = w.value[0].expect("lane 0");
                [sim::half_to_f32(v as u16), sim::half_to_f32((v >> 16) as u16)]
            })
            .collect();
        assert_eq!(colour, want, "the colour, lane 0: {:#x?}", o.tlb);
        /* 3 * (1 + .. + 16), less 2(k + 1) for flat 1, 7, 13 and k + 1 for
         * centroid 9, 14 */
        assert_eq!(want, [408.0 - 2.0 * (2.0 + 8.0 + 14.0) - (10.0 + 15.0); 4]);
    });
}

/// The 69-row vertex program on the interpreter: 4 fixed-function rows,
/// gl_PointSize in row 4, and the 64 varying rows after it, in the order
/// the fragment shader reads them, every one written: varying k's
/// component c is `u_v[k][c]`, here 4k + c + 1.
#[test]
fn the_vertex_shader_writes_69_rows() {
    let src = vs(16, 0, true);
    let _g = lock();
    let mut mem = vec![0u8; 32 << 20];
    let a = Arena::new(&mut mem);
    let vs_ = a.alloc_str(&src).expect("arena");
    let fs_src = fs(16, 0);
    let fs_ = a.alloc_str(&fs_src).expect("arena");
    let v = lower::compile_to_ir(&a, sema::Stage::Vertex, vs_).unwrap_or_else(|e| panic!("the VS lowers: {}", e.msg));
    let fr =
        lower::compile_to_ir(&a, sema::Stage::Fragment, fs_).unwrap_or_else(|e| panic!("the FS lowers: {}", e.msg));
    let p = compile_program(&v.module, &fr.module, Prim::Triangles, TARGET, &a).expect("the program compiles");
    assert_eq!(p.vs.vpm_output_size, 9);
    /* u_v laid out as the reflection says (dword offsets) */
    let row = v.reflection.uniforms.iter().find(|r| r.name == "u_v").expect("u_v");
    let mut block = vec![0u32; usize::from(v.reflection.uniform_dwords)];
    for d in 0..64 {
        block[usize::from(row.dword_off) + d] = (d as f32 + 1.0).to_bits();
    }
    let uniforms: Vec<u32> =
        p.vs.code
            .uniforms
            .iter()
            .map(|u| match u.contents {
                0 | 40 | 41 => u.data,
                1 => block[u.data as usize],
                2..=5 => 1.0f32.to_bits(),
                c => panic!("uniform contents {c} not modelled by this test"),
            })
            .collect();
    let pos = [0.25f32, 0.5, 0.75, 1.0];
    let rows: Vec<[u32; LANES]> =
        pos.iter().take(usize::from(p.vs.vattr_sizes[0])).map(|c| [c.to_bits(); LANES]).collect();
    let env = Env { threads: p.vs.threads, ..Env::default() };
    let o = sim::run(p.vs.code.words, &uniforms, &Inputs::Vertex(VsInputs { vpm: rows }), &env)
        .unwrap_or_else(|e| panic!("the vertex program runs: {e}"));
    assert_eq!(o.vpm_out.len(), 69, "4 + 1 + 64 rows");
    let out: Vec<[u32; LANES]> = o.vpm_out.iter().map(|r| r.expect("every output row written")).collect();
    assert_eq!(f32::from_bits(out[4][0]), 1.0, "gl_PointSize in row 4");
    for (i, &(slot, comp)) in p.fs.input_slots.iter().enumerate() {
        let k = f32::from(slot - 32);
        assert_eq!(f32::from_bits(out[5 + i][0]), 4.0 * k + f32::from(comp) + 1.0, "row {} (input {i})", 5 + i);
    }
}
