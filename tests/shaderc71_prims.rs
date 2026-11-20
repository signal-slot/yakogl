// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The V3D 7.1 (Raspberry Pi 5) online back end's points and lines variants
//! of the fragment shader over every `tools/shaders/es100` program: each
//! program goes through `chain71::compile_program71` at `Prim::Triangles`,
//! `Prim::Points` and `Prim::Lines`, and every variant compiles exactly when
//! the Triangles program does (the count is pinned; the one refusal,
//! es100/varyings15.frag's `Alloc(DoesNotFit)`, is the same at all three).
//!
//! The programs are `tests/v3d71_online_corpus.rs`'s es100 rows: each
//! source paired with `tools/v3d/es100_ref71.sh`'s stub of the other stage
//! (a stub VS writes every varying the FS declares from a uniform vec4, a
//! stub FS adds every varying the VS declares into gl_FragColor). The stub
//! code is that test's, copied (it is plan #384's file, not shared).
//!
//! Every variant that compiles is checked: each word of its fragment,
//! vertex and coordinate shader decodes with codec71 to its instruction
//! (`same_instr71`), and the instructions pass verify71,
//! `registers_fit_threads71` and `branches_are_structured71`. And the
//! variants' relation (chain71.rs `compile_program71_keyed`, Mesa
//! nir_to_vir.c:4869-4878):
//!
//! * the vertex and coordinate shaders are the Triangles program's, word
//!   for word, at every primitive;
//! * the points variant is the Triangles program word for word (the
//!   link-time program already takes gl_PointCoord's implicit varyings);
//! * the lines variant's fragment shader is the Triangles one exactly when
//!   the shader does not read gl_PointCoord; when it does, it has no
//!   implicit varying (`uses_implicit_point_line_varyings` false).
//!
//! Two pins the sim71-prims-self study (plan #386) left as tests:
//!
//! * `the_points_program_is_the_triangles_program_in_every_field`: not only
//!   the words — the code of every stage (words, instructions, uniform
//!   stream and branch edges), every prog_data field and the attributes of
//!   the points program equal the Triangles program's, for every es100
//!   program that compiles at Triangles;
//! * `the_lines_variant_reads_gl_point_coord_as_zero_in_sim71`: each es100
//!   shader that reads gl_PointCoord, at `Prim::Lines`, has its words decode
//!   and verify, and run in `sim71::run71` on fixed inputs gives the colour
//!   chain71's doc comment promises off a point: gl_PointCoord is 0.0
//!   (GLSL ES 1.00 section 7.2, `point_coord_as_zero`).
//!
//! Mesa's instruction counts (`tools/v3d/es100_counts_71.tsv`) are of its
//! Triangles key (`v3d_shc --prim triangles`, the default), whose FS takes no
//! implicit point varying, like our lines variant: for the sources that read
//! gl_PointCoord, our three variants and Mesa's row are printed side by side.
//! Printed, not enforced. Mesa's points variant (`v3d_shc --device 71 --prim
//! points`) needs a built v3d_shc (`V3D_SHC`, tools/v3d/README.md H2); none is
//! read here.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test shaderc71_prims -- --nocapture
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-v3d"))]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use yakogl::shaderc::online::sema::Stage;
use yakogl::shaderc::online::v3d::chain::Prim;
use yakogl::shaderc::online::v3d::chain71::{compile_program71, Code71, FsProg71, V3dProgram71, VsProg71, TARGET71};
use yakogl::shaderc::online::v3d::codec71::{decode71, same_instr71};
use yakogl::shaderc::online::v3d::sched71_cf::branches_are_structured71;
use yakogl::shaderc::online::v3d::select::{QUNIFORM_CONSTANT, QUNIFORM_FB_PNTC_Y_OFFSET, QUNIFORM_FB_PNTC_Y_SCALE};
use yakogl::shaderc::online::v3d::sim::{half_to_f32, Env, FsInputs, Inputs, Varying, LANES};
use yakogl::shaderc::online::v3d::sim71::run71;
use yakogl::shaderc::online::v3d::verify71::{registers_fit_threads71, verify71};
use yakogl::shaderc::online::v3d::vop71::{Instr71, Uniform};
use yakogl::shaderc::online::{lower, store};

/// tools/shaders/es100's sources.
const ES100_PROGRAMS: usize = 126;
/// Of them, the programs our 7.1 back end compiles, at every primitive
/// (tests/v3d71_online_corpus.rs `ES100_COMPILED` for Triangles).
const ES100_COMPILED: usize = 125;
/// The sources that read gl_PointCoord (es100/pointcoord.frag and
/// es100/pointcoordvary.frag): the only ones whose lines variant differs.
const READS_POINT_COORD: usize = 2;

static ES100_71: &str = include_str!("../tools/v3d/es100_counts_71.tsv");

const PRIMS: [Prim; 3] = [Prim::Triangles, Prim::Points, Prim::Lines];

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/shaders")
}

fn read(rel: &str) -> String {
    std::fs::read_to_string(root().join(rel)).unwrap_or_else(|e| panic!("tools/shaders/{rel}: {e}"))
}

fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).expect("tools/shaders is readable") {
        let p = e.expect("a directory entry").path();
        if p.is_dir() {
            sources(&p, out);
        } else if matches!(p.extension().and_then(|x| x.to_str()), Some("vert" | "frag")) {
            out.push(p);
        }
    }
}

/// One stage's words decode to its instructions, which pass verify71, fit
/// its thread count and have structured branches.
fn check_stage(what: &str, c: &Code71<'_>, threads: u32, single_seg: bool) {
    assert_eq!(c.words.len(), c.instrs.len(), "{what}");
    assert!(!c.words.is_empty(), "{what}: no words");
    for (n, (&w, i)) in c.words.iter().zip(c.instrs).enumerate() {
        let back = decode71(w).unwrap_or_else(|e| panic!("{what}: word {n} ({w:#018x}) does not decode: {e:?}"));
        assert!(same_instr71(i, &back), "{what}: word {n} decodes to another instruction");
    }
    assert_eq!(verify71(c.instrs, single_seg), Ok(()), "{what}: verify71");
    assert_eq!(registers_fit_threads71(c.instrs, threads), Ok(()), "{what}: registers at {threads} threads");
    assert_eq!(branches_are_structured71(c.instrs, c.back_edges, c.fwd_branches), Ok(()), "{what}: branches");
}

/// What one variant compiled to: each stage's words, and whether the FS
/// takes the implicit point varyings.
#[derive(Clone, PartialEq, Eq, Debug)]
struct Variant {
    fs: Vec<u64>,
    vs: Vec<u64>,
    cs: Vec<u64>,
    implicit: bool,
}

fn variant(key: &str, prim: Prim, p: &V3dProgram71<'_>) -> Variant {
    check_stage(&format!("{key} {prim:?} fs"), &p.fs.code, p.fs.threads, p.fs.single_seg);
    check_stage(&format!("{key} {prim:?} vs"), &p.vs.code, p.vs.threads, p.vs.single_seg);
    check_stage(&format!("{key} {prim:?} cs"), &p.cs.code, p.cs.threads, p.cs.single_seg);
    Variant {
        fs: p.fs.code.words.to_vec(),
        vs: p.vs.code.words.to_vec(),
        cs: p.cs.code.words.to_vec(),
        implicit: p.fs.uses_implicit_point_line_varyings,
    }
}

/// The scratch region is one per process (`store::with_scratch` refuses a
/// second borrower): the harness runs this file's tests in parallel, so they
/// take turns (tests/v3d71_shapes.rs's idiom).
static SCRATCH: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// `vs` + `fs` at 7.1 for each of [`PRIMS`], in that order: the variant or
/// the refusal's message (the front end's, or `V3d71Error`'s Debug form).
fn compile(key: &str, names: [&str; 2], vs: &str, fs: &str) -> Vec<Result<Variant, String>> {
    let _turn = SCRATCH.lock().unwrap_or_else(|e| e.into_inner());
    store::with_scratch(|a| {
        let mut m = Vec::new();
        for (stage, name, src) in [(Stage::Vertex, names[0], vs), (Stage::Fragment, names[1], fs)] {
            match lower::compile_to_ir(a, stage, a.alloc_str(src).expect("scratch")) {
                Ok(l) => m.push(l.module),
                Err(e) => return vec![Err(format!("front end {name}: {}", e.msg)); PRIMS.len()],
            }
        }
        let v = &a.alloc_filled(1, m[0]).expect("scratch")[0];
        let f = &a.alloc_filled(1, m[1]).expect("scratch")[0];
        PRIMS
            .iter()
            .map(|&prim| {
                compile_program71(v, f, prim, TARGET71, a).map(|p| variant(key, prim, &p)).map_err(|e| format!("{e:?}"))
            })
            .collect()
    })
    .expect("scratch")
}

// ---------------------------------------------------------------------------
// es100_ref71.sh's stubs, as tests/v3d71_online_corpus.rs has them.
// ---------------------------------------------------------------------------

/// The text without `//` and `/* */` comments, each a space
/// (tools/vivante/es100.py `strip_comments`, which es100_ref71.sh uses).
fn strip_comments(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < b.len() {
        if b[i..].starts_with(b"//") {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
            out.push(' ');
        } else if b[i..].starts_with(b"/*") {
            i += 2;
            while i < b.len() && !b[i..].starts_with(b"*/") {
                i += 1;
            }
            i = (i + 2).min(b.len());
            out.push(' ');
        } else {
            out.push(b[i] as char);
            i += 1;
        }
    }
    out
}

const TYPES: [&str; 7] = ["float", "vec2", "vec3", "vec4", "mat2", "mat3", "mat4"];

/// `(type, name, array size or 0)` of the source's varying declarations
/// (es100_ref71.sh `varyings`).
fn varyings(text: &str) -> Vec<(String, String, usize)> {
    let word = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let t = strip_comments(text);
    let b = t.as_bytes();
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(k) = t[from..].find("varying").map(|k| k + from) {
        from = k + 7;
        if (k > 0 && word(b[k - 1])) || b.get(k + 7).is_some_and(|&c| word(c)) {
            continue;
        }
        let Some(end) = t[k + 7..].find(';').map(|e| e + k + 7) else { break };
        let mut toks = t[k + 7..end].split_whitespace().peekable();
        if toks.peek().is_some_and(|p| ["highp", "mediump", "lowp"].contains(p)) {
            toks.next();
        }
        let Some(ty) = toks.next().filter(|ty| TYPES.contains(ty)) else { continue };
        let decls: String = toks.collect::<Vec<_>>().join(" ");
        for d in decls.split(',') {
            let d = d.trim();
            let (name, n) = match d.split_once('[') {
                Some((n, rest)) => (n.trim(), rest.trim_end_matches(']').trim().parse().expect("a varying array size")),
                None => (d, 0),
            };
            assert!(!name.is_empty() && name.bytes().all(word), "cannot read varying `{d}`");
            out.push((ty.to_string(), name.to_string(), n));
        }
        from = end;
    }
    out
}

/// An expression of type `ty` from the uniform vec4 `src`.
fn value(ty: &str, src: &str) -> String {
    match ty {
        "float" => format!("{src}.x"),
        "vec4" => format!("vec4({src})"),
        t if t.starts_with("vec") => format!("{src}.{}", &"xyzw"[..t[3..].parse::<usize>().unwrap()]),
        t => {
            let n: usize = t[3..].parse().unwrap();
            format!("{t}({})", vec![format!("{src}.x"); n * n].join(", "))
        }
    }
}

fn stub_vs(list: &[(String, String, usize)]) -> String {
    let mut s: Vec<String> =
        ["#version 100", "precision highp float;", "attribute vec4 a_pos;", "uniform vec4 u_stub;"]
            .map(String::from)
            .to_vec();
    let mut body = vec!["    gl_Position = a_pos;".to_string()];
    for (ty, name, arr) in list {
        s.push(format!("varying {ty} {name}{};", if *arr > 0 { format!("[{arr}]") } else { String::new() }));
        for i in 0..(*arr).max(1) {
            let e = if *arr > 0 { format!("{name}[{i}]") } else { name.clone() };
            body.push(format!("    {e} = {};", value(ty, "u_stub")));
        }
    }
    s.extend(["void main()".to_string(), "{".to_string()]);
    s.extend(body);
    s.extend(["}".to_string(), String::new()]);
    s.join("\n")
}

fn stub_fs(list: &[(String, String, usize)]) -> String {
    let mut s: Vec<String> = vec!["#version 100".into(), "precision highp float;".into()];
    let mut body = vec!["    vec4 c = vec4(0.0);".to_string()];
    for (ty, name, arr) in list {
        s.push(format!("varying {ty} {name}{};", if *arr > 0 { format!("[{arr}]") } else { String::new() }));
        for i in 0..(*arr).max(1) {
            let e = if *arr > 0 { format!("{name}[{i}]") } else { name.clone() };
            if let Some(n) = ty.strip_prefix("mat") {
                let n: usize = n.parse().unwrap();
                let sum: Vec<String> = (0..n).map(|k| format!("{e}[{k}].x")).collect();
                body.push(format!("    c.x += {};", sum.join(" + ")));
            } else if ty == "float" {
                body.push(format!("    c += vec4({e});"));
            } else {
                let pad =
                    if ty == "vec4" { String::new() } else { ", 0.0".repeat(4 - ty[3..].parse::<usize>().unwrap()) };
                body.push(format!("    c += vec4({e}{pad});"));
            }
        }
    }
    s.extend(["void main()".to_string(), "{".to_string()]);
    s.extend(body);
    s.extend(["    gl_FragColor = c;".to_string(), "}".to_string(), String::new()]);
    s.join("\n")
}

#[test]
fn every_es100_program_compiles_at_points_and_lines_as_at_triangles() {
    let mut files = Vec::new();
    sources(&root().join("es100"), &mut files);
    let mut rels: Vec<String> =
        files.iter().map(|p| p.strip_prefix(root()).unwrap().to_string_lossy().into_owned()).collect();
    rels.sort();
    /* The population first: an empty walk would make "the counts agree" true. */
    assert_eq!(rels.len(), ES100_PROGRAMS, "tools/shaders/es100 holds {ES100_PROGRAMS} sources; re-derive the pins");

    /* Mesa's row per source (its Triangles key): instructions when it compiled */
    let mesa: BTreeMap<&str, Option<usize>> = ES100_71
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .map(|l| {
            let c: Vec<&str> = l.split('\t').collect();
            (c[0], (c[2] == "yes").then(|| c[4].parse().unwrap()))
        })
        .collect();

    let mut compiled = [0usize; 3];
    let mut refused: [BTreeMap<String, String>; 3] = Default::default();
    let mut reads_pntc = 0usize;
    println!("shaderc71 prims: source, FS instructions at triangles / points / lines, Mesa 7.1 (triangles key)");
    for rel in &rels {
        let src = read(rel);
        let vary = varyings(&src);
        let (vs, fs, names) = if rel.ends_with(".vert") {
            (src.clone(), stub_fs(&vary), [rel.as_str(), "<stub fs>"])
        } else {
            (stub_vs(&vary), src.clone(), ["<stub vs>", rel.as_str()])
        };
        let got = compile(rel, names, &vs, &fs);
        for (k, r) in got.iter().enumerate() {
            match r {
                Ok(_) => compiled[k] += 1,
                Err(e) => {
                    refused[k].insert(rel.clone(), e.clone());
                }
            }
        }
        let [Ok(tri), Ok(pts), Ok(lin)] = &got[..] else { continue };
        let what = |p: &str| format!("{rel} {p}");
        assert_eq!(pts, tri, "{}: the points variant is the Triangles program", what("Points"));
        for (p, v) in [("Points", pts), ("Lines", lin)] {
            assert_eq!(v.vs, tri.vs, "{}: vertex shader", what(p));
            assert_eq!(v.cs, tri.cs, "{}: coordinate shader", what(p));
        }
        assert!(!lin.implicit, "{}: no implicit varying on lines", what("Lines"));
        assert_eq!(
            lin.fs == tri.fs,
            !tri.implicit,
            "{}: the FS differs exactly when it reads gl_PointCoord",
            what("Lines")
        );
        if tri.implicit {
            reads_pntc += 1;
            let m = mesa.get(rel.as_str()).copied().flatten();
            println!(
                "  {rel}\tfs\t{} / {} / {}\tMesa {}",
                tri.fs.len(),
                pts.fs.len(),
                lin.fs.len(),
                m.map_or("refused".to_string(), |m| m.to_string())
            );
        }
    }
    println!(
        "shaderc71 prims: {} programs; compiled at triangles {}, points {}, lines {}; {reads_pntc} read gl_PointCoord",
        rels.len(),
        compiled[0],
        compiled[1],
        compiled[2]
    );
    for (k, prim) in PRIMS.iter().enumerate() {
        for (rel, m) in &refused[k] {
            println!("  refused at {prim:?}: {rel}: {m}");
        }
    }
    /* the same refusals, with the same message, at every primitive */
    assert_eq!(refused[1], refused[0], "the points variant refuses what Triangles refuses");
    assert_eq!(refused[2], refused[0], "the lines variant refuses what Triangles refuses");
    assert_eq!(
        refused[0].get("es100/varyings15.frag").map(String::as_str),
        Some("Alloc(DoesNotFit)"),
        "the one es100 refusal (tools/shaders/known-uncompilable-v3d71.txt)"
    );
    assert_eq!(compiled, [ES100_COMPILED; 3], "es100 programs compiled at 7.1: a moved count is a decision");
    assert_eq!(reads_pntc, READS_POINT_COORD, "es100 sources reading gl_PointCoord");
}

// ---------------------------------------------------------------------------
// The Points pin, every field. A program borrows its arena, so every field of
// the two programs is copied into an owned snapshot and the snapshots
// compared as one value (sim71-prims-self: every Points program equals its
// Triangles program in every field).
// ---------------------------------------------------------------------------

/// One [`Code71`], owned.
#[derive(Clone, Debug, PartialEq, Eq)]
struct CodeSnap {
    words: Vec<u64>,
    instrs: Vec<Instr71>,
    uniforms: Vec<Uniform>,
    back_edges: Vec<(u32, u32)>,
    fwd_branches: Vec<(u32, u32)>,
}

impl CodeSnap {
    fn of(c: &Code71<'_>) -> CodeSnap {
        CodeSnap {
            words: c.words.to_vec(),
            instrs: c.instrs.to_vec(),
            uniforms: c.uniforms.to_vec(),
            back_edges: c.back_edges.to_vec(),
            fwd_branches: c.fwd_branches.to_vec(),
        }
    }
}

/// One [`FsProg71`], owned.
#[derive(Clone, Debug, PartialEq, Eq)]
struct FsSnap {
    code: CodeSnap,
    threads: u32,
    single_seg: bool,
    num_inputs: u32,
    input_slots: Vec<(u8, u8)>,
    flat_shade_flags: u32,
    noperspective_flags: u32,
    centroid_flags: u32,
    flat_shade_flags_hi: u32,
    noperspective_flags_hi: u32,
    centroid_flags_hi: u32,
    writes_z: bool,
    disable_ez: bool,
    uses_center_w: bool,
    lock_scoreboard_on_first_thrsw: bool,
    uses_implicit_point_line_varyings: bool,
    uses_pid: bool,
    rt_mask: u8,
    force_per_sample_msaa: bool,
    tmu_dirty_rcl: bool,
    uses_discard: bool,
}

impl FsSnap {
    fn of(f: &FsProg71<'_>) -> FsSnap {
        FsSnap {
            code: CodeSnap::of(&f.code),
            threads: f.threads,
            single_seg: f.single_seg,
            num_inputs: f.num_inputs,
            input_slots: f.input_slots.to_vec(),
            flat_shade_flags: f.flat_shade_flags,
            noperspective_flags: f.noperspective_flags,
            centroid_flags: f.centroid_flags,
            flat_shade_flags_hi: f.flat_shade_flags_hi,
            noperspective_flags_hi: f.noperspective_flags_hi,
            centroid_flags_hi: f.centroid_flags_hi,
            writes_z: f.writes_z,
            disable_ez: f.disable_ez,
            uses_center_w: f.uses_center_w,
            lock_scoreboard_on_first_thrsw: f.lock_scoreboard_on_first_thrsw,
            uses_implicit_point_line_varyings: f.uses_implicit_point_line_varyings,
            uses_pid: f.uses_pid,
            rt_mask: f.rt_mask,
            force_per_sample_msaa: f.force_per_sample_msaa,
            tmu_dirty_rcl: f.tmu_dirty_rcl,
            uses_discard: f.uses_discard,
        }
    }
}

/// One [`VsProg71`], owned.
#[derive(Clone, Debug, PartialEq, Eq)]
struct VsSnap {
    code: CodeSnap,
    threads: u32,
    single_seg: bool,
    vpm_input_size: u32,
    vpm_output_size: u32,
    separate_segments: bool,
    vcm_cache_size: u32,
    uses_vid: bool,
    uses_iid: bool,
    uses_biid: bool,
    writes_psiz: bool,
    vattr_sizes: [u8; 16],
    tmu_dirty_rcl: bool,
}

impl VsSnap {
    fn of(v: &VsProg71<'_>) -> VsSnap {
        VsSnap {
            code: CodeSnap::of(&v.code),
            threads: v.threads,
            single_seg: v.single_seg,
            vpm_input_size: v.vpm_input_size,
            vpm_output_size: v.vpm_output_size,
            separate_segments: v.separate_segments,
            vcm_cache_size: v.vcm_cache_size,
            uses_vid: v.uses_vid,
            uses_iid: v.uses_iid,
            uses_biid: v.uses_biid,
            writes_psiz: v.writes_psiz,
            vattr_sizes: v.vattr_sizes,
            tmu_dirty_rcl: v.tmu_dirty_rcl,
        }
    }
}

/// One [`V3dProgram71`], owned.
#[derive(Clone, Debug, PartialEq, Eq)]
struct ProgSnap {
    fs: FsSnap,
    vs: VsSnap,
    cs: VsSnap,
    attributes: Vec<(u8, Option<u8>)>,
}

impl ProgSnap {
    fn of(p: &V3dProgram71<'_>) -> ProgSnap {
        ProgSnap {
            fs: FsSnap::of(&p.fs),
            vs: VsSnap::of(&p.vs),
            cs: VsSnap::of(&p.cs),
            attributes: p.attributes.to_vec(),
        }
    }
}

/// `vs` + `fs` at 7.1: the Triangles program as a snapshot, and the Points
/// program's snapshot or refusal message. `None` is a refusal of Triangles
/// itself or of the front end (the refusals are pinned by
/// [`every_es100_program_compiles_at_points_and_lines_as_at_triangles`]).
fn pair_snaps(vs: &str, fs: &str) -> Option<(ProgSnap, Result<ProgSnap, String>)> {
    let _turn = SCRATCH.lock().unwrap_or_else(|e| e.into_inner());
    store::with_scratch(|a| {
        let mut m = Vec::new();
        for (stage, src) in [(Stage::Vertex, vs), (Stage::Fragment, fs)] {
            match lower::compile_to_ir(a, stage, a.alloc_str(src).expect("scratch")) {
                Ok(l) => m.push(l.module),
                Err(_) => return None,
            }
        }
        let v = &a.alloc_filled(1, m[0]).expect("scratch")[0];
        let f = &a.alloc_filled(1, m[1]).expect("scratch")[0];
        let Ok(tri) = compile_program71(v, f, Prim::Triangles, TARGET71, a) else { return None };
        let pts =
            compile_program71(v, f, Prim::Points, TARGET71, a).map(|p| ProgSnap::of(&p)).map_err(|e| format!("{e:?}"));
        Some((ProgSnap::of(&tri), pts))
    })
    .expect("scratch")
}

#[test]
fn the_points_program_is_the_triangles_program_in_every_field() {
    let mut files = Vec::new();
    sources(&root().join("es100"), &mut files);
    let mut rels: Vec<String> =
        files.iter().map(|p| p.strip_prefix(root()).unwrap().to_string_lossy().into_owned()).collect();
    rels.sort();
    assert_eq!(rels.len(), ES100_PROGRAMS, "tools/shaders/es100 holds {ES100_PROGRAMS} sources; re-derive the pins");
    let mut pinned = 0usize;
    for rel in &rels {
        let src = read(rel);
        let vary = varyings(&src);
        let (vs, fs) =
            if rel.ends_with(".vert") { (src.clone(), stub_fs(&vary)) } else { (stub_vs(&vary), src.clone()) };
        let Some((tri, pts)) = pair_snaps(&vs, &fs) else { continue };
        let pts = pts.unwrap_or_else(|e| panic!("{rel}: Triangles compiles and Points does not: {e}"));
        assert_eq!(pts, tri, "{rel}: the points program is the Triangles program in every field");
        pinned += 1;
    }
    assert_eq!(pinned, ES100_COMPILED, "programs whose points build is pinned to Triangles field by field");
}

// ---------------------------------------------------------------------------
// The Lines pin: off a point gl_PointCoord is undefined (GLSL ES 1.00
// section 7.2), and chain71's lines variant promises it as 0.0
// (`point_coord_as_zero`). Each reader runs in sim71 with W 1.0 and every
// fragment input 0.5; the colour says whether gl_PointCoord read as 0.0.
// ---------------------------------------------------------------------------

/// The value of every fragment input of the run (exact as an f16).
const FIXED_VARYING: f32 = 0.5;

/// The colour the source must draw once gl_PointCoord reads 0.0, given every
/// fragment input 0.5: `pointcoord.frag` is `vec4(gl_PointCoord, 0.0, 1.0)`,
/// `pointcoordvary.frag` is `vec4(gl_PointCoord, v_color.b, 1.0)`.
fn fixed_input_colour(rel: &str) -> [f32; 4] {
    match rel {
        "es100/pointcoord.frag" => [0.0, 0.0, 0.0, 1.0],
        "es100/pointcoordvary.frag" => [0.0, 0.0, FIXED_VARYING, 1.0],
        r => panic!("no expectation for {r}: a new gl_PointCoord reader, derive one from its source"),
    }
}

#[test]
fn the_lines_variant_reads_gl_point_coord_as_zero_in_sim71() {
    let mut files = Vec::new();
    sources(&root().join("es100"), &mut files);
    let mut rels: Vec<String> =
        files.iter().map(|p| p.strip_prefix(root()).unwrap().to_string_lossy().into_owned()).collect();
    rels.sort();
    rels.retain(|rel| rel.ends_with(".frag") && strip_comments(&read(rel)).contains("gl_PointCoord"));
    assert_eq!(
        rels,
        ["es100/pointcoord.frag".to_string(), "es100/pointcoordvary.frag".to_string()],
        "the es100 sources that read gl_PointCoord ({READS_POINT_COORD}): re-derive the expectations"
    );
    for rel in rels {
        let src = read(&rel);
        let vs = stub_vs(&varyings(&src));
        let expect = fixed_input_colour(&rel);
        let _turn = SCRATCH.lock().unwrap_or_else(|e| e.into_inner());
        let colour: [[f32; 4]; LANES] = store::with_scratch(|a| {
            let mut m = Vec::new();
            for (stage, s) in [(Stage::Vertex, &vs[..]), (Stage::Fragment, &src[..])] {
                let l = lower::compile_to_ir(a, stage, a.alloc_str(s).expect("scratch"))
                    .unwrap_or_else(|e| panic!("{rel} {stage:?}: front end: {}", e.msg));
                m.push(l.module);
            }
            let v = &a.alloc_filled(1, m[0]).expect("scratch")[0];
            let f = &a.alloc_filled(1, m[1]).expect("scratch")[0];
            let tri = compile_program71(v, f, Prim::Triangles, TARGET71, a)
                .unwrap_or_else(|e| panic!("{rel} Triangles: {e:?}"));
            assert!(
                tri.fs.uses_implicit_point_line_varyings,
                "{rel}: the Triangles program takes the implicit point varyings"
            );
            let p = compile_program71(v, f, Prim::Lines, TARGET71, a).unwrap_or_else(|e| panic!("{rel} Lines: {e:?}"));
            assert!(!p.fs.uses_implicit_point_line_varyings, "{rel}: the lines variant takes no implicit varying");
            let what = |s: &str| format!("{rel} Lines {s}");
            check_stage(&what("fs"), &p.fs.code, p.fs.threads, p.fs.single_seg);
            check_stage(&what("vs"), &p.vs.code, p.vs.threads, p.vs.single_seg);
            check_stage(&what("cs"), &p.cs.code, p.cs.threads, p.cs.single_seg);

            /* Its own uniform stream: the readers declare no uniforms, so the
             * stream is constants (the TLB configuration), and the lines
             * variant carries no point coordinate transform. */
            let stream: Vec<u32> =
                p.fs.code
                    .uniforms
                    .iter()
                    .map(|u| match u.contents {
                        QUNIFORM_CONSTANT => u.data,
                        QUNIFORM_FB_PNTC_Y_SCALE | QUNIFORM_FB_PNTC_Y_OFFSET => {
                            panic!("{rel}: the lines variant carries a point coordinate transform")
                        }
                        c => panic!("{rel}: uniform contents {c:#x} not modelled here"),
                    })
                    .collect();
            let one = [1.0f32.to_bits(); LANES];
            let fixed = [FIXED_VARYING.to_bits(); LANES];
            let inputs = Inputs::Fragment(std::boxed::Box::new(FsInputs {
                w: one,
                w_centroid: one,
                z: fixed,
                x: fixed,
                y: fixed,
                back_facing: [false; LANES],
                /* smooth with W 1.0: the value of every input slot is C */
                varyings: (0..p.fs.num_inputs).map(|_| Varying { vary: [0; LANES], c: fixed }).collect(),
            }));
            let o = run71(p.fs.code.words, &stream, &inputs, &Env { threads: p.fs.threads, ..Env::default() })
                .unwrap_or_else(|e| panic!("{rel}: sim71: {e}"));
            assert!(o.msf.iter().all(|&m| m != 0), "{rel}: a lane discarded");

            /* RT0's colour: the two vfpack writes of f16 pairs, the low
             * channel first (vir_emit_tlb_color_write, nir_to_vir.c:1953-1984). */
            assert_eq!(o.tlb.len(), 2, "{rel}: two colour TLB writes");
            let mut spec = 0xff;
            let mut pairs: Vec<[u32; LANES]> = Vec::new();
            for t in &o.tlb {
                if let Some(c) = t.config {
                    spec = c & 0xff;
                }
                assert_eq!(spec, 0xff, "{rel}: TLB specifier {spec:#x} (only RT0 f16 vec4, 0xff, is read)");
                pairs.push(t.value.map(|v| v.unwrap_or_else(|| panic!("{rel}: an undefined colour in a kept lane"))));
            }
            let h = |w: u32, hi: bool| half_to_f32(if hi { (w >> 16) as u16 } else { w as u16 });
            core::array::from_fn(|l| {
                [h(pairs[0][l], false), h(pairs[0][l], true), h(pairs[1][l], false), h(pairs[1][l], true)]
            })
        })
        .expect("scratch");
        let want = format!("{expect:?}");
        for (l, c) in colour.iter().enumerate() {
            assert_eq!(format!("{c:?}"), want, "{rel}: lane {l} off a point: gl_PointCoord must read 0.0");
        }
    }
}
