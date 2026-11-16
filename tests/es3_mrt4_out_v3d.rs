// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The V3D online compiler's four TLB colour outputs (RC14, plan #368): a
//! fragment shader with outputs at locations 0..3 is compiled in-process
//! (`compile_program`, `MAX_RTS` = 4) and run on the QPU interpreter, and
//! the TLB writes it makes are compared with what Mesa 26.2.1's
//! `vir_emit_tlb_color_write` (src/broadcom/compiler/nir_to_vir.c:1953-2035,
//! called for rt 0..V3D_MAX_DRAW_BUFFERS by `emit_frag_end`, :2141) emits:
//!
//! - one write group per colour output the shader declares and writes, in RT
//!   order whatever the declaration or assignment order;
//! - the config word `0xffffff00 | per-pixel (1 << 2) | (7 - rt) << 3 | type
//!   | size`, carried by the group's first write through TLBU (the others
//!   through TLB); the one config the compiler leaves out is `~0`, the f16
//!   vec4 of RT 0;
//! - a 16-bit float output is f16 pairs (`TLB_TYPE_F16_COLOR | SWAP_HI_LO`,
//!   :1975-1983): `vfpack(r, g)`, and `vfpack(b, a)` only for three or four
//!   components, `TLB_VEC_SIZE_4_F16` for them and `TLB_VEC_SIZE_2_F16` for
//!   one or two;
//! - an integer output (and Mesa's `f32_color_rb` targets) is 32-bit words,
//!   `TLB_TYPE_F32_COLOR | (components - 1)`, one `mov` per component
//!   (:1974-1977, :2005-2021).
//!
//! Render target 3 gets every format class. Mesa's `f32_color_rb` key for a
//! float output on a 32-bit float target (RGBA32F, RG32F, R32F) has no
//! counterpart in the compiler's inputs (`Target` holds only `vpm_size`);
//! that is the G1 gap of docs/notes/v3d-fragment-out.md, not a four-target
//! case, and not pinned here.
//!
//! `#[ignore]`d tests hold Mesa's result where the compiler differs today;
//! the reason names the defect (item v3d-shout4 fixes them).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_mrt4_out_v3d
//! ```
#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-v3d", not(feature = "report-es2")))]

use std::sync::Mutex;
use yakogl::shaderc::online::arena::Arena;
use yakogl::shaderc::online::v3d::chain::{compile_program, Prim, Target};
use yakogl::shaderc::online::v3d::prepare::MAX_RTS;
use yakogl::shaderc::online::v3d::select::{tlb_config_32b, tlb_config_f16};
use yakogl::shaderc::online::v3d::sim::{self, Env, FsInputs, Inputs, LANES};
use yakogl::shaderc::online::{lower, sema};

/// The compiler keeps its output in one arena that every compile resets.
static COMPILING: Mutex<()> = Mutex::new(());

const VS: &str = "#version 300 es\nin vec4 p;\nvoid main() { gl_Position = p; }\n";

/// One TLB write of lane 0: the TLBU config (`None` for a TLB write) and word.
type Write = (Option<u32>, u32);

/// Compile `fs` with the VS above and run it: the program's `rt_mask` and the
/// TLB writes. `Err` is the front end's or the compiler's message.
fn try_run_fs(fs: &str) -> Result<(u8, Vec<Write>), String> {
    let _g = COMPILING.lock().unwrap_or_else(|e| e.into_inner());
    let mut mem = vec![0u8; 16 << 20];
    let a = Arena::new(&mut mem);
    let vs = a.alloc_str(VS).expect("arena");
    let fs = a.alloc_str(fs).expect("arena");
    let v = lower::compile_to_ir(&a, sema::Stage::Vertex, vs).map_err(|e| e.msg.to_string())?;
    let f = lower::compile_to_ir(&a, sema::Stage::Fragment, fs).map_err(|e| format!("FS: {}", e.msg))?;
    let p = compile_program(&v.module, &f.module, Prim::Triangles, Target { vpm_size: 65536 }, &a)
        .map_err(|e| format!("compile: {e:?}"))?;
    let uniforms: Vec<u32> =
        p.fs.code
            .uniforms
            .iter()
            .map(|u| match u.contents {
                0 => u.data,
                c => panic!("uniform contents {c} not modelled by this test"),
            })
            .collect();
    let inputs = FsInputs {
        w: [1.0f32.to_bits(); LANES],
        w_centroid: [1.0f32.to_bits(); LANES],
        z: [0; LANES],
        x: [0; LANES],
        y: [0; LANES],
        back_facing: [false; LANES],
        varyings: Vec::new(),
    };
    assert_eq!(p.fs.num_inputs, 0, "the shaders here read no varying");
    let env = Env { threads: p.fs.threads, ..Env::default() };
    let o = sim::run(p.fs.code.words, &uniforms, &Inputs::Fragment(Box::new(inputs)), &env)
        .map_err(|e| format!("run: {e}"))?;
    let w = o.tlb.iter().map(|w| (w.config, w.value[0].unwrap_or(0xdead_beef))).collect();
    Ok((p.fs.rt_mask, w))
}

fn run_fs(fs: &str) -> (u8, Vec<Write>) {
    try_run_fs(fs).unwrap_or_else(|e| panic!("{e}\n{fs}"))
}

/* ---- what Mesa emits, derived from nir_to_vir.c, not from select.rs -------- */

/// A colour output's class and component count.
#[derive(Clone, Copy, Debug)]
enum Kind {
    /// 16-bit float (`f32_color_rb` clear): f16 pairs.
    F(u32),
    /// signed integer: 32-bit words.
    I(u32),
    /// unsigned integer: 32-bit words.
    U(u32),
}

impl Kind {
    fn n(self) -> u32 {
        match self {
            Kind::F(n) | Kind::I(n) | Kind::U(n) => n,
        }
    }
    fn glsl(self) -> String {
        let (vec, scalar) = match self {
            Kind::F(_) => ("vec", "float"),
            Kind::I(_) => ("ivec", "int"),
            Kind::U(_) => ("uvec", "uint"),
        };
        match self.n() {
            1 => scalar.to_string(),
            n => format!("{vec}{n}"),
        }
    }
    /// Component `j` of RT `rt`: a GLSL literal and the 32-bit word it makes
    /// (for a float, its f16 half-word).
    fn lit(self, rt: u32, j: u32) -> (String, u32) {
        match self {
            Kind::F(_) => {
                let v = (rt + j) % 7 + 1;
                (format!("{v}.0"), [0, 0x3c00, 0x4000, 0x4200, 0x4400, 0x4500, 0x4600, 0x4700][v as usize])
            }
            Kind::I(_) => {
                let v = -((rt * 10 + j + 1) as i32);
                (format!("({v})"), v as u32)
            }
            Kind::U(_) => {
                let v = 0xf000_0000 + rt * 10 + j + 1;
                (format!("{v:#x}u"), v)
            }
        }
    }
    /// The right side of `o<rt> = ...`.
    fn value(self, rt: u32) -> String {
        let args: Vec<String> = (0..self.n()).map(|j| self.lit(rt, j).0).collect();
        match self.n() {
            1 => args[0].clone(),
            _ => format!("{}({})", self.glsl(), args.join(", ")),
        }
    }
}

/// Mesa's TLB config for RT `rt` of `kind`, written out from nir_to_vir.c
/// :1957-1983 (and cross-checked against `select.rs`' helpers).
fn mesa_conf(rt: u32, kind: Kind) -> u32 {
    let mut conf = 0xffff_ff00u32 | (1 << 2) | ((7 - rt) << 3);
    match kind {
        Kind::F(n) => {
            conf |= (3 << 6) | (1 << 1);
            conf |= u32::from(n >= 3);
            if n >= 3 {
                assert_eq!(conf, tlb_config_f16(rt, n));
            }
        }
        Kind::I(n) | Kind::U(n) => {
            conf |= n - 1;
            assert_eq!(conf, tlb_config_32b(rt, n));
        }
    }
    conf
}

/// Mesa's writes for one output: the TLBU config on the first write unless it
/// is `~0`, then one word per f16 pair or per component.
fn mesa_writes(rt: u32, kind: Kind) -> Vec<Write> {
    let conf = mesa_conf(rt, kind);
    let words: Vec<u32> = match kind {
        Kind::F(n) => {
            let h = |j| kind.lit(rt, j).1;
            let g = if n >= 2 { h(1) } else { 0 };
            let mut w = vec![h(0) | g << 16];
            if n >= 3 {
                let a = if n >= 4 { h(3) } else { 0 };
                w.push(h(2) | a << 16);
            }
            w
        }
        _ => (0..kind.n()).map(|j| kind.lit(rt, j).1).collect(),
    };
    words.iter().enumerate().map(|(i, &w)| (if i == 0 && conf != !0 { Some(conf) } else { None }, w)).collect()
}

/// The shader: the outputs declared in `decl` order (RT numbers), assigned in
/// `assign` order, of `kinds[rt]`.
fn shader(kinds: [Option<Kind>; 4], decl: &[u32], assign: &[u32]) -> String {
    let mut s = String::from("#version 300 es\nprecision highp int;\nprecision highp float;\n");
    for &rt in decl {
        let k = kinds[rt as usize].unwrap();
        s += &format!("layout(location = {rt}) out {} o{rt};\n", k.glsl());
    }
    s += "void main() {\n";
    for &rt in assign {
        s += &format!("  o{rt} = {};\n", kinds[rt as usize].unwrap().value(rt));
    }
    s + "}\n"
}

/// Mesa's whole write sequence: RT 0..3 in order, the declared ones only.
fn mesa_all(kinds: [Option<Kind>; 4]) -> Vec<Write> {
    (0..4u32).filter_map(|rt| kinds[rt as usize].map(|k| mesa_writes(rt, k))).flatten().collect()
}

/// The RT a config word names (bits 3..5, reversed), or 0 for the implicit
/// f16 vec4 of RT 0.
fn rt_of(conf: Option<u32>) -> u32 {
    conf.map_or(0, |c| 7 - ((c >> 3) & 7))
}

/// The writes grouped by RT: (rt, config, words). A group starts at a TLBU
/// write, or at the first write when it has no config (RT 0, f16 vec4).
fn groups(w: &[Write]) -> Vec<(u32, Option<u32>, Vec<u32>)> {
    let mut g: Vec<(u32, Option<u32>, Vec<u32>)> = Vec::new();
    for &(c, v) in w {
        if c.is_some() || g.is_empty() {
            g.push((rt_of(c), c, vec![v]));
        } else {
            g.last_mut().unwrap().2.push(v);
        }
    }
    g
}

const F4: Option<Kind> = Some(Kind::F(4));

fn check(kinds: [Option<Kind>; 4], decl: &[u32], assign: &[u32]) {
    let fs = shader(kinds, decl, assign);
    let (mask, w) = run_fs(&fs);
    let want = mesa_all(kinds);
    let rts: Vec<u32> = (0..4u32).filter(|&r| kinds[r as usize].is_some()).collect();
    assert_eq!(
        u32::from(mask),
        rts.iter().fold(0, |m, r| m | 1 << r),
        "rt_mask, the attachments the program writes\n{fs}"
    );
    assert_eq!(w, want, "TLB writes (config, word) in RT order\n{fs}");
    assert_eq!(groups(&w).iter().map(|g| g.0).collect::<Vec<_>>(), rts, "one group per RT, in RT order\n{fs}");
}

/* ---- four float outputs ------------------------------------------------------ */

#[test]
fn the_compiler_has_four_render_targets() {
    assert_eq!(MAX_RTS, 4);
}

/// Four `vec4` outputs: two f16 words each; RT 0's group has no config (it is
/// `~0`), RT 1..3's first word goes through TLBU with `(7 - rt) << 3`.
#[test]
fn four_vec4_outputs_write_four_groups_of_two_f16_words() {
    let k = [F4, F4, F4, F4];
    check(k, &[0, 1, 2, 3], &[0, 1, 2, 3]);
    let (_, w) = run_fs(&shader(k, &[0, 1, 2, 3], &[0, 1, 2, 3]));
    assert_eq!(w.len(), 8, "two words an RT");
    let confs: Vec<Option<u32>> = w.iter().map(|x| x.0).collect();
    assert_eq!(
        confs,
        [None, None, Some(0xffff_fff7), None, Some(0xffff_ffef), None, Some(0xffff_ffe7), None],
        "RT 0 without a config, RT 1..3 with 0xffffff00 | 0xc0 | 3 | 4 | (7 - rt) << 3"
    );
    assert_eq!(rt_of(confs[6]), 3, "the last group names RT 3");
}

/// The outputs declared in a shuffled order and assigned in another: the
/// writes are in RT order.
#[test]
fn shuffled_declaration_and_assignment_order_writes_in_rt_order() {
    let k = [F4, F4, F4, F4];
    check(k, &[3, 1, 0, 2], &[2, 0, 3, 1]);
    check(k, &[2, 3, 1, 0], &[3, 2, 1, 0]);
    check(k, &[1, 0, 3, 2], &[0, 1, 2, 3]);
}

/// `layout(location = 0) out vec4 o[4]`: the array takes locations 0..3 and
/// is written as four RTs.
#[test]
fn an_output_array_of_four_writes_rt_0_to_3() {
    let k = [F4, F4, F4, F4];
    let mut s =
        String::from("#version 300 es\nprecision highp float;\nlayout(location = 0) out vec4 o[4];\nvoid main() {\n");
    for rt in 0..4 {
        s += &format!("  o[{rt}] = {};\n", Kind::F(4).value(rt));
    }
    s += "}\n";
    let (mask, w) = run_fs(&s);
    assert_eq!(mask, 0b1111);
    assert_eq!(w, mesa_all(k), "as four separate outputs");
}

/// The array written in a loop (the shape of dEQP's `fragment_out.array`):
/// RT i holds i+1 .. i+4, the literals `Kind::F` gives for rt + j.
#[test]
fn an_output_array_written_in_a_loop() {
    let k = [F4, F4, F4, F4];
    let s = "#version 300 es\nprecision highp float;\nlayout(location = 0) out vec4 o[4];\n\
             void main() { for (int i = 0; i < 4; i++) { float f = float(i);\n\
             o[i] = vec4(f + 1.0, f + 2.0, f + 3.0, f + 4.0); } }\n";
    let (mask, w) = run_fs(s);
    assert_eq!(mask, 0b1111);
    assert_eq!(w, mesa_all(k));
}

/* ---- only render target 3 ---------------------------------------------------- */

/// A shader that writes only RT 3: one group, its config names RT 3 (not the
/// `~0` default, so TLBU carries it), and no write for RT 0..2.
#[test]
fn a_shader_that_writes_only_rt_3() {
    let k = [None, None, None, F4];
    check(k, &[3], &[3]);
    let (mask, w) = run_fs(&shader(k, &[3], &[3]));
    assert_eq!(mask, 0b1000);
    assert_eq!(w.iter().filter(|x| x.0.is_some()).count(), 1);
    assert_eq!(w[0].0, Some(0xffff_ffe7));
    assert_eq!(groups(&w).len(), 1);
}

/// Sparse sets: nothing for the RTs between or before.
#[test]
fn sparse_outputs_leave_the_others_out() {
    check([None, F4, None, F4], &[3, 1], &[3, 1]);
    check([F4, None, None, F4], &[0, 3], &[3, 0]);
    check([None, None, Some(Kind::U(4)), F4], &[2, 3], &[3, 2]);
    check([None, None, None, Some(Kind::I(4))], &[3], &[3]);
}

/* ---- mixed classes on render target 3 ----------------------------------------- */

/// RT 0..2 `vec4`, RT 3 each class the compiler writes as Mesa does: f16 vec4
/// and vec3 (`TLB_VEC_SIZE_4_F16`), `ivec4`, `int`, `uvec4`, `uvec2` (32-bit
/// words, size - 1).
#[test]
fn rt_3_takes_every_format_class() {
    for k3 in [Kind::F(4), Kind::F(3), Kind::I(4), Kind::I(1), Kind::U(4), Kind::U(2), Kind::I(3), Kind::U(1)] {
        check([F4, F4, F4, Some(k3)], &[0, 1, 2, 3], &[0, 1, 2, 3]);
    }
}

/// Integer classes on all four, and integer rows mixed with float ones.
#[test]
fn four_integer_outputs_and_mixed_integer_float_rows() {
    let i4 = Some(Kind::I(4));
    let u4 = Some(Kind::U(4));
    check([i4, i4, i4, i4], &[0, 1, 2, 3], &[0, 1, 2, 3]);
    check([u4, u4, u4, u4], &[3, 2, 1, 0], &[0, 1, 2, 3]);
    check([F4, i4, u4, F4], &[0, 1, 2, 3], &[3, 2, 1, 0]);
    check([i4, F4, F4, u4], &[0, 1, 2, 3], &[0, 1, 2, 3]);
    check([Some(Kind::U(2)), Some(Kind::I(1)), Some(Kind::U(1)), Some(Kind::I(2))], &[0, 1, 2, 3], &[0, 1, 2, 3]);
    check([Some(Kind::I(3)), F4, Some(Kind::U(3)), Some(Kind::I(3))], &[0, 1, 2, 3], &[0, 1, 2, 3]);
}

/// The configs of RT 3 by class and width, from Mesa's formula.
#[test]
fn rt_3_configs_by_class_and_width() {
    assert_eq!(tlb_config_32b(3, 1), 0xffff_ff24);
    assert_eq!(tlb_config_32b(3, 2), 0xffff_ff25);
    assert_eq!(tlb_config_32b(3, 3), 0xffff_ff26);
    assert_eq!(tlb_config_32b(3, 4), 0xffff_ff27);
    assert_eq!(tlb_config_f16(3, 4), 0xffff_ffe7);
    assert_eq!(tlb_config_f16(3, 3), 0xffff_ffe7);
    /* Mesa gives one or two f16 components the 2-wide size (bit 0 clear) */
    assert_eq!(mesa_conf(3, Kind::F(2)), 0xffff_ffe6);
    assert_eq!(mesa_conf(3, Kind::F(1)), 0xffff_ffe6);
}

/// Writes per RT: two for a float vec3/vec4, a word per component for an
/// integer one, whatever the RT.
#[test]
fn the_number_of_writes_per_rt() {
    for (k3, n) in [(Kind::F(4), 2), (Kind::F(3), 2), (Kind::I(4), 4), (Kind::U(2), 2), (Kind::I(1), 1)] {
        let (_, w) = run_fs(&shader([F4, F4, F4, Some(k3)], &[0, 1, 2, 3], &[0, 1, 2, 3]));
        let g = groups(&w);
        assert_eq!(g.iter().map(|x| x.2.len()).collect::<Vec<_>>(), [2, 2, 2, n], "{k3:?}");
        assert_eq!(g[3].0, 3);
    }
}

/// Mesa's f16 branch for a `vec2` or `float` output (:1979-1983, :2028-2032):
/// `TLB_VEC_SIZE_2_F16` and one `vfpack(r, g)`, no `vfpack(b, a)`.
/// `select.rs` writes every float output with `tlb_config_f16(rt, 4)` and two
/// packs (emit_frag_end), so RT 3's `vec2` has the 4-wide size and a word too
/// many.
#[test]
fn rt_3_vec2_float_is_one_f16_word() {
    check([F4, F4, F4, Some(Kind::F(2))], &[0, 1, 2, 3], &[0, 1, 2, 3]);
}

#[test]
fn rt_3_scalar_float_is_one_f16_word() {
    check([F4, F4, F4, Some(Kind::F(1))], &[0, 1, 2, 3], &[0, 1, 2, 3]);
}

/// The same at RT 0, where Mesa's config is no longer `~0`: TLBU carries
/// `0xfffffffe`.
#[test]
fn a_vec2_float_output_at_rt_0_carries_its_config() {
    check([Some(Kind::F(2)), None, None, None], &[0], &[0]);
}

/// A narrow float output is written with its own size and one word.
#[test]
fn a_narrow_float_output_is_written_with_its_own_size() {
    let (_, w) = run_fs(&shader([F4, F4, F4, Some(Kind::F(2))], &[0, 1, 2, 3], &[0, 1, 2, 3]));
    let g = groups(&w);
    assert_eq!(g.len(), 4);
    assert_eq!(g[3].1, Some(tlb_config_f16(3, 2)));
    assert_eq!(g[3].2.len(), 1);
}
