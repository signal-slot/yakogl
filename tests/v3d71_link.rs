// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The V3D 7.1 GL link entry (`shaderc::online::link71`): the programs of
//! gl_test's six P9 cases (5, 58, 59, 66, 67, 102; tests/v3d71_gltest_shaders.rs
//! names them: `color_uv` and the float `clear` quad) linked through
//! `link_sources71`, as the GL core's glLinkProgram would on a 7.1 table,
//! give the `ShcShader` whose code words, uniform streams and prog_data are
//! the pinned 7.1 headers' (`tools/v3d/gltest-shaders71/ours_<p>.h`, the
//! `ours71_<p>_{fs,vs,cs}_code[]` and `_uniforms[][2]` arrays and the
//! `OURS71_<P>_*` values). The headers are read as text here, so the link
//! is compared with the committed bytes, not with a second compile.
//!
//! Also: the 7.1 record is not the 4.2 one (its words differ, its
//! reflection is the same), the Lines and Points keys (chain71 compiles
//! them; `color_uv` reads no `gl_PointCoord`, so both are its Triangles
//! program), and the refusals: `blend_key != 0` and a non-zero
//! `compile_rt_f32` key, each by its name.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d71_link
//! ```

#![cfg(all(feature = "shaderc-online", feature = "gpu-v3d"))]

use std::collections::HashMap;

use yakogl::shaderc::online::codegen::{self, BackEndError, CompileError, ProgramKey, ProgramPrim, Stage};
use yakogl::shaderc::online::link71::{compile_program71_shader, link_sources71, RT_F32_NOT_YET};
use yakogl::shaderc::online::sema;
use yakogl::shaderc::online::v3d::chain::V3dError;
use yakogl::shaderc::online::{lower, store};
use yakogl::shaderc::{LodKey, ShcShader, ShcV3dUniform, ShcV3dVs, SHC_V3D_NO_RECORD};
use yakogl::v3d::backend::{CLEAR_FS, CLEAR_VS};

static COLOR_UV_VS: &str = include_str!("../tools/shaders/gl_test_es100/color_uv.vert");
static COLOR_UV_FS: &str = include_str!("../tools/shaders/gl_test_es100/color_uv.frag");

/// A NUL-terminated source of the back end as `&str`.
fn c_source(s: &'static [u8]) -> &'static str {
    core::str::from_utf8(s.strip_suffix(b"\0").expect("NUL-terminated")).expect("UTF-8")
}

/// The two programs of the six cases: name, sources, pinned header.
fn programs() -> [(&'static str, &'static str, &'static str, &'static str); 2] {
    [
        ("color_uv", COLOR_UV_VS, COLOR_UV_FS, include_str!("../tools/v3d/gltest-shaders71/ours_color_uv.h")),
        ("clear", c_source(CLEAR_VS), c_source(CLEAR_FS), include_str!("../tools/v3d/gltest-shaders71/ours_clear.h")),
    ]
}

/// What a pinned header says: its arrays and its integer `#define`s.
struct Pinned {
    code: HashMap<String, Vec<u64>>,
    uniforms: HashMap<String, Vec<(u32, u32)>>,
    defines: HashMap<String, u32>,
}

fn int(s: &str) -> u64 {
    let s = s.trim().trim_end_matches("ull").trim_end_matches('u');
    match s.strip_prefix("0x") {
        Some(h) => u64::from_str_radix(h, 16).unwrap_or_else(|e| panic!("{s}: {e}")),
        None => s.parse().unwrap_or_else(|e| panic!("{s}: {e}")),
    }
}

/// Reads `static const ... <prefix>_<stage>_code[..] = { .. };`,
/// `<prefix>_<stage>_uniforms[..][2] = { { c, d }, .. };` and every
/// `#define <PREFIX>_<NAME> <integer>`.
fn parse(header: &str, prefix: &str) -> Pinned {
    let mut p = Pinned { code: HashMap::new(), uniforms: HashMap::new(), defines: HashMap::new() };
    let upper = prefix.to_uppercase();
    let mut lines = header.lines();
    while let Some(l) = lines.next() {
        if let Some(rest) = l.strip_prefix(&format!("#define {upper}_")) {
            let mut it = rest.splitn(2, ' ');
            let (name, value) = (it.next().unwrap(), it.next().unwrap_or("").trim());
            let value = value.split("/*").next().unwrap().trim();
            if value.chars().next().is_some_and(|c| c.is_ascii_digit()) {
                p.defines.insert(name.to_string(), int(value) as u32);
            }
            continue;
        }
        let Some(decl) = l.strip_prefix("static const ") else { continue };
        let name_at = decl.find(prefix).expect("prefixed array");
        let name = &decl[name_at + prefix.len() + 1..];
        let (stage, kind) = name.split_once('_').unwrap();
        let mut body = String::new();
        for l in lines.by_ref() {
            if l.starts_with("};") {
                break;
            }
            body.push_str(l);
        }
        if kind.starts_with("code[") {
            let words = body.split(',').map(str::trim).filter(|s| !s.is_empty()).map(int).collect();
            p.code.insert(stage.to_string(), words);
        } else if kind.starts_with("uniforms[") {
            let pairs = body
                .split('}')
                .map(|s| s.trim().trim_start_matches(',').trim().trim_start_matches('{'))
                .filter(|s| !s.trim().is_empty())
                .map(|s| {
                    let (c, d) = s.split_once(',').unwrap();
                    (int(c) as u32, int(d) as u32)
                })
                .collect();
            p.uniforms.insert(stage.to_string(), pairs);
        } else {
            panic!("an array the shape does not list: {decl}");
        }
    }
    p
}

/// The record's words of one program, back as 64-bit words.
fn words(sh: &ShcShader, off: u32, bytes: u32) -> Vec<u64> {
    // SAFETY: the record's `usc_dw` dwords, kept alive by the open keep.
    let dw = unsafe { core::slice::from_raw_parts(sh.usc, sh.usc_dw as usize) };
    let (a, n) = (off as usize / 4, bytes as usize / 4);
    dw[a..a + n].chunks(2).map(|c| c[0] as u64 | (c[1] as u64) << 32).collect()
}

fn stream(u: *const ShcV3dUniform, n: u32) -> Vec<(u32, u32)> {
    if n == 0 {
        return Vec::new();
    }
    // SAFETY: the record's stream of `n` entries, in the open keep.
    unsafe { core::slice::from_raw_parts(u, n as usize) }.iter().map(|u| (u.contents, u.data)).collect()
}

/// Every test holds this: `compile_rt_f32` is a process-wide key, and the
/// refusal test sets it while the others link.
static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn serial() -> std::sync::MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(|e| e.into_inner())
}

/// `f` with a keep transaction open and the scratch lent.
fn with_link<R>(f: impl FnOnce(&yakogl::shaderc::online::arena::Arena<'_>) -> R) -> R {
    let _keep = store::begin_keep().expect("keep");
    store::with_scratch(f).expect("scratch")
}

fn link71(vs: &str, fs: &str) -> Result<ShcShader, CompileError> {
    with_link(|a| link_sources71(vs.as_bytes(), None, LodKey::NONE, fs.as_bytes(), &[], a, store::keep()).map(|r| r.0))
}

fn vs_fields(p: &Pinned, s: &str, v: &ShcV3dVs) {
    let d = |k: &str| p.defines[&format!("{}_{k}", s.to_uppercase())];
    assert_eq!(u32::from(v.threads), d("THREADS"), "{s}");
    assert_eq!(u32::from(v.single_seg), d("SINGLE_SEG"), "{s}");
    assert_eq!(v.vpm_input_size, d("VPM_INPUT_SIZE"), "{s}");
    assert_eq!(v.vpm_output_size, d("VPM_OUTPUT_SIZE"), "{s}");
    assert_eq!(u32::from(v.separate_segments), d("SEPARATE_SEGMENTS"), "{s}");
    assert_eq!(v.vcm_cache_size, d("VCM_CACHE_SIZE"), "{s}");
    assert_eq!(u32::from(v.uses_vid), d("USES_VID"), "{s}");
    assert_eq!(u32::from(v.uses_iid), d("USES_IID"), "{s}");
    assert_eq!(u32::from(v.uses_biid), d("USES_BIID"), "{s}");
    assert_eq!(u32::from(v.writes_psiz), d("WRITES_PSIZ"), "{s}");
}

/// The six cases' two programs, linked at 7.1, are the pinned headers'.
#[test]
fn the_six_cases_programs_link_to_the_pinned_7_1_programs() {
    let _serial = serial();
    for (name, vs, fs, header) in programs() {
        let prefix = format!("ours71_{name}");
        let pin = parse(header, &prefix);
        assert_eq!(pin.code.len(), 3, "{name}: three programs in the header");
        let _keep = store::begin_keep().expect("keep");
        let sh = store::with_scratch(|a| {
            link_sources71(vs.as_bytes(), None, LodKey::NONE, fs.as_bytes(), &[], a, store::keep()).map(|r| r.0)
        })
        .expect("scratch")
        .unwrap_or_else(|e| panic!("{name}: {e:?}"));
        let p = sh.v3d;
        assert_eq!(p.linked, 1, "{name}");
        let total: usize = pin.code.values().map(Vec::len).sum();
        assert_eq!(sh.usc_dw as usize, total * 2, "{name}: FS, VS, CS and nothing else");
        assert_eq!(
            (p.fs.code_off, p.vs.code_off, p.cs.code_off),
            (0, p.fs.code_bytes, p.fs.code_bytes + p.vs.code_bytes)
        );
        for (s, off, bytes, u, n) in [
            ("fs", p.fs.code_off, p.fs.code_bytes, p.fs.uniforms, p.fs.num_uniforms),
            ("vs", p.vs.code_off, p.vs.code_bytes, p.vs.uniforms, p.vs.num_uniforms),
            ("cs", p.cs.code_off, p.cs.code_bytes, p.cs.uniforms, p.cs.num_uniforms),
        ] {
            assert_eq!(words(&sh, off, bytes), pin.code[s], "{name} {s}: the code words");
            let want = pin.uniforms.get(s).cloned().unwrap_or_default();
            assert_eq!(stream(u, n), want, "{name} {s}: the uniform stream");
            assert_eq!(n, pin.defines[&format!("{}_NUM_UNIFORMS", s.to_uppercase())], "{name} {s}");
        }
        /* the prog_data the 7.1 record reads */
        let d = |k: &str| pin.defines[k];
        assert_eq!(u32::from(p.fs.threads), d("FS_THREADS"), "{name}");
        assert_eq!(u32::from(p.fs.single_seg), d("FS_SINGLE_SEG"), "{name}");
        assert_eq!(p.fs.num_inputs, d("FS_NUM_INPUTS"), "{name}");
        assert_eq!(p.fs.flat_shade_flags, d("FS_FLAT_SHADE_FLAGS"), "{name}");
        assert_eq!(p.fs.noperspective_flags, d("FS_NOPERSPECTIVE_FLAGS"), "{name}");
        assert_eq!(p.fs.centroid_flags, d("FS_CENTROID_FLAGS"), "{name}");
        assert_eq!(u32::from(p.fs.writes_z), d("FS_WRITES_Z"), "{name}");
        assert_eq!(u32::from(p.fs.disable_ez), d("FS_DISABLE_EZ"), "{name}");
        assert_eq!(u32::from(p.fs.uses_center_w), d("FS_USES_CENTER_W"), "{name}");
        assert_eq!(u32::from(p.fs.lock_scoreboard_on_first_thrsw), d("FS_LOCK_SCOREBOARD_ON_FIRST_THRSW"), "{name}");
        assert_eq!(
            u32::from(p.fs.uses_implicit_point_line_varyings),
            d("FS_USES_IMPLICIT_POINT_LINE_VARYINGS"),
            "{name}"
        );
        assert_eq!(u32::from(p.fs.uses_pid), d("FS_USES_PID"), "{name}");
        vs_fields(&pin, "vs", &p.vs);
        vs_fields(&pin, "cs", &p.cs);
        /* the attribute records: `<P>_ATTR_<NAME>_RECORD` at the location
         * its comment gives, and NUM_ATTRS of them */
        assert_eq!(u32::from(p.num_records), d("NUM_ATTRS"), "{name}");
        let recs: Vec<(u8, u32)> = header
            .lines()
            .filter(|l| l.contains("_RECORD ") && l.contains("/* location "))
            .map(|l| {
                let r = l.split_whitespace().nth(2).unwrap().parse().unwrap();
                let loc = l.split("/* location ").nth(1).unwrap().trim_end_matches(" */").parse().unwrap();
                (loc, r)
            })
            .collect();
        assert_eq!(recs.len(), d("NUM_ATTRS") as usize, "{name}");
        for (loc, r) in &recs {
            assert_eq!(u32::from(p.record_of_location[*loc as usize]), *r, "{name}: location {loc}");
        }
        let unrecorded = p.record_of_location.iter().filter(|&&r| r == SHC_V3D_NO_RECORD).count();
        assert_eq!(unrecorded, 16 - recs.len(), "{name}");
        /* the stage's reflection carries over (glLinkProgram's record is
         * the fragment shader's) */
        assert_eq!(sh.num_outputs, 1, "{name}");
    }
}

/// The 7.1 record is laid out as the 4.2 one, with the same reflection,
/// but its words are the 7.1 chain's, not the 4.2 chain's.
#[test]
fn the_7_1_record_has_the_4_2_reflection_and_its_own_words() {
    let _serial = serial();
    for (name, vs, fs, _) in programs() {
        let _keep = store::begin_keep().expect("keep");
        let (s71, s42) = store::with_scratch(|a| {
            let l71 = link_sources71(vs.as_bytes(), None, LodKey::NONE, fs.as_bytes(), &[], a, store::keep());
            let l42 = codegen::link_sources(vs.as_bytes(), None, LodKey::NONE, fs.as_bytes(), &[], a, store::keep());
            let (s71, b71) = l71.unwrap_or_else(|e| panic!("{name} 7.1: {e:?}"));
            let (s42, b42) = l42.unwrap_or_else(|e| panic!("{name} 4.2: {e:?}"));
            assert_eq!((b71.0.len(), b71.1.len()), (b42.0.len(), b42.1.len()), "{name}: the block table");
            assert_eq!((b71.2[0].len(), b71.2[1].len()), (b42.2[0].len(), b42.2[1].len()), "{name}: uniform rows");
            (s71, s42)
        })
        .expect("scratch");
        assert_eq!(
            (s71.num_varyings, s71.num_uniforms, s71.uniform_dwords, s71.num_outputs),
            (s42.num_varyings, s42.num_uniforms, s42.uniform_dwords, s42.num_outputs),
            "{name}"
        );
        assert_eq!(s71.v3d.record_of_location, s42.v3d.record_of_location, "{name}");
        assert_eq!(s71.v3d.fs.num_inputs, s42.v3d.fs.num_inputs, "{name}");
        assert_eq!(s71.v3d.vs.vattr_sizes, s42.v3d.vs.vattr_sizes, "{name}");
        assert_ne!(
            words(&s71, s71.v3d.fs.code_off, s71.v3d.fs.code_bytes),
            words(&s42, s42.v3d.fs.code_off, s42.v3d.fs.code_bytes),
            "{name}: the 7.1 link must not carry 4.2 code"
        );
    }
}

/// `blend_key != 0` by the 4.2 entry's name and a non-zero `compile_rt_f32`
/// key by its 7.1 name; Lines and Points link (chain71 compiles both, the
/// 4.2 chain refuses them) to `color_uv`'s Triangles code, which reads no
/// `gl_PointCoord`.
#[test]
fn the_7_1_link_refuses_by_name_and_links_lines_and_points() {
    let _serial = serial();
    /* the FS, VS and CS code words of color_uv at `key`, or the refusal */
    let compile = |key: ProgramKey| -> Result<[Vec<u64>; 3], CompileError> {
        with_link(|a| {
            let v = lower::compile_to_ir(a, sema::Stage::Vertex, a.alloc_str(COLOR_UV_VS).unwrap())
                .map_err(|e| e.msg)
                .unwrap();
            let f = lower::compile_to_ir(a, sema::Stage::Fragment, a.alloc_str(COLOR_UV_FS).unwrap())
                .map_err(|e| e.msg)
                .unwrap();
            compile_program71_shader(&v, &f, key, a, store::keep()).map(|sh| {
                let p = sh.v3d;
                [
                    words(&sh, p.fs.code_off, p.fs.code_bytes),
                    words(&sh, p.vs.code_off, p.vs.code_bytes),
                    words(&sh, p.cs.code_off, p.cs.code_bytes),
                ]
            })
        })
    };
    let not_yet = |w| CompileError::BackEnd(BackEndError::Program(V3dError::NotYet(w)));
    assert_eq!(
        compile(ProgramKey { blend_key: 1, ..ProgramKey::LINK }).map(|_| ()),
        Err(CompileError::NotImplemented(Stage::CodeGen))
    );
    /* the triangles key links (the control); lines and points give its code */
    let tri = compile(ProgramKey::LINK).expect("triangles links");
    for prim in [ProgramPrim::Lines, ProgramPrim::Points] {
        let got = compile(ProgramKey { prim, ..ProgramKey::LINK }).unwrap_or_else(|e| panic!("{prim:?}: {e:?}"));
        assert_eq!(got, tri, "{prim:?}: color_uv reads no gl_PointCoord, so the Triangles program");
    }

    struct Restore(u8);
    impl Drop for Restore {
        fn drop(&mut self) {
            yakogl::shaderc::set_compile_rt_f32(self.0);
        }
    }
    let r = link71(COLOR_UV_VS, COLOR_UV_FS).map(|_| ());
    assert_eq!(r, Ok(()), "rt_f32 0 links");
    let _restore = Restore(yakogl::shaderc::set_compile_rt_f32(1));
    assert_eq!(link71(COLOR_UV_VS, COLOR_UV_FS).map(|_| ()), Err(not_yet(RT_F32_NOT_YET)));
}
