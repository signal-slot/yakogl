// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The QPU interpreter (`src/shaderc/online/v3d/sim.rs`) validated on
//! MESA's code, before it is used to judge ours.
//!
//! For each real program pair of the corpus that Mesa compiles
//! (`tests/fixtures/v3d_mesa_programs.txt`, written by
//! `tools/v3d/sim_fixture.py`), and whose two GLSL sources our front end
//! lowers and `eval.rs` runs, Mesa's FS, VS and CS run on the interpreter
//! with inputs built from random values, and the IR evaluator runs the same
//! GLSL on the same values:
//!
//! * FS: the TLB colour writes (Mesa writes f16 pairs for RT0: `vfpack(r,
//!   g)`, `vfpack(b, a)`, `vir_emit_tlb_color_write`) against the IR's
//!   colour rounded to f16, and the discarded lanes.
//! * VS: the VPM rows (`v3d_nir_setup_vpm_layout_vs`: scaled x and y as
//!   `f2i32(ffloor(p * scale * rcp(w)))`, Zs, 1/Wc, then the point size when
//!   the program keeps it (ours do when the VS writes gl_PointSize; Mesa's
//!   fixtures, compiled for triangles, never), then the varyings in the FS's
//!   input order, 0 for one the VS does not write).
//! * CS: clip x, y, z, w, then the scaled x and y, then the point size when
//!   kept.
//!
//! Every pair lands in exactly one of: agree, disagree (first differing
//! stage, lane and value), unmodelled (the interpreter's reason), or not
//! tried (the front end, the evaluator or this harness could not run it,
//! with the reason). The population is pinned: tried == agree + disagree +
//! unmodelled, and each count.
//!
//! Comparisons are bit for bit, except where the program ran an SFU op
//! (RECIP, RSQRT, EXP, LOG, SIN), whose precision only the hardware knows:
//! then floats are compared to a relative 1e-5, f16 words to one ulp, and the
//! scaled coordinates to one unit.

// The V3D compiler exists only in a gpu-v3d build (tools/check.sh runs this
// file in the gpu-v3d rows).
#![cfg(all(feature = "shaderc-online", feature = "gpu-v3d"))]

use std::collections::BTreeMap;
use yakogl::shaderc::online::arena::Arena;
use yakogl::shaderc::online::eval;
use yakogl::shaderc::online::ir::{IoDecl, IoKind, IoSlot, Module};
use yakogl::shaderc::online::v3d::select::{
    QUNIFORM_FB_PNTC_Y_OFFSET, QUNIFORM_FB_PNTC_Y_SCALE, QUNIFORM_FB_WPOS_Y_OFFSET, QUNIFORM_FB_WPOS_Y_SCALE,
};
use yakogl::shaderc::online::v3d::sim::{self, FsInputs, Inputs, Varying, VsInputs, LANES};
use yakogl::shaderc::online::{lower, sema};

static FIXTURE: &str = include_str!("fixtures/v3d_mesa_programs.txt");

// ---------------------------------------------------------------------------
// The fixture.
// ---------------------------------------------------------------------------

#[derive(Default, Debug, Clone)]
struct Stage {
    meta: BTreeMap<String, String>,
    inputs: Vec<(u32, u32)>,
    vattr: Vec<u32>,
    code: Vec<u64>,
    uniforms: Vec<(u32, u32)>,
}

/// (slot, vec4s, state) of one uniform in one stage.
type MesaUniform = (Option<u32>, u32, Option<String>);

#[derive(Default, Debug, Clone)]
struct Pair {
    vs_path: String,
    fs_path: String,
    /// (stage, dir) -> name -> location
    io: BTreeMap<(String, String), BTreeMap<String, u32>>,
    /// name, location, records
    attrs: Vec<(String, u32, Vec<Option<u32>>)>,
    /// (stage "frag"/"vert", name) -> every entry Mesa lists for it
    uniforms: BTreeMap<(String, String), Vec<MesaUniform>>,
    /// (stage, sampler name) -> Mesa's binding: the texture index its code
    /// carries in TMU config P0 (v3d_shc.c, uniform_names)
    bindings: BTreeMap<(String, String), u32>,
    fs: Stage,
    vs: Stage,
    cs: Stage,
    /// Built by OUR compiler (v3d::chain), not read from Mesa's fixture.
    ours: bool,
    /// A mutant of a fixture program (tmu_mutants): its stream may name a
    /// texture the fixture's bindings do not, on purpose.
    mutant: bool,
}

fn header(key: &str) -> usize {
    let p = format!("# {key}: ");
    FIXTURE
        .lines()
        .find_map(|l| l.strip_prefix(p.as_str()))
        .unwrap_or_else(|| panic!("no '{key}' header"))
        .parse()
        .expect("a number")
}

fn hex(s: &str) -> u64 {
    u64::from_str_radix(s.trim_start_matches("0x"), 16).unwrap_or_else(|e| panic!("{s}: {e}"))
}

/// Every real pair of the corpus (the fixture's `real` lines), whether Mesa
/// compiles it or not.
fn real_pairs() -> Vec<(String, String)> {
    FIXTURE
        .lines()
        .filter_map(|l| l.strip_prefix("real "))
        .map(|l| {
            let (vs, fs) = l.split_once(' ').expect("real <vs> <fs>");
            (vs.to_string(), fs.to_string())
        })
        .collect()
}

fn parse() -> Vec<Pair> {
    let mut out = Vec::new();
    let mut p: Option<Pair> = None;
    for line in FIXTURE.lines() {
        if line.starts_with('#') || line.is_empty() || line.starts_with("real ") {
            continue;
        }
        let w: Vec<&str> = line.split(' ').collect();
        if w[0] == "pair" {
            p = Some(Pair { vs_path: w[1].into(), fs_path: w[2].into(), ..Default::default() });
            continue;
        }
        let cur = p.as_mut().unwrap_or_else(|| panic!("a line outside a pair: {line}"));
        match w[0] {
            "end" => out.push(p.take().unwrap()),
            "io" => {
                cur.io.entry((w[1].into(), w[2].into())).or_default().insert(w[3].into(), w[4].parse().unwrap());
            }
            "attr" => {
                let recs = w[3].split(',').map(|r| if r == "-" { None } else { Some(r.parse().unwrap()) }).collect();
                cur.attrs.push((w[1].into(), w[2].parse().unwrap(), recs));
            }
            "uniform" => {
                let slot = w[3].strip_prefix("slot=").map(|s| s.parse().unwrap());
                if let Some(b) = w[3].strip_prefix("binding=") {
                    cur.bindings.insert((w[1].into(), w[2].into()), b.parse().unwrap());
                }
                cur.uniforms.entry((w[1].into(), w[2].into())).or_default().push((
                    slot,
                    w[4].parse().unwrap(),
                    w.get(5).map(|s| s.to_string()),
                ));
            }
            "fs" | "vs" | "cs" => {
                let st = match w[0] {
                    "fs" => &mut cur.fs,
                    "vs" => &mut cur.vs,
                    _ => &mut cur.cs,
                };
                match w[1] {
                    "meta" => {
                        for kv in &w[2..] {
                            let (k, v) = kv.split_once('=').expect("key=value");
                            st.meta.insert(k.into(), v.into());
                        }
                    }
                    "input" => st.inputs.push((w[2].parse().unwrap(), w[3].parse().unwrap())),
                    "vattr" => st.vattr = w[2].split(',').map(|x| x.parse().unwrap()).collect(),
                    "code" => st.code = w[2..].iter().filter(|s| !s.is_empty()).map(|s| hex(s)).collect(),
                    "uniforms" => {
                        st.uniforms = w[2..]
                            .iter()
                            .filter(|s| !s.is_empty())
                            .map(|s| {
                                let (c, d) = s.split_once(':').unwrap();
                                (c.parse().unwrap(), hex(d) as u32)
                            })
                            .collect()
                    }
                    k => panic!("unknown stage line {k}"),
                }
            }
            k => panic!("unknown line {k}"),
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Inputs.
// ---------------------------------------------------------------------------

/// The generator, with two modes: `.1` positive inputs only (the retry, see
/// run_pair); `.2` special values ([`Rng::attr`]): half the attribute
/// components are 0 (a quarter) or one of {0.5, -0.5, 1, -1}, for the negative control's
/// extra draws ([`control`]). On the default draws no attribute is 0 and no
/// two are equal, so `all(uv == 0)` never holds and `|x| + |y| == 0` cannot
/// be told from `|y| - |x| == 0`. Never 0 in w, which would put W at 0 for
/// a position attribute, and never in a uniform.
struct Rng(u64, bool, bool);
impl Rng {
    fn next(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 16) as u32
    }
    /// A float in [-2, 2] with |x| >= 0.25: no denormal, no near-zero W.
    fn float(&mut self) -> f32 {
        let x = (self.next() % 7001) as f32 / 4000.0 + 0.25;
        // The second field: positive inputs only (the retry for a program
        // whose operand domain the first draw left, see run_pair).
        if self.next() & 1 == 0 || self.1 {
            x
        } else {
            -x
        }
    }
    /// Attribute component `c`: [`Rng::float`], or in the special-values
    /// mode half the time one of a few values that repeat.
    fn attr(&mut self, c: usize) -> f32 {
        const SPECIAL: [f32; 4] = [0.5, -0.5, 1.0, -1.0];
        if self.2 && self.next().is_multiple_of(2) {
            // 0 half the time (so both of a vec2 are 0 in one lane in 16).
            if self.next().is_multiple_of(2) {
                if c < 3 {
                    return 0.0;
                }
            } else {
                return SPECIAL[self.next() as usize % SPECIAL.len()];
            }
        }
        self.float()
    }
    fn pos(&mut self) -> f32 {
        self.float().abs()
    }
}

/// A GL uniform's shape: (columns, rows, integer or bool).
fn shape(gl_type: u16) -> Option<(u32, u32, bool)> {
    Some(match gl_type {
        0x1406 => (1, 1, false),
        0x8b50 => (1, 2, false),
        0x8b51 => (1, 3, false),
        0x8b52 => (1, 4, false),
        // int, bool, and uint and uvecN (GL_UNSIGNED_INT 0x1405,
        // GL_UNSIGNED_INT_VEC2..4 0x8dc6..0x8dc8; drawn 1..4 below).
        0x1404 | 0x8b56 | 0x1405 => (1, 1, true),
        0x8b53 | 0x8b57 | 0x8dc6 => (1, 2, true),
        0x8b54 | 0x8b58 | 0x8dc7 => (1, 3, true),
        0x8b55 | 0x8b59 | 0x8dc8 => (1, 4, true),
        0x8b5a => (2, 2, false),
        0x8b5b => (3, 3, false),
        0x8b5c => (4, 4, false),
        _ => return None,
    })
}

/// The shift [`trig_undetermined`] gives sin/cos, relative to max(1, |x|):
/// 2^-20, past the f32 argument reduction's error that Mesa's sequence (and
/// ours) has (5.3e-6 at x = 91, measured for qtparticleshaders_21's hash;
/// the bound there is 8.7e-5), and far below anything a picture shows.
const TRIG_SHIFT: f32 = 1.0 / 1_048_576.0;

/// The message of an output the language does not fix, one string (a
/// histogram key).
const TRIG_UNDETERMINED: &str = "unmeasured: undefined-precision trig (ES 3.00 4.5.1)";

/// Whether the IR's output `pick` moves past `close` when every sin/cos
/// result is shifted by +-[`TRIG_SHIFT`]: then the exact-sin value the IR
/// gives is one the language leaves open, and a disagreement there is not a
/// verdict. Asked only at a disagreement, so a pair whose outputs do not
/// depend on the shift is judged as before (a real defect in a shader that
/// happens to use sin still shows). A program with no sin/cos answers no.
/// Only the stage being compared is shifted: a hash in a vertex shader
/// feeding a varying still reads as a disagreement at the FS (the
/// conservative side; none in the corpus today).
fn trig_undetermined(
    m: &Module<'_>,
    input: &eval::Inputs<'_>,
    tex: &[eval::Texture],
    want: u32,
    pick: &dyn Fn(&eval::Outcome) -> Option<u32>,
    close: &dyn Fn(u32, u32) -> bool,
) -> bool {
    use yakogl::shaderc::online::ir::{AluOp, Op};
    let trig = m.instrs.iter().any(|i| matches!(i, Op::Alu(AluOp::Fsin | AluOp::Fcos, _)));
    trig && [TRIG_SHIFT, -TRIG_SHIFT].iter().any(|&d| {
        eval::run_with_trig_error(m, input, tex, d).ok().and_then(|o| pick(&o)).is_some_and(|v| !close(want, v))
    })
}

/// [`trig_undetermined`] on one shader with three outputs: Qt Quick 3D's
/// hash `fract(sin(k * 12.9898) * 43758.5453)` moves past the f16 tolerance
/// under the shift (the language does not fix it), `sin(k)` itself does not
/// (a disagreement there stays a disagreement), and `k * 2.0` has no sin
/// behind it. k = 7: the argument is 90.93, where the f32 reduction was
/// measured 5.3e-6 off. Revert check: a TRIG_SHIFT of 0 makes the first
/// assertion fail.
#[test]
fn only_an_output_the_trig_shift_moves_is_undetermined() {
    let mut mem = vec![0u8; 4 << 20];
    let a = Arena::new(&mut mem);
    let src = a
        .alloc_str(
            "precision highp float;\nuniform float k;\n\
             void main() { gl_FragColor = vec4(fract(sin(k * 12.9898) * 43758.5453), sin(k), k * 2.0, 1.0); }\n",
        )
        .unwrap();
    let l = lower::compile_to_ir(&a, sema::Stage::Fragment, src).map_err(|e| e.msg).unwrap();
    let m = &l.module;
    let d = m.uniforms.iter().find(|d| d.name == "k").expect("k");
    let mut uniforms = vec![0u32; m.uniform_dwords as usize];
    uniforms[d.dword_off as usize] = 7.0f32.to_bits();
    let input = eval::Inputs { io: &[], uniforms: &uniforms };
    let exact = eval::run(m, &input).expect("runs").output(IoSlot::FragColor(0)).expect("stored");
    let close =
        |a: u32, b: u32| close_f16(sim::f32_to_half(f32::from_bits(a)), sim::f32_to_half(f32::from_bits(b)), true);
    let at = |k: usize| move |o: &eval::Outcome| o.output(IoSlot::FragColor(0)).map(|c| c[k]);
    assert!(trig_undetermined(m, &input, &[], exact[0], &at(0), &close), "the hash is not fixed by the language");
    assert!(!trig_undetermined(m, &input, &[], exact[1], &at(1), &close), "sin(k) itself stays within the tolerance");
    assert!(!trig_undetermined(m, &input, &[], exact[2], &at(2), &close), "k * 2.0 does not move");
}

/// Why a pair is not tried, or why a stage stops.
enum Skip {
    NotTried(String),
    Unmodelled(String),
}

/// The uniform values of both layouts, from one draw per (name, element,
/// column, row): the IR's std430 block, and Mesa's per-stage vec4 slots
/// (`QUNIFORM_UNIFORM` data is the dword offset slot * 4 + component, a
/// column or array element per slot; `uniform_names`' `vec4s` says so).
struct Uniforms {
    fs_block: Vec<u32>,
    vs_block: Vec<u32>,
    /// (stage "frag"/"vert", Mesa dword) -> value
    mesa: BTreeMap<(String, u32), u32>,
}

/// Uniform values the source restricts or the draw must hit: the draw
/// keeps inputs inside the program's defined behaviour, or on the path a
/// control must see, so that programs are compared on what matters. Each
/// entry: a stage source, the uniform, its element and component (0 for a
/// non-array or a scalar), the value's bits, and the reason.
/// The reason for the Qt radial-gradient entries of [`UNIFORM_OVERRIDES`].
const RADIAL_GRADIENT_DOMAIN: &str = "Qt's radial gradient: with focalToCenter = 0, centerRadius = 1 and \
     focalRadius = 0.5, det = |coord|^2 >= 0 and the focal test is |coord| >= 0, so \
     texture2D(vec2(2|coord| - 1, 0.5)) reaches the colour; on random uniforms det < 0 \
     left vec4(0) in every lane and the mutants on coord survived every draw \
     (measured; caught with these)";

/// The reason for the skybox_300 entries of [`UNIFORM_OVERRIDES`].
const SKYBOX_LEVEL: &str = "Qt's skybox: textureLod(skybox_image, eye, mix(0.0, w, z)) of \
     ubuf.skyboxProperties; z = 1, w = 0.75 makes the level of detail 0.75, which dEQP's \
     NEAREST_MIPMAP_NEAREST rule reads as level 1, clear of both level boundaries \
     (0.5 and 1.5); on random uniforms the deciding draw read level 0, so no agreeing \
     pair of Mesa's read past the first level (measured). Not 1.0: the TMUB control \
     trades the level of detail into T, and a T of 1.0 tied with the eye vector's unit \
     component on every lane (a face edge: measured, 'unmeasured')";

/// The reason for the dEQP loop entries of [`UNIFORM_OVERRIDES`].
const LOOP_VALUES: &str = "dEQP's values (es2fShaderLoopTests.cpp: ui_one..ui_three = 1..3); on random values \
     a uniform-bounded loop runs up to the unroller's pass bound, a count the dEQP case never draws";

/// The reason for the dEQP indexing entries of [`UNIFORM_OVERRIDES`].
const DYNIDX_VALUES: &str = "dEQP's values (es2fShaderIndexingTests.cpp: ui_zero..ui_three = 0..3, \
     ui_four = 4); any other value writes lanes the reads never see and leaves read lanes undefined, \
     or reads past the vector's end";

const UNIFORM_OVERRIDES: &[(&str, &str, u32, u32, u32, &str)] = &[
    ("qt3d/skybox_300.frag", "ubuf.skyboxProperties", 0, 2, 0x3f80_0000, SKYBOX_LEVEL), // z = 1.0
    ("qt3d/skybox_300.frag", "ubuf.skyboxProperties", 0, 3, 0x3f40_0000, SKYBOX_LEVEL), // w = 0.75
    ("es100/dynidx.frag", "ui_zero", 0, 0, 0, DYNIDX_VALUES),
    ("es100/dynidx.frag", "ui_one", 0, 0, 1, DYNIDX_VALUES),
    ("es100/dynidx.frag", "ui_two", 0, 0, 2, DYNIDX_VALUES),
    ("es100/dynidx.frag", "ui_three", 0, 0, 3, DYNIDX_VALUES),
    ("es100/dynidx_loop.frag", "ui_four", 0, 0, 4, DYNIDX_VALUES),
    ("es100/loop_uniform.frag", "ui_three", 0, 0, 3, LOOP_VALUES),
    ("es100/ret_dynloop_never.frag", "ui_one", 0, 0, 1, LOOP_VALUES),
    ("es100/loop_uniform.dowhile.vert", "ui_three", 0, 0, 3, LOOP_VALUES),
    ("es100/loop_nested.frag", "ui_one", 0, 0, 1, LOOP_VALUES),
    ("es100/loop_nested.frag", "ui_two", 0, 0, 2, LOOP_VALUES),
    ("es100/loop_nested.frag", "ui_three", 0, 0, 3, LOOP_VALUES),
    ("es100/loop_nested.vert", "ui_two", 0, 0, 2, LOOP_VALUES),
    ("es100/loop_nested.vert", "ui_three", 0, 0, 3, LOOP_VALUES),
    ("es100/loop_nested.while.frag", "ui_one", 0, 0, 1, LOOP_VALUES),
    ("es100/loop_nested.while.frag", "ui_two", 0, 0, 2, LOOP_VALUES),
    ("es100/loop_nested.while.frag", "ui_three", 0, 0, 3, LOOP_VALUES),
    ("es100/loop_nested.dowhile.frag", "ui_one", 0, 0, 1, LOOP_VALUES),
    ("es100/loop_nested.dowhile.frag", "ui_two", 0, 0, 2, LOOP_VALUES),
    ("es100/loop_nested.dowhile.frag", "ui_three", 0, 0, 3, LOOP_VALUES),
    ("es100/discard_dynloop.frag", "ui_one", 0, 0, 1, LOOP_VALUES),
    ("es100/discard_dynloop.frag", "ui_two", 0, 0, 2, LOOP_VALUES),
    ("es100/tmparr_const.frag", "ui_zero", 0, 0, 0, DYNIDX_VALUES),
    ("es100/tmparr_const.frag", "ui_one", 0, 0, 1, DYNIDX_VALUES),
    ("es100/tmparr_const.frag", "ui_two", 0, 0, 2, DYNIDX_VALUES),
    ("es100/tmparr_const.frag", "ui_three", 0, 0, 3, DYNIDX_VALUES),
    ("es100/tmparr_const.vert", "ui_zero", 0, 0, 0, DYNIDX_VALUES),
    ("es100/tmparr_const.vert", "ui_one", 0, 0, 1, DYNIDX_VALUES),
    ("es100/tmparr_const.vert", "ui_two", 0, 0, 2, DYNIDX_VALUES),
    ("es100/tmparr_const.vert", "ui_three", 0, 0, 3, DYNIDX_VALUES),
    (
        "qt/radialgradient.frag",
        "ubuf.focalToCenter",
        0,
        0,
        0x0000_0000, // 0.0
        RADIAL_GRADIENT_DOMAIN,
    ),
    (
        "qt/radialgradient.frag",
        "ubuf.focalToCenter",
        0,
        1,
        0x0000_0000, // 0.0
        RADIAL_GRADIENT_DOMAIN,
    ),
    (
        "qt/radialgradient.frag",
        "ubuf.centerRadius",
        0,
        0,
        0x3f80_0000, // 1.0
        RADIAL_GRADIENT_DOMAIN,
    ),
    (
        "qt/radialgradient.frag",
        "ubuf.focalRadius",
        0,
        0,
        0x3f00_0000, // 0.5
        RADIAL_GRADIENT_DOMAIN,
    ),
    (
        "qt/radialgradient.frag",
        "ubuf.opacity",
        0,
        0,
        0x3f80_0000, // 1.0
        RADIAL_GRADIENT_DOMAIN,
    ),
    (
        "qt/shapecurve_rg.frag",
        "ubuf.focalToCenter",
        0,
        0,
        0x0000_0000, // 0.0
        RADIAL_GRADIENT_DOMAIN,
    ),
    (
        "qt/shapecurve_rg.frag",
        "ubuf.focalToCenter",
        0,
        1,
        0x0000_0000, // 0.0
        RADIAL_GRADIENT_DOMAIN,
    ),
    (
        "qt/shapecurve_rg.frag",
        "ubuf.centerRadius",
        0,
        0,
        0x3f80_0000, // 1.0
        RADIAL_GRADIENT_DOMAIN,
    ),
    (
        "qt/shapecurve_rg.frag",
        "ubuf.focalRadius",
        0,
        0,
        0x3f00_0000, // 0.5
        RADIAL_GRADIENT_DOMAIN,
    ),
    (
        "qt/shapecurve_rg.frag",
        "ubuf.opacity",
        0,
        0,
        0x3f80_0000, // 1.0
        RADIAL_GRADIENT_DOMAIN,
    ),
    (
        "qt/shapecurve_rg.frag",
        "ubuf.debug",
        0,
        0,
        0x0000_0000, // 0.0: the mix keeps baseColor()
        RADIAL_GRADIENT_DOMAIN,
    ),
    (
        "es100/matdyn.frag",
        "u_i",
        0,
        0,
        1,
        "arr[i], arr[i + 1] with float arr[3] and col[i - 1] with vec2 col are in \
         range only for i == 1; GLSL ES 1.00 r17 sections 4.1.9 and 5.5 make a \
         non-constant index out of range undefined behaviour, and on u_i = 0 the \
         IR read col[1] where Mesa's code read col[0]",
    ),
    (
        "femtovg/fill_image.glyph.frag",
        "frag",
        11,
        0,
        0x3fc0_0000, // 1.5: glyphTextureType = int(frag[11].x) = 1
        "glyphTextureType 1 (femtovg's alpha-only glyph atlas) is the only path \
         on which renderImage()'s texel reaches the colour (otherwise result is \
         set to vec4(1)); random draws never hit it, and the TMUT/TMUS piece \
         control of this pair survived until they did (caught with it, measured)",
    ),
];

/// One uniform value a scenario sets: (uniform, element, component, bits).
type ScenarioValue = (&'static str, u32, u32, u32);

thread_local! {
    /// Uniform values a scenario of [`executed_instruction_counts_are_pinned`]
    /// sets in every pair it runs; they come before [`UNIFORM_OVERRIDES`].
    /// Empty everywhere else.
    static SCENARIO: core::cell::RefCell<Vec<ScenarioValue>> = const { core::cell::RefCell::new(Vec::new()) };
}

/// The value the running scenario, else [`UNIFORM_OVERRIDES`], gives element
/// `e`, component `r` of uniform `name` of pair `p`.
fn uniform_override(p: &Pair, name: &str, e: u32, r: u32) -> Option<u32> {
    if let Some(v) = SCENARIO.with(|s| s.borrow().iter().find(|x| x.0 == name && x.1 == e && x.2 == r).map(|x| x.3)) {
        return Some(v);
    }
    UNIFORM_OVERRIDES
        .iter()
        .find(|(src, u, oe, orr, _, _)| (p.fs_path == *src || p.vs_path == *src) && *u == name && *oe == e && *orr == r)
        .map(|(_, _, _, _, v, _)| *v)
}

fn build_uniforms(p: &Pair, fsm: &Module, vsm: &Module, rng: &mut Rng) -> Result<Uniforms, Skip> {
    let mut vals: BTreeMap<(String, u32, u32, u32), u32> = BTreeMap::new();
    let mut u = Uniforms {
        fs_block: vec![0; fsm.uniform_dwords as usize],
        vs_block: vec![0; vsm.uniform_dwords as usize],
        mesa: BTreeMap::new(),
    };
    for (st, m) in [("frag", fsm), ("vert", vsm)] {
        let slots = mesa_slots(p, st, m)?;
        for d in m.uniforms.iter() {
            if d.dword_off == yakogl::shaderc::SHC_UNIFORM_NO_BLOCK {
                continue;
            }
            let Some((cols, rows, int)) = shape(d.gl_type) else {
                return Err(Skip::NotTried(format!("uniform {} of GL type {:#x}", d.name, d.gl_type)));
            };
            let bool_ = matches!(d.gl_type, 0x8b56..=0x8b59);
            let unsigned = matches!(d.gl_type, 0x1405 | 0x8dc6..=0x8dc8);
            let len = d.array_len.max(1) as u32;
            let stride = d.dwords as u32 / len;
            let colstride = if cols == 2 { 2 } else { 4 };
            let mesa_slot = slots.get(d.name).copied();
            for e in 0..len {
                for c in 0..cols {
                    for r in 0..rows {
                        let v = *vals.entry((d.name.to_string(), e, c, r)).or_insert_with(|| {
                            if bool_ {
                                // Mesa's GL stores true as ~0 on a driver with
                                // native integers (st_extensions.c:1356); the
                                // IR reads any non-zero dword as true (I2b).
                                if rng.next() & 1 == 0 {
                                    0
                                } else {
                                    u32::MAX
                                }
                            } else if let Some(v) = uniform_override(p, d.name, e, c * rows + r) {
                                v
                            } else if unsigned {
                                // 1..4: the corpus's uints are counts that
                                // divide (Qt Quick 3D particles'
                                // qt_countPerSlice), and a division by zero
                                // is undefined, so the evaluator refuses it
                                1 + rng.next() % 4
                            } else if int {
                                rng.next() % 4
                            } else {
                                rng.float().to_bits()
                            }
                        });
                        let ir = (d.dword_off as u32 + e * stride + c * colstride + r) as usize;
                        let block = if st == "frag" { &mut u.fs_block } else { &mut u.vs_block };
                        if let Some(x) = block.get_mut(ir) {
                            *x = v;
                        }
                        if let Some(slot) = mesa_slot {
                            u.mesa.insert((st.to_string(), (slot + e * cols + c) * 4 + r), v);
                        }
                    }
                }
            }
        }
    }
    // Mesa's own state uniforms, which the IR does not have. The only one a
    // triangle draw reads is gl_FbWposYTransform (a gl_FragCoord read):
    // nir_lower_wpos_ytransform makes y' = (y + adjY) * T[base] + T[base + 1]
    // (nir_lower_wpos_ytransform.c:104-110), and with v3d's conventions
    // (origin upper left, pixel centre half-integer; v3d_shc.c:189-192) and
    // an ES shader's (lower left, half-integer) adjY is 0 and base is 0.
    // T[0], T[1] are the window transform the IR's gl_FragCoord.y is given
    // (WPOS_SCALE, WPOS_OFFSET, the ones our QUNIFORM_FB_WPOS_Y_SCALE/_OFFSET
    // reads); T[2], T[3], the other orientation, are not read.
    for ((st, _), entries) in p.uniforms.iter() {
        let Some((slot, _, state)) = entries.first() else { continue };
        match (state.as_deref(), slot) {
            (None, _) => {}
            (Some("FB_WPOS_Y_TRANSFORM"), Some(slot)) => {
                for (c, v) in [WPOS_SCALE, WPOS_OFFSET, 1.0, 0.0].iter().enumerate() {
                    u.mesa.insert((st.clone(), slot * 4 + c as u32), v.to_bits());
                }
            }
            (Some(other), _) => return Err(Skip::NotTried(format!("state uniform {other}"))),
        }
    }
    Ok(u)
}

/// Mesa's vec4 slot of each IR uniform row of one stage. A row Mesa names
/// itself has its slot. A struct uniform is one Mesa uniform (`ubuf`, 28
/// vec4s) and several IR rows (`ubuf.mvp`, ...): its members take vec4 slots
/// in declaration order, one per column or array element (the vec4 count
/// Mesa reports is checked against the sum).
fn mesa_slots<'m>(p: &Pair, st: &str, m: &Module<'m>) -> Result<BTreeMap<&'m str, u32>, Skip> {
    let mut out = BTreeMap::new();
    // struct -> (next slot, end)
    let mut next: BTreeMap<String, (u32, u32)> = BTreeMap::new();
    // Mesa may list one name more than once (es300/cube: `axis` in the VS
    // twice, after its link step moved an FS expression into the VS). Which
    // entry a dword belongs to is then not something this mapping can tell.
    for ((s, name), v) in p.uniforms.iter() {
        if s == st && v.len() > 1 {
            return Err(Skip::NotTried(format!("Mesa lists {st} uniform {name} {} times", v.len())));
        }
    }
    let one = |k: (String, String)| p.uniforms.get(&k).and_then(|v| v.first());
    for d in m.uniforms.iter().filter(|d| d.dword_off != yakogl::shaderc::SHC_UNIFORM_NO_BLOCK) {
        if let Some((slot, _, _)) = one((st.to_string(), d.name.to_string())) {
            if let Some(s) = slot {
                out.insert(d.name, *s);
            }
            continue;
        }
        let Some((outer, _)) = d.name.split_once('.') else { continue };
        let Some((Some(base), vec4s, _)) = one((st.to_string(), outer.to_string())) else { continue };
        let (at, end) = next.entry(outer.to_string()).or_insert((*base, base + vec4s));
        let Some((cols, _, _)) = shape(d.gl_type) else {
            return Err(Skip::NotTried(format!("uniform {} of GL type {:#x}", d.name, d.gl_type)));
        };
        out.insert(d.name, *at);
        *at += cols * d.array_len.max(1) as u32;
        if *at > *end {
            return Err(Skip::NotTried(format!("struct uniform {outer}: members overrun its {vec4s} vec4s")));
        }
    }
    for (outer, (at, end)) in next {
        if at != end {
            return Err(Skip::NotTried(format!("struct uniform {outer}: members fill {at} of {end} vec4 slots")));
        }
    }
    Ok(out)
}

/// The system values a lane's vertex carries.
const INSTANCE_ID: u32 = 3;
/// The vertex ID of each lane: lane + 7, so an ID read as the lane number
/// shows, and past the small integers whose float bits' low halves are 0.
/// A vertex shader that indexes an array by the ID gets IDs inside it:
/// every such shader in the corpus is a Qt Quick 3D sprite particle that
/// draws a 4-vertex strip per instance and reads `const vec2 _252[4]` (or
/// its twins) at gl_VertexID, and an ID past 3 is an out-of-bounds read,
/// undefined; with lane + 7 there the corner carried nothing a draw could
/// vary, and a mutant on `texcoord.x * qt_spriteConfig.y` survived every
/// draw of qtparticleshaders_58 (2026-09-26). (lane + 3) % 4 still differs
/// from the lane number in every lane. The array length is checked, not
/// assumed: another length is refused as unmodelled.
fn vertex_ids(p: &Pair) -> Result<[u32; LANES], Skip> {
    let full = format!("{}/tools/shaders/{}", env!("CARGO_MANIFEST_DIR"), p.vs_path);
    let text = std::fs::read_to_string(&full).map_err(|e| Skip::NotTried(format!("read {}: {e}", p.vs_path)))?;
    let indexed = ["[cornerIndex]", "[gl_VertexID]"].iter().find_map(|k| {
        let at = text.find(k)?;
        let name = text[..at].rsplit(|c: char| !(c.is_alphanumeric() || c == '_')).next()?;
        Some(name.to_string())
    });
    let Some(name) = indexed else {
        return Ok(core::array::from_fn(|l| l as u32 + 7));
    };
    if !text.contains(&format!("{name}[4] =")) {
        return Err(Skip::Unmodelled(format!("{}: {name} indexed by the vertex ID is not a [4] array", p.vs_path)));
    }
    Ok(core::array::from_fn(|l| (l as u32 + 3) % 4))
}

/// Where the harness puts a stage's uniform storage (Gallium's constant
/// buffer 0) for LDUNIFA: any address serves; this one is not 0, so an
/// address computed from nothing does not land in it.
const UNIFORM_BASE: u32 = 0x1000_0000;

/// The memory a stage's LDUNIFA may read: its uniform storage, the vec4
/// slots of `QUNIFORM_UNIFORM` (slot * 4 + component dwords), with padding
/// left undefined. Mesa's program is the only one that reads it: ours emits
/// no UBO address yet. And the textures, the same set for both stages. The
/// thread TIDX reads is the program's last one (`threads` - 1), so a spill
/// address's per-thread term is not 0.
fn env(u: &Uniforms, stage: &str, st: &Stage, texs: &[sim::Texture]) -> sim::Env {
    let dwords: Vec<(u32, u32)> = u.mesa.iter().filter(|((s, _), _)| s == stage).map(|((_, d), v)| (*d, *v)).collect();
    let n = dwords.iter().map(|(d, _)| *d as usize + 1).max().unwrap_or(0);
    let mut words = vec![None; n];
    for (d, v) in dwords {
        words[d as usize] = Some(v);
    }
    // The program's thread count, as its prog_data says: absent is passed as
    // 0, which the interpreter refuses at a TMU look-up rather than assume.
    let threads: Option<u32> = st.meta.get("threads").and_then(|t| t.parse().ok());
    sim::Env {
        memory: vec![sim::Region { base: UNIFORM_BASE, words }],
        textures: texs.to_vec(),
        // no sampler entries: NEAREST, the texture's minification filter,
        // CLAMP_TO_EDGE, as the harness textures were always sampled
        samplers: Vec::new(),
        thread: threads.map_or(0, |t| t.max(1) - 1),
        threads: threads.unwrap_or(0),
        // derivatives stay unmodelled: the corpus' pinned counts were
        // taken without a quad layout
        quads: false,
    }
}

/// The textures eval.rs samples for stage `st`, by the IR's sampler
/// binding, so that the IR reads the texels Mesa's program reads.
///
/// Mesa's code names a texture by its index in P0 (QUNIFORM_TMU_CONFIG_P0,
/// bits 24..31), which is the sampler's `data.binding` after
/// gl_nir_lower_samplers_as_deref.c:388; v3d_shc prints that binding per
/// sampler name, and element k of a sampler array is binding + k. The IR's
/// sampler with the same name reads the harness texture of that index. An
/// IR sampler Mesa does not list (it dropped an unused one) gets a texture
/// no index has (every texel -1), so a look-up through it cannot agree.
/// Our program's P0 names the IR's reflection binding itself (v3d-m5's
/// convention, v3d-backend.md). Every index in the stage's P0 words must be
/// one of the listed bindings, or the mapping is not believed and the pair
/// is not tried. (Our side is UNEXERCISED until our compiler selects a
/// texture look-up: chain.rs refuses every TMU access today.)
fn eval_textures(
    p: &Pair,
    st: &Stage,
    stage: &str,
    m: &Module,
    all: &[sim::Texture],
) -> Result<Vec<eval::Texture>, Skip> {
    let indices: std::collections::BTreeSet<u32> =
        st.uniforms.iter().filter(|(c, _)| *c == 40).map(|(_, d)| d >> 24).collect();
    if indices.is_empty() {
        return Ok(Vec::new());
    }
    // Of the sampler's kind, so a look-up through it reaches its texels
    // (and disagrees) rather than stopping on the kind.
    let unbound = |cube: bool| {
        let mut t = texture(0, cube, false);
        for l in &mut t.levels {
            for f in &mut l.faces {
                f.fill([-1.0; 4]);
            }
        }
        t
    };
    let mut listed = std::collections::BTreeSet::new();
    let mut out: Vec<eval::Texture> = Vec::new();
    for d in m.uniforms.iter().filter(|d| d.dword_off == yakogl::shaderc::SHC_UNIFORM_NO_BLOCK) {
        // The index the program's P0 names for this sampler: for Mesa's
        // program the binding v3d_shc printed; for ours the reflection
        // binding itself (v3d-backend.md, "Textures: the P0/P1 uniform
        // words": bits 24..31 name the sampler's binding, as Op::Tex does).
        let mesa = if p.ours {
            Some(u32::from(d.binding))
        } else {
            p.bindings.get(&(stage.to_string(), d.name.to_string())).copied()
        };
        let cube = d.gl_type == GL_SAMPLER_CUBE;
        for k in 0..u32::from(d.array_len.max(1)) {
            let b = usize::from(d.binding) + k as usize;
            while out.len() <= b {
                out.push(unbound(false));
            }
            out[b] = unbound(cube);
            if let Some(t) = mesa.and_then(|mb| all.get((mb + k) as usize)) {
                listed.insert(mesa.unwrap() + k);
                out[b] = t.clone();
            }
        }
    }
    if let Some(i) = indices.iter().find(|i| !listed.contains(i)).filter(|_| !p.mutant) {
        return Err(Skip::NotTried(format!(
            "{stage}: {} reads texture index {i}, which no IR sampler's binding names",
            who(p)
        )));
    }
    Ok(out)
}

/// How many textures the harness binds, by index: 16, every sampler binding
/// our compiler can emit (reflect.rs hands out 0..15; the GL core reports
/// GL_MAX_TEXTURE_IMAGE_UNITS = 16, context.rs CAP_TEXTURE_UNITS), so no
/// legal binding reads "not tried" for want of a texture.
const TEXTURE_UNITS: u32 = 16;
/// Odd, so that a coordinate of 0.5 (a gradient ramp sampled along its
/// middle row, vec2(t, 0.5): the Qt gradient shaders) lands on a texel
/// centre, 2.5, not on the edge 2.0 that a 4-texel side puts it on.
const TEXTURE_SIZE: u32 = 5;

/// What the texture model can and cannot vouch for (v3d-tex, from Mesa
/// 26.2.1 and our Slint path: femtovg 0.26 and Slint patched for no_std,
/// driver/samples/zephyr/slint/nostd): every texture on the path is
/// RGBA8 (text is composed on the CPU into a premultiplied RGBA8 texture per
/// Text item), with a 16-bit return, one level and the swizzle in the texture
/// record (so the program always sees R, G, B, A). The sampler is the
/// driver's, and on this path it is LINEAR for text and images alike
/// (ImageRendering::Smooth, CLAMP_TO_EDGE; gradients REPEAT on T), which
/// neither the interpreter nor eval.rs models: they sample NEAREST. So an
/// agreement vouches for the compiled TMU sequence (the config words, the
/// coordinate writes, the result order and their piece controls), never for
/// the filtered values a board shows; that evidence is the board's, and it
/// exists: gl_test case 109 "texlinear" PASSed on the Pi 4 (build 27f7382c;
/// docs/online-shaderc/v3d-backend.md, "LINEAR on the board"): LINEAR with
/// CLAMP_TO_EDGE and with REPEAT, 1024 of 1024 pixels within dEQP's
/// sampleLinear2D threshold, the largest error 0.56 of a 1/255 step (dEQP
/// allows 3.98). The TMU's rounding of an 8-bit texel n/255 to f16 is a
/// board question too; these texels are exact.
///
/// The texture bound to unit `u` (2D or cube, one level or TEXTURE_LEVELS,
/// as [`pair_textures`] decides): one level is 5x5 RGBA, texel (i, j) of
/// face f (0 for 2D, the GL face index for a cube) is
/// ((8u + i + 1)/128 + j/1024, (8u + j + 1)/128, (32u + i + 5j)/512 + f/4,
/// 1 - (32u + i + 5j)/1024); a mipmapped one is TEXTURE_MIP_SIZE wide (see
/// TEXTURE_LEVELS). Every value is exact in f16, so a 16-bit TMU result
/// carries it unrounded. EVERY channel names the unit (distinct across the
/// 16 at every texel), so a program that reads only one channel still sees
/// a wrong texture (the Qt distance-field text reads only .x); red names
/// the row as well as the column, so two look-ups one row apart differ in
/// .x too (qt/outlinedtext reads .x at sampleCoord and one row up: with red
/// naming the column only, its first two results were equal and trading
/// their LDTMU destinations survived, 2026-09-29); blue names the face and
/// alpha the level, so a wrong face or level shows; and none is symmetric
/// in i and j, so a swapped coordinate shows.
fn texture(u: u32, cube: bool, mip: bool) -> sim::Texture {
    use yakogl::shaderc::online::texref::{Level, MinFilter};
    let levels = if mip { TEXTURE_LEVELS } else { 1 };
    let level = |k: u32| {
        let n = ((if mip { TEXTURE_MIP_SIZE } else { TEXTURE_SIZE }) >> k).max(1);
        let face = |f: u32| -> Vec<[f32; 4]> {
            (0..n * n)
                .map(|x| {
                    let (i, j) = ((x % n) as f32, (x / n) as f32);
                    let (u, f, k) = (u as f32, f as f32, k as f32);
                    if mip {
                        let t = i + 7.0 * j;
                        [
                            (8.0 * u + i + 1.0) / 128.0 + j / 1024.0,
                            (8.0 * u + j + 1.0) / 128.0,
                            (64.0 * u + t) / 1024.0 + f / 8.0,
                            1.0 - (64.0 * u + t) / 2048.0 - k / 16.0,
                        ]
                    } else {
                        let t = i + 5.0 * j;
                        [
                            (8.0 * u + i + 1.0) / 128.0 + j / 1024.0,
                            (8.0 * u + j + 1.0) / 128.0,
                            (32.0 * u + t) / 512.0 + f / 4.0,
                            1.0 - (32.0 * u + t) / 1024.0 - k / 16.0,
                        ]
                    }
                })
                .collect()
        };
        Level { width: n, height: n, faces: (0..if cube { 6 } else { 1 }).map(face).collect() }
    };
    sim::Texture {
        cube,
        min: if mip { MinFilter::NearestMipmapNearest } else { MinFilter::Nearest },
        levels: (0..levels).map(level).collect(),
    }
}

/// The levels of a mipmapped harness texture: 7x7, 3x3, 1x1. Odd at every
/// level, as TEXTURE_SIZE is at its one, so a coordinate of 0.5 (a cube
/// face's centre, a ramp's middle row) is a texel centre on every level,
/// never an edge: from 5, level 1 would be 2x2, where 0.5 is the edge and
/// every such look-up is refused (measured: skybox_300's TMUB control read
/// "unmeasured" there). Its texels as [`texture`]'s, with 7 texels a row:
/// (8u + i + 1)/128 + j/1024, (8u + j + 1)/128, (64u + i + 7j)/1024 + f/8,
/// 1 - (64u + i + 7j)/2048 - k/16 for face f of level k; each exact in f16,
/// each channel still naming the unit.
const TEXTURE_LEVELS: u32 = 3;
const TEXTURE_MIP_SIZE: u32 = 7;

/// `GL_SAMPLER_CUBE`, a uniform's `gl_type` for a samplerCube.
const GL_SAMPLER_CUBE: u16 = 0x8b60;

/// The textures a pair's two programs read, by the index their P0 names:
/// for each index its kind and its levels, decided from the IR's look-ups
/// through it (both stages). A cube sampler's index is a cube; an index
/// every look-up of which has an explicit level of detail (`TexLod::Lod`,
/// and every vertex look-up is one: the front end makes an implicit one
/// `Lod(0.0)`) gets TEXTURE_LEVELS levels and NEAREST_MIPMAP_NEAREST, so
/// the level a program asks for shows in the value; every other index has
/// one level, read the same at any level of detail, so an implicit
/// look-up (the derivatives', which neither side models) stays comparable.
/// An index read both as 2D and as cube is not tried. An index the programs
/// do not read takes the kind and levels of its neighbour `u ^ 1`, so the
/// P0 control (texture index bit 24 flipped, tmu_mutants) reads a texture
/// of the kind its look-up asks for, with other texels, and must disagree
/// on the values; of the other kind, every draw would stop on the kind and
/// the control could only read "unmeasured".
fn pair_textures(p: &Pair, fsm: &Module, vsm: &Module) -> Result<Vec<sim::Texture>, Skip> {
    use yakogl::shaderc::online::ir::{Op, TexDim, TexLod};
    let mut plan: BTreeMap<u32, (bool, bool)> = BTreeMap::new();
    for (stage, m) in [("frag", fsm), ("vert", vsm)] {
        for op in m.instrs {
            let Op::Tex { dim, sampler, lod, .. } = *op else { continue };
            let Some((d, k)) =
                m.uniforms.iter().filter(|d| d.dword_off == yakogl::shaderc::SHC_UNIFORM_NO_BLOCK).find_map(|d| {
                    let k = u32::from(sampler).checked_sub(u32::from(d.binding))?;
                    (k < u32::from(d.array_len.max(1))).then_some((d, k))
                })
            else {
                continue;
            };
            let index = if p.ours {
                Some(u32::from(sampler))
            } else {
                p.bindings.get(&(stage.to_string(), d.name.to_string())).map(|b| b + k)
            };
            let Some(index) = index else { continue };
            let (cube, explicit) = (dim == TexDim::Cube, matches!(lod, TexLod::Lod(_)));
            let e = plan.entry(index).or_insert((cube, true));
            if e.0 != cube {
                return Err(Skip::NotTried(format!("texture index {index} is read as 2D and as cube")));
            }
            e.1 &= explicit;
        }
    }
    Ok((0..TEXTURE_UNITS)
        .map(|u| {
            let (cube, mip) = plan.get(&u).or_else(|| plan.get(&(u ^ 1))).copied().unwrap_or((false, false));
            texture(u, cube, mip)
        })
        .collect())
}

/// Which lanes see a back-facing primitive: a third of them, so both
/// answers of gl_FrontFacing are in every run.
fn back_facing(lane: usize) -> bool {
    lane % 3 == 1
}

/// The spill buffer (QUNIFORM_SPILL_OFFSET) and its per-thread size
/// (QUNIFORM_SPILL_SIZE_PER_THREAD): any address serves that no other
/// region uses; the size only has to keep the threads apart.
const SPILL_BASE: u32 = 0x3000_0000;
const SPILL_SIZE_PER_THREAD: u32 = 0x1_0000;

const X_SCALE: f32 = 8192.0;
/// gl_FragCoord.y's window transform, y' = y * WPOS_SCALE + WPOS_OFFSET: a
/// window of height 64, the range the harness draws y from. Not the
/// identity, so a program that drops the transform, or swaps its two words,
/// computes a different y' from the IR's everywhere but y = 32.
const WPOS_SCALE: f32 = -1.0;
/// gl_PointCoord's y transform our QUNIFORM_FB_PNTC_Y_SCALE / _OFFSET carry:
/// a window target's (v3d-glue's job builder, Mesa's STATE_FB_PNTC_Y_TRANSFORM
/// with FlipY), so the flip is exercised.
const PNTC_SCALE: f32 = -1.0;
const PNTC_OFFSET: f32 = 1.0;
const WPOS_OFFSET: f32 = 64.0;
const Y_SCALE: f32 = -6144.0;
const Z_SCALE: f32 = 0.5;
const Z_OFFSET: f32 = 0.5;

/// Mesa's uniform stream for one stage, resolved to values. Contents that
/// feed an I/O path the interpreter does not model are named.
fn stream(st: &Stage, stage: &str, u: &Uniforms, ours: bool) -> Result<Vec<u32>, Skip> {
    let mut out = Vec::new();
    for &(contents, data) in &st.uniforms {
        out.push(match contents {
            0 => data,
            // Our QUNIFORM_UNIFORM's `data` is the dword of the stage's
            // uniform block as the IR lays it out (select.rs QUNIFORM_UNIFORM),
            // the block eval.rs reads; Mesa's is a vec4-slot dword offset.
            1 if ours => {
                let block = if stage == "frag" { &u.fs_block } else { &u.vs_block };
                *block.get(data as usize).ok_or_else(|| {
                    Skip::NotTried(format!("{stage}: our QUNIFORM_UNIFORM dword {data} is past the block"))
                })?
            }
            1 => *u
                .mesa
                .get(&(stage.to_string(), data))
                .ok_or_else(|| Skip::NotTried(format!("{stage} QUNIFORM_UNIFORM dword {data} maps to no uniform")))?,
            // QUNIFORM_UBO_ADDR: v3d_unit_data_create(unit, offset)
            // (v3d_compiler.h:371); unit 0 is Gallium's constant buffer, the
            // stage's uniform storage, which the harness places at
            // UNIFORM_BASE (see env()).
            50 if data >> 24 == 0 => UNIFORM_BASE + (data & 0xff_ffff),
            50 => return Err(Skip::NotTried(format!("{stage}: a UBO address for buffer {}", data >> 24))),
            2 => X_SCALE.to_bits(),
            3 => Y_SCALE.to_bits(),
            4 => Z_OFFSET.to_bits(),
            5 => Z_SCALE.to_bits(),
            // QUNIFORM_TMU_CONFIG_P0/P1: the compiler's packed word with the
            // texture (P0) or sampler (P1) unit in bits 24..31
            // (v3d_tex.c:303-306), where the driver writes the texture or
            // sampler state address (v3d_uniforms.c:129-160). The interpreter
            // takes the unit, so the harness passes the value unrelocated.
            // Ours name the sampler's binding in bits 24..31 (v3d-backend.md,
            // "Textures: the P0/P1 uniform words"); the harness binds texture
            // k to binding k, so the word is passed as it is, like Mesa's.
            40 | 41 if data >> 24 < TEXTURE_UNITS => data,
            // Our QUNIFORM_FB_WPOS_Y_SCALE / _OFFSET (select.rs), data 0 as
            // the driver requires; Mesa's path has FB_WPOS_Y_TRANSFORM instead.
            c if ours && c == QUNIFORM_FB_WPOS_Y_SCALE && data == 0 => WPOS_SCALE.to_bits(),
            c if ours && c == QUNIFORM_FB_WPOS_Y_OFFSET && data == 0 => WPOS_OFFSET.to_bits(),
            c if ours && c == QUNIFORM_FB_PNTC_Y_SCALE && data == 0 => PNTC_SCALE.to_bits(),
            c if ours && c == QUNIFORM_FB_PNTC_Y_OFFSET && data == 0 => PNTC_OFFSET.to_bits(),
            40 | 41 => return Err(Skip::NotTried(format!("{stage}: TMU config for unit {}", data >> 24))),
            // QUNIFORM_TEXTURE_WIDTH/HEIGHT of unit `data` (v3d_uniforms.c:
            // 58-68, level 0).
            44 | 45 if data < TEXTURE_UNITS => TEXTURE_SIZE,
            // QUNIFORM_SPILL_OFFSET / _SIZE_PER_THREAD (v3d_compiler.h;
            // vir_register_allocate.c:426-455: spill_base = TIDX * size +
            // EIDX * 4 + offset): the harness's scratch buffer, which the
            // interpreter keeps as memory the program writes (sim.rs, the
            // TMU general writes).
            65 if !ours => SPILL_BASE,
            66 if !ours => SPILL_SIZE_PER_THREAD,
            // v3d_compiler.h: 46..49 texture depth, array size, levels,
            // samples; 50 a UBO address.
            c => return Err(Skip::Unmodelled(format!("uniform contents {c} (texture/UBO/spill)"))),
        });
    }
    Ok(out)
}

fn compile<'a>(a: &Arena<'a>, stage: sema::Stage, path: &str) -> Result<lower::Lowered<'a>, Skip> {
    let full = format!("{}/tools/shaders/{path}", env!("CARGO_MANIFEST_DIR"));
    let text = std::fs::read_to_string(&full).map_err(|e| Skip::NotTried(format!("read {path}: {e}")))?;
    let src = a.alloc_str(&text).map_err(|_| Skip::NotTried("arena".into()))?;
    lower::compile_to_ir(a, stage, src).map_err(|e| Skip::NotTried(format!("front end: {}", e.msg)))
}

// ---------------------------------------------------------------------------
// Comparison.
// ---------------------------------------------------------------------------

enum Verdict {
    Agree,
    Disagree(String),
    Unmodelled(String),
    NotTried(String),
}

thread_local! {
    /// Values compared by the current run (a pair that "agrees" having
    /// compared nothing would be a vacuous pass).
    static COMPARED: core::cell::Cell<usize> = const { core::cell::Cell::new(0) };
}

fn compared() {
    COMPARED.with(|c| c.set(c.get() + 1));
}

fn close_f32(want: u32, got: u32, approx: bool) -> bool {
    if want == got {
        return true;
    }
    if !approx {
        return false;
    }
    let (w, g) = (f32::from_bits(want), f32::from_bits(got));
    (w - g).abs() <= 1e-5 * w.abs().max(1.0)
}

fn close_f16(want: u16, got: u16, approx: bool) -> bool {
    want == got || (approx && (want as i32 - got as i32).abs() <= 1 && (want ^ got) & 0x8000 == 0)
}

fn close_int(want: u32, got: u32, approx: bool) -> bool {
    want == got || (approx && (want as i32).wrapping_sub(got as i32).abs() <= 1)
}

fn f(x: f32) -> u32 {
    x.to_bits()
}

/// `V3D_SIM_PAIR=<substring>`: run only the pairs whose name holds it, and
/// print what the run is built from.
fn debug() -> bool {
    std::env::var("V3D_SIM_PAIR").is_ok()
}

/// One pair's verdict, and whether it came from the positive-input retry.
/// A NaN stops the interpreter (its hardware treatment is unknown); when the
/// first draw produced one (a `log` or `rsqrt` of a negative operand), the
/// pair runs once more with positive inputs only, and the report counts it.
fn run_pair(p: &Pair, seed: u64) -> (Verdict, bool) {
    run_pair_draw(p, seed, false)
}

/// [`run_pair`], with special values among the attributes when `zeros`.
fn run_pair_draw(p: &Pair, seed: u64, zeros: bool) -> (Verdict, bool) {
    let first = run_pair_inner(p, seed, false, zeros);
    let nan = matches!(&first, Err(Skip::Unmodelled(w)) if w.contains("NaN"))
        || matches!(&first, Err(Skip::NotTried(w)) if w.contains("negative float converted"));
    let (r, retried) = if nan { (run_pair_inner(p, seed, true, zeros), true) } else { (first, false) };
    let v = match r {
        Ok(None) => Verdict::Agree,
        Ok(Some(d)) => Verdict::Disagree(d),
        Err(Skip::Unmodelled(w)) => Verdict::Unmodelled(w),
        Err(Skip::NotTried(w)) => Verdict::NotTried(w),
    };
    (v, retried)
}

/// Locations a VARYING declaration covers. The IR splits a matrix varying
/// into one declaration per column (`m`, `m[1]`, each still typed as the
/// matrix, as tests/backend_differential.rs notes), so a declaration with a
/// split sibling covers one; an unsplit matrix covers one per column.
fn varying_locations(d: &IoDecl, decls: &[IoDecl]) -> u8 {
    let base = d.name.split('[').next().unwrap_or(d.name);
    let split = decls.iter().filter(|o| o.kind == d.kind && o.name.split('[').next() == Some(base)).count() > 1;
    if split {
        1
    } else {
        locations(d.gl_type)
    }
}

/// Locations an attribute declaration covers (one per column).
fn locations(gl_type: u16) -> u8 {
    match gl_type {
        0x8b5a => 2,
        0x8b5b => 3,
        0x8b5c => 4,
        _ => 1,
    }
}

/// What one run of a program produced, for the comparison of two programs
/// (ours and Mesa's) on the same inputs.
#[derive(Default, Clone, Debug)]
struct Record {
    /// The colour TLB words (two f16 pairs for RT0).
    colour: Vec<[u32; LANES]>,
    discarded: [bool; LANES],
    /// VS rows 0..3 (scaled x, y, Zs, 1/Wc) and CS rows 0..5 (clip xyzw,
    /// scaled x, y): the layout both compilers share. The varying rows are
    /// not compared across compilers: their order is each compiler's FS
    /// input order, which Mesa's link step repacks.
    vs_rows: Vec<Option<[u32; LANES]>>,
    cs_rows: Vec<Option<[u32; LANES]>>,
    /// An SFU op ran (its precision is the hardware's).
    approx: bool,
    /// TMU look-ups the interpreter performed, all stages.
    tmu_lookups: usize,
    /// LDTMUs executed, and those that took a word requested with no thread
    /// switch since (sim::Outcome::ldtmus_without_switch), all stages.
    ldtmus: usize,
    ldtmus_without_switch: usize,
    /// The cube faces (bit n: texref::Face::ALL[n]) and the levels (bit k)
    /// the interpreter's look-ups read, all stages.
    cube_faces: u8,
    levels: u32,
    /// Instructions the interpreter ran, all stages (sim::Outcome::
    /// instructions): what shows a counted loop's early exit, which no value
    /// can (the unroller guards every pass after a break).
    instructions: usize,
    /// Of those, the fragment shader's.
    fs_instructions: usize,
}

thread_local! {
    static RECORD: core::cell::RefCell<Record> = core::cell::RefCell::new(Record::default());
}

/// What the negative control made of one mutant.
#[derive(Debug, PartialEq, Eq)]
enum Control {
    /// It disagreed on a draw on which the unmutated program agrees.
    Caught,
    /// It agreed on every draw on which it could be compared.
    Survived,
    /// No draw compared it: the unmutated program or the mutant stopped on
    /// each. Neither caught nor survived.
    Unmeasured,
}

/// Draws the control may use per mutant: the pair's own draw first, then
/// others, every second one with special values among the attributes.
const CONTROL_DRAWS: u64 = 16;

/// The negative control for mutant `q` of pair `p`: caught only by a real
/// disagreement, on a draw on which `p` itself agrees. A mutant that stops
/// the run (unmodelled) is not caught by that.
fn control(p: &Pair, q: &Pair, name: &str) -> Control {
    let mut measured = false;
    for k in 0..CONTROL_DRAWS {
        let (seed, zeros) = (seed_of(name).wrapping_add(k.wrapping_mul(0x9e37_79b9_7f4a_7c15)), k % 2 == 1);
        if !matches!(run_pair_draw(p, seed, zeros).0, Verdict::Agree) {
            continue;
        }
        match run_pair_draw(q, seed, zeros).0 {
            Verdict::Disagree(_) => return Control::Caught,
            // The mutant moved an output past the tolerance where the IR's
            // value is one sin/cos's undefined precision leaves open. On
            // this draw the unmutated program agreed, so the mutant is told
            // apart from it: a catch. (Counted as neither, the four OIT
            // qt_rand pairs' first mutants "survived", 2026-09-26.)
            Verdict::Unmodelled(w) if w == TRIG_UNDETERMINED => return Control::Caught,
            Verdict::Agree => measured = true,
            Verdict::Unmodelled(_) | Verdict::NotTried(_) => {}
        }
    }
    if measured {
        Control::Survived
    } else {
        Control::Unmeasured
    }
}

/// The negative control for one stage of pair `p`: the first mutant
/// ([`mutate`]) that some draw compares, and what the control made of it.
/// `None` when the stage has nothing to mutate. A mutant no draw compares
/// (it stops the run, e.g. with a NaN operand, on every draw) is listed in
/// the third field and the next one is tried; if none is compared the
/// result is [`Control::Unmeasured`].
fn control_stage(p: &Pair, stage: &str, name: &str) -> Option<(String, Control, Vec<String>)> {
    let code = if stage == "fs" { &p.fs.code } else { &p.vs.code };
    let mut skipped = Vec::new();
    for nth in 0.. {
        let Some((m, what)) = mutate(code, nth) else {
            return if nth == 0 {
                None
            } else {
                Some((skipped.pop().unwrap_or_default(), Control::Unmeasured, skipped))
            };
        };
        let mut q = p.clone();
        if stage == "fs" {
            q.fs.code = m;
        } else {
            q.vs.code = m;
        }
        let what = format!("{name} {stage} {what}");
        match control(p, &q, name) {
            Control::Unmeasured => skipped.push(what),
            c => return Some((what, c, skipped)),
        }
    }
    unreachable!("the range is unbounded")
}

/// A pair's seed, from its name (FNV-1a), so a pair's inputs do not move
/// when the fixture gains or loses another pair: an index-based seed moved
/// every later pair's inputs (and the pins) when utri was added.
fn seed_of(name: &str) -> u64 {
    name.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3))
}

/// Whose program `p` is, for the messages: the same run compares Mesa's and
/// ours with the IR, and a verdict must say which program it is about.
fn who(p: &Pair) -> &'static str {
    if p.ours {
        "our code"
    } else {
        "Mesa's code"
    }
}

fn run_pair_inner(p: &Pair, seed: u64, positive: bool, zeros: bool) -> Result<Option<String>, Skip> {
    RECORD.with(|r| *r.borrow_mut() = Record::default());
    let mut mem = vec![0u8; 16 << 20];
    let a = Arena::new(&mut mem);
    let fsl = compile(&a, sema::Stage::Fragment, &p.fs_path)?;
    let vsl = compile(&a, sema::Stage::Vertex, &p.vs_path)?;
    let (fsm, vsm) = (&fsl.module, &vsl.module);
    if debug() {
        println!(
            "FS IR inputs: {:?}",
            fsm.inputs.iter().map(|d| (d.name, d.kind, d.location, d.gl_type)).collect::<Vec<_>>()
        );
        println!(
            "VS IR outputs: {:?}",
            vsm.outputs.iter().map(|d| (d.name, d.kind, d.location, d.gl_type)).collect::<Vec<_>>()
        );
        println!("Mesa FS input slots: {:?}; io {:?}", p.fs.inputs, p.io);
        for (st, m) in [("frag", fsm), ("vert", vsm)] {
            println!(
                "IR {st} uniforms: {:?}",
                m.uniforms.iter().map(|d| (d.name, d.dword_off, d.dwords, d.gl_type, d.array_len)).collect::<Vec<_>>()
            );
        }
        println!("Mesa uniforms: {:?}", p.uniforms);
        for (st, code) in [("fs", &p.fs.code), ("vs", &p.vs.code)] {
            println!("--- {} {st}", if p.ours { "ours" } else { "Mesa" });
            for (i, w) in code.iter().enumerate() {
                let t = yakogl::shaderc::online::v3d::decode::decode(*w).map(|i| {
                    let mut s = String::new();
                    yakogl::shaderc::online::v3d::listing::write_instr(&mut s, &i).unwrap();
                    s
                });
                println!("{i:4} {t:?}");
            }
        }
    }
    let mut rng = Rng(seed, positive, zeros);
    let u = build_uniforms(p, fsm, vsm, &mut rng)?;
    let texs = pair_textures(p, fsm, vsm)?;
    let sim_err = |stage: &'static str| move |e: String| Skip::Unmodelled(format!("{stage}: {e}"));

    // ---------------- the IR, VS first: its outputs are the FS's varyings ----------------
    let mut attr_vals: BTreeMap<(String, u32, usize), [u32; 4]> = BTreeMap::new();
    let mut vs_io: Vec<Vec<(IoSlot, [u32; 4])>> = vec![Vec::new(); LANES];
    for d in vsm.inputs.iter().filter(|d| d.kind == IoKind::Attribute) {
        for k in 0..locations(d.gl_type) {
            for (l, io) in vs_io.iter_mut().enumerate() {
                let v: [u32; 4] = core::array::from_fn(|c| f(rng.attr(c)));
                attr_vals.insert((d.name.to_string(), k as u32, l), v);
                io.push((IoSlot::Attrib(d.location + k), v));
            }
        }
    }
    let vids = vertex_ids(p)?;
    for (l, io) in vs_io.iter_mut().enumerate() {
        io.push((IoSlot::VertexId, [vids[l]; 4]));
        io.push((IoSlot::InstanceId, [INSTANCE_ID; 4]));
    }
    let vs_tex = eval_textures(p, &p.vs, "vert", vsm, &texs)?;
    let vs_eval: Vec<eval::Outcome> = vs_io
        .iter()
        .map(|io| eval::run_with_textures(vsm, &eval::Inputs { io, uniforms: &u.vs_block }, &vs_tex))
        .collect::<Result<_, _>>()
        .map_err(|e| Skip::NotTried(format!("evaluator (VS): {e}")))?;

    // ---------------- Mesa's VS and CS on the interpreter ----------------
    let mut mesa_vs_rows: Vec<Option<[u32; LANES]>> = Vec::new();
    // Whether the VS keeps a gl_PointSize row: ours say so in their record;
    // Mesa's fixture programs were compiled for triangles (no
    // per_vertex_point_size), which never keep it.
    let psiz_row = |st: &Stage| -> Result<bool, Skip> {
        match st.meta.get("writes_psiz").map(String::as_str) {
            Some("True") => Ok(true),
            Some("False") => Ok(false),
            None if !p.ours => Ok(false),
            other => Err(Skip::NotTried(format!("our program's writes_psiz is {other:?}"))),
        }
    };
    let vs_psiz = psiz_row(&p.vs)?;
    for (stname, st) in [("VS", &p.vs), ("CS", &p.cs)] {
        let is_cs = stname == "CS";
        // VPM input rows: the system values first (instance id, then vertex
        // id; ntq_setup_vs_inputs), then each record's vattr_sizes
        // components, in record order (ntq_emit_load_input).
        let mut rows: Vec<[u32; LANES]> = Vec::new();
        if st.meta.get("uses_iid").map(String::as_str) == Some("True") {
            rows.push([INSTANCE_ID; LANES]);
        }
        let uses_vid = st.meta.get("uses_vid").map(String::as_str) == Some("True");
        if uses_vid {
            rows.push(vertex_ids(p)?);
        }
        // GFXH-1602: a coordinate shader that reads a builtin and none of
        // attribute 0 still has one row of attribute 0 loaded (the driver's
        // v3d_job.c attribute records; Mesa nir_to_vir.c:2907-2915 skips
        // it). Filled with a value no program should read.
        let reads_builtin = uses_vid || st.meta.get("uses_iid").map(String::as_str) == Some("True");
        if is_cs && reads_builtin && st.vattr.first() == Some(&0) {
            rows.push([f32::to_bits(-7777.0); LANES]);
        }
        for (rec, &size) in st.vattr.iter().enumerate() {
            if size == 0 {
                continue;
            }
            let found = p.attrs.iter().find_map(|(name, _, recs)| {
                recs.iter().position(|r| *r == Some(rec as u32)).map(|k| (name.clone(), k as u32))
            });
            let Some((name, k)) = found else {
                if p.ours {
                    // Our `attributes` lists (location, record) for a
                    // declaration's FIRST location only; a matrix's later
                    // columns have records (vattr_sizes) and no location.
                    return Err(Skip::NotTried(format!(
                        "{stname}: our VPM record {rec} is listed for no attribute location (a matrix column)"
                    )));
                }
                return Err(Skip::NotTried(format!("{stname}: VPM record {rec} has no attribute")));
            };
            for comp in 0..size as usize {
                let mut row = [0u32; LANES];
                for (l, r) in row.iter_mut().enumerate() {
                    let Some(v) = attr_vals.get(&(name.clone(), k, l)) else {
                        return Err(Skip::NotTried(format!("{stname}: attribute {name} is not an IR input")));
                    };
                    *r = v[comp];
                }
                rows.push(row);
            }
        }
        let s = stream(st, "vert", &u, p.ours)?;
        let got = sim::run(&st.code, &s, &Inputs::Vertex(VsInputs { vpm: rows }), &env(&u, "vert", st, &texs))
            .map_err(sim_err(stname))?;
        let approx = !got.approximate.is_empty();
        let row = |r: usize| got.vpm_out.get(r).copied().flatten();
        RECORD.with(|rec| {
            let mut rec = rec.borrow_mut();
            rec.approx |= approx;
            rec.tmu_lookups += got.tmu_lookups;
            rec.cube_faces |= got.cube_faces;
            rec.levels |= got.levels;
            rec.ldtmus += got.ldtmus;
            rec.ldtmus_without_switch += got.ldtmus_without_switch;
            rec.instructions += got.instructions;
            let rows: Vec<_> = (0..if is_cs { 6 } else { 4 }).map(row).collect();
            if is_cs {
                rec.cs_rows = rows;
            } else {
                rec.vs_rows = rows;
            }
        });
        for (l, o) in vs_eval.iter().enumerate() {
            let Some(pos) = o.output(IoSlot::Position) else {
                return Err(Skip::NotTried("the VS writes no gl_Position".into()));
            };
            let pf = pos.map(f32::from_bits);
            let rcp = 1.0 / pf[3];
            let scaled = |c: usize, s: f32| ((pf[c] * s) * rcp).floor();
            let (sx, sy) = (scaled(0, X_SCALE), scaled(1, Y_SCALE));
            if !(-2.0e9..2.0e9).contains(&sx) || !(-2.0e9..2.0e9).contains(&sy) {
                return Err(Skip::NotTried("scaled position out of the i32 range".into()));
            }
            let mut want: Vec<(usize, u32, &str)> = Vec::new();
            let base = if is_cs {
                for (c, v) in pos.iter().enumerate() {
                    want.push((c, *v, "clip position"));
                }
                4
            } else {
                0
            };
            want.push((base, sx as i32 as u32, "scaled x"));
            want.push((base + 1, sy as i32 as u32, "scaled y"));
            if !is_cs {
                want.push((2, f((pf[2] * Z_SCALE) * rcp + Z_OFFSET), "Zs"));
                want.push((3, f(rcp), "1/Wc"));
            }
            // The point size after the fixed-function rows (Mesa's
            // per_vertex_point_size layout, v3d_nir_lower_io.c:410-431).
            if psiz_row(st)? {
                let Some(ps) = o.output(IoSlot::PointSize) else {
                    return Ok(Some(format!("{stname}: a point-size row, and the IR writes no gl_PointSize")));
                };
                want.push((if is_cs { 6 } else { 4 }, ps[0], "point size"));
            }
            for (r, wv, what) in want {
                let Some(g) = row(r) else {
                    return Ok(Some(format!("{stname}: VPM row {r} ({what}) never written")));
                };
                let g = g[l];
                compared();
                let ok = if what.starts_with("scaled") { close_int(wv, g, approx) } else { close_f32(wv, g, approx) };
                if wv != eval::UNSET && !ok {
                    let input = eval::Inputs { io: &vs_io[l], uniforms: &u.vs_block };
                    let exact = wv;
                    let pick = |o: &eval::Outcome| -> Option<u32> {
                        /* the same row the IR gave, from the shifted run */
                        let pos = o.output(IoSlot::Position)?.map(f32::from_bits);
                        let rcp = 1.0 / pos[3];
                        Some(match (r, what) {
                            (_, "scaled x") => ((pos[0] * X_SCALE) * rcp).floor() as i32 as u32,
                            (_, "scaled y") => ((pos[1] * Y_SCALE) * rcp).floor() as i32 as u32,
                            (_, "Zs") => f((pos[2] * Z_SCALE) * rcp + Z_OFFSET),
                            (_, "1/Wc") => f(rcp),
                            (c, "clip position") => pos[c].to_bits(),
                            (_, "point size") => o.output(IoSlot::PointSize)?[0],
                            _ => return None,
                        })
                    };
                    let close = |a: u32, b: u32| {
                        if what.starts_with("scaled") {
                            close_int(a, b, approx)
                        } else {
                            close_f32(a, b, approx)
                        }
                    };
                    if trig_undetermined(vsm, &input, &vs_tex, exact, &pick, &close) {
                        return Err(Skip::Unmodelled(TRIG_UNDETERMINED.into()));
                    }
                    return Ok(Some(format!(
                        "{stname} lane {l} row {r} ({what}): the IR gives {} ({wv:#010x}), {} gives {} ({g:#010x})",
                        f32::from_bits(wv),
                        who(p),
                        f32::from_bits(g)
                    )));
                }
            }
        }
        if !is_cs {
            mesa_vs_rows = got.vpm_out.clone();
        }
    }

    // ---------------- FS ----------------
    let flag = |k: &str| u32::from_str_radix(p.fs.meta.get(k).map_or("0", |s| s.trim_start_matches("0x")), 16).unwrap();
    let (flat, noper, centroid) = (flag("flat_shade_flags"), flag("noperspective_flags"), flag("centroid_flags"));
    let mut w = [0u32; LANES];
    let mut wc = [0u32; LANES];
    let mut z = [0u32; LANES];
    let mut x = [0u32; LANES];
    let mut y = [0u32; LANES];
    // W and centroid W are powers of two, different ones, so that the
    // varying construction below is exact and a W/Wc mix-up still shows.
    let pow2 = [0.5f32, 1.0, 2.0, 4.0];
    for l in 0..LANES {
        let k = rng.next() as usize % 4;
        w[l] = f(pow2[k]);
        wc[l] = f(pow2[(k + 1 + rng.next() as usize % 3) % 4]);
        z[l] = f(rng.pos() / 2.0);
        x[l] = f((rng.next() % 64) as f32 + 0.5);
        y[l] = f((rng.next() % 64) as f32 + 0.5);
    }
    // The implicit point coordinates a points draw feeds the FS (x, y in
    // (0, 1), on 1/32 steps so their smooth construction is exact), for
    // our programs that read gl_PointCoord; the IR sees y through the
    // target's transform, as our program applies it.
    let implicit_point = p.fs.meta.get("implicit_point").map(String::as_str) == Some("True");
    let mut pc = [[0u32; LANES]; 2];
    for l in 0..LANES {
        for lane in pc.iter_mut() {
            lane[l] = f((rng.next() % 32) as f32 / 32.0 + 1.0 / 64.0);
        }
    }
    // The IR's FS reads what the IR's VS wrote, lane by lane, matched by the
    // IR's own names; 0 for a varying the VS does not write.
    let mut fs_io: Vec<Vec<(IoSlot, [u32; 4])>> = vec![Vec::new(); LANES];
    for (l, io) in fs_io.iter_mut().enumerate() {
        for d in fsm.inputs.iter().filter(|d| d.kind == IoKind::Varying) {
            let vd = vsm.outputs.iter().find(|o| o.kind == IoKind::Varying && o.name == d.name);
            for k in 0..varying_locations(d, fsm.inputs) {
                let v = vd.and_then(|vd| vs_eval[l].output(IoSlot::Varying(vd.location + k))).unwrap_or([0; 4]);
                io.push((IoSlot::Varying(d.location + k), v));
            }
        }
        let fw = 1.0 / f32::from_bits(w[l]);
        // gl_FragCoord: the interpreter's FXCD / FYCD are x[l] / y[l], the
        // IR sees y through the window transform both compilers are given.
        let wy = f32::from_bits(y[l]) * WPOS_SCALE + WPOS_OFFSET;
        io.push((IoSlot::FragCoord, [x[l], f(wy), z[l], f(fw)]));
        // gl_FrontFacing: true (~0, the IR's bool) where REVF reads 0.
        io.push((IoSlot::FrontFacing, [if back_facing(l) { 0 } else { u32::MAX }; 4]));
        let py = f32::from_bits(pc[1][l]) * PNTC_SCALE + PNTC_OFFSET;
        io.push((IoSlot::PointCoord, [pc[0][l], f(py), 0, 0]));
    }
    // Mesa's FS reads what Mesa's VS wrote: FS input i is VS VPM row 4 + i,
    // one row later when the VS keeps its point size
    // (v3d_nir_setup_vpm_layout_vs: the varyings follow the FS's input
    // order). Mesa's link step repacks varyings (the FS input slots are not
    // SPIR-V locations: matvary's v_m[0].x and v_m[1].y become VAR0.x and
    // VAR0.y), so the two programs are joined through Mesa's own contract,
    // never through a name. LDVARY's parts are chosen so that what the FS
    // computes from them (emit_smooth/noperspective/flat_varying) is exactly
    // the VS's value: C = v/2 and vary = (v/2)/W, W a power of two; flat: C
    // = v; noperspective: vary = C = v/2.
    let mut varyings = Vec::new();
    // Our gl_PointCoord program reads the two implicit point varyings first,
    // smooth (Mesa's emit_fragment_varying with no variable): vary = (v/2)/W,
    // C = v/2, which gives v back exactly for W a power of two.
    if implicit_point {
        for lane in pc {
            let mut v = Varying { vary: [0; LANES], c: [0; LANES] };
            for l in 0..LANES {
                let half = f32::from_bits(lane[l]) * 0.5;
                let (pv, cv) = (half / f32::from_bits(w[l]), half);
                assert_eq!((pv * f32::from_bits(w[l]) + cv).to_bits(), lane[l], "the point coordinate is not exact");
                v.vary[l] = f(pv);
                v.c[l] = f(cv);
            }
            varyings.push(v);
        }
    }
    for i in 0..p.fs.inputs.len() {
        // The flag words hold 32 inputs each, and the fixture and our JSON
        // carry the first word only: an input past it has no known
        // interpolation, so the draw is not judged (a shift by i >= 32
        // overflowed here once the chain compiled a 60-input shader).
        if i >= 32 {
            return Err(Skip::NotTried(format!("FS input {i}: past the 32 the flag words describe")));
        }
        let r = 4 + usize::from(vs_psiz) + i;
        let Some(Some(row)) = mesa_vs_rows.get(r) else {
            return Ok(Some(format!("VS: VPM row {r} (FS input {i}) never written")));
        };
        let mut v = Varying { vary: [0; LANES], c: [0; LANES] };
        if flat & (1 << i) != 0 {
            // Flat: C is the provoking vertex's word as it is, and the
            // ldvary result is noise the program must not read
            // (emit_flat_varying). The word is often an int or a uint (Qt
            // Quick 3D's `flat out uint instanceIndex`), so it is passed as
            // bits, never judged or rebuilt as a float.
            for vary in v.vary.iter_mut() {
                *vary = f(rng.float());
            }
            v.c = *row;
            varyings.push(v);
            continue;
        }
        for l in 0..LANES {
            let value = row[l];
            let vf = f32::from_bits(value);
            if !vf.is_finite() || (vf != 0.0 && vf.abs() < 1e-30) {
                return Err(Skip::NotTried(format!("the VS ({}) writes {vf} to FS input {i}", who(p))));
            }
            let half = vf * 0.5;
            let ww = f32::from_bits(if centroid & (1 << i) != 0 { wc[l] } else { w[l] });
            let (pv, cv) = if flat & (1 << i) != 0 {
                (rng.float(), vf)
            } else if noper & (1 << i) != 0 {
                (half, half)
            } else {
                (half / ww, half)
            };
            let back = if flat & (1 << i) != 0 {
                cv
            } else if noper & (1 << i) != 0 {
                pv + cv
            } else {
                pv * ww + cv
            };
            assert_eq!(back.to_bits(), value, "the varying construction is not exact for {vf}");
            v.vary[l] = f(pv);
            v.c[l] = f(cv);
        }
        varyings.push(v);
    }
    let fs_stream = stream(&p.fs, "frag", &u, p.ours)?;
    if debug() {
        println!("FS uniform stream: {:?}", fs_stream.iter().map(|x| f32::from_bits(*x)).collect::<Vec<_>>());
    }
    // The interpreter runs first so that a program the evaluator cannot
    // run still says how far the interpreter got (a texture program, while
    // eval.rs has no textures): that is not a validation, and it stays
    // "not tried", but it names the next refusal.
    let fs_inputs = FsInputs { w, w_centroid: wc, z, x, y, back_facing: core::array::from_fn(back_facing), varyings };
    let got = sim::run(&p.fs.code, &fs_stream, &Inputs::Fragment(Box::new(fs_inputs)), &env(&u, "frag", &p.fs, &texs));
    let fs_tex = eval_textures(p, &p.fs, "frag", fsm, &texs)?;
    let mut fs_want = Vec::new();
    for io in &fs_io {
        let o = eval::run_with_textures(fsm, &eval::Inputs { io, uniforms: &u.fs_block }, &fs_tex).map_err(|e| {
            let sim = match &got {
                Ok(_) => "ran to the end".to_string(),
                Err(e) => format!("stopped: {e}"),
            };
            Skip::NotTried(format!("evaluator (FS): {e} (the interpreter {sim})"))
        })?;
        fs_want.push(o);
    }
    if debug() {
        let c: Vec<_> = fs_want.iter().map(|o| o.output_f32(IoSlot::FragColor(0))).collect();
        println!("FS IR colour per lane: {c:?}");
    }
    let got = got.map_err(sim_err("FS"))?;
    let approx = !got.approximate.is_empty();
    // The TLB writes: a depth write (its specifier's type is 2 << 6,
    // nir_to_vir.c TLB_TYPE_DEPTH) is skipped; the colour writes are f16
    // pairs for RT0 under the default specifier 0xff.
    // A depth write whose specifier says "invariant" (TLB_V42_DEPTH_TYPE_
    // INVARIANT, bit 3 clear: the Z from FEP) carries a value the hardware
    // ignores, so it may be undefined; every other lane written must be
    // defined.
    // Colour words per render target: Mesa's config byte (vir_emit_tlb_color_
    // write, nir_to_vir.c:1953-1984) names the RT reversed ("7 = RT 0") in
    // bits 3..5, and a config holds until the next one; 0xff, the default,
    // is RT 0's f16 vec4. Any other type or size is unmodelled by name.
    let mut colour_rt: [Vec<[u32; LANES]>; 4] = Default::default();
    let mut spec = 0xffu32;
    for t in &got.tlb {
        if let Some(c) = t.config {
            spec = c & 0xff;
        }
        // An undefined lane matters only where the pixel is kept: a
        // discarded lane (multisample flags 0 at the end) shows nothing.
        let undefined = t.value.iter().zip(got.msf.iter()).any(|(v, m)| v.is_none() && *m != 0);
        if spec >> 6 == 2 {
            if undefined && spec & (1 << 3) != 0 {
                return Ok(Some(format!("FS: a per-pixel depth write of an undefined value (specifier {spec:#x})")));
            }
            spec = 0xff;
            continue;
        }
        let rt = 7 - ((spec >> 3) & 7) as usize;
        if spec >> 6 != 3 || spec & 0b111 != 0b111 || rt >= colour_rt.len() {
            return Err(Skip::Unmodelled(format!("FS: TLB specifier {spec:#x}")));
        }
        if undefined {
            return Ok(Some("FS: a colour TLB write of an undefined value".to_string()));
        }
        colour_rt[rt].push(t.value.map(|v| v.unwrap_or(0)));
    }
    // What the ours-vs-Mesa comparison reads: RT 0, the only one Mesa's
    // fixture programs write (they were compiled for one colour buffer).
    let colour: Vec<[u32; LANES]> = colour_rt[0].clone();
    RECORD.with(|rec| {
        let mut rec = rec.borrow_mut();
        rec.approx |= approx;
        rec.tmu_lookups += got.tmu_lookups;
        rec.cube_faces |= got.cube_faces;
        rec.levels |= got.levels;
        rec.ldtmus += got.ldtmus;
        rec.ldtmus_without_switch += got.ldtmus_without_switch;
        rec.instructions += got.instructions;
        rec.fs_instructions += got.instructions;
        rec.colour = colour.clone();
        rec.discarded = got.msf.map(|m| m == 0);
    });
    for (l, want) in fs_want.iter().enumerate() {
        let discarded = got.msf[l] == 0;
        if discarded != want.discarded {
            return Ok(Some(format!("FS lane {l}: discard {discarded}, the IR says {}", want.discarded)));
        }
        if discarded {
            continue;
        }
        for (rt, words) in colour_rt.iter().enumerate() {
            let Some(wc) = want.output(IoSlot::FragColor(rt as u8)) else {
                if !words.is_empty() {
                    return Ok(Some(format!("FS: RT {rt} written, and the IR writes no colour there")));
                }
                continue;
            };
            // Mesa's fixture programs were compiled for one colour buffer
            // (its fs_key cbufs), so an RT past 0 they leave unwritten is
            // that key, not a defect; ours must write every RT the IR does.
            if words.is_empty() && rt > 0 && !p.ours {
                continue;
            }
            if words.len() != 2 {
                return Ok(Some(format!("FS: RT {rt}: {} colour TLB writes, want 2", words.len())));
            }
            for (k, wv) in wc.iter().enumerate() {
                let word = words[k / 2][l];
                let g = if k % 2 == 0 { word as u16 } else { (word >> 16) as u16 };
                let wf = f32::from_bits(*wv);
                if !wf.is_finite() || *wv == eval::UNSET {
                    continue;
                }
                let wh = sim::f32_to_half(wf);
                compared();
                if !close_f16(wh, g, approx) {
                    let input = eval::Inputs { io: &fs_io[l], uniforms: &u.fs_block };
                    let pick = |o: &eval::Outcome| o.output(IoSlot::FragColor(rt as u8)).map(|c| c[k]);
                    let close = |a: u32, b: u32| {
                        close_f16(sim::f32_to_half(f32::from_bits(a)), sim::f32_to_half(f32::from_bits(b)), approx)
                    };
                    if trig_undetermined(fsm, &input, &fs_tex, *wv, &pick, &close) {
                        return Err(Skip::Unmodelled(TRIG_UNDETERMINED.into()));
                    }
                    return Ok(Some(format!(
                        "FS lane {l} RT {rt} colour[{k}]: the IR says {wf} (f16 {wh:#06x}), {} gives {} ({g:#06x})",
                        who(p),
                        sim::half_to_f32(g)
                    )));
                }
            }
        }
    }
    Ok(None)
}

/// A copy of `code` with one instruction changed so that it computes
/// something else on (nearly) any input: the first FADD made FSUB, or else
/// the first FMUL made to read its first operand's low half as an f16 (an
/// ABS would not do: most operands here are positive, and the first draft
/// of this control survived 27 times for exactly that reason). `None` when
/// the program has neither.
fn mutate(code: &[u64], nth: usize) -> Option<(Vec<u64>, String)> {
    use yakogl::shaderc::online::v3d::decode::decode;
    use yakogl::shaderc::online::v3d::encode::encode;
    use yakogl::shaderc::online::v3d::isa::{small_imm_value, AddOp, Instr, MulOp, Mux, Unpack};
    let mut seen = 0;
    for (i, &w) in code.iter().enumerate() {
        let Ok(Instr::Alu(mut a)) = decode(w) else { continue };
        // x + 0 made x - 0 computes the same value (up to the sign of a zero
        // result): an equivalent mutant, which no draw can catch. The
        // encoder orders FADD's operands, so the 0 is the SECOND operand of
        // the word even where the scheduler's listing shows it first
        // (es100/intlit's expanded loop computes 0.0 + 0.25, 2026-09-24).
        let adds_zero = a.add.b.mux == Mux::B && a.sig.small_imm_b && small_imm_value(a.raddr_b) == Some(0);
        let what = if a.add.op == AddOp::Fadd && !adds_zero {
            a.add.op = AddOp::Fsub;
            "fadd -> fsub"
        } else if a.mul.op == MulOp::Fmul && a.mul.a.unpack == Unpack::None {
            a.mul.a.unpack = Unpack::L;
            "fmul a -> a.l"
        } else {
            continue;
        };
        if let Ok(m) = encode(&Instr::Alu(a)) {
            seen += 1;
            if seen <= nth {
                continue;
            }
            let mut out = code.to_vec();
            out[i] = m;
            return Some((out, format!("instruction {i}: {what}")));
        }
    }
    None
}

/// Texture programs whose texture results no compared output can show,
/// with the reason, checked against the source: their piece controls are
/// their own outcome, never "caught" and never "survived".
/// Piece mutants that are the same program as the original, so no input
/// can tell them apart (an equivalent mutant is not a gap in the
/// instrument): (fragment source, piece, why). Their own outcome, never
/// "caught"; every entry must match a mutant, or the test fails.
const TMU_EQUIVALENT: &[(&str, &str, &str)] = &[
    (
        "es300/idxtex.frag",
        "TMUT <-> TMUS sources",
        "texture(ftex, vec2(0.75, 0.75)): s == t, so trading their sources \
         changes nothing",
    ),
    (
        "qt3d/lightmapuvraster_300.frag",
        "TMUT <-> TMUS sources",
        "its vertex shader writes v_uv = vec2(0.0), so s == t == 0 in every \
         look-up and trading their sources changes nothing",
    ),
];

/// Texture pieces of OUR code no control draw can compare: (pair, piece,
/// why), each established by printing every control draw's verdict. Their
/// own outcome, neither caught nor survived. Keyed by the pair ("vs + fs"):
/// a vertex-shader look-up's mutant depends on the vertex shader. Step (l)
/// (alloc.rs, the hoist-aware register choice, 2026-09-29): es100/cubelod's
/// TMUT <-> TMUS mutant left the list (now caught: the S value is written
/// before TMUT), and the two vtex_struct_sampler pairs came in.
const TMU_UNMEASURED: &[(&str, &str, &str)] = &[
    (
        "es100/cubebias.vert + es100/cubebias.frag",
        "TMUT <-> TMUS sources",
        "our list scheduler's order (order.rs) writes TMUT before the S value's last write, so the traded \
         TMUT reads the unscaled x: every draw's direction lies within 1e-3 of a face edge, which the \
         interpreter refuses (measured 2026-09-26)",
    ),
    (
        "es100/vtex_struct_sampler.vert + es100/va.frag",
        "TMUT <-> TMUB sources",
        "the bias's uniform load shares the TMUT write's word (sched.rs hoist), so the traded TMUT reads \
         that register's old value, the VPM row number 2: the coordinate 2.8e-45 lies on a texel edge on \
         every draw, which the interpreter refuses (measured 2026-09-29)",
    ),
    (
        "es100/vtex_struct_sampler_array.vert + es100/va.frag",
        "TMUT <-> TMUB sources",
        "as es100/vtex_struct_sampler: the traded TMUT reads the old VPM row number, a coordinate on a \
         texel edge on every draw (measured 2026-09-29)",
    ),
];

const TMU_UNOBSERVABLE: &[(&str, &str)] = &[(
    "qt3d/lightmapuvraster_300.frag",
    "its texture reads feed render targets 1..3 only (render target 0 is \
     position = vec4(v_pos, 1.0)), and the fixture's FS key binds render \
     target 0 alone (v3d_shc.c:832, fs_key.cbufs = 1), so Mesa's program \
     writes no other target",
)];

/// ldtmu_trade on the shape a tight schedule gives: the first LDTMU's word
/// read in the second LDTMU's own word (`fmul r4, r0, r0 ; ldtmu.r1`, as
/// femtovg filter_image's `fmul r4, r0.l, rf9 ; ldtmu.r1` since step (l)).
/// The destinations trade and that read follows the first word to r1; the
/// later `fadd r2, r0, r1` is untouched, so it reads the two words swapped.
/// Where the window reads the second destination's old value the mutant does
/// not apply, and says why. Revert check: without the redirect, the first
/// case is Err (the pre-step-(n) "a destination is read" refusal).
#[test]
fn the_ldtmu_mutant_redirects_a_read_made_before_the_second_result_lands() {
    use yakogl::shaderc::online::v3d::decode::decode;
    use yakogl::shaderc::online::v3d::encode::encode;
    use yakogl::shaderc::online::v3d::isa::{AddOp, Alu, Instr, MulOp, Mux};
    let ldtmu = |mut x: Alu, r: u8| {
        x.sig.ldtmu = true;
        (x.sig_addr, x.sig_magic) = (r, true);
        x
    };
    let fmul = |d: u8, a: Mux, b: Mux| {
        let mut x = Alu::nop();
        x.mul.op = MulOp::Fmul;
        (x.mul.a.mux, x.mul.b.mux, x.mul.waddr, x.mul.magic) = (a, b, d, true);
        x
    };
    let mut fadd = Alu::nop();
    fadd.add.op = AddOp::Fadd;
    (fadd.add.a.mux, fadd.add.b.mux, fadd.add.waddr, fadd.add.magic) = (Mux::R0, Mux::R1, 2, true);
    let enc = |xs: &[Alu]| xs.iter().map(|x| encode(&Instr::Alu(*x)).expect("encodes")).collect::<Vec<u64>>();
    let code = enc(&[ldtmu(Alu::nop(), 0), ldtmu(fmul(4, Mux::R0, Mux::R0), 1), fadd]);
    let want = enc(&[ldtmu(Alu::nop(), 1), ldtmu(fmul(4, Mux::R1, Mux::R1), 0), fadd]);
    let got = ldtmu_trade(&code).expect("applies: the early read is redirected");
    assert_eq!(
        got.iter().map(|w| decode(*w).ok()).collect::<Vec<_>>(),
        want.iter().map(|w| decode(*w).ok()).collect::<Vec<_>>(),
        "destinations traded, the early read follows the first word to r1"
    );
    /* the window reads r1's old value: the first word would land there first */
    let code = enc(&[ldtmu(Alu::nop(), 0), ldtmu(fmul(4, Mux::R1, Mux::R0), 1), fadd]);
    assert_eq!(ldtmu_trade(&code), Err("the second destination's old value is read before the second LDTMU"));
}

/// The LDTMU-destination mutant of `code`: its first two LDTMUs trade
/// destinations, so every read after the second one gets the other result
/// word (a compiler that put a texel's words in the wrong registers). A read
/// of the first destination between the two LDTMUs (the first word used
/// before the second lands, which a tight schedule does: Mesa's and ours
/// since step (l)) is redirected to the second destination's register, where
/// the first word now lands, so it keeps its value and the mutant stays a
/// valid program. Not applicable, with the reason: fewer than two LDTMUs,
/// one destination for both, a branch between them, a write of either
/// destination in their words or between them (it would clobber or race the
/// moved result), a read of the second destination's old value between them
/// (the first word would land there first), or a redirect the word cannot
/// encode (it needs a register-file port the word uses for another value).
fn ldtmu_trade(code: &[u64]) -> Result<Vec<u64>, &'static str> {
    use yakogl::shaderc::online::v3d::decode::decode;
    use yakogl::shaderc::online::v3d::encode::encode;
    use yakogl::shaderc::online::v3d::isa::{add_op_args, mul_op_args, num_src, AddOp, Alu, Instr, MulOp, Mux};
    let alu = |w: u64| match decode(w) {
        Ok(Instr::Alu(a)) => Some(a),
        _ => None,
    };
    let ld: Vec<usize> = (0..code.len()).filter(|&i| alu(code[i]).is_some_and(|a| a.sig.ldtmu)).take(2).collect();
    let [i, k] = *ld.as_slice() else { return Err("fewer than two LDTMUs") };
    let (mut first, mut second) = (alu(code[i]).ok_or("undecodable")?, alu(code[k]).ok_or("undecodable")?);
    let (da, db) = ((first.sig_addr, first.sig_magic), (second.sig_addr, second.sig_magic));
    if da == db {
        return Err("both LDTMUs write one register");
    }
    // The four operand slots: (mux, read by the operation).
    let slots = |x: &Alu| {
        [
            (x.add.a.mux, num_src(add_op_args(x.add.op)) >= 1),
            (x.add.b.mux, num_src(add_op_args(x.add.op)) >= 2),
            (x.mul.a.mux, num_src(mul_op_args(x.mul.op)) >= 1),
            (x.mul.b.mux, num_src(mul_op_args(x.mul.op)) >= 2),
        ]
    };
    let reads = |x: &Alu, m: Mux, d: (u8, bool)| match m {
        Mux::A => !d.1 && x.raddr_a == d.0,
        Mux::B => !d.1 && !x.sig.small_imm_b && x.raddr_b == d.0,
        acc => d.1 && acc as u8 == d.0,
    };
    let alu_writes = |x: &Alu, d: (u8, bool)| {
        (x.add.op != AddOp::Nop && (x.add.waddr, x.add.magic) == d)
            || (x.mul.op != MulOp::Nop && (x.mul.waddr, x.mul.magic) == d)
    };
    let mut out = code.to_vec();
    for w in i..=k {
        let mut x = if w == k { second } else { alu(code[w]).ok_or("a branch between the two LDTMUs")? };
        let sig_writes = |d: (u8, bool)| {
            (w != i && w != k && x.sig.writes_address() && (x.sig_addr, x.sig_magic) == d)
                || (d == (5, true) && (x.sig.ldvary || x.sig.ldunif))
        };
        if [da, db].iter().any(|&d| alu_writes(&x, d) || sig_writes(d)) {
            return Err("a destination is written in or between the two LDTMUs' words");
        }
        if w == i {
            continue; // its reads see the registers' old values either way
        }
        let used = slots(&x);
        if used.iter().any(|&(m, u)| u && reads(&x, m, db)) {
            return Err("the second destination's old value is read before the second LDTMU");
        }
        let redirect: Vec<usize> = (0..4).filter(|&n| used[n].1 && reads(&x, used[n].0, da)).collect();
        if redirect.is_empty() {
            continue;
        }
        let to = if db.1 {
            Mux::ALL[db.0 as usize]
        } else {
            // a register-file port no other operand of the word uses
            let free = |port: Mux| {
                (port == Mux::A || !x.sig.small_imm_b)
                    && used.iter().enumerate().all(|(n, &(m, u))| !u || m != port || redirect.contains(&n))
            };
            let port = [Mux::A, Mux::B]
                .into_iter()
                .find(|&p| free(p))
                .ok_or("the redirect needs a register-file port the word uses")?;
            if port == Mux::A {
                x.raddr_a = db.0;
            } else {
                x.raddr_b = db.0;
            }
            port
        };
        for n in redirect {
            match n {
                0 => x.add.a.mux = to,
                1 => x.add.b.mux = to,
                2 => x.mul.a.mux = to,
                _ => x.mul.b.mux = to,
            }
        }
        if w == k {
            second = x;
        } else {
            out[w] = encode(&Instr::Alu(x)).map_err(|_| "the redirect does not encode")?;
        }
    }
    core::mem::swap(&mut first.sig_addr, &mut second.sig_addr);
    core::mem::swap(&mut first.sig_magic, &mut second.sig_magic);
    out[i] = encode(&Instr::Alu(first)).map_err(|_| "the swap does not encode")?;
    out[k] = encode(&Instr::Alu(second)).map_err(|_| "the swap does not encode")?;
    Ok(out)
}

/// The pieces of a texture request (sim.rs, "Texture reads"), each as a
/// mutant of the FS of `p`: what our compiler's texture path must get right,
/// shown to be visible to the instrument on Mesa's programs before ours
/// emit any. `None` for a piece the program has no instance of.
fn tmu_mutants(p: &Pair) -> Vec<(&'static str, Result<Pair, &'static str>)> {
    use yakogl::shaderc::online::v3d::decode::decode;
    use yakogl::shaderc::online::v3d::encode::encode;
    use yakogl::shaderc::online::v3d::isa::{add_op_args, num_src, Instr, Mux};
    // The stage whose look-ups are mutated: the fragment shader's, or the
    // vertex shader's where only it samples (a vertex-stage look-up).
    let in_fs = p.fs.uniforms.iter().any(|(c, _)| *c == 40) || !p.vs.uniforms.iter().any(|(c, _)| *c == 40);
    let src = if in_fs { &p.fs } else { &p.vs };
    let with_stream = |contents: u32, flip: u32| {
        let k = src.uniforms.iter().position(|(c, _)| *c == contents)?;
        let mut q = p.clone();
        let st = if in_fs { &mut q.fs } else { &mut q.vs };
        st.uniforms[k].1 ^= flip;
        q.mutant = true;
        Some(q)
    };
    let alu = |w: u64| match decode(w) {
        Ok(Instr::Alu(a)) => Some(a),
        _ => None,
    };
    let code = &src.code;
    // (c) The coordinates: the first TMUT write and the write to one of
    // `to` in the same request (the S write that retires it: TMUS, TMUSLOD
    // or a cube's TMUSCM; or the cube's TMUR; or TMUB, the level of
    // detail), both a MOV on the mul ALU, trade sources. The same request:
    // after the retiring write before the TMUT and up to the one after it,
    // in either order (Mesa's scheduler writes TMUB before TMUT in
    // skybox_300). A register-file source
    // moves with its read address; not applicable where the receiving
    // instruction's other operands use that port for something else, or
    // read a small immediate.
    let trade = |to: &[u8]| {
        use yakogl::shaderc::online::v3d::isa::{Alu, MulOp};
        let mov_to = |w: u64, targets: &[u8]| {
            alu(w).filter(|a| a.mul.op == MulOp::Mov && a.mul.magic && targets.contains(&a.mul.waddr))
        };
        const RETIRE: [u8; 4] = [33, 40, 41, 42];
        let i = code.iter().position(|w| mov_to(*w, &[34]).is_some())?;
        let start = code[..i].iter().rposition(|w| mov_to(*w, &RETIRE).is_some()).map_or(0, |r| r + 1);
        let end = i + 1 + code[i + 1..].iter().position(|w| mov_to(*w, &RETIRE).is_some())?;
        let k = start + code[start..=end].iter().position(|w| mov_to(*w, to).is_some())?;
        let (t, s) = (mov_to(code[i], &[34])?, mov_to(code[k], to)?);
        // A source: the mux, and the address it reads through that mux.
        let src = |x: &Alu| match x.mul.a.mux {
            Mux::A => Some((Mux::A, Some(x.raddr_a))),
            Mux::B if x.sig.small_imm_b => None,
            Mux::B => Some((Mux::B, Some(x.raddr_b))),
            acc => Some((acc, None)),
        };
        // Install `from` as `x`'s mul operand a, if its port is free.
        let install = |mut x: Alu, from: (Mux, Option<u8>)| -> Option<Alu> {
            let others = [
                (x.add.a.mux, num_src(add_op_args(x.add.op)) >= 1),
                (x.add.b.mux, num_src(add_op_args(x.add.op)) >= 2),
            ];
            let port_used = |m: Mux| others.iter().any(|(o, used)| *used && *o == m);
            match from {
                (Mux::A, Some(r)) => {
                    if port_used(Mux::A) && x.raddr_a != r {
                        return None;
                    }
                    x.raddr_a = r;
                }
                (Mux::B, Some(r)) => {
                    if x.sig.small_imm_b || (port_used(Mux::B) && x.raddr_b != r) {
                        return None;
                    }
                    x.raddr_b = r;
                }
                _ => {}
            }
            x.mul.a.mux = from.0;
            Some(x)
        };
        let (st, ss) = (src(&t)?, src(&s)?);
        if st == ss {
            return None;
        }
        let (t2, s2) = (install(t, ss)?, install(s, st)?);
        let mut q = p.clone();
        let st = if in_fs { &mut q.fs } else { &mut q.vs };
        st.code[i] = encode(&Instr::Alu(t2)).ok()?;
        st.code[k] = encode(&Instr::Alu(s2)).ok()?;
        q.mutant = true;
        Some(q)
    };
    // (d) The results: the first two LDTMUs trade destinations
    // (ldtmu_trade).
    let results = ldtmu_trade(code).map(|m| {
        let mut q = p.clone();
        let st = if in_fs { &mut q.fs } else { &mut q.vs };
        st.code = m;
        q.mutant = true;
        q
    });
    vec![
        // (a) P0's texture index, bits 24..31: the other texture of a pair.
        ("P0 texture index", with_stream(40, 1 << 24).ok_or("no P0 word")),
        // (b) P1's output type, bit 0: 32-bit words for f16 pairs.
        ("P1 output type", with_stream(41, 1).ok_or("no P1 word")),
        ("TMUT <-> TMUS sources", trade(&[33, 40, 42]).ok_or("no tradable TMUT / S pair")),
        // (e) A cube's R (its third coordinate: the face), and (f) TMUB (an
        // explicit level of detail, or a bias), each traded with T.
        ("TMUT <-> TMUR sources", trade(&[35]).ok_or("no tradable TMUT / R pair")),
        ("TMUT <-> TMUB sources", trade(&[37]).ok_or("no tradable TMUT / B pair")),
        ("LDTMU destinations", results),
    ]
}

#[test]
fn the_interpreter_runs_mesas_code_as_the_ir_says() {
    let pairs = parse();
    assert_eq!(pairs.len(), header("compiled"), "the fixture holds as many pairs as its header says");
    let (mut agree, mut disagree) = (Vec::new(), Vec::new());
    let mut unmodelled: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut not_tried: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut retried = Vec::new();
    // Agreeing pairs in which the interpreter performed a TMU look-up: what
    // vouches for the texture model, counted where the look-up ran.
    let mut sampled: Vec<String> = Vec::new();
    // Their LDTMUs, and the pairs with one that took a word requested with
    // no thread switch since (legal: it stalls; counted for the A/B on it).
    let mut ldtmus = 0usize;
    let mut without_switch: Vec<String> = Vec::new();
    // Per cube face (+X, -X, +Y, -Y, +Z, -Z) and per level: how many of
    // those pairs read it. Counted per item, not unioned, so a face one pair
    // reads and a face none reads are not one row.
    let (mut face_pairs, mut level_pairs) = ([0usize; 6], [0usize; 3]);
    for p in pairs.iter() {
        let name = format!("{} + {}", p.vs_path, p.fs_path);
        if let Ok(only) = std::env::var("V3D_SIM_PAIR") {
            if !name.contains(&only) {
                continue;
            }
        }
        let (verdict, positive) = run_pair(p, seed_of(&name));
        if positive {
            retried.push(name.clone());
        }
        if matches!(verdict, Verdict::Agree) && RECORD.with(|r| r.borrow().tmu_lookups) > 0 {
            sampled.push(name.clone());
            let (faces, levels) = RECORD.with(|r| (r.borrow().cube_faces, r.borrow().levels));
            for (f, n) in face_pairs.iter_mut().enumerate() {
                *n += usize::from(faces & (1 << f) != 0);
            }
            for (k, n) in level_pairs.iter_mut().enumerate() {
                *n += usize::from(levels & (1 << k) != 0);
            }
            let (n, w) = RECORD.with(|r| (r.borrow().ldtmus, r.borrow().ldtmus_without_switch));
            ldtmus += n;
            if w > 0 {
                without_switch.push(format!("{name}: {w} of {n}"));
            }
        }
        match verdict {
            Verdict::Agree => agree.push(name),
            Verdict::Disagree(d) => disagree.push(format!("{name}: {d}")),
            Verdict::Unmodelled(w) => unmodelled.entry(w).or_default().push(name),
            Verdict::NotTried(w) => not_tried.entry(w).or_default().push(name),
        }
    }
    let n_un: usize = unmodelled.values().map(Vec::len).sum();
    let n_nt: usize = not_tried.values().map(Vec::len).sum();
    let tried = agree.len() + disagree.len() + n_un;
    println!(
        "v3d interpreter on Mesa's code: {} pairs; {tried} tried: {} agree, {} disagree, {n_un} unmodelled; {n_nt} not tried",
        pairs.len(),
        agree.len(),
        disagree.len()
    );
    println!("  decided on the positive-input retry: {} {retried:?}", retried.len());
    for d in &disagree {
        println!("  DISAGREE {d}");
    }
    for (w, ps) in &unmodelled {
        println!("  unmodelled {} x {w}  (e.g. {})", ps.len(), ps[0]);
    }
    for (w, ps) in &not_tried {
        println!("  not tried {} x {w}  (e.g. {})", ps.len(), ps[0]);
    }
    if debug() {
        return;
    }
    // Every agreeing pair compared something.
    let mut total = 0usize;
    for p in pairs.iter() {
        let name = format!("{} + {}", p.vs_path, p.fs_path);
        if !agree.contains(&name) {
            continue;
        }
        COMPARED.with(|c| c.set(0));
        let _ = run_pair(p, seed_of(&name));
        let n = COMPARED.with(|c| c.get());
        assert!(n > 0, "{name} agrees having compared nothing");
        total += n;
    }
    println!("  values compared over the agreeing pairs: {total}");

    // Negative control: each agreeing pair with one instruction of Mesa's FS
    // or VS changed must stop agreeing. A mutant that still agrees is either
    // an instrument that cannot see the change or a change that cannot show
    // on these inputs; each is printed.
    let (mut caught, mut survived, mut none, mut unmeasured) = (0usize, Vec::new(), 0usize, Vec::new());
    let mut skipped: Vec<String> = Vec::new();
    for p in pairs.iter() {
        let name = format!("{} + {}", p.vs_path, p.fs_path);
        if !agree.contains(&name) {
            continue;
        }
        for stage in ["fs", "vs"] {
            let Some((what, c, skip)) = control_stage(p, stage, &name) else {
                none += 1;
                continue;
            };
            skipped.extend(skip);
            match c {
                Control::Caught => caught += 1,
                Control::Survived => survived.push(what),
                Control::Unmeasured => unmeasured.push(what),
            }
        }
    }
    println!(
        "  negative control: {} mutants, {caught} caught, {} survived, {} unmeasured, {none} programs with nothing to mutate",
        caught + survived.len() + unmeasured.len(),
        survived.len(),
        unmeasured.len()
    );
    for s in &survived {
        println!("    survived: {s}");
    }
    for s in &unmeasured {
        println!("    unmeasured: {s}");
    }
    for s in &skipped {
        println!("    no draw compared (the next mutant was used): {s}");
    }

    assert_eq!(tried + n_nt, pairs.len());
    assert!(disagree.is_empty(), "{} pairs disagree", disagree.len());
    // The populations, pinned (origin/main b96e77df plus the spill/scratch
    // TMU model, flags kept across a thread switch, matdyn's u_i override
    // and the texture piece controls; es100/utri's Mesa side is not tried:
    // Mesa lists u_scale/u_bias once per component. The front end decides
    // what is tried, so its growth moves these on purpose). A texture
    // program is tried only where the fixture proves which texture each
    // sampler reads (see eval_textures).
    // A build that reports ES 2.0 (report-es2, every dEQP-GLES2 online image)
    // refuses the corpus's GLSL ES 3.00 sources in the front end (pp.rs
    // out_of_scope), so fewer pairs are tried there; each build has its own
    // pins, both measured (2026-09-24, origin/main b96e77df plus this change).
    // 2026-10-04 (work/online-only, ES 3 build): es300/texsize.frag now
    // declares its sampler2DArray's precision (GLSL ES 3.00 4.5.4), so the
    // front end takes it: tried +1, as unmodelled (uniform contents 47,
    // Mesa's texture size), not tried -1.
    let es2 = cfg!(feature = "report-es2");
    assert_eq!(
        (pairs.len(), tried, agree.len(), n_un, n_nt),
        if es2 { (141, 78, 78, 0, 63) } else { (141, 101, 98, 3, 40) },
        "the population moved; re-derive the pins"
    );
    assert_eq!(retried.len(), if es2 { 2 } else { 5 }, "the positive-input retries moved: {retried:?}");
    assert_eq!(total, if es2 { 17_100 } else { 21_676 }, "the compared values moved");
    println!("  agreeing pairs that sampled a texture: {sampled:?}");
    assert_eq!(sampled.len(), if es2 { 26 } else { 34 }, "the texture pairs moved: {sampled:?}");
    println!("  their LDTMUs: {ldtmus}; pairs reading with no switch since the request: {without_switch:?}");
    println!(
        "  pairs that read each cube face (+X -X +Y -Y +Z -Z): {face_pairs:?}; each level (0 1 2): {level_pairs:?}"
    );
    // Pinned. Every cube face is read by some agreeing pair of Mesa's (the
    // three Qt cube programs), and level 1 by skybox_300's textureLod, the
    // one explicit level of detail Mesa's agreeing pairs carry (pinned to
    // 0.75 by UNIFORM_OVERRIDES, SKYBOX_LEVEL); level 2 by none, so the
    // level choice past the first mipmap is pinned by texref's C fixture
    // alone. A report-es2 build tries none of them (GLSL ES 3.00).
    assert_eq!(
        (face_pairs, level_pairs),
        if es2 { ([0; 6], [26, 0, 0]) } else { ([2, 1, 3, 1, 1, 2], [33, 1, 0]) },
        "the cube faces and levels Mesa's agreeing pairs read moved"
    );
    // Pinned, measured on f44e20fe: Mesa switches threads between every
    // request and its LDTMUs (ntq_flush_tmu), so none reads without one.
    // The same counter read every one of ours before e1165634, so it can
    // see both.
    assert_eq!(ldtmus, if es2 { 66 } else { 84 }, "the LDTMUs of Mesa's texture pairs moved");
    assert!(without_switch.is_empty(), "Mesa read a TMU word with no switch since its request: {without_switch:?}");
    // The texture pieces, each mutated in every pair that sampled: each
    // must be caught (a disagreement on a draw on which the pair agrees).
    let mut pieces: BTreeMap<&str, (usize, usize, Vec<String>)> = BTreeMap::new(); // caught, n/a, other
    let mut unobservable = Vec::new();
    let mut equivalent: Vec<String> = Vec::new();
    let mut not_applicable: BTreeMap<(&str, &str), usize> = BTreeMap::new();
    for name in &sampled {
        let p = pairs.iter().find(|p| format!("{} + {}", p.vs_path, p.fs_path) == *name).expect("a sampled pair");
        if let Some((_, why)) = TMU_UNOBSERVABLE.iter().find(|(src, _)| p.fs_path == *src) {
            unobservable.push(format!("{name}: {why}"));
            continue;
        }
        for (piece, q) in tmu_mutants(p) {
            if let Some((_, _, why)) = TMU_EQUIVALENT.iter().find(|(src, pc, _)| p.fs_path == *src && *pc == piece) {
                equivalent.push(format!("{name} {piece}: {why}"));
                continue;
            }
            let e = pieces.entry(piece).or_default();
            match q.map(|q| control(p, &q, name)) {
                Ok(Control::Caught) => e.0 += 1,
                Err(why) => {
                    e.1 += 1;
                    *not_applicable.entry((piece, why)).or_default() += 1;
                }
                Ok(c) => e.2.push(format!("{name}: {c:?}")),
            }
        }
    }
    for (piece, (c, na, other)) in &pieces {
        println!("  texture piece {piece}: {c} caught, {na} not applicable, {} not caught {other:?}", other.len());
    }
    for ((piece, why), n) in &not_applicable {
        println!("  texture piece {piece} not applicable: {why}: {n}");
    }
    for u in &unobservable {
        println!("  texture pieces unobservable in {u}");
    }
    // lightmapuvraster is ES 3.00: a report-es2 build does not try it.
    assert_eq!(unobservable.len(), if es2 { 0 } else { TMU_UNOBSERVABLE.len() }, "a TMU_UNOBSERVABLE entry moved");
    for e in &equivalent {
        println!("  texture piece equivalent (the same program), {e}");
    }
    // idxtex is ES 3.00: a report-es2 build does not try it. Mesa's
    // lightmapuvraster pieces are TMU_UNOBSERVABLE, so its entry is ours only.
    let mesa_equivalent =
        TMU_EQUIVALENT.iter().filter(|(src, _, _)| !TMU_UNOBSERVABLE.iter().any(|(u, _)| u == src)).count();
    assert_eq!(equivalent.len(), if es2 { 0 } else { mesa_equivalent }, "a TMU_EQUIVALENT entry moved");
    let counts: Vec<(&str, usize, usize)> = pieces.iter().map(|(k, (c, na, _))| (*k, *c, *na)).collect();
    assert_eq!(
        counts,
        if es2 {
            [
                /* 4 -> 26 caught when the mutant redirects a read of the
                 * first LDTMU's word made before the second lands (step
                 * (n), ldtmu_trade): Mesa reads it early in 22 of 26 */
                ("LDTMU destinations", 26, 0),
                ("P0 texture index", 26, 0),
                ("P1 output type", 26, 0),
                ("TMUT <-> TMUB sources", 0, 26),
                ("TMUT <-> TMUR sources", 0, 26),
                ("TMUT <-> TMUS sources", 18, 8),
            ]
        } else {
            [
                /* 7 -> 31 with ldtmu_trade's redirect (step (n)); the 2
                 * left have one LDTMU */
                ("LDTMU destinations", 31, 2),
                ("P0 texture index", 33, 0),
                ("P1 output type", 33, 0),
                ("TMUT <-> TMUB sources", 1, 32),
                ("TMUT <-> TMUR sources", 3, 30),
                ("TMUT <-> TMUS sources", 24, 8),
            ]
        },
        "the texture piece controls moved"
    );
    assert!(pieces.values().all(|(_, _, o)| o.is_empty()), "a texture piece was not caught: {pieces:?}");
    // Every mutant is caught by a disagreement. The four that survived the
    // pair's own draw alone (anyall fs 1 and vs 6: all(a_uv == 0) never
    // held with |x| >= 0.25; boolany fs 26: u_b was false; loopcarry fs 3:
    // u_k.x < 0, so no trip) are caught on the control's other draws.
    assert!(survived.is_empty(), "a mutant of Mesa's code survived every draw: {survived:?}");
    assert!(unmeasured.is_empty(), "a mutant of Mesa's code was never compared: {unmeasured:?}");
    assert_eq!((caught, none), if es2 { (140, 16) } else { (176, 20) }, "the negative control moved");
    /* The one mutant no draw could compare is in an ES 3.00 pair, which a
     * report-es2 build does not try. */
    let want_skipped: &[&str] =
        if es2 { &[] } else { &["es300/flatlast.vert + es300/flatlast.frag vs instruction 3: fmul a -> a.l"] };
    assert_eq!(skipped, want_skipped, "the mutants no draw could compare moved (each stops the run on every draw)");
}

// ---------------------------------------------------------------------------
// The three-way check (docs/online-shaderc/v3d-backend.md, "How it is
// checked without a board", step 3): OUR compiler's program (a) against the
// IR (b), and against Mesa's program (c) where Mesa compiles the pair.
// ---------------------------------------------------------------------------

use yakogl::shaderc::online::v3d::chain::{compile_program, Prim, Target, V3dError};

/// The VPM of the Raspberry Pi 4's V3D 4.2, measured on the board
/// (2026-09-24): the target every program here is compiled for.
const TARGET: Target = Target { vpm_size: 65536 };

/// Why our compiler did not produce a program.
enum Ours {
    /// The front end refused a source (not our back end's business); the
    /// reason is printed with the count.
    FrontEnd(String),
    /// A named construct the back end does not compile yet: coverage.
    NotYet(&'static str),
    /// Any other refusal: to be looked at (a defect, or a missing NotYet).
    Other(String),
}

/// OUR program for a pair, as a [`Pair`] the harness runs like Mesa's.
fn ours_pair(vs_path: &str, fs_path: &str) -> Result<Pair, Ours> {
    let mut mem = vec![0u8; 16 << 20];
    let a = Arena::new(&mut mem);
    let fe = |e: Skip| match e {
        Skip::NotTried(w) | Skip::Unmodelled(w) => Ours::FrontEnd(w),
    };
    let fsl = compile(&a, sema::Stage::Fragment, fs_path).map_err(fe)?;
    let vsl = compile(&a, sema::Stage::Vertex, vs_path).map_err(fe)?;
    let (fsm, vsm) = (&fsl.module, &vsl.module);
    let prog = compile_program(vsm, fsm, Prim::Triangles, TARGET, &a).map_err(|e| match e {
        V3dError::NotYet(what) => Ours::NotYet(what),
        e => Ours::Other(format!("{e:?}")),
    })?;
    if debug() {
        println!(
            "ours {vs_path} + {fs_path}: attributes {:?}, vs vattr {:?}, cs vattr {:?}, fs input_slots {:?}",
            prog.attributes, prog.vs.vattr_sizes, prog.cs.vattr_sizes, prog.fs.input_slots
        );
        println!(
            "IR VS inputs: {:?}",
            vsm.inputs.iter().map(|d| (d.name, d.kind, d.location, d.gl_type)).collect::<Vec<_>>()
        );
    }
    let hex = |x: u32| format!("0x{x:08x}");
    let tf = |b: bool| if b { "True" } else { "False" }.to_string();
    let code = |c: &yakogl::shaderc::online::v3d::chain::Code| -> (Vec<u64>, Vec<(u32, u32)>) {
        (c.words.to_vec(), c.uniforms.iter().map(|u| (u.contents, u.data)).collect())
    };
    let mut p = Pair { vs_path: vs_path.into(), fs_path: fs_path.into(), ours: true, ..Default::default() };
    (p.fs.code, p.fs.uniforms) = code(&prog.fs.code);
    p.fs.inputs = prog.fs.input_slots.iter().map(|&(s, c)| (s as u32, c as u32)).collect();
    // The thread count each program was compiled for, as Mesa's prog_data
    // gives the fixture's: TIDX and the TMU output FIFO bound read it.
    p.fs.meta.insert("threads".into(), prog.fs.threads.to_string());
    p.fs.meta.insert("flat_shade_flags".into(), hex(prog.fs.flat_shade_flags));
    p.fs.meta.insert("noperspective_flags".into(), hex(prog.fs.noperspective_flags));
    p.fs.meta.insert("centroid_flags".into(), hex(prog.fs.centroid_flags));
    p.fs.meta.insert("implicit_point".into(), tf(prog.fs.uses_implicit_point_line_varyings));
    for (st, vp) in [(&mut p.vs, &prog.vs), (&mut p.cs, &prog.cs)] {
        (st.code, st.uniforms) = code(&vp.code);
        st.vattr = vp.vattr_sizes.iter().map(|&x| x as u32).collect();
        st.meta.insert("threads".into(), vp.threads.to_string());
        st.meta.insert("uses_vid".into(), tf(vp.uses_vid));
        st.meta.insert("uses_iid".into(), tf(vp.uses_iid || vp.uses_biid));
        st.meta.insert("writes_psiz".into(), tf(vp.writes_psiz));
    }
    // (location, record) per attribute location, to Mesa's shape: per IR
    // attribute declaration, the record of each of its columns.
    for d in vsm.inputs.iter().filter(|d| d.kind == IoKind::Attribute) {
        let recs = (0..locations(d.gl_type))
            .map(|k| prog.attributes.iter().find(|(loc, _)| *loc == d.location + k).and_then(|(_, r)| r.map(u32::from)))
            .collect();
        p.attrs.push((d.name.to_string(), d.location as u32, recs));
    }
    Ok(p)
}

/// Two programs' outputs on the same inputs, compared: the colour, the
/// discards, and the rows both compilers lay out the same way.
fn compare_records(a: &Record, c: &Record) -> Option<String> {
    let approx = a.approx || c.approx;
    if a.discarded != c.discarded {
        return Some(format!("discards: ours {:?}, Mesa's {:?}", a.discarded, c.discarded));
    }
    if a.colour.len() != c.colour.len() {
        return Some(format!("{} colour TLB writes, Mesa's {}", a.colour.len(), c.colour.len()));
    }
    for (k, (x, y)) in a.colour.iter().zip(&c.colour).enumerate() {
        for l in (0..LANES).filter(|&l| !a.discarded[l]) {
            for h in 0..2 {
                let (p, q) = ((x[l] >> (16 * h)) as u16, (y[l] >> (16 * h)) as u16);
                if !close_f16(q, p, approx) {
                    return Some(format!("colour word {k} half {h} lane {l}: ours {p:#06x}, Mesa's {q:#06x}"));
                }
            }
        }
    }
    for (what, xs, ys) in [("VS", &a.vs_rows, &c.vs_rows), ("CS", &a.cs_rows, &c.cs_rows)] {
        for (r, (x, y)) in xs.iter().zip(ys.iter()).enumerate() {
            let (Some(x), Some(y)) = (x, y) else {
                return Some(format!("{what} row {r}: written by one program only"));
            };
            let scaled = (what == "VS" && r < 2) || (what == "CS" && (4..6).contains(&r));
            for l in 0..LANES {
                let ok = if scaled { close_int(y[l], x[l], approx) } else { close_f32(y[l], x[l], approx) };
                if !ok {
                    return Some(format!("{what} row {r} lane {l}: ours {:#010x}, Mesa's {:#010x}", x[l], y[l]));
                }
            }
        }
    }
    None
}

/// Programs the back end's own tests compile that the corpus pairing does
/// not form (chain.rs: passthrough_and_interp_color), run here too.
const EXTRA_PAIRS: &[(&str, &str)] = &[
    ("gl_test_es100/passthrough.vert", "gl_test_es100/interp_color.frag"),
    // dEQP-GLES2 conditionals.if.{sequence,complex}_condition's fragment
    // cases (a constant bool in a condition), compared with the IR only.
    ("gl_test_es100/passthrough.vert", "es100/cond_sequence.frag"),
    ("gl_test_es100/passthrough.vert", "es100/cond_complex.frag"),
    // Ilt / Ile on data: no corpus program compiled here executes one.
    ("gl_test_es100/passthrough.vert", "es100/int_compare.frag"),
    // dEQP-GLES2 light_amount (2 directional + 2 omni lights): two expanded
    // loops, a vertex shader at 2 threads.
    ("es100/light_amount_2_2.vert", "es100/light_amount.frag"),
    // 16 lights: fits once the constant index folds and each uniform is
    // loaded at its first read (prepare.rs fold, select.rs Sel::val).
    ("es100/light_amount_0_16.vert", "es100/light_amount.frag"),
    // Nested loops with uniform bounds (data exits), both stages: dEQP's
    // nested, nested_sequence and nested_tricky_dataflow shapes.
    ("es100/dynidx.vert", "es100/loop_nested.frag"),
    ("es100/loop_nested.vert", "es100/va.frag"),
    ("es100/dynidx.vert", "es100/loop_nested.while.frag"),
    ("es100/dynidx.vert", "es100/loop_nested.dowhile.frag"),
    // One loop with a uniform bound, not nested: dEQP's for_uniform_iterations
    // fragment and do_while_uniform_iterations vertex basic cases.
    ("es100/dynidx.vert", "es100/loop_uniform.frag"),
    ("es100/loop_uniform.dowhile.vert", "es100/va.frag"),
    // dEQP return_in_dynamic_loop_never_fragment: two swapped pairs carried.
    ("es100/dynidx.vert", "es100/ret_dynloop_never.frag"),
    // A discard inside a loop with a uniform bound (dEQP discard.dynamic_loop_*).
    ("es100/discard_dynloop.vert", "es100/discard_dynloop.frag"),
    // gl_FrontFacing, from REVF.
    ("es100/dynidx.vert", "es100/frontfacing.frag"),
    // A varying written one lane at a time on some paths (imageparticle_tabled).
    ("es100/partial_varying.vert", "es100/partial_varying.frag"),
    // dEQP functions.control_flow.return_in_nested_loop, both stages.
    ("es100/dynidx.vert", "es100/ret_nested.frag"),
    ("es100/ret_nested.vert", "es100/va.frag"),
    // A texture look-up in the vertex shader (explicit level 0, TMUSLOD,
    // and the last thread switch before the VPM writes), through a sampler
    // in a uniform struct and in a struct array (binding 1).
    ("es100/vtex_struct_sampler.vert", "es100/va.frag"),
    ("es100/vtex_struct_sampler_array.vert", "es100/va.frag"),
    // dEQP-GLES2 vertex_arrays (a gl_PointSize write, dropped for triangles).
    ("es100/va_vec2.vert", "es100/va.frag"),
    ("es100/va_vec4x2.vert", "es100/va.frag"),
    // dEQP-GLES2 clipping and rasterization (gl_PointSize from an attribute
    // and from a uniform; rasterization's varying is highp in the VS).
    ("es100/clipping.vert", "es100/clipping.frag"),
    ("es100/rasterization.vert", "es100/rasterization.frag"),
    // dEQP-GLES2 attribute_location: sixteen vec4 attributes summed (the
    // lazy attribute reads, chain.rs vertex) and a float in a hole.
    ("es100/attrib_max_vec4.vert", "es100/attrib.frag"),
    ("es100/attrib_hole_float.vert", "es100/attrib.frag"),
    // float -> bool on data (F2b), -0.0 included.
    ("gl_test_es100/passthrough.vert", "es100/f2b.frag"),
    // Integer division and remainder on data (nir_lower_idiv's sequence).
    ("gl_test_es100/passthrough.vert", "es100/int_div.frag"),
    ("es300/int_div.vert", "es300/int_div.frag"),
    // The counted loop: a kept loop that runs its whole count on most lanes,
    // and a loop with a run-time bound that must leave by its exit flag.
    ("gl_test_es100/passthrough.vert", "es100/loop_full.frag"),
    ("es300/loop_exit.vert", "es300/loop_exit.frag"),
    // A uniform array at a run-time index, past both ends (the clamp).
    ("gl_test_es100/passthrough.vert", "es100/uniform_idx.frag"),
    // dEQP shaders.indexing: a vector written and read at run-time indices.
    ("es100/dynidx.vert", "es100/dynidx.frag"),
    ("es100/dynidx.vert", "es100/dynidx_loop.frag"),
    // gl_PointSize written in both arms of an if: the merged value in the
    // point-size row every program that writes it keeps.
    ("es100/psiz_if.vert", "es100/va.frag"),
    // dEQP shaders.indexing.tmp_array: 40 constants read at run-time
    // indices, which fit only with each constant loaded at its read
    // (select.rs Pressure::Remat), in either stage.
    ("es100/dynidx.vert", "es100/tmparr_const.frag"),
    ("es100/tmparr_const.vert", "es100/va.frag"),
];

#[test]
fn our_programs_agree_with_the_ir_and_with_mesa() {
    let mut reals = real_pairs();
    assert_eq!(reals.len(), header("real pairs"), "the fixture lists as many real pairs as its header says");
    for (vs, fs) in EXTRA_PAIRS {
        assert!(
            !reals.iter().any(|(a, b)| a == vs && b == fs),
            "{vs} + {fs} is a real pair now; drop it from EXTRA_PAIRS"
        );
        reals.push((vs.to_string(), fs.to_string()));
    }
    let mesa: BTreeMap<(String, String), Pair> =
        parse().into_iter().map(|p| ((p.vs_path.clone(), p.fs_path.clone()), p)).collect();

    let mut front_end = 0usize;
    let mut not_yet: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut other: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut compiled = Vec::new();
    // (a) against the IR
    let (mut agree, mut disagree) = (Vec::new(), Vec::new());
    let mut unmodelled: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut not_tried: BTreeMap<String, Vec<String>> = BTreeMap::new();
    // (a) against (c)
    let (mut vs_mesa_agree, mut vs_mesa_disagree, mut vs_mesa_absent) = (Vec::new(), Vec::new(), Vec::new());
    let mut vs_mesa_skipped: Vec<String> = Vec::new();
    let (mut compared_total, mut mutants_caught, mut mutants_none) = (0usize, 0usize, 0usize);
    let mut mutants_survived: Vec<String> = Vec::new();
    let mut mutants_unmeasured: Vec<String> = Vec::new();
    let mut mutants_skipped: Vec<String> = Vec::new();
    // Our texture programs' piece controls: caught, not applicable, other.
    let mut ours_pieces: BTreeMap<&str, (usize, usize, Vec<String>)> = BTreeMap::new();
    let (mut ours_sampled, mut ours_equivalent) = (Vec::new(), Vec::new());
    let mut ours_unmeasured: Vec<String> = Vec::new();
    // Per piece and reason, our pairs where the mutant does not apply.
    let mut ours_not_applicable: BTreeMap<(&str, &str), Vec<String>> = BTreeMap::new();
    let (mut ours_ldtmus, mut ours_without_total, mut ours_without_switch) = (0usize, 0usize, Vec::new());
    // Per cube face (+X -X +Y -Y +Z -Z) and per level: how many of our
    // texture pairs read it (the same count as Mesa's, per item).
    let (mut ours_face_pairs, mut ours_level_pairs) = ([0usize; 6], [0usize; 3]);
    for (vs, fs) in reals.iter() {
        let name = format!("{vs} + {fs}");
        let ours = match ours_pair(vs, fs) {
            Ok(p) => p,
            Err(Ours::FrontEnd(w)) => {
                front_end += 1;
                if debug() {
                    println!("{name}: front end: {w}");
                }
                continue;
            }
            Err(Ours::NotYet(w)) => {
                if debug() {
                    println!("{name}: NotYet {w}");
                }
                *not_yet.entry(w).or_default() += 1;
                continue;
            }
            Err(Ours::Other(w)) => {
                other.entry(w).or_default().push(name);
                continue;
            }
        };
        compiled.push(name.clone());
        let seed = seed_of(&name);
        COMPARED.with(|c| c.set(0));
        let (va, ra) = run_pair(&ours, seed);
        // The counted loop's early exit (es300/loop_exit: bounds 0..5 read at
        // run time, a cap of 4096 passes). No value can show it: the unroller
        // guards every pass after a break. Measured 2026-09-24: 276
        // instructions with the exit, 118,862 with it ignored.
        if name == "es300/loop_exit.vert + es300/loop_exit.frag" && matches!(va, Verdict::Agree) {
            let n = RECORD.with(|r| r.borrow().instructions);
            assert!(n <= 1000, "{name}: {n} instructions: the counted loop did not leave by its exit flag");
        }
        // Nested counted loops (es100/loop_nested: bounds 1..3 read at run
        // time, a cap of 4096 passes each). An inner loop must also leave on
        // its outer loops' break flags: a lane whose outer loop has broken
        // can never set the inner flag (the store is guarded). No value can
        // show it. Measured 2026-09-24: 4,199 (FS) and 3,759 (VS)
        // instructions with the outer flags; 953,799 (VS) and past the
        // interpreter's 1,000,000 (FS) without them.
        if name.contains("es100/loop_nested.") && matches!(va, Verdict::Agree) {
            let n = RECORD.with(|r| r.borrow().instructions);
            assert!(n <= 10_000, "{name}: {n} instructions: a nested loop did not leave by its outer loop's flag");
        }
        let rec_a = RECORD.with(|r| r.borrow().clone());
        let ours_ran = matches!(va, Verdict::Agree | Verdict::Disagree(_));
        if matches!(va, Verdict::Agree) {
            let n = COMPARED.with(|c| c.get());
            assert!(n > 0, "{name}: ours agrees having compared nothing");
            compared_total += n;
            // Negative control: our program with one instruction changed in
            // the FS, then in the VS, must stop agreeing.
            for stage in ["fs", "vs"] {
                let Some((what, c, skip)) = control_stage(&ours, stage, &name) else {
                    mutants_none += 1;
                    continue;
                };
                mutants_skipped.extend(skip);
                match c {
                    Control::Caught => mutants_caught += 1,
                    Control::Survived => mutants_survived.push(what),
                    Control::Unmeasured => mutants_unmeasured.push(what),
                }
            }
            // The texture pieces of OUR program (the same four as Mesa's,
            // tmu_mutants), where its own draw performed a TMU look-up.
            if rec_a.tmu_lookups > 0 {
                ours_sampled.push(name.clone());
                for (f, n) in ours_face_pairs.iter_mut().enumerate() {
                    *n += usize::from(rec_a.cube_faces & (1 << f) != 0);
                }
                for (k, n) in ours_level_pairs.iter_mut().enumerate() {
                    *n += usize::from(rec_a.levels & (1 << k) != 0);
                }
                ours_ldtmus += rec_a.ldtmus;
                ours_without_total += rec_a.ldtmus_without_switch;
                if rec_a.ldtmus_without_switch > 0 {
                    ours_without_switch.push(format!("{name}: {} of {}", rec_a.ldtmus_without_switch, rec_a.ldtmus));
                }
                // TMU_UNOBSERVABLE is about Mesa's fixture program (render target
                // 0 alone); ours writes every target and is compared on each.
                for (piece, q) in tmu_mutants(&ours) {
                    if let Some((_, _, why)) = TMU_EQUIVALENT.iter().find(|(src, pc, _)| *fs == *src && *pc == piece) {
                        ours_equivalent.push(format!("{name} {piece}: {why}"));
                        continue;
                    }
                    let e = ours_pieces.entry(piece).or_default();
                    match q.map(|q| control(&ours, &q, &name)) {
                        Ok(Control::Caught) => e.0 += 1,
                        Err(why) => {
                            e.1 += 1;
                            ours_not_applicable.entry((piece, why)).or_default().push(name.clone());
                        }
                        Ok(Control::Unmeasured)
                            if TMU_UNMEASURED.iter().any(|(pair, pc, _)| name == *pair && *pc == piece) =>
                        {
                            ours_unmeasured.push(format!("{name} {piece}"))
                        }
                        Ok(c) => e.2.push(format!("{name}: {c:?}")),
                    }
                }
            }
        }
        match va {
            Verdict::Agree => agree.push(name.clone()),
            Verdict::Disagree(d) => disagree.push(format!("{name}: {d}")),
            Verdict::Unmodelled(w) => unmodelled.entry(w).or_default().push(name.clone()),
            Verdict::NotTried(w) => not_tried.entry(w).or_default().push(name.clone()),
        }
        match mesa.get(&(vs.clone(), fs.clone())) {
            None => vs_mesa_absent.push(name),
            Some(m) => {
                let (vc, rc) = run_pair(m, seed);
                let rec_c = RECORD.with(|r| r.borrow().clone());
                let ran = |v: &Verdict| matches!(v, Verdict::Agree | Verdict::Disagree(_));
                // Both runs must have finished: a run that stopped part way
                // leaves a partial record (a VS but no FS), which is not an
                // output to compare with.
                if !ours_ran || !ran(&vc) || ra != rc {
                    vs_mesa_skipped.push(name);
                } else {
                    match compare_records(&rec_a, &rec_c) {
                        None => vs_mesa_agree.push(name),
                        Some(d) => vs_mesa_disagree.push(format!("{name}: {d}")),
                    }
                }
            }
        }
    }
    let n_un: usize = unmodelled.values().map(Vec::len).sum();
    let n_nt: usize = not_tried.values().map(Vec::len).sum();
    let tried = agree.len() + disagree.len() + n_un;
    let n_other: usize = other.values().map(Vec::len).sum();
    let n_not_yet: usize = not_yet.values().sum();
    println!(
        "our compiler on {} pairs ({} real, {} extra): {front_end} refused by the front end, {n_not_yet} NotYet, \
         {n_other} other refusals, {} compiled",
        reals.len(),
        reals.len() - EXTRA_PAIRS.len(),
        EXTRA_PAIRS.len(),
        compiled.len()
    );
    for (w, n) in &not_yet {
        println!("  NotYet {n} x {w}");
    }
    for (w, ps) in &other {
        println!("  OTHER {} x {w}  (e.g. {})", ps.len(), ps[0]);
    }
    println!(
        "(a) ours vs (b) the IR: {tried} tried: {} agree, {} disagree, {n_un} unmodelled; {n_nt} not tried",
        agree.len(),
        disagree.len()
    );
    for d in &disagree {
        println!("  DISAGREE {d}");
    }
    for (w, ps) in &unmodelled {
        println!("  unmodelled {} x {w}  (e.g. {})", ps.len(), ps[0]);
    }
    // The undefined-precision bucket, every pair by name: none of them is
    // agree or disagree, and which pairs they are is pinned below.
    let trig: Vec<&str> =
        unmodelled.get(TRIG_UNDETERMINED).map(|v| v.iter().map(String::as_str).collect()).unwrap_or_default();
    println!("  {} x {TRIG_UNDETERMINED}: {trig:?}", trig.len());
    for (w, ps) in &not_tried {
        println!("  not tried {} x {w}  (e.g. {})", ps.len(), ps[0]);
    }
    println!(
        "(a) ours vs (c) Mesa's: {} agree, {} disagree, {} not compared (either run stopped), {} with no Mesa program",
        vs_mesa_agree.len(),
        vs_mesa_disagree.len(),
        vs_mesa_skipped.len(),
        vs_mesa_absent.len()
    );
    for d in &vs_mesa_disagree {
        println!("  DISAGREE WITH MESA {d}");
    }
    println!("  no Mesa program: {vs_mesa_absent:?}");
    println!("  not compared: {vs_mesa_skipped:?}");
    println!("  compiled: {compiled:?}");
    println!("  values compared, (a) vs (b): {compared_total}");
    println!(
        "  negative control on our programs: {} mutants, {mutants_caught} caught, {} survived, {} unmeasured, {mutants_none} with nothing to mutate",
        mutants_caught + mutants_survived.len() + mutants_unmeasured.len(),
        mutants_survived.len(),
        mutants_unmeasured.len()
    );
    for m in &mutants_survived {
        println!("    survived: {m}");
    }
    for m in &mutants_unmeasured {
        println!("    unmeasured: {m}");
    }
    for m in &mutants_skipped {
        println!("    no draw compared (the next mutant was used): {m}");
    }
    assert_eq!(tried + n_nt, compiled.len());
    assert_eq!(
        vs_mesa_agree.len() + vs_mesa_disagree.len() + vs_mesa_skipped.len() + vs_mesa_absent.len(),
        compiled.len()
    );
    if debug() {
        return;
    }
    assert_eq!(front_end + n_not_yet + n_other + compiled.len(), reals.len());
    assert!(disagree.is_empty(), "{} of our programs compute something other than their IR", disagree.len());
    assert!(vs_mesa_disagree.is_empty(), "{} of our programs differ from Mesa's", vs_mesa_disagree.len());
    assert!(mutants_survived.is_empty(), "a mutant of our code survived every draw: {mutants_survived:?}");
    assert!(mutants_unmeasured.is_empty(), "a mutant of our code was never compared: {mutants_unmeasured:?}");
    // The populations, pinned (HEAD 10c11d0a with v3d-m5's uncommitted back
    // end of 2026-09-24, which emits QUNIFORM_UNIFORM). They move when the back end compiles more, which is
    // the point: re-derive them then, deliberately.
    // A build that reports ES 2.0 (report-es2, every dEQP-GLES2 online image)
    // refuses the corpus's GLSL ES 3.00 sources in the front end (pp.rs
    // out_of_scope), so fewer pairs are tried there; each build has its own
    // pins, both measured (2026-09-24, origin/main 657eb8cb).
    // 2026-09-24 (frontend3, ES 3 build only): front end 65 -> 64, not yet
    // 147 -> 148. ES 3.00 gl_FragDepth is declared now, so qt3d/grid_300
    // passes the front end and stops in this back end instead (a population
    // change, not a new failure). With v3d-m5's discard commit (rebased on
    // 2fb2d8f2): 45 / 48 compiled; grid_300 stops here on "uniform
    // u_buf.gridFlags of GL type 0x1405".
    // 2026-09-26 (v3d-m5, gl_VertexID, ES 3 build only): not yet 50 -> 46,
    // compiled 206 -> 210, (a) tried 202 -> 206 and agree 191 -> 195: the
    // four es300/vid.vert pairs (vcolor, dynidx, facing, uniloop), each
    // run with the ID row (lane + 7) ahead of the attributes; (c) agree
    // 88 -> 92: all four agree with Mesa's own programs as well; controls
    // compared 43126 -> 44022, 8 new mutants (4 pairs x FS, VS): 7 caught,
    // 1 with nothing to mutate (19 -> 20).
    // 2026-09-26 (v3d-m5, gl_InstanceID, ES 3 build only): not yet 46 -> 37,
    // compiled 210 -> 219: the nine pairs that read gl_InstanceID (the
    // es300/inst pair and eight Qt Quick 3D particle pairs). All nine are
    // compared: (a) 206/195 -> 215/204, (c) 92 -> 93 agree (only inst has a
    // Mesa program), controls 44022/370 -> 46198/388, every mutant caught.
    // Three instrument fixes came with them, each measured: uint uniforms are
    // drawn (1..4; the harness skipped GL type 0x1405); the evaluator judges
    // an undefined division or shift on the lanes the result defines, not on
    // a scalar's spare lanes (eval.rs; 8 pairs were "unsigned division by
    // zero" on block padding); and a vertex shader that indexes an array by
    // gl_VertexID gets IDs inside it (vertex_ids; a mutant on the sprite
    // particles' texcoord survived on out-of-bounds corners).
    // 2026-09-26 (v3d-m5, flat varyings, ES 3 build only): not yet 37 -> 28,
    // compiled 219 -> 228 (es300/flatlast and eight particle pairs; eight
    // more flat ones stop at gl_FragData[n > 0], 10 -> 18). (a) 224 tried:
    // 209 agree, 0 disagree, 15 unmodelled, of which 4 are the new
    // TRIG_UNDETERMINED bucket (particles _21 _38 _73 _78, whose qt_rand
    // hash amplifies sin's reduction error; ES 3.00 4.5.1 leaves trig
    // precision undefined). (c) 93 -> 94 (flatlast agrees with Mesa too).
    // Controls 47478 compared, 398 caught. A flat input is fed as bits.
    // 2026-09-26 (v3d-m5, multiple render targets, ES 3 build only): not
    // yet 24 -> 6, compiled 232 -> 250 (the 18 pairs that write an output
    // past location 0), (a) 242 tried / 222 agree / 0 disagree / 20
    // unmodelled (the trig bucket 4 -> 8, all qt_rand; lightmapuvraster_300
    // a NaN), (c) 95 agree (clear_mrt_300 agrees with Mesa on RT 0, the one
    // Mesa's fixture program writes), controls 52342 compared, 423 of 423
    // caught, level-0 readers 104, LDTMUs 748, texture pairs 106. The FS
    // comparison now reads each RT from Mesa's TLB config byte, and a
    // mutant whose difference lands in the trig bucket on a draw where the
    // unmutated program agreed counts as caught (it is told apart from the
    // program; four OIT mutants "survived" before).
    // 2026-09-26 (v3d-m5, derivatives, both builds): not yet -4, compiled +4
    // (the Qt wireframe pairs; grid_300 stops at gl_FragDepth), and all four
    // "not tried": the evaluator has no derivatives and the interpreter
    // stops at FDX (the quad lane layout is not modelled), so they are the
    // board's to measure. No Mesa program for them.
    // 2026-09-26 (v3d-m5, look-ups in uniform-branch arms, ES 3 build): (a)
    // 223 -> 224 agree, 20 -> 19 unmodelled: lightmapuvraster_300's NaN
    // (a normalize in the arm its uniform flag leaves untaken) is no longer
    // computed, as its three `if (flag) { texture(...) }` are branches now;
    // it agrees with Mesa too ((c) 96 -> 97).
    // 2026-09-30 (v3d/loops-regression, both builds): +3 pairs, all
    // compiled: es100/loop_uniform (one for loop, uniform bound, FS),
    // loop_uniform.dowhile (one do-while, uniform bound, VS), dEQP's
    // shaders.loops basic cases, and ret_dynloop_never (dEQP
    // return_in_dynamic_loop_never_fragment). All three disagreed with the IR before the
    // loop end's copies became one parallel copy (select.rs phi_copies).
    // 2026-10-04 (work/online-only, ES 3 build): es300/texsize.frag's
    // precision fix moves it from the front end's refusals to NotYet
    // ("textureSize of a 3D or array sampler", the V3D's own gap).
    let es2 = cfg!(feature = "report-es2");
    assert_eq!(
        (reals.len(), front_end, n_not_yet, n_other, compiled.len()),
        if es2 { (324, 126, 0, 0, 198) } else { (324, 64, 6, 0, 254) },
        "our compiler's coverage moved; re-derive the pins"
    );
    assert_eq!(
        (tried, agree.len(), n_un, n_nt),
        if es2 { (192, 181, 11, 6) } else { (246, 227, 19, 8) },
        "(a) vs (b) moved"
    );
    // The pairs whose output the language does not fix (qt_rand's
    // fract(sin(dot(co, k)) * 43758.5) on a flat particle index), measured
    // 2026-09-26 on c8e4ef1a; GLSL ES 3.00 only, so none in a report-es2
    // build. A pair entering or leaving this bucket is re-measured, never
    // re-pinned on sight.
    let trig_want: &[&str] = if es2 {
        &[]
    } else {
        &[
            "qt3d/qtparticleshaders_21.vert + qt3d/qtparticleshaders_21.frag",
            "qt3d/qtparticleshaders_29.vert + qt3d/qtparticleshaders_29.frag",
            "qt3d/qtparticleshaders_36.vert + qt3d/qtparticleshaders_36.frag",
            "qt3d/qtparticleshaders_38.vert + qt3d/qtparticleshaders_38.frag",
            "qt3d/qtparticleshaders_45.vert + qt3d/qtparticleshaders_45.frag",
            "qt3d/qtparticleshaders_73.vert + qt3d/qtparticleshaders_73.frag",
            "qt3d/qtparticleshaders_74.vert + qt3d/qtparticleshaders_74.frag",
            "qt3d/qtparticleshaders_78.vert + qt3d/qtparticleshaders_78.frag",
        ]
    };
    let mut trig_sorted = trig.clone();
    trig_sorted.sort_unstable();
    assert_eq!(trig_sorted, trig_want, "the undefined-precision trig bucket moved");
    assert_eq!(
        (vs_mesa_agree.len(), vs_mesa_skipped.len(), vs_mesa_absent.len()),
        if es2 { (77, 4, 117) } else { (97, 6, 151) },
        "(a) vs (c) moved"
    );
    assert_eq!(
        (compared_total, mutants_caught, mutants_none),
        if es2 { (40790, 344, 18) } else { (53654, 433, 21) },
        "the controls moved"
    );
    // A mutant no draw could compare is a blind spot, so each pair that has
    // any is listed with its count and the measured reason, and anything else
    // fails: another pair's skip, or a count that moved.
    for m in &mutants_skipped {
        assert!(
            MUTANTS_SKIPPED.iter().any(|(pair, _, _)| m.starts_with(pair)),
            "a mutant of our code no draw could compare: {m}"
        );
    }
    for (pair, n, why) in MUTANTS_SKIPPED {
        let got = mutants_skipped.iter().filter(|m| m.starts_with(pair)).count();
        assert_eq!(got, *n, "{pair}: mutants no draw could compare moved ({why})");
    }
    // Our texture programs: each piece caught by a disagreement, as on Mesa's.
    println!("  our pairs that sampled a texture: {} {ours_sampled:?}", ours_sampled.len());
    println!(
        "  our pairs that read each cube face (+X -X +Y -Y +Z -Z): {ours_face_pairs:?}; each level (0 1 2): {ours_level_pairs:?}"
    );
    // Pinned (measured on e3553bc6, m5's cube look-up 293418df): every cube
    // face is read by an agreeing pair of ours -- es100/cubebias,
    // es100/cubelod (direction (1, 0, 0)), qt3d/cubeMapToHemisphere_300,
    // qt3d/skybox_300 and qt3d/skyboxcube_300 (all six) -- and level 1 by
    // cubelod and skybox_300 (explicit levels, TMUSLOD / P2 disable_autolod);
    // level 2 by none, so the level choice past the first mipmap is pinned by
    // texref's C fixture alone. -X rests on skyboxcube_300 alone. A report-es2
    // build tries the two ES 1.00 cube pairs only. A change that stops our cube
    // code reaching a face, or reaching it with the right values, moves this.
    // Level 0, ES 3 build: 80 -> 88 with gl_InstanceID (2026-09-26), the eight
    // Qt Quick 3D particle pairs, which read their particle and sprite
    // textures at level 0. 88 -> 92 with flat varyings: the four new
    // agreeing particle pairs (flatlast samples nothing). 105 -> 106 when
    // lightmapuvraster_300 agrees (look-ups in uniform-branch arms).
    assert_eq!(
        (ours_face_pairs, ours_level_pairs),
        if es2 { ([2, 0, 1, 1, 1, 1], [75, 1, 0]) } else { ([4, 1, 4, 2, 2, 3], [106, 2, 0]) },
        "the cube faces and levels our agreeing pairs read moved"
    );
    println!(
        "  their LDTMUs: {ours_ldtmus}; pairs reading with no switch since the request: {} {ours_without_switch:?}",
        ours_without_switch.len()
    );
    // Pinned. Since e1165634 our code switches threads between a request
    // and its LDTMUs, as Mesa does; without the switch the board hung
    // (sim::NO_SWITCH_HANG), and the interpreter now refuses such a read in
    // a threaded program, so the last two numbers can only move with one
    // thread. Before it they were (185, 185, 72) / (180, 180, 70), measured
    // on f44e20fe. ES 3 build 404 -> 514 with gl_InstanceID (2026-09-26):
    // the eight Qt Quick 3D particle pairs' texture reads, none without a
    // switch. 514 -> 576 with flat varyings (the four new agreeing
    // particle pairs), none without a switch. 798 -> 606 / ES2 392 -> 200
    // when a counted loop leaves on its uniform break (unroll.rs, 2026-09-26):
    // the femtovg blur's passes past its tap count no longer run. 606 ->
    // 602 / ES2 200 -> 192 when a uniform `if` with a look-up in an arm is a branch
    // (prepare.rs, step (b)): a look-up in the arm a draw does not take is
    // not requested.
    assert_eq!(
        (ours_ldtmus, ours_without_total, ours_without_switch.len()),
        if es2 { (192, 0, 0) } else { (602, 0, 0) },
        "our LDTMUs read with no switch since their request moved: {ours_without_switch:?}"
    );
    for (piece, (c, na, other)) in &ours_pieces {
        println!("  our texture piece {piece}: {c} caught, {na} not applicable, {} not caught {other:?}", other.len());
    }
    for u in &ours_equivalent {
        println!("  our texture piece not counted: {u}");
    }
    for u in &ours_unmeasured {
        println!("  our texture piece unmeasured: {u}");
    }
    for ((piece, why), names) in &ours_not_applicable {
        println!("  our texture piece {piece} not applicable: {why}: {} {names:?}", names.len());
    }
    // Every TMU_UNMEASURED entry is reached, and unmeasured: one that
    // becomes measurable is caught or survives, and leaves the list.
    assert_eq!(
        ours_unmeasured.len(),
        TMU_UNMEASURED.len(),
        "our texture pieces no draw compares moved: {ours_unmeasured:?}"
    );
    assert!(
        ours_pieces.values().all(|(_, _, o)| o.is_empty()),
        "a texture piece of our code was not caught: {ours_pieces:?}"
    );
    let ours_counts: Vec<(&str, usize, usize)> = ours_pieces.iter().map(|(k, (c, na, _))| (*k, *c, *na)).collect();
    // Pinned, measured on the batch-7 merge 69c19e97 plus tri_discard. LDTMU
    // was not applicable on every one of ours (our code read each LDTMU's
    // result at once, so trading the destinations makes an invalid program,
    // a read before the result lands) until the list scheduler's order
    // (order.rs, 2026-09-26): 0 -> 3 caught, where both LDTMUs come before
    // the first read of either; TMUT <-> TMUS 86 -> 84 / ES2 56 -> 54, the
    // two in TMU_UNMEASURED then (see it for step (l)'s change).
    // ES 3 build 82 -> 90 with gl_InstanceID (2026-09-26): the eight Qt Quick
    // 3D particle pairs. 90 -> 94 with flat varyings: four more particle
    // pairs. 106 -> 107 with textureSize: qt3d/lightmapdilate_300. 107 ->
    // 108 when qt3d/lightmapuvraster_300 agrees (look-ups in branch arms).
    assert_eq!(ours_sampled.len(), if es2 { 76 } else { 108 }, "our texture pairs moved: {ours_sampled:?}");
    // TMU_EQUIVALENT's entries, both ES 3.00: es300/idxtex's s == t, reached
    // by ours since its vertex shader's textureLod compiles, and
    // lightmapuvraster_300's v_uv = vec2(0.0), since it agrees (look-ups in
    // branch arms).
    assert_eq!(
        ours_equivalent.iter().map(|e| e.split(':').next().unwrap_or("")).collect::<Vec<_>>(),
        if es2 {
            Vec::<&str>::new()
        } else {
            std::vec![
                "es300/idxtex.vert + es300/idxtex.frag TMUT <-> TMUS sources",
                "qt3d/lightmapuvraster_300.vert + qt3d/lightmapuvraster_300.frag TMUT <-> TMUS sources",
            ]
        },
        "our texture pieces a draw cannot tell apart moved"
    );
    assert_eq!(
        ours_counts,
        if es2 {
            [
                /* 3 -> 32 caught when a look-up lane read only through an
                 * FMUL/FADD's input unpack has no FMOV (step (f),
                 * 2026-09-29): the first LDTMU's result is no longer read
                 * before the second lands, so the trade is applicable;
                 * 32 -> 37 when VFPACK reads the lanes too (step (h)), for
                 * the same reason: es100 cubebias, gl_test_es100 compact
                 * and texture, qt opaquetexture (both vertex shaders);
                 * 37 -> 30 with step (l) (alloc.rs, the hoist-aware
                 * register choice): es100 cubelod, femtovg filter_image
                 * (both), qt conicalgradient and radialgradient (both
                 * vertex shaders) now read the first LDTMU's result before
                 * the second lands (the tighter schedule), so the trade
                 * makes an invalid program: not applicable; 30 -> 72 when
                 * ldtmu_trade redirects such a read to where the first word
                 * then lands (step (n)): the 42 early readers are caught,
                 * the 4 left (qt distancefield(outline)text, both vertex
                 * shaders) have one LDTMU */
                ("LDTMU destinations", 72, 4),
                ("P0 texture index", 76, 0),
                ("P1 output type", 76, 0),
                /* 0 -> 4 caught when loads took accumulators round-robin
                 * (2026-09-26): four pairs where the trade was not
                 * applicable became applicable, and each is caught; the
                 * cause per pair was not traced */
                /* 4 -> 2 with step (l): the two vtex_struct_sampler
                 * pairs are TMU_UNMEASURED */
                ("TMUT <-> TMUB sources", 2, 72),
                ("TMUT <-> TMUR sources", 1, 75),
                /* 54 -> 55 with step (l): es100/cubelod left
                 * TMU_UNMEASURED, caught */
                ("TMUT <-> TMUS sources", 55, 20),
            ]
        } else {
            /* +8 each with gl_InstanceID, +4 with flat varyings and +12
             * with multiple render targets (2026-09-26): the particle pairs;
             * +1 with textureSize: qt3d/lightmapdilate_300; +1 when
             * qt3d/lightmapuvraster_300 agrees (look-ups in branch arms:
             * P0 and P1 caught, its s/t trade TMU_EQUIVALENT) */
            [
                /* 3 -> 33 caught with step (f), as in the ES2 build;
                 * 33 -> 40 with step (h): the ES2 build's five and qt3d
                 * skybox_300, skyboxcube_300; 40 -> 45 with step (l):
                 * the ES2 build's seven become not applicable, twelve
                 * qt3d particle pairs (1, 3, 10, 12, 35, 56, 58, 60, 61,
                 * 70, 86, 90) become applicable and are caught; 45 -> 102
                 * with ldtmu_trade's redirect (step (n)): the 57 early
                 * readers are caught, the 6 left (the ES2 build's four,
                 * es300 idxtex, qt3d cubeMapToHemisphere_300) have one
                 * LDTMU */
                ("LDTMU destinations", 102, 6),
                ("P0 texture index", 108, 0),
                ("P1 output type", 108, 0),
                /* 1 -> 5 caught when loads took accumulators round-robin
                 * (2026-09-26): four pairs where the trade was not
                 * applicable became applicable, and each is caught; the
                 * cause per pair was not traced */
                /* 5 -> 3 and 84 -> 85 with step (l), as in the ES2
                 * build */
                ("TMUT <-> TMUB sources", 3, 103),
                ("TMUT <-> TMUR sources", 4, 104),
                ("TMUT <-> TMUS sources", 85, 20),
            ]
        },
        "our texture piece controls moved"
    );
}

/// The pairs whose mutants of our code no draw can compare: (pair, count,
/// why). Each entry was established by printing every control draw's verdict,
/// never assumed.
// qt3d/environmentmap's fs had 15 here (texel-edge refusals on the 4x4
// harness textures, and NaN operands): with the 5x5 textures its first fs
// mutant is compared on a draw and caught, so the entry was removed
// (measured on the batch-7 merge 0f6ac998). A new blind spot is added back
// here with its count and its measured reason.
const MUTANTS_SKIPPED: &[(&str, usize, &str)] = &[(
    "es300/flatlast.vert + es300/flatlast.frag",
    /* GLSL ES 3.00: not compiled by a report-es2 build */
    if cfg!(feature = "report-es2") { 0 } else { 1 },
    "the VS's first mutant reads col.a as its low f16 half, so uint(col.a * 4.0 + 0.5) converts a negative \
     float or a NaN, undefined in ES 3.00, and the interpreter stops every draw (Ftouz out of the u32 range, \
     or a NaN operand; measured 2026-09-26); the next mutant is caught",
)];

/// Our instruction counts, pinned, beside Mesa's for the same pairs (the
/// fixture's programs): the compiler-performance work (docs, "Code size
/// against Mesa") must lower these and never raise them. Every femtovg
/// fragment program is pinned by name (the Slint cluster's shaders: the
/// FillImage ones are the backdrop's and the rings', 9.2-9.5 ns/px on the
/// Pi 4); the totals cover every pair both compilers compile, per build.
/// A change is a deliberate re-pin with the reason, never a silent one.
#[test]
fn our_instruction_counts_are_pinned() {
    let es2 = cfg!(feature = "report-es2");
    let (mut ours_t, mut mesa_t, mut n) = ([0usize; 3], [0usize; 3], 0usize);
    let mut femtovg: Vec<(String, usize, usize)> = Vec::new();
    for m in parse() {
        let Ok(o) = ours_pair(&m.vs_path, &m.fs_path) else { continue };
        n += 1;
        for (k, (a, b)) in [(&o.fs, &m.fs), (&o.vs, &m.vs), (&o.cs, &m.cs)].into_iter().enumerate() {
            ours_t[k] += a.code.len();
            mesa_t[k] += b.code.len();
        }
        if m.fs_path.starts_with("femtovg/") {
            let f = m.fs_path.trim_start_matches("femtovg/").to_string();
            femtovg.push((f, o.fs.code.len(), m.fs.code.len()));
        }
    }
    femtovg.sort();
    println!("our instruction counts on {n} pairs (fs, vs, cs): ours {ours_t:?}, Mesa {mesa_t:?}");
    for (f, o, m) in &femtovg {
        println!("  femtovg/{f}: fs ours {o}, Mesa {m} ({:.2}x)", *o as f64 / *m as f64);
    }
    let got: Vec<(&str, usize)> = femtovg.iter().map(|(f, o, _)| (f.as_str(), *o)).collect();
    assert_eq!(got, FEMTOVG_FS_COUNTS, "a femtovg fragment program's length moved: re-pin with the reason");
    assert_eq!((n, ours_t), if es2 { OURS_TOTALS_ES2 } else { OURS_TOTALS }, "our instruction totals moved");
}

/// Loop exits taken by two branches in a row: in a counted loop's body,
/// two forward branches with the same target and no other branch between
/// them (the loop's own test, then a uniform `break`, each with its own
/// three delay slots), where Mesa merges the two terminators into one if
/// (nir_opt_loop.c opt_loop_merge_terminators :649-712). Counted over every
/// pair we compile, per stage, the programs printed by name. 4 before step
/// (p) in both builds (femtovg filter_image and its nodiscard form, es100
/// deadloop and loopcarry); 0 since select.rs merge_exits.
#[test]
fn back_to_back_loop_exits_are_counted() {
    let mut pairs_found: Vec<String> = Vec::new();
    let mut total = 0usize;
    for (vs, fs) in real_pairs() {
        let mut mem = vec![0u8; 16 << 20];
        let a = Arena::new(&mut mem);
        let (Ok(fsl), Ok(vsl)) = (compile(&a, sema::Stage::Fragment, &fs), compile(&a, sema::Stage::Vertex, &vs))
        else {
            continue;
        };
        let Ok(prog) = compile_program(&vsl.module, &fsl.module, Prim::Triangles, TARGET, &a) else { continue };
        for (st, c) in [("fs", &prog.fs.code), ("vs", &prog.vs.code), ("cs", &prog.cs.code)] {
            let mut n = 0;
            for &(edge, head) in c.back_edges {
                let mut fwd: Vec<(u32, u32)> =
                    c.fwd_branches.iter().copied().filter(|&(b, _)| b >= head && b < edge).collect();
                fwd.sort_unstable();
                n += fwd.windows(2).filter(|w| w[0].1 == w[1].1).count();
            }
            if n > 0 {
                total += n;
                pairs_found.push(format!("{vs} + {fs} {st}: {n}"));
            }
        }
    }
    println!("back-to-back loop exits: {total}");
    for p in &pairs_found {
        println!("  two exits in a row: {p}");
    }
    assert_eq!(total, 0, "two exits in a row, not merged: {pairs_found:#?}");
}

/// How often sched.rs hoist's 32-word window stops a move short of an
/// earlier legal word (Scheduled::window_cuts), against the moves hoist
/// makes (Scheduled::hoists), over every pair we compile, per stage. The
/// allocator's register choice (alloc.rs, step (l)) plans a move back to
/// `earliest` by operation index; a cut is where the window, not a
/// register, ended it. Measured 2026-09-29 (step (o), 68a83016): 292 cuts in
/// the ES 3 build (fs 73, vs 88, cs 131) in 19 programs, printed by name
/// (the Qt particle vertex shaders, es100 varyings15, femtovg's conic
/// gradients 4-5 each); the window removed, they cost one word in all
/// (es300 divnest 131 -> 130, ES 3 only; no executed run moves) and 37%
/// more test time, so the window stays. Pinned: a change to hoist or to
/// what reaches it moves these.
#[test]
fn the_hoist_windows_cuts_are_counted() {
    let es2 = cfg!(feature = "report-es2");
    let (mut hoists, mut cuts) = ([0u32; 3], [0u32; 3]);
    let mut cut_programs: Vec<String> = Vec::new();
    for (vs, fs) in real_pairs() {
        let mut mem = vec![0u8; 16 << 20];
        let a = Arena::new(&mut mem);
        let (Ok(fsl), Ok(vsl)) = (compile(&a, sema::Stage::Fragment, &fs), compile(&a, sema::Stage::Vertex, &vs))
        else {
            continue;
        };
        let Ok(prog) = compile_program(&vsl.module, &fsl.module, Prim::Triangles, TARGET, &a) else { continue };
        for (k, (st, c)) in
            [("fs", &prog.fs.code), ("vs", &prog.vs.code), ("cs", &prog.cs.code)].into_iter().enumerate()
        {
            hoists[k] += c.hoists;
            cuts[k] += c.window_cuts;
            if c.window_cuts > 0 {
                cut_programs.push(format!("{vs} + {fs} {st}: {} of {} hoists", c.window_cuts, c.hoists));
            }
        }
    }
    println!("hoists (fs, vs, cs): {hoists:?}; cut by the window: {cuts:?}");
    for p in &cut_programs {
        println!("  window cut: {p}");
    }
    assert_eq!(
        (hoists, cuts),
        if es2 { ([1313, 3520, 1493], [66, 29, 67]) } else { ([1849, 8557, 3756], [73, 88, 131]) },
        "the hoist counts moved"
    );
}

/// Executed fragment-shader instructions per femtovg pair, ours and
/// Mesa's, in the interpreter: what a fragment costs, which the program
/// length does not say once a branch skips code. Two scenarios set the
/// uniforms that choose femtovg's paths (the rest are the pair's usual
/// random draw): "cluster" is the Slint cluster's trace (scissorRadius 0,
/// texType 0; records/pi4/glcount_c510dbae_trace_frame300.txt), "rounded"
/// takes scissorMask's rounded-rectangle arm (scissorRadius 1). Every run
/// must agree with the IR, or its count means nothing; a pair that does not
/// is a failure here, not a skipped row. Ours are pinned; a change is a
/// deliberate re-pin with the reason.
#[test]
fn executed_instruction_counts_are_pinned() {
    let scenarios: [(&str, [ScenarioValue; 2]); 2] = [
        ("cluster", [("frag", 12, 3, 0f32.to_bits()), ("frag", 10, 2, 0f32.to_bits())]),
        ("rounded", [("frag", 12, 3, 1f32.to_bits()), ("frag", 10, 2, 0f32.to_bits())]),
    ];
    let mut rows: Vec<(String, usize, usize)> = Vec::new();
    let mut not_agreeing: Vec<String> = Vec::new();
    for (scenario, set) in scenarios {
        SCENARIO.with(|s| *s.borrow_mut() = set.to_vec());
        for m in parse().into_iter().filter(|m| m.fs_path.starts_with("femtovg/") && !m.ours) {
            let name = format!("{} + {}", m.vs_path, m.fs_path);
            let Ok(o) = ours_pair(&m.vs_path, &m.fs_path) else {
                not_agreeing.push(format!("{scenario} {name}: ours does not compile"));
                continue;
            };
            let mut n = [0usize; 2];
            for (k, p) in [&o, &m].into_iter().enumerate() {
                let (v, _) = run_pair(p, seed_of(&name));
                let why = match v {
                    Verdict::Agree => None,
                    Verdict::Disagree(w) => Some(format!("disagrees: {w}")),
                    Verdict::Unmodelled(w) => Some(format!("unmodelled: {w}")),
                    Verdict::NotTried(w) => Some(format!("not tried: {w}")),
                };
                if let Some(w) = why {
                    not_agreeing.push(format!("{scenario} {name} ({}): {w}", who(p)));
                }
                n[k] = RECORD.with(|r| r.borrow().fs_instructions);
            }
            let f = m.fs_path.trim_start_matches("femtovg/");
            rows.push((format!("{scenario} {f}"), n[0], n[1]));
        }
    }
    SCENARIO.with(|s| s.borrow_mut().clear());
    rows.sort();
    let (mut ours_t, mut mesa_t) = (0usize, 0usize);
    for (f, o, m) in &rows {
        println!("  executed {f}: fs ours {o}, Mesa {m} ({:.2}x)", *o as f64 / *m as f64);
        ours_t += o;
        mesa_t += m;
    }
    println!("executed fs instructions, {} runs: ours {ours_t}, Mesa {mesa_t}", rows.len());
    assert_eq!(not_agreeing, Vec::<String>::new(), "every counted run agrees with the IR");
    assert_eq!(rows.len(), 2 * FEMTOVG_FS_COUNTS.len(), "every femtovg pair, in both scenarios");
    let got: Vec<(&str, usize)> = rows.iter().map(|(f, o, _)| (f.as_str(), *o)).collect();
    assert_eq!(got, EXECUTED_FS_COUNTS, "a femtovg fragment's executed count moved: re-pin with the reason");
}

/// Our executed fragment instructions per femtovg pair and scenario
/// ([`executed_instruction_counts_are_pinned`]). Measured 2026-09-26 on
/// a0b4b403, both builds (ours 16962 over the 58 runs; cluster
/// fill_image.nodiscard 104); since the uniform branch round scissorMask's
/// arms (60b90e4a: 16516; 85); after one word per varying (e3f7e123:
/// 16360; 82); after the scheduler's hoist (edb7728a: 15182; 80); since
/// round-robin loads and x + 0.0 (7a7c2b89: 15168; 79); after the arms'
/// common work before the branch (690d3c9a: 15062; 79); after plain
/// ldunif into r5 (2f4e8ea3: 14922; 76); after a Copy's source written
/// into its destination register (30922372: 14898; 75); since abs() from a
/// select's result (777064fa: 14874; 75); after no r5 right after the
/// varyings (e417b782: 14836; 75); after the list scheduler's order
/// (order.rs, 26eca726: 14494; 74); after a counted loop leaves on its
/// uniform break (unroll.rs, 17347264: filter_image 2289 -> 156, Mesa 65;
/// 5962; 74); after a kept loop's uniform break guard is a branch and its
/// pass-start guard is gone (prepare.rs uniform_locals,
/// clear_at_pass_start, 54759e30: filter_image 156 -> 143; 5910; 74);
/// since a uniform `if` with a look-up in an arm is a branch (prepare.rs
/// arms_cost, select.rs last_thrsw, alloc.rs holes: filter_image 143 ->
/// 110, nodiscard 132 -> 99; 5778; 74); since a kept loop's uniform
/// `break` is a branch to its exit (prepare.rs uniform_break, SOp::LoopBreak:
/// filter_image 110 -> 74, nodiscard 99 -> 63: the pass that breaks jumps
/// out, the guards round the rest of a pass are gone and the back edge
/// tests the counter alone) (5634; 74); since a look-up lane that only
/// FMUL, FADD, FSUB, FMIN or FMAX read is read through their own L/H input
/// unpack with no FMOV (select.rs folded_unpacks, step (f): the glyph
/// programs 1 to 5 fewer, filter_image 74 -> 72, nodiscard 63 -> 61; no
/// row rises) (5564; 74); since a branch takes the words before it that
/// may run in its delay slots (sched.rs branch_back, Mesa's emit_branch,
/// qpu_schedule.c:2445-2502; step (g), with the counter's one `sub c, c, 1`
/// and hoist's flag rule): every rounded run 3 or 4 fewer (the uniform
/// `if`s' slots), the cluster's conic ones 1 fewer, the rest and
/// filter_image 72 (no pass runs) unchanged; no row rises, as below
/// (5486; 72); since a value the scheduler may hoist takes the free
/// register that lets it move furthest (alloc.rs, step (l)): 23 cluster and
/// 25 rounded runs fewer, 1 to 8 each (filter_image 72 -> 68: its prologue),
/// none longer (5311; 68); since a MOV into the register file puts its
/// source there (alloc.rs), a loop's uniforms defined before it are loaded
/// before its head (select.rs) and a conditional branch's flag push ranks
/// above work its delay slots can take (order.rs; step (m)): filter_image
/// 68 -> 62 (its prologue), fill_color 45 -> 44, none longer
/// (5271; 62); since the loop's test and a break take one branch
/// (select.rs merge_exits, step (p)): filter_image 62 -> 58 and nodiscard
/// 53 -> 49 (its path out at once), none longer (5255; 58). Mesa's
/// for the same runs, printed by the test:
/// cluster fill_image.nodiscard 68, rounded 78; filter_image 65 (ours 2574
/// on a0b4b403, when the counted loop always ran its 25 passes: the
/// unroller gave no exit flag for a constant head test; since unroll.rs's
/// early exit it leaves at the break, as Mesa does). Totals ours 16962 on
/// a0b4b403, Mesa 5216 over the 58 runs.
const EXECUTED_FS_COUNTS: &[(&str, usize)] = &[
    ("cluster fill_color.frag", 44),
    ("cluster fill_color.glyph.frag", 68),
    ("cluster fill_color.glyph.nodiscard.frag", 59),
    ("cluster fill_color.nodiscard.frag", 41),
    ("cluster fill_gradient.frag", 93),
    ("cluster fill_gradient.glyph.frag", 118),
    ("cluster fill_gradient.glyph.nodiscard.frag", 110),
    ("cluster fill_gradient.nodiscard.frag", 92),
    ("cluster fill_gradient_conic.frag", 118),
    ("cluster fill_gradient_conic.glyph.frag", 144),
    ("cluster fill_gradient_conic.glyph.nodiscard.frag", 136),
    ("cluster fill_gradient_conic.nodiscard.frag", 117),
    ("cluster fill_image.frag", 71),
    ("cluster fill_image.glyph.frag", 97),
    ("cluster fill_image.glyph.nodiscard.frag", 89),
    ("cluster fill_image.nodiscard.frag", 70),
    ("cluster fill_image_gradient.frag", 86),
    ("cluster fill_image_gradient.glyph.frag", 111),
    ("cluster fill_image_gradient.glyph.nodiscard.frag", 104),
    ("cluster fill_image_gradient.nodiscard.frag", 85),
    ("cluster fill_image_gradient_conic.frag", 110),
    ("cluster fill_image_gradient_conic.glyph.frag", 135),
    ("cluster fill_image_gradient_conic.glyph.nodiscard.frag", 128),
    ("cluster fill_image_gradient_conic.nodiscard.frag", 109),
    ("cluster filter_image.frag", 58),
    ("cluster filter_image.nodiscard.frag", 49),
    ("cluster stencil.frag", 17),
    ("cluster stencil.nodiscard.frag", 8),
    ("cluster texture_copy_unclipped.frag", 26),
    ("rounded fill_color.frag", 56),
    ("rounded fill_color.glyph.frag", 80),
    ("rounded fill_color.glyph.nodiscard.frag", 70),
    ("rounded fill_color.nodiscard.frag", 53),
    ("rounded fill_gradient.frag", 105),
    ("rounded fill_gradient.glyph.frag", 130),
    ("rounded fill_gradient.glyph.nodiscard.frag", 122),
    ("rounded fill_gradient.nodiscard.frag", 104),
    ("rounded fill_gradient_conic.frag", 130),
    ("rounded fill_gradient_conic.glyph.frag", 156),
    ("rounded fill_gradient_conic.glyph.nodiscard.frag", 148),
    ("rounded fill_gradient_conic.nodiscard.frag", 129),
    ("rounded fill_image.frag", 82),
    ("rounded fill_image.glyph.frag", 107),
    ("rounded fill_image.glyph.nodiscard.frag", 99),
    ("rounded fill_image.nodiscard.frag", 81),
    ("rounded fill_image_gradient.frag", 97),
    ("rounded fill_image_gradient.glyph.frag", 121),
    ("rounded fill_image_gradient.glyph.nodiscard.frag", 114),
    ("rounded fill_image_gradient.nodiscard.frag", 96),
    ("rounded fill_image_gradient_conic.frag", 121),
    ("rounded fill_image_gradient_conic.glyph.frag", 145),
    ("rounded fill_image_gradient_conic.glyph.nodiscard.frag", 138),
    ("rounded fill_image_gradient_conic.nodiscard.frag", 120),
    ("rounded filter_image.frag", 58),
    ("rounded filter_image.nodiscard.frag", 49),
    ("rounded stencil.frag", 17),
    ("rounded stencil.nodiscard.frag", 8),
    ("rounded texture_copy_unclipped.frag", 26),
];

/// Our femtovg fragment program lengths (instructions). Measured 2026-09-26
/// on 17c5645a (fill_image 151, fill_color 98, totals [8339, 4072, 3128] /
/// ES2 [7106, 3066, 2486]); after prepare's exact algebraic rewrites
/// (1b07d92d: fill_image 130, fill_color 78, [7635, 3995, 3058]); since
/// common-subexpression reuse (2a728298: fill_image 120, fill_color 75,
/// [7402, 3913, 3058]); since one flag push per condition and no
/// same-register moves (fb0ca9cb: fill_image 115, fill_color 74,
/// [7024, 3901, 3058]); after abs() as the reader's input unpack and
/// max(x, x) as x (7df939df: fill_image 114, fill_color 73,
/// [6895, 3901, 3058]); after the look-up's switch as the last pair and a
/// THRSW merged back onto placed words (a0b4b403: fill_image 107,
/// fill_color 70, [6524, 3898, 3058]); after the uniform branch round
/// scissorMask's arms (60b90e4a: fill_image 124, fill_color 88,
/// [7017, 3898, 3058]; a program is longer by the branch and the work both
/// arms repeat, and executes less, EXECUTED_FS_COUNTS); after one word per
/// varying (ldvary moved back, an ldvary's accumulator round-robin) and
/// textureSize (2c741f3e: fill_image 121, fill_color 85, [7252, 3926,
/// 3082]); after operations hoisted into earlier free units (edb7728a:
/// fill_image 118, fill_color 80, [6884, 3860, 2991]); after uniform loads
/// take accumulators round-robin and x + 0.0 is x (7a7c2b89: fill_image
/// 117, fill_color 80, [6872, 3561, 2884]); after the work both arms of a
/// uniform branch share goes before it and a register-file load is
/// round-robin (690d3c9a: fill_image 108, fill_color 73, [6600, 3551,
/// 2884]); after a fragment shader's short loads take r5 as a plain
/// ldunif (2f4e8ea3: fill_image 105, fill_color 73, [6502, 3551, 2884]);
/// after a Copy that ends its source writes it into the destination's
/// register (30922372: fill_image 104, fill_color 72, [6477, 3551,
/// 2884]); after abs() folds from a select's result too (777064fa:
/// fill_image 104, fill_color 72, [6465, 3551, 2884]); after no r5 for a
/// load right after the varyings (e417b782: fill_image 104, fill_color 71,
/// [6441, 3551, 2884]); after the list scheduler's order, kept where it
/// packs shorter (order.rs, 26eca726: fill_image 102, fill_color 71,
/// [6294, 3522, 2804]); since a counted loop leaves on its uniform break
/// (unroll.rs: filter_image 153 -> 156, the flag test on its back edge),
/// then its uniform break guard a branch and its pass-start guard gone
/// (prepare.rs: filter_image 156 -> 143), then a uniform `if` with a
/// look-up in an arm a branch (filter_image 143 -> 160, nodiscard 132 ->
/// 149: the look-ups' arm, its merges and a new last-switch pair after
/// it; executed 143 -> 110), then a kept loop's uniform `break` a branch
/// to its exit (prepare.rs uniform_break: filter_image 160 -> 119,
/// nodiscard 149 -> 108; executed 110 -> 74 / 99 -> 63), then a pass's two
/// look-ups under one thread switch (select.rs issue_order and
/// Sel::flush_tmu: filter_image 119 -> 117, nodiscard 108 -> 106; the
/// executed pins do not move, their runs take no pass), then a look-up
/// lane that only FMUL, FADD, FSUB, FMIN or FMAX read is read through their
/// own L/H input unpack, with no FMOV (select.rs folded_unpacks, step (f):
/// filter_image 117 -> 106, nodiscard 106 -> 95 (executed with a blur,
/// chain.rs: 63 + 54 -> 61 + 45 a pass), the glyph programs 1 to 5 fewer;
/// fill_image_gradient and its conic form, each with and without discard,
/// 1 MORE: in the gradient arm the uniform FMIN reads now takes the
/// register of the RSQRT -> RECIP chain, so the FMIN no longer fills the
/// SFU gap and a NOP does; no executed pin of theirs moves), then a branch
/// takes the words before it that may run in its delay slots (sched.rs
/// branch_back, Mesa's emit_branch, qpu_schedule.c:2445-2502; step (g), with
/// the counter's one `sub c, c, 1` and hoist's flag rule): every program 3
/// fewer (work in its uniform branches' delay slots), the conic ones 3 or
/// 4, filter_image 106 -> 102 and nodiscard
/// 95 -> 91 (the loop's branches too; executed with a blur, chain.rs: 61 +
/// 45 -> 61 + 41 a pass, Mesa's 41), then a value the scheduler may hoist
/// takes the free register that lets it move furthest (alloc.rs, step (l);
/// it undoes the gradient arm's +1 above): every program but four 1 to 8
/// fewer (fill_image_gradient 113 -> 108, fill_image 99 -> 93, filter_image
/// 102 -> 98; executed 61 + 41 -> 59 + 41), none longer, then step (m)
/// (alloc.rs MOV-source class, select.rs loop-invariant uniforms, order.rs
/// the back edge's push): filter_image 98 -> 91 and nodiscard 89 -> 82
/// (executed with a blur, chain.rs: 53 + 40 a pass, Mesa's 57 + 41), the
/// glyph programs 1 fewer, fill_color 1 fewer, none longer, then step (p)
/// (select.rs merge_exits: the loop's test and the break in one branch):
/// filter_image 91 -> 87 and nodiscard 82 -> 78 (executed with a blur:
/// 49 + 36 a pass), none longer, as below.
/// Mesa's
/// for the same programs: fill_image 94, fill_color 67.
const FEMTOVG_FS_COUNTS: &[(&str, usize)] = &[
    ("fill_color.frag", 67),
    ("fill_color.glyph.frag", 91),
    ("fill_color.glyph.nodiscard.frag", 81),
    ("fill_color.nodiscard.frag", 64),
    ("fill_gradient.frag", 116),
    ("fill_gradient.glyph.frag", 141),
    ("fill_gradient.glyph.nodiscard.frag", 133),
    ("fill_gradient.nodiscard.frag", 115),
    ("fill_gradient_conic.frag", 141),
    ("fill_gradient_conic.glyph.frag", 167),
    ("fill_gradient_conic.glyph.nodiscard.frag", 159),
    ("fill_gradient_conic.nodiscard.frag", 140),
    ("fill_image.frag", 93),
    ("fill_image.glyph.frag", 118),
    ("fill_image.glyph.nodiscard.frag", 110),
    ("fill_image.nodiscard.frag", 92),
    ("fill_image_gradient.frag", 108),
    ("fill_image_gradient.glyph.frag", 132),
    ("fill_image_gradient.glyph.nodiscard.frag", 125),
    ("fill_image_gradient.nodiscard.frag", 107),
    ("fill_image_gradient_conic.frag", 132),
    ("fill_image_gradient_conic.glyph.frag", 156),
    ("fill_image_gradient_conic.glyph.nodiscard.frag", 149),
    ("fill_image_gradient_conic.nodiscard.frag", 131),
    ("filter_image.frag", 87),
    ("filter_image.nodiscard.frag", 78),
    ("stencil.frag", 17),
    ("stencil.nodiscard.frag", 8),
    ("texture_copy_unclipped.frag", 26),
];
/// (pairs, [fs, vs, cs] instruction totals), both builds.
/// ES3 build: 103 pairs since textureSize (qt3d/lightmapdilate_300 joins;
/// the totals grew by 419, 28 and 24 to [7252, 3926, 3082]; the femtovg
/// rows did not move), then the scheduler's hoist (edb7728a: [6884, 3860,
/// 2991] / ES2 [5576, 2915, 2360]), then round-robin loads and x + 0.0
/// (7a7c2b89: [6872, 3561, 2884] / ES2 [5568, 2653, 2276]), then the arms'
/// common work before the branch (690d3c9a: [6600, 3551, 2884] / ES2
/// [5297, 2653, 2276]), then plain ldunif into r5 (2f4e8ea3: [6502, 3551,
/// 2884] / ES2 [5212, 2653, 2276]), then the Copy coalescing (30922372:
/// [6477, 3551, 2884] / ES2 [5186, 2653, 2276]), then abs() from a select
/// (777064fa: [6465, 3551, 2884] / ES2 [5174, 2653, 2276]), then no r5
/// right after the varyings (e417b782: [6441, 3551, 2884] / ES2 [5151,
/// 2653, 2276]), then the list scheduler's order (order.rs, 26eca726:
/// [6294, 3522, 2804] / ES2 [5059, 2637, 2204]), then a counted loop
/// leaving on its uniform break (unroll.rs: +14 / +14, the flag test), then
/// a kept loop's uniform break guard a branch and its pass-start guard gone
/// (prepare.rs: [6267, 3522, 2804] / ES2 [5033, 2637, 2204]; per program,
/// fs words / executed on the interpreter: filter_image 156 -> 143, es300
/// dynidx 130 -> 134 / 481 -> 382, uniloop 53 -> 48 / 79 -> 69, deadloop
/// 52 -> 45 / 196 -> 161, loopcarry 53 -> 46 / 53 -> 46), then a uniform
/// `if` with a look-up in an arm a branch ([6340, 3522, 2804] / ES2 [5067,
/// 2637, 2204]; per program, fs words / executed: filter_image 143 -> 160
/// / 143 -> 110, nodiscard 132 -> 149 / 132 -> 99, and, ES 3 only, qt3d
/// lightmapuvraster_300 129 -> 168, which now agrees (executed 112; it was
/// unmodelled on a NaN in an untaken arm); no other program moves), then a
/// kept loop's uniform `break` a branch to its exit (prepare.rs
/// uniform_break, SOp::LoopBreak: [6211, 3522, 2804] / ES2 [4967, 2637,
/// 2204]; per program, fs words: filter_image 160 -> 119, nodiscard 149 ->
/// 108, es100 deadloop 45 -> 36, loopcarry 46 -> 37, and, ES 3 only, es300
/// dynidx 134 -> 112, uniloop 48 -> 41; each a loop whose bound or break is
/// uniform, and all agree with the IR; measured against origin/main
/// 6ced5426 per pair; no other program moves), then independent look-ups
/// of one straight-line stretch under one thread switch (select.rs
/// issue_order and Sel::flush_tmu: [6136, 3522, 2804] / ES2 [4961, 2637,
/// 2204]; per program, fs words: filter_image 119 -> 117, nodiscard 108 ->
/// 106 (a pass's two look-ups), gl_test_es100 texture2 28 -> 26 (its two
/// samplers' look-ups), and, ES 3 only, qt3d lightmapdilate_300 386 -> 317
/// (its nine look-ups, the per-lane `if`s round them flattened, two of 2
/// words to a switch); all agree with the IR;
/// measured against 2951fef9 per pair; no other program moves), then a
/// look-up lane that only FMUL, FADD, FSUB, FMIN or FMAX read is read through
/// their own L/H input unpack, with no FMOV (select.rs folded_unpacks,
/// step (f): [6058, 3522, 2804] / ES2 [4896, 2637, 2204]; per program, fs
/// words, beside the femtovg ones in FEMTOVG_FS_COUNTS: es100 hdrscale
/// 18 -> 14, textured 25 -> 21, gl_test_es100 texture2 26 -> 20, and, ES 3
/// only, es300 hdrsample 21 -> 17, idxtex 32 -> 31, qt3d
/// lightmapuvraster_300 168 -> 161, oitcomposite_weightedblended_300 23 ->
/// 22; all agree with the IR; measured against 6168fbd3 per pair; no
/// other program moves), then a lane every reader of which takes the L/H
/// unpack by encode.rs's table, FCMP, VFPACK, FFLOOR/FCEIL and FMOV now
/// included, has no FMOV (select.rs fold_unpacks, which replaced
/// folded_unpacks, step (h): [6048, 3522, 2804] / ES2 [4892, 2637, 2204];
/// per program, fs words: gl_test_es100 texture 13 -> 11 and compact 13 ->
/// 11 (the colour written straight from the look-up, VFPACK reading .l /
/// .h), and, ES 3 only, es300 cube 27 -> 25, qt3d skyboxcube_300 22 -> 20,
/// skybox_300 29 -> 28, cubeMapToHemisphere_300 31 -> 30; all agree with
/// the IR; measured against 8472118d per pair; no other program moves, no
/// femtovg program either: each of their remaining lane FMOVs feeds a
/// select), then a branch takes the words before it that may run in its
/// delay slots (sched.rs branch_back, Mesa's emit_branch, qpu_schedule.c:
/// 2445-2502), a counted loop counts with one `sub c, c, 1` (select.rs
/// SOp::LoopEnd: the flag push reads c after the Copy) and a flag push is
/// hoisted where no flag use is in between (sched.rs hoist; step (g):
/// [5928, 3522, 2804] / ES2 [4794, 2637, 2204]; per program, fs words,
/// beside the femtovg ones in FEMTOVG_FS_COUNTS (3 or 4 fewer each; the
/// fvgtest cover and discard programs 3 fewer): es100 loopcarry 37 -> 32,
/// deadloop 36 -> 32, and, ES 3 only, qt3d lightmapuvraster_300 161 -> 149,
/// es300 divnest 163 -> 160, dynidx 112 -> 109, uniloop 41 -> 38, divloop
/// 57 -> 56; all agree with the IR; measured against defeaa73 per pair, 35
/// / 30 programs fall; no program rises; vertex shaders have no branch
/// here and do not move); since a value the scheduler may hoist takes the
/// free register that lets it move furthest (alloc.rs, step (l)):
/// [5723, 3287, 2623] / ES2 [4632, 2458, 2055], every stage shorter (idiv
/// 210 -> 201; vertex shaders' uniform loads and SFU results move into
/// earlier words); measured against 5fff6d69 per pair, no program rises;
/// step (m): [5653, 3287, 2623] / ES2 [4599, 2458, 2055] (filter_image -7,
/// qt3d lightmapdilate_300 302 -> 279, es300 divloop 55 -> 52, divnest
/// 135 -> 131, es100 databreak 65 -> 63; per pair against 435e6a94, no
/// program rises); step (p), two loop exits in one branch: [5637, 3287,
/// 2623] / ES2 [4583, 2458, 2055] (filter_image both forms, es100 deadloop
/// and loopcarry 4 fewer each; nothing else moves).
const OURS_TOTALS: (usize, [usize; 3]) = (103, [5637, 3287, 2623]);
const OURS_TOTALS_ES2: (usize, [usize; 3]) = (81, [4583, 2458, 2055]);
