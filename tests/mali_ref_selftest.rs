// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Self-test of `tests/mali_ref/mod.rs`, the shared byte-compare of a Mali
//! program against an expect.json reference. Two references the compiler
//! already matches (`tools/mali/testdata/cases.txt`: `tri` and
//! `tri_varying`, every region `equal`), a changed shader that must differ,
//! one refused shape, the uniform words of each GL type, and the struct
//! member values of the `deqp_fclane2_*` references against gl_ref.py's
//! (python3, a `GAP` without it) and against the words of their FAU.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_ref_selftest
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

#[test]
fn tri_matches_its_reference() {
    mali_ref::assert_matches("tri");
}

#[test]
fn tri_varying_matches_its_reference() {
    mali_ref::assert_matches("tri_varying");
}

#[test]
fn a_changed_shader_is_reported_with_a_word_diff() {
    let r = mali_ref::Reference::load("tri");
    let fs = r.fs.replace("vec4(1.0, 0.0, 0.0, 1.0)", "vec4(0.0, 1.0, 0.0, 1.0)");
    assert_ne!(fs, r.fs);
    let pair = mali_ref::compile_pair(&r.vs, &fs).expect("the changed pair compiles");
    let d = mali_ref::differences(&r, &pair);
    assert!(!d.is_empty(), "a different colour must differ from the reference");
    assert!(d.iter().any(|m| m.starts_with("fs.fragment")), "{d:?}");
    assert!(d.iter().all(|m| !m.starts_with("vs.")), "{d:?}");
}

/// A vertex shader that writes gl_PointSize is compared by its position
/// program without the store (the reference's draw is of triangles), with
/// the secondary offset counted from that program and Mesa's FAU table.
#[test]
fn a_point_size_shader_is_compared_by_its_no_psiz_program() {
    let r = mali_ref::Reference::load("deqp_varylink_va_multi3");
    assert!(r.vs.contains("gl_PointSize"));
    let pair = mali_ref::compile_pair(&r.vs, &r.fs).expect("the pair compiles");
    assert!(pair.vs.no_psiz.is_some_and(|off| off > 0), "a no_psiz copy after the position program");
    let d = mali_ref::differences(&r, &pair);
    assert!(d.is_empty(), "{}", d.join("\n"));
}

#[test]
fn a_refused_shape_stays_refused() {
    /* an int uniform's arithmetic (its conversion alone compiles,
     * deqp_uniarray_struct_member_vs) */
    let vs = "attribute vec2 pos; uniform int n; void main(){ gl_Position = vec4(pos, float(n + 1), 1.0); }";
    let fs = "precision mediump float; void main(){ gl_FragColor = vec4(1.0); }";
    mali_ref::assert_refused(vs, fs, "a uniform other than a float, vec2, vec3, vec4 or mat4");
}

#[test]
#[should_panic(expected = "but the pair compiles")]
fn a_pair_that_compiles_is_not_refused() {
    let r = mali_ref::Reference::load("tri");
    mali_ref::assert_refused(&r.vs, &r.fs, "anything");
}

/// The word `gl_ref.py --uniform-values` leaves in a reference follows the
/// uniform's GL type, as Mesa uploads it (measured: the ivec3 of
/// `deqp_intval_reslib_mixed_fs` is `00000010 00000011 00000012`, the bvec3
/// of `deqp_intval_reslib_bvec_fs` is `ffffffff 00000000 ffffffff`).
#[test]
fn a_uniform_word_follows_its_gl_type() {
    use yakogl::gl::consts as gl;
    let ty = |t: u32| u16::try_from(t).expect("a GL type fits 16 bits");
    let names = ["u_f".to_string(), "u_i".to_string(), "u_b".to_string()];
    let rows = [
        ("u_f".to_string(), 0, 2, ty(gl::GL_FLOAT_VEC2)),
        ("u_i".to_string(), 2, 3, ty(gl::GL_INT_VEC3)),
        ("u_b".to_string(), 5, 3, ty(gl::GL_BOOL_VEC3)),
    ];
    let words: Vec<u32> = (0..8).map(|d| mali_ref::uniform_value(&names, &rows, d)).collect();
    assert_eq!(words, [16f32.to_bits(), 17f32.to_bits(), 32, 33, 34, u32::MAX, 0, u32::MAX]);
}

/// The `deqp_fclane2_*` references, drawn with their struct members set
/// (`mali_ref::STRUCT_VALUES`).
const STRUCT_SHAPES: &[&str] = &[
    "deqp_fclane2_struct_basic_float",
    "deqp_fclane2_struct_basic_int",
    "deqp_fclane2_struct_basic_bool",
    "deqp_fclane2_struct_arrmember_float",
    "deqp_fclane2_struct_array_float",
    "deqp_fclane2_struct_nested_float",
];

/// The fragment shader's uniform rows as the front end reflects them
/// (name, first dword, dwords, GL type), the rows the code generator's
/// record carries: the code generator refuses these shapes, so this asks
/// the front end (`reflect::interface`).
fn fragment_rows(fs: &str) -> Vec<(String, u16, u16, u16)> {
    use yakogl::shaderc::online::{arena::Arena, reflect, sema::Stage};
    let mut buf = vec![0u8; 4 << 20];
    let a = Arena::new(&mut buf);
    let i = reflect::interface(&a, Stage::Fragment, fs).unwrap_or_else(|e| panic!("the front end reads it: {e:?}"));
    i.reflection
        .uniforms
        .iter()
        .filter(|u| u.dword_off != reflect::NO_BLOCK)
        .map(|u| (u.name.to_string(), u.dword_off, u.dwords, u.gl_type))
        .collect()
}

/// Every block dword of the fragment rows, in row order, as
/// `mali_ref::uniform_value` gives it for the reference's draw.
fn member_words(r: &mali_ref::Reference) -> Vec<u32> {
    let names: Vec<String> = mali_ref::uniform_leaves(&r.vs, &r.fs).into_iter().map(|l| l.0).collect();
    let rows = fragment_rows(&r.fs);
    assert!(rows.iter().all(|r| r.0.contains('.')), "{}: struct-member rows only: {rows:?}", r.name);
    rows.iter().flat_map(|row| row.1..row.1 + row.2).map(|d| mali_ref::uniform_value(&names, &rows, d)).collect()
}

/// The fragment program's FAU words of a reference's expect.json.
fn fragment_fau(r: &mali_ref::Reference) -> Vec<u32> {
    let fs = &r.json[r.json.find("\"fs.fragment\"").expect("an fs.fragment region")..];
    let fau = &fs[fs.find("\"fau\": [").expect("a fau list") + 8..];
    fau[..fau.find(']').expect("the end of the fau list")]
        .split(',')
        .map(|w| w.trim().trim_matches('"'))
        .filter(|w| !w.is_empty())
        .map(|w| u32::from_str_radix(w, 16).expect("a hex FAU word"))
        .collect()
}

/// The struct-member values of every `deqp_fclane2_*` shape are the words
/// Mesa uploaded in its draw: the member words, in declaration order, are
/// one run of the reference's fragment FAU (16.0, 32.0, ... as fp32 for a
/// mediump float member, 16, 32, ... for an int). Of `_basic_bool` Mesa
/// pushes the same five words in another order (its program reads `m0`
/// negated, `.not`), so there the run holds the same words. The references
/// are in `STRUCT_VALUES`, so `differences` compares with these values.
#[test]
fn struct_member_values_are_mesas_fau_words() {
    let sorted = |w: &[u32]| {
        let mut w = w.to_vec();
        w.sort_unstable();
        w
    };
    for name in STRUCT_SHAPES {
        assert!(mali_ref::STRUCT_VALUES.contains(name), "{name}: not in STRUCT_VALUES");
        let r = mali_ref::Reference::load(name);
        let ours = member_words(&r);
        let fau = fragment_fau(&r);
        let found = fau.windows(ours.len()).any(|w| {
            if name.ends_with("_bool") {
                sorted(w) == sorted(&ours)
            } else {
                w == ours.as_slice()
            }
        });
        assert!(found, "{name}: the member words {ours:08x?} are not a run of the reference's FAU {fau:08x?}");
    }
}

/// What `gl_ref.uniform_settings()` (gl_ref.py's `--uniform-values`) sets:
/// one line per uniform, its GL name, type and values.
const GL_REF_SETTINGS: &str = r#"
import sys
sys.path.insert(0, sys.argv[1])
import gl_ref
vs, fs = open(sys.argv[2]).read(), open(sys.argv[3]).read()
for leaf, ty, floats, values in gl_ref.uniform_settings(vs, fs):
    print(leaf, ty, " ".join(str(v) for v in values))
"#;

/// The member values the module computes are gl_ref.py's: for each
/// `deqp_fclane2_*` shape, `gl_ref.uniform_settings()` gives the same GL
/// names and types in the same order as `mali_ref::uniform_leaves`, and its
/// values, as Mesa uploads them (fp32 bits, the integer, a true bool all
/// ones), are the words `mali_ref::uniform_value` gives the front end's
/// rows. Without python3 this prints `GAP` and checks nothing (a gap, not
/// a pass); `struct_member_values_are_mesas_fau_words` checks the values
/// against the references' words without it.
#[test]
fn struct_member_values_match_gl_ref() {
    let root = env!("CARGO_MANIFEST_DIR");
    for name in STRUCT_SHAPES {
        let p = format!("{root}/tools/mali/testdata/{name}");
        let tools = format!("{root}/tools/mali");
        let (vert, frag) = (format!("{p}.vert"), format!("{p}.frag"));
        let out = match std::process::Command::new("python3")
            .args(["-I", "-c", GL_REF_SETTINGS, &tools, &vert, &frag])
            .output()
        {
            Ok(o) => o,
            Err(e) => {
                println!("GAP struct_member_values_match_gl_ref: python3 does not run ({e})");
                return;
            }
        };
        assert!(out.status.success(), "{name}: gl_ref.py: {}", String::from_utf8_lossy(&out.stderr));
        let theirs: Vec<(String, String, Vec<f64>)> = String::from_utf8_lossy(&out.stdout)
            .lines()
            .map(|l| {
                let w: Vec<&str> = l.split_whitespace().collect();
                (w[0].to_string(), w[1].to_string(), w[2..].iter().map(|v| v.parse().expect("a value")).collect())
            })
            .collect();
        let r = mali_ref::Reference::load(name);
        let names: Vec<(String, String)> = theirs.iter().map(|t| (t.0.clone(), t.1.clone())).collect();
        assert_eq!(mali_ref::uniform_leaves(&r.vs, &r.fs), names, "{name}: the members and their order");
        let want: Vec<u32> = theirs
            .iter()
            .flat_map(|(_, ty, values)| {
                let float = matches!(ty.as_str(), "float" | "vec2" | "vec3" | "vec4" | "mat2" | "mat3" | "mat4");
                let boolean = ty.starts_with("bool") || ty.starts_with("bvec");
                values.iter().map(move |&v| match (float, boolean) {
                    (true, _) => (v as f32).to_bits(),
                    (_, true) if v != 0.0 => u32::MAX,
                    (_, true) => 0,
                    _ => v as i64 as u32,
                })
            })
            .collect();
        assert_eq!(member_words(&r), want, "{name}: the member words");
    }
}

/// A reference with a struct-member row that is not in `STRUCT_VALUES` was
/// drawn before gl_ref.py set members and keeps every uniform word 0:
/// `deqp_unitype_vsstruct_partial` compiles and matches that way.
#[test]
fn an_older_struct_reference_keeps_zero_uniforms() {
    let r = mali_ref::Reference::load("deqp_unitype_vsstruct_partial");
    assert!(!mali_ref::STRUCT_VALUES.contains(&r.name.as_str()));
    let pair = mali_ref::compile_pair(&r.vs, &r.fs).expect("the pair compiles");
    assert!(pair.vs.uniforms.iter().any(|u| u.0.contains('.')), "a struct-member row");
    let d = mali_ref::differences(&r, &pair);
    assert!(d.is_empty(), "{}", d.join("\n"));
}

/// The measure of tools/mali/route.sh and tools/mali/corpus_check.py
/// ([`mali_ref::measure_list`]): with YAKOGL_MALI_MEASURE_LIST naming a
/// list file, one row per line on stdout (YAKOGL_MALI_MEASURE_SHARD=k/n
/// takes the k-th of n shares). Without it, the self-test of the rows.
#[test]
fn the_measure_reads_a_list() {
    if let Some(list) = std::env::var_os("YAKOGL_MALI_MEASURE_LIST") {
        let shard = std::env::var("YAKOGL_MALI_MEASURE_SHARD")
            .ok()
            .and_then(|s| s.split_once('/').and_then(|(k, n)| Some((k.parse().ok()?, n.parse().ok()?))))
            .unwrap_or((0, 1));
        mali_ref::measure_list(std::path::Path::new(&list), shard);
        return;
    }
    use mali_ref::{measure, measure_row, Rule, StageVerdict};
    let tri = mali_ref::Reference::load("tri");
    let m = measure(&tri.vs, &tri.fs, Some(&tri.json), Rule::Either);
    assert_eq!((m.verdict, &m.vs, &m.fs), ("EQUAL", &StageVerdict::Equal, &StageVerdict::Equal));
    let digest = m.digest.clone().expect("a digest");
    assert_eq!(measure_row("tri", &m), format!("tri\tEQUAL\t{digest}\t-\tEQUAL\t-\tEQUAL\t-"));
    /* the same program against another program's reference: one digest */
    let other = mali_ref::Reference::load("tri_varying");
    let m = measure(&tri.vs, &tri.fs, Some(&other.json), Rule::Values);
    assert_eq!(m.verdict, "DIFFER", "{}", m.detail);
    assert_eq!(m.digest.as_deref(), Some(digest.as_str()));
    assert!(m.detail.starts_with("vs: vs.varying: 0 words ours, 11 reference, first at word 0"), "{}", m.detail);
    let m = measure(&tri.vs, &tri.fs, None, Rule::Values);
    assert_eq!((m.verdict, &m.vs, &m.fs), ("NOREF", &StageVerdict::NoRef, &StageVerdict::NoRef));
    /* a refused vertex shader: its stage REFUSED with the log, the other
     * stage compiled and BLOCKED */
    let vs = "attribute vec2 pos; uniform int n; void main(){ gl_Position = vec4(pos, float(n), 1.0); }";
    let m = measure(vs, &tri.fs, Some(&tri.json), Rule::Values);
    assert_eq!(m.verdict, "REFUSED");
    assert!(m.detail.starts_with("vs (") && m.detail.contains("a uniform other than a float"), "{}", m.detail);
    assert!(matches!(&m.vs, StageVerdict::Refused(d) if d == &m.detail), "{:?}", m.vs);
    assert_eq!(m.fs, StageVerdict::Blocked);
    let row = measure_row("x", &m);
    assert!(row.starts_with("x\tREFUSED\t-\tvs ("), "{row}");
    assert_eq!(row.matches('\t').count(), 7, "eight fields: {row}");
}

/// Issue #8783: the measure reads each uniform word by the rule
/// [`mali_ref::differences`] uses (an `int` is its integer, a `bool` all
/// ones or 0, `gl_DepthRange` near, far, diff are 0, 1, 1, a mat3 is
/// counted packed over its 4-dword columns), so these references, which
/// their own tests assert byte-equal, measure EQUAL under both rules.
#[test]
fn the_measure_reads_uniform_words_as_the_tests_do() {
    for name in [
        "deqp_misc_cmp_ine0_hp",
        "deqp_unitype_depthrange_vs",
        "deqp_unitype_vsmat3_highp",
        "deqp_unitype_vsmat3_mediump",
        "deqp_intval_reslib_bvec_fs",
        "deqp_intval_uniapi_bool_vs",
    ] {
        let r = mali_ref::Reference::load(name);
        for rule in [mali_ref::Rule::Values, mali_ref::Rule::Either] {
            let m = mali_ref::measure(&r.vs, &r.fs, Some(&r.json), rule);
            assert_eq!(m.verdict, "EQUAL", "{name} ({rule:?}): {}", m.detail);
        }
    }
}
