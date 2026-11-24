// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Shared byte-compare of a Mali program against a Mesa 26.2.1 reference,
//! for `tests/mali_deqp_*.rs`. Include it with `mod mali_ref;` (the way
//! `tests/common` is used); the including test carries
//! `#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]`.
//!
//! A reference is `tools/mali/testdata/<name>.{vert,frag,expect.json}`: the
//! pair and pan_shc's per-region record of what Mesa's drm-shim draw gave
//! (`vs.position`, `vs.varying`, `fs.fragment`: words, `work_reg_count`,
//! `preload`, `fau`; and the top-level `secondary_offset`), the gm2_*
//! format of `tools/mali/testdata/cases.txt`.
//!
//! * [`Reference::load`] reads one; [`compile_pair`] builds a pair the way
//!   glCompileShader and glLinkProgram do on the Mali (the vertex shader
//!   again for the fragment shader with no map, `compile_vs_linked`, then
//!   `link`), as `tests/mali_glmark2_corpus.rs` does.
//! * [`assert_matches`] compares per region: the words (a differing word is
//!   listed with both disassemblies), the facts, the FAU words (computed
//!   for gl_ref.py's draw, a 64x64 target, and its `--uniform-values`),
//!   the secondary offset and the zero padding. Every `deqp_*` reference
//!   is compared this way. [`assert_matches_unset`] is for a reference of
//!   another family drawn without that option (`gm2_*`, `varymerge_*`),
//!   every uniform word 0; it refuses a `deqp_*` name.
//! * Struct uniforms: gl_ref.py's `--uniform-values` sets every struct
//!   member by its dEQP name (`u.m2[1]`, `u[2].m1`, `u.m1.ms2[0].mss1`),
//!   each the next value index ([`uniform_leaves`], `uniform_settings()`
//!   in gl_ref.py), and Mesa uploads a mediump member as an fp32 word
//!   ([`uniform_value`]). Only the references drawn since gl_ref.py could
//!   do so carry those values; which ones is not in the expect.json, so it
//!   is the explicit list [`STRUCT_VALUES`]. Any other reference whose
//!   program has a struct-member uniform row (`deqp_unitype_vsstruct_*`,
//!   drawn before) is compared against every uniform word 0.
//!   `tests/mali_ref_selftest.rs` checks the member values against
//!   gl_ref.py's for the `deqp_fclane2_*` shapes.
//! * [`assert_refused`] / [`assert_refused_ref`]: the pair still refuses,
//!   with the code generator's `MaliError` text containing a given text.
//!
//! What the public API reaches: everything above. The words are
//! `ShcShader::usc_words`, the facts are `MaliShader::of(&sh)`
//! (`work_reg_count`, `preload`, `secondary`, `fau`), the uniform rows are
//! `ShcShader::uniforms`. Not reached: the unit tests' `compile_linked`
//! path (the vertex shader's constant lanes through `vs_constants` /
//! `compile_fs_for_vs`, private functions) and the loop-cap edits
//! (`CapEdit`): a reference whose result depends on them is compared by
//! the unit tests in `src/shaderc/online/mali/tests.rs`, not here. This
//! module runs the link the GL core runs (`src/gl/program.rs`).
#![allow(dead_code)]
use core::ffi::c_char;
use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::Mutex;
use yakogl::mali::frame::{f32_to_f16, fb_wpos_y_transform};
use yakogl::shaderc::online::mali::bir::Sysval;
use yakogl::shaderc::online::mali::fau::FauWord;
use yakogl::shaderc::online::mali::isa;
use yakogl::shaderc::online::mali::MaliShader;
use yakogl::shaderc::{ShcShader, SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX};

const LOG: usize = 4096;

/// gl_ref.py's default draw: a 64x64 target, the viewport all of it, depth
/// range 0,1: scale and offset both (32, 32, 0.5).
const VIEWPORT_SCALE: [u32; 3] = [0x4200_0000, 0x4200_0000, 0x3f00_0000];
const VIEWPORT_OFFSET: [u32; 3] = [0x4200_0000, 0x4200_0000, 0x3f00_0000];

/// A reference on disk.
pub struct Reference {
    pub name: String,
    pub vs: String,
    pub fs: String,
    pub json: String,
    /// The draw set the uniforms (gl_ref.py's `--uniform-values`, the
    /// default of `tools/mali/deqp_ref.sh`): each FAU word of a uniform is
    /// compared against that value. False for a reference drawn without it
    /// ([`Reference::load_unset`]): every uniform word is 0.
    pub uniform_values: bool,
}

impl Reference {
    pub fn load(name: &str) -> Reference {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tools/mali/testdata");
        let read = |ext: &str| {
            let p = dir.join(format!("{name}.{ext}"));
            std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
        };
        Reference {
            name: name.to_string(),
            vs: read("vert"),
            fs: read("frag"),
            json: read("expect.json"),
            uniform_values: true,
        }
    }

    /// [`Reference::load`] of a reference drawn without `--uniform-values`
    /// (`drm_shim_ref.sh run`, as the `gm2_*` families are). Not for a
    /// `deqp_*` reference: those are all drawn with the option (a struct
    /// uniform's members set only for those in [`STRUCT_VALUES`], which
    /// [`differences`] sees by itself).
    pub fn load_unset(name: &str) -> Reference {
        assert!(!name.starts_with("deqp_"), "{name}: a deqp_* reference is drawn with --uniform-values; use load");
        Reference { uniform_values: false, ..Reference::load(name) }
    }
}

/// One stage as the tests read it: its words and facts, copied out.
pub struct Stage {
    /// The words, work registers, preload, FAU words and varying program,
    /// printed: the input of [`Pair::digest`].
    pub facts: String,
    pub code: Vec<u64>,
    pub work_reg_count: u8,
    pub preload: u64,
    /// (offset in bytes, preload, work_reg_count) of the varying program.
    pub secondary: Option<(u32, u64, u8)>,
    /// The offset in bytes of the position program without the point size
    /// store (`MaliShader::no_psiz_offset`), which a draw of triangles runs.
    pub no_psiz: Option<u32>,
    pub fau: Vec<FauWord>,
    /// The uniform rows (name, first dword, dwords, GL type).
    pub uniforms: Vec<(String, u16, u16, u16)>,
}

pub struct Pair {
    pub vs: Stage,
    pub fs: Stage,
}

impl Pair {
    /// The digest of what a reference compares of the program (both
    /// stages' [`Stage::facts`]): equal digests, equal programs.
    pub fn digest(&self) -> String {
        format!("{:016x}", fnv1a(format!("{}|{}", self.vs.facts, self.fs.facts).as_bytes()))
    }
}

fn log_text(log: &[c_char]) -> String {
    let s: String = log.iter().take_while(|c| **c != 0).map(|c| *c as u8 as char).collect();
    s.trim().replace('\n', " / ")
}

/// The code generator's message (`MaliError::message`) out of a compile
/// log, or the whole log when the refusal is not the code generator's.
fn message(log: &str) -> String {
    for stage in ["vertex", "fragment"] {
        let pre = format!("online shaderc: the code generator refused this {stage} shader: Mali(");
        if let Some(m) = log.strip_prefix(&pre).and_then(|m| m.strip_suffix(')')) {
            return m.to_string();
        }
    }
    log.to_string()
}

fn compile(stage: u32, src: &str) -> Result<ShcShader, String> {
    compile_rc(stage, src).map_err(|(_, log)| message(&log))
}

/// glCompileShader of `src`: Err is the entry point's return code and its
/// whole log.
fn compile_rc(stage: u32, src: &str) -> Result<ShcShader, (i32, String)> {
    let mut out = ShcShader::ZERO;
    let mut log = [0 as c_char; LOG];
    let z = format!("{src}\0");
    let f = yakogl::shaderc::online::SHC_ONLINE.compile.expect("the online back end compiles");
    // SAFETY: a NUL-terminated source, a real output, and a log of the size announced.
    let rc = unsafe { f(stage, z.as_ptr() as *const c_char, &mut out, log.as_mut_ptr(), log.len()) };
    if rc == 0 {
        Ok(out)
    } else {
        Err((rc, log_text(&log)))
    }
}

/// The FNV-1a hash of `bytes`: a digest that is the same in every build
/// (std's hasher is not), so that two measures of two trees compare.
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, &b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3))
}

fn stage_of(sh: &ShcShader) -> Stage {
    let code = sh.usc_words().chunks(2).map(|c| c[0] as u64 | (c[1] as u64) << 32).collect();
    let uniforms = if sh.uniforms.is_null() {
        Vec::new()
    } else {
        // SAFETY: the rows live in the record's keep memory, as long as `sh`.
        let rows = unsafe { core::slice::from_raw_parts(sh.uniforms, sh.num_uniforms as usize) };
        rows.iter()
            .map(|u| {
                let b: Vec<u8> = u.name.iter().map(|&b| b as u8).take_while(|&b| b != 0).collect();
                (String::from_utf8_lossy(&b).into_owned(), u.dword_off, u.dwords, u.gl_type)
            })
            .collect()
    };
    let m = MaliShader::of(sh).expect("a Mali record");
    /* what the reference compares of a stage, in the form the unit tests'
     * route measure printed it (tests.rs `route_digest`), so a digest
     * stays the same across the move of the measure to this module */
    let facts = format!("{:?}", (&code, m.work_reg_count, m.preload, m.fau, m.secondary));
    Stage {
        facts,
        code,
        work_reg_count: m.work_reg_count,
        preload: m.preload,
        secondary: m.secondary.map(|s| (s.offset, s.preload, s.work_reg_count)),
        no_psiz: m.no_psiz_offset,
        fau: m.fau.to_vec(),
        uniforms,
    }
}

/// A pair as glCompileShader and glLinkProgram build it on the Mali
/// (`src/gl/program.rs`): both shaders compile, the vertex shader is built
/// again for the fragment shader (`compile_vs_linked` with no map), then
/// the link. Err is the first refusal: the code generator's message, or the
/// log when it is not the code generator's.
pub fn compile_pair(vs_src: &str, fs_src: &str) -> Result<Pair, String> {
    route(vs_src, fs_src).program.map_err(|r| match r.step {
        Step::Link => r.log,
        _ => message(&r.log),
    })
}

/// A step of glCompileShader and glLinkProgram on the Mali.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// glCompileShader of the vertex shader (`compile`).
    Vertex,
    /// glCompileShader of the fragment shader (`compile`).
    Fragment,
    /// glLinkProgram's vertex shader for the fragment shader
    /// (`compile_vs_linked` with no map).
    VsLinked,
    /// glLinkProgram's `link`.
    Link,
}

impl Step {
    /// The step's name in a measure row (tools/mali/route.sh).
    pub fn name(self) -> &'static str {
        match self {
            Step::Vertex => "vs",
            Step::Fragment => "fs",
            Step::VsLinked => "vs_linked",
            Step::Link => "link",
        }
    }
}

/// A program refused on the route: the step, its return code and the log
/// the entry point wrote.
#[derive(Clone, Debug)]
pub struct Refusal {
    pub step: Step,
    pub rc: i32,
    pub log: String,
}

/// A program as the GL core builds it, with each stage's own compile.
pub struct Route {
    /// glCompileShader of each stage, both made whatever the other's
    /// outcome (the GL core compiles each shader object by itself): Err is
    /// the return code and the log.
    pub vs: Result<(), (i32, String)>,
    pub fs: Result<(), (i32, String)>,
    /// The program: Err is the first refusal in the GL core's order (the
    /// vertex compile, the fragment compile, `compile_vs_linked`, `link`).
    pub program: Result<Pair, Refusal>,
}

/// The program of `vs_src` and `fs_src` through the back end's entry
/// points in the GL core's order ([`compile_pair`]), keeping each step.
pub fn route(vs_src: &str, fs_src: &str) -> Route {
    /* the compiler's scratch arena and record cache are process-wide, and
     * the test harness runs tests on threads: one pair at a time */
    static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());
    let _one = ONE_AT_A_TIME.lock().unwrap_or_else(|p| p.into_inner());
    yakogl::shaderc::online::backend::forget_compiled();
    let v = compile_rc(SHC_STAGE_VERTEX, vs_src);
    let f = compile_rc(SHC_STAGE_FRAGMENT, fs_src);
    let own = |r: &Result<ShcShader, (i32, String)>| r.as_ref().map(|_| ()).map_err(Clone::clone);
    let (vs_own, fs_own) = (own(&v), own(&f));
    let refused = |step, (rc, log)| Refusal { step, rc, log };
    let program = match (v, f) {
        (Err(e), _) => Err(refused(Step::Vertex, e)),
        (_, Err(e)) => Err(refused(Step::Fragment, e)),
        (Ok(vs), Ok(mut fs)) => link_pair(vs_src, vs, &mut fs).map(|vs| Pair { vs: stage_of(&vs), fs: stage_of(&fs) }),
    };
    Route { vs: vs_own, fs: fs_own, program }
}

/// glLinkProgram's two steps: the vertex shader for the fragment shader,
/// then the link, which may replace `fs`. Ok is the program's vertex shader.
fn link_pair(vs_src: &str, vs: ShcShader, fs: &mut ShcShader) -> Result<ShcShader, Refusal> {
    let mut log = [0 as c_char; LOG];
    let relink = yakogl::shaderc::online::SHC_ONLINE.compile_vs_linked.expect("the online back end relinks");
    let vs_z = format!("{vs_src}\0");
    let mut out = ShcShader::ZERO;
    // SAFETY: a NUL-terminated source, no map, a valid fragment shader, a
    // real output and a log of the size announced.
    let rc = unsafe {
        relink(vs_z.as_ptr() as *const c_char, core::ptr::null(), &*fs, &mut out, log.as_mut_ptr(), log.len())
    };
    let vs = match rc {
        0 => out,
        1 => vs,
        _ => return Err(Refusal { step: Step::VsLinked, rc, log: log_text(&log) }),
    };
    let link = yakogl::shaderc::online::SHC_ONLINE.link.expect("the online back end links");
    // SAFETY: two distinct valid shaders and a log of the size announced.
    let rc = unsafe { link(&vs, fs, log.as_mut_ptr(), log.len()) };
    if rc != 0 {
        return Err(Refusal { step: Step::Link, rc, log: log_text(&log) });
    }
    Ok(vs)
}

/* ---- the reference's side --------------------------------------------- */

#[derive(Debug, Default)]
struct Region {
    words: Vec<u64>,
    work_reg_count: u8,
    preload: u64,
    fau: Option<Vec<u32>>,
}

fn after<'a>(s: &'a str, key: &str) -> Option<&'a str> {
    let k = format!("\"{key}\":");
    s.find(&k).map(|i| s[i + k.len()..].trim_start())
}

fn hex_array(s: &str) -> Option<Vec<u64>> {
    if s.starts_with("null") {
        return None;
    }
    let end = s.find(']')?;
    Some(s[1..end].split(',').filter_map(|x| u64::from_str_radix(x.trim().trim_matches('"'), 16).ok()).collect())
}

fn region(json: &str, name: &str) -> Option<Region> {
    let r = after(json, name)?;
    let end = r.find("\n  }").unwrap_or(r.len());
    let r = &r[..end];
    let num = |k: &str| after(r, k).map(|t| t.split(|c: char| !c.is_ascii_alphanumeric()).next().unwrap_or(""));
    Some(Region {
        words: hex_array(after(r, "words")?)?,
        work_reg_count: num("work_reg_count")?.parse().ok()?,
        preload: u64::from_str_radix(after(r, "preload")?.trim_matches('"').get(2..18)?, 16).ok()?,
        fau: hex_array(after(r, "fau")?).map(|v| v.into_iter().map(|w| w as u32).collect()),
    })
}

fn secondary_offset(json: &str) -> Option<u32> {
    after(json, "secondary_offset")?.split(',').next()?.trim().parse().ok()
}

/* ---- comparison -------------------------------------------------------- */

fn trim(w: &[u64]) -> Vec<u64> {
    let mut v = w.to_vec();
    while v.last() == Some(&0) {
        v.pop();
    }
    v
}

fn describe(w: u64) -> String {
    let mut s = String::new();
    match isa::describe_word(w, &mut s) {
        Ok(()) => s,
        Err(e) => format!("{e:?}"),
    }
}

/// None when equal; else every differing word with both disassemblies.
pub fn word_diff(name: &str, ours: &[u64], theirs: &[u64]) -> Option<String> {
    if ours == theirs {
        return None;
    }
    let mut m = format!("{name}: {} words ours, {} reference\n", ours.len(), theirs.len());
    for i in 0..ours.len().max(theirs.len()) {
        let (o, t) = (ours.get(i), theirs.get(i));
        let mark = if o == t { ' ' } else { '*' };
        let _ = writeln!(
            m,
            "{mark}[{i:2}] ours {:<18} {}\n      ref  {:<18} {}",
            o.map_or("(none)".into(), |w| format!("{w:016x}")),
            o.map_or(String::new(), |w| describe(*w)),
            t.map_or("(none)".into(), |w| format!("{w:016x}")),
            t.map_or(String::new(), |w| describe(*w)),
        );
    }
    Some(m)
}

/// A program's text without comments (`/* */` and `//`, as gl_ref.py's
/// `strip_comments`).
fn strip_comments(text: &str) -> String {
    let mut t = String::new();
    let mut rest = text;
    while let Some(a) = rest.find("/*") {
        t.push_str(&rest[..a]);
        rest = rest[a..].find("*/").map_or("", |b| &rest[a + b + 2..]);
    }
    t.push_str(rest);
    t.lines().map(|l| l.find("//").map_or(l, |i| &l[..i])).collect::<Vec<_>>().join("\n")
}

/// The `#define NAME <int>` macros of a text (gl_ref.py's `_scan_defines`).
fn defines(text: &str) -> Vec<(String, u32)> {
    let mut out = Vec::new();
    for l in text.lines() {
        let l = l.trim_start();
        let Some(rest) = l.strip_prefix('#') else { continue };
        let w: Vec<&str> = rest.split_whitespace().collect();
        if let ["define", name, value] = w[..] {
            if let Ok(v) = value.trim_matches(['(', ')']).parse() {
                out.push((name.to_string(), v));
            }
        }
    }
    out
}

/// An array size (gl_ref.py's `_resolve_size`): a literal, a `#define`d
/// integer, or a sum of those; None when not an array.
fn array_size(tok: Option<&str>, defs: &[(String, u32)]) -> Option<u32> {
    let tok = tok?.trim();
    if tok.contains('+') {
        return Some(tok.split('+').map(|t| array_size(Some(t), defs).expect("an array size gl_ref.py reads")).sum());
    }
    tok.parse().ok().or_else(|| defs.iter().find(|d| d.0 == tok).map(|d| d.1))
}

/// `name` or `name[size]`, split.
fn declarator(decl: &str) -> (&str, Option<&str>) {
    match decl.find('[') {
        Some(i) => (decl[..i].trim(), Some(decl[i + 1..].trim_end().trim_end_matches(']'))),
        None => (decl.trim(), None),
    }
}

/// A named struct: its name and its members (name, type, array size or
/// None) in declaration order.
type Struct = (String, Vec<(String, String, Option<u32>)>);

/// The named structs of the texts (gl_ref.py's `struct_types()`), the first
/// definition of a name winning.
fn struct_types(texts: &[&str]) -> Vec<Struct> {
    let mut out: Vec<Struct> = Vec::new();
    for text in texts {
        let t = strip_comments(text);
        let defs = defines(&t);
        let mut rest = t.as_str();
        while let Some(i) = rest.find("struct") {
            let before_ok = i == 0 || !rest.as_bytes()[i - 1].is_ascii_alphanumeric() && rest.as_bytes()[i - 1] != b'_';
            rest = &rest[i + "struct".len()..];
            let (Some(open), true) = (rest.find('{'), before_ok) else { continue };
            let name = rest[..open].trim();
            let Some(close) = rest[open..].find('}') else { break };
            let body = &rest[open + 1..open + close];
            if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') || body.contains('{') {
                continue;
            }
            let mut members = Vec::new();
            for stmt in body.split(';').map(str::trim).filter(|s| !s.is_empty()) {
                let mut w = stmt.split_whitespace().peekable();
                if w.peek().is_some_and(|p| matches!(*p, "lowp" | "mediump" | "highp")) {
                    w.next();
                }
                let ty = w.next().unwrap_or("").to_string();
                for decl in w.collect::<Vec<_>>().join(" ").split(',') {
                    let (m, size) = declarator(decl);
                    members.push((m.to_string(), ty.clone(), array_size(size, &defs)));
                }
            }
            if !out.iter().any(|s| s.0 == name) {
                out.push((name.to_string(), members));
            }
            rest = &rest[open + close..];
        }
    }
    out
}

const SAMPLERS: &[&str] = &["sampler2D", "samplerCube", "sampler3D", "sampler2DShadow", "samplerExternalOES"];

/// The uniforms gl_ref.py's `--uniform-values` numbers, in its
/// `uniform_settings()` order, as (GL name, type): value index k of the
/// k-th entry. The uniforms are those of `uniforms()`, the vertex shader's,
/// then the fragment shader's not already declared; a sampler keeps its
/// place; a struct uniform is its members (`uniform_leaves()`), depth first
/// in declaration order, each by the name dEQP sets it with: `u.m0`, an
/// array member element by element `u.m2[0]`, an array of structs element
/// by element `u[0].m0`, a nested struct `u.m1.ms0`. Any other array
/// declarator stays one entry, `u[3]` (its location lookup is -1, so it is
/// never set).
pub fn uniform_leaves(vs: &str, fs: &str) -> Vec<(String, String)> {
    let structs = struct_types(&[vs, fs]);
    let defs = defines(&(strip_comments(vs) + "\n" + &strip_comments(fs)));
    let mut decls: Vec<(String, String)> = Vec::new();
    for text in [vs, fs] {
        /* a directive (`#version 100`) is no part of the next statement */
        let t = strip_comments(text);
        let t: String = t.lines().filter(|l| !l.trim_start().starts_with('#')).collect::<Vec<_>>().join("\n");
        for stmt in t.split([';', '{', '}']) {
            let w: Vec<&str> = stmt.split_whitespace().collect();
            if w.first() != Some(&"uniform") {
                continue;
            }
            let skip = if matches!(w.get(1), Some(&("lowp" | "mediump" | "highp"))) { 3 } else { 2 };
            let ty = w.get(skip - 1).copied().unwrap_or("");
            let names = w.get(skip..).unwrap_or(&[]).join(" ");
            for name in names.split(',').map(str::trim).filter(|n| !n.is_empty()) {
                if !decls.iter().any(|d| d.0 == name) {
                    decls.push((name.to_string(), ty.to_string()));
                }
            }
        }
    }
    fn expand(out: &mut Vec<(String, String)>, structs: &[Struct], name: &str, ty: &str, size: Option<u32>) {
        let elements: Vec<String> = match size {
            None => vec![name.to_string()],
            Some(n) => (0..n).map(|i| format!("{name}[{i}]")).collect(),
        };
        for e in elements {
            match structs.iter().find(|s| s.0 == ty) {
                Some((_, members)) => {
                    for (m, mty, msize) in members {
                        expand(out, structs, &format!("{e}.{m}"), mty, *msize);
                    }
                }
                None => out.push((e, ty.to_string())),
            }
        }
    }
    let mut out = Vec::new();
    for (decl, ty) in decls {
        if SAMPLERS.contains(&ty.as_str()) || !structs.iter().any(|s| s.0 == ty) {
            out.push((decl, ty));
            continue;
        }
        let (name, size) = declarator(&decl);
        expand(&mut out, &structs, name, &ty, array_size(size, &defs));
    }
    out
}

/// The references drawn with their struct members set: gl_ref.py's
/// `--uniform-values` since item fclane2-struct of plan #387. Every other
/// reference with a struct-member uniform row was drawn before gl_ref.py
/// could set one (`deqp_unitype_vsstruct_*`, `deqp_uniarray_struct*`,
/// `deqp_unif2_*struct*`, ...): its words are compared against every
/// uniform 0. A reference redrawn with the members set joins this list in
/// the commit that redraws it.
pub const STRUCT_VALUES: &[&str] = &[
    "deqp_boolpix_lk_uniform_struct",
    "deqp_boolpix_lk_uniform_struct_fragment_only_vec4",
    "deqp_boolpix_lk_uniform_struct_partial",
    "deqp_boolpix_lk_uniform_struct_use_case_rip",
    "deqp_boolpix_lk_uniform_struct_vec4",
    "deqp_fclane2_struct_arrmember_float",
    "deqp_fclane2_struct_array_float",
    "deqp_fclane2_struct_basic_bool",
    "deqp_fclane2_struct_basic_float",
    "deqp_fclane2_struct_basic_int",
    "deqp_fclane2_struct_nested_float",
];

/// Dwords of one element of an array row of GL type `ty` (the front end's
/// std430 stride, `reflect::type_layout`: a vec3 pads to 4, a mat3's
/// columns to 4).
fn element_dwords(ty: u32) -> u16 {
    use yakogl::gl::consts as gl;
    match ty {
        gl::GL_FLOAT | gl::GL_INT | gl::GL_BOOL => 1,
        gl::GL_FLOAT_VEC2 | gl::GL_INT_VEC2 | gl::GL_BOOL_VEC2 => 2,
        gl::GL_FLOAT_MAT3 => 12,
        gl::GL_FLOAT_MAT4 => 16,
        _ => 4,
    }
}

/// The value gl_ref.py's `--uniform-values` gave uniform dword `d`
/// (`uniform_components()`: component c of value index k is 16 (k + 1) +
/// c, k the place of the uniform's entry in `names`, [`uniform_leaves`]),
/// as the word Mesa uploads: fp32 bits (a mediump struct member too), the
/// integer of an `int` / `ivecN`, and of a `bool` / `bvecN` (true where c
/// is even) Mesa's true, all ones, or 0. A row named as an entry is that
/// entry; a struct member's array row (`u.m2`) is its elements' entries
/// (`u.m2[0]`, `u.m2[1]`, ...). Any other array declarator gets none (its
/// location lookup is -1), so 0.
pub fn uniform_value(names: &[String], rows: &[(String, u16, u16, u16)], d: u16) -> u32 {
    use yakogl::gl::consts as gl;
    let (name, first, count, ty) =
        rows.iter().find(|r| d >= r.1 && d < r.1 + r.2).expect("a uniform row holds the dword");
    /* the built-in's state, glDepthRangef(0, 1) by default: near, far, diff */
    match name.as_str() {
        "gl_DepthRange.near" => return 0f32.to_bits(),
        "gl_DepthRange.far" | "gl_DepthRange.diff" => return 1f32.to_bits(),
        _ => {}
    }
    let member = name.contains('.');
    let stride = element_dwords(u32::from(*ty));
    let (k, at) = if let Some(k) = names.iter().position(|n| n == name) {
        (k, d - first)
    } else if let Some(k) = names.iter().position(|n| member && *n == format!("{name}[{}]", (d - first) / stride)) {
        (k, (d - first) % stride)
    } else if !member && names.iter().any(|n| n.strip_prefix(name.as_str()).is_some_and(|t| t.starts_with('['))) {
        return 0;
    } else {
        panic!("{name}: the uniform is declared ({names:?})");
    };
    /* gl_ref.py counts a matrix's components packed; the front end strides
     * a mat3's columns to 4 dwords (its 12 dwords) */
    let mat3 = u32::from(*ty) == gl::GL_FLOAT_MAT3 || *count == 12 && !member;
    let c = if mat3 { at / 4 * 3 + at % 4 } else { at };
    let value = 16 * (k as u32 + 1) + u32::from(c);
    match u32::from(*ty) {
        gl::GL_INT | gl::GL_INT_VEC2 | gl::GL_INT_VEC3 | gl::GL_INT_VEC4 => value,
        gl::GL_BOOL | gl::GL_BOOL_VEC2 | gl::GL_BOOL_VEC3 | gl::GL_BOOL_VEC4 => {
            if c % 2 == 0 {
                u32::MAX
            } else {
                0
            }
        }
        _ => (value as f32).to_bits(),
    }
}

fn fau_values(f: &[FauWord], uv: &dyn Fn(u16) -> u32) -> Vec<u32> {
    let mut v: Vec<u32> = f
        .iter()
        .map(|w| match *w {
            FauWord::Sysval(Sysval::ViewportScale, c) => VIEWPORT_SCALE[c as usize],
            FauWord::Sysval(Sysval::ViewportOffset, c) => VIEWPORT_OFFSET[c as usize],
            FauWord::BlendConstant(_) => 0,
            FauWord::Constant(c) => c,
            FauWord::Uniform { dword } => uv(dword),
            FauWord::UniformHalves { lo, hi } => {
                let h = |d: Option<u16>| d.map_or(0, |d| f32_to_f16(uv(d)) as u32);
                h(Some(lo)) | h(hi) << 16
            }
            /* gl_ref.py draws into a pbuffer, which Mesa does not flip */
            FauWord::FbWposYTransform(c) => {
                let height = (2.0 * f32::from_bits(VIEWPORT_OFFSET[1])) as u32;
                fb_wpos_y_transform(false, height, usize::from(c)).to_bits()
            }
        })
        .collect();
    if v.len() % 2 == 1 {
        v.push(0);
    }
    v
}

fn facts(
    out: &mut Vec<String>,
    name: &str,
    r: &Region,
    wrc: u8,
    preload: u64,
    fau: &[FauWord],
    uv: &dyn Fn(u16) -> u32,
) {
    if wrc != r.work_reg_count {
        out.push(format!("{name}: work_reg_count ours {wrc}, reference {}", r.work_reg_count));
    }
    if preload != r.preload {
        out.push(format!("{name}: preload ours {preload:#x}, reference {:#x}", r.preload));
    }
    let ours = fau_values(fau, uv);
    if Some(&ours) != r.fau.as_ref() {
        out.push(format!("{name}: FAU words ours {ours:08x?}, reference {:08x?}", r.fau));
    }
}

fn padded(words: usize) -> usize {
    (words * 8 + 16).div_ceil(128) * 16
}

/// Every difference of `pair` (built from `r`'s sources) against `r`'s
/// expect.json, one entry each; empty when the words and facts are equal.
pub fn differences(r: &Reference, pair: &Pair) -> Vec<String> {
    differences_with(r, pair, STRUCT_VALUES.contains(&r.name.as_str()))
}

/// [`differences`], with `members` saying whether the draw set the struct
/// members' values (gl_ref.py's `--uniform-values` today does; a committed
/// reference did only if it is in [`STRUCT_VALUES`]). The measure
/// ([`measure`]) passes true: tools/mali/corpus_check.py draws with today's
/// gl_ref.py, and its rule `either` falls back to every uniform 0 for an
/// older reference.
pub fn differences_with(r: &Reference, pair: &Pair, members: bool) -> Vec<String> {
    let mut d = Vec::new();
    let names: Vec<String> = uniform_leaves(&r.vs, &r.fs).into_iter().map(|l| l.0).collect();
    /* a reference with a struct-member uniform row is compared with
     * gl_ref.py's member values when it is in [`STRUCT_VALUES`], else
     * against every uniform 0 (drawn before gl_ref.py set members:
     * deqp_unitype_vsstruct_*); so is one loaded with
     * [`Reference::load_unset`] (`uniform_values` false) */
    let member = |r: &(String, u16, u16, u16)| r.0.contains('.') && !r.0.starts_with("gl_");
    let values = r.uniform_values && (members || !pair.vs.uniforms.iter().chain(pair.fs.uniforms.iter()).any(member));
    let uv_v = |x: u16| if values { uniform_value(&names, &pair.vs.uniforms, x) } else { 0 };
    let uv_f = |x: u16| if values { uniform_value(&names, &pair.fs.uniforms, x) } else { 0 };

    /* the vertex stage: position, then varying (empty when none). A
     * shader that writes gl_PointSize has two position programs, with the
     * store and, after it, without (`no_psiz`): gl_ref.py draws triangles,
     * so the reference's `vs.position` is the second (deqp_varylink_draw_*,
     * va_multi3), and its `secondary_offset` counts from that program */
    let end = pair.vs.secondary.map_or(pair.vs.code.len(), |(off, ..)| off as usize / 8);
    let start = pair.vs.no_psiz.map_or(0, |off| off as usize / 8);
    let with_psiz = trim(&pair.vs.code[..if start == 0 { end } else { start }]);
    let pos = trim(&pair.vs.code[start..end]);
    let var = trim(&pair.vs.code[end..]);
    let rp = region(&r.json, "vs.position").expect("the reference has vs.position");
    d.extend(word_diff("vs.position", &pos, &rp.words));
    facts(&mut d, "vs.position", &rp, pair.vs.work_reg_count, pair.vs.preload, &pair.vs.fau, &uv_v);
    match region(&r.json, "vs.varying") {
        Some(rv) => {
            d.extend(word_diff("vs.varying", &var, &rv.words));
            match pair.vs.secondary {
                Some((off, preload, wrc)) => {
                    facts(&mut d, "vs.varying", &rv, wrc, preload, &pair.vs.fau, &uv_v);
                    let off = off - (8 * start) as u32;
                    if Some(off) != secondary_offset(&r.json) {
                        d.push(format!("secondary_offset ours {off}, reference {:?}", secondary_offset(&r.json)));
                    }
                }
                None => d.push("vs.varying: we have no varying program".into()),
            }
        }
        None => {
            if !var.is_empty() || pair.vs.secondary.is_some() {
                d.push("vs.varying: we have a varying program, the reference has none".into());
            }
            if secondary_offset(&r.json).is_some() {
                d.push("secondary_offset: the reference has one without a vs.varying".into());
            }
        }
    }
    let want = if start == 0 { 0 } else { padded(with_psiz.len()) }
        + padded(pos.len())
        + if var.is_empty() { 0 } else { padded(var.len()) };
    if pair.vs.code.len() != want && rp.words == pos {
        d.push(format!("vs: {} words with padding, expected {want}", pair.vs.code.len()));
    }

    /* the fragment stage */
    let rf = region(&r.json, "fs.fragment").expect("the reference has fs.fragment");
    let fw = trim(&pair.fs.code);
    d.extend(word_diff("fs.fragment", &fw, &rf.words));
    facts(&mut d, "fs.fragment", &rf, pair.fs.work_reg_count, pair.fs.preload, &pair.fs.fau, &uv_f);
    if pair.fs.code.len() != padded(rf.words.len()) && rf.words == fw {
        d.push(format!("fs: {} words with padding, expected {}", pair.fs.code.len(), padded(rf.words.len())));
    }
    d
}

/// The reference's pair compiles and is byte-equal to its expect.json in
/// every region; the panic text lists each difference.
pub fn assert_matches(name: &str) {
    assert_matches_ref(&Reference::load(name));
}

/// [`assert_matches`] of a reference drawn without `--uniform-values`
/// ([`Reference::load_unset`]).
pub fn assert_matches_unset(name: &str) {
    assert_matches_ref(&Reference::load_unset(name));
}

fn assert_matches_ref(r: &Reference) {
    let name = &r.name;
    let pair = compile_pair(&r.vs, &r.fs).unwrap_or_else(|e| panic!("{name}: the pair is refused: {e}"));
    let d = differences(r, &pair);
    assert!(d.is_empty(), "{name}: differs from its reference:\n{}", d.join("\n"));
}

/// `vs`+`fs` is refused, and the refusal text contains `text` (a
/// `MaliError::message`, e.g. "a loop in a vertex shader").
pub fn assert_refused(vs: &str, fs: &str, text: &str) {
    match compile_pair(vs, fs) {
        Ok(_) => panic!("expected a refusal containing {text:?}, but the pair compiles"),
        Err(e) => assert!(e.contains(text), "expected a refusal containing {text:?}, got: {e}"),
    }
}

/// [`assert_refused`] of a reference's sources.
pub fn assert_refused_ref(name: &str, text: &str) {
    let r = Reference::load(name);
    assert_refused(&r.vs, &r.fs, text);
}

/// What [`assert_refused_ref`] would panic with, or None: a pin table checks
/// every reference and names each one that moved at once.
pub fn refusal_change(name: &str, text: &str) -> Option<String> {
    let r = Reference::load(name);
    match compile_pair(&r.vs, &r.fs) {
        Ok(_) => Some(format!("{name}: now compiles; remove its #[ignore] and its pin")),
        Err(e) if e.contains(text) => None,
        Err(e) => Some(format!("{name}: expected a refusal containing {text:?}, got: {e}")),
    }
}

/* ---- the measure: a list of programs, one row each ---------------------- */

/// How a measure reads a reference's uniform words.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rule {
    /// The draw set the uniforms (`--uniform-values`, every `deqp_*`
    /// reference and every one tools/mali/corpus_check.py makes):
    /// [`differences`] as [`assert_matches`] reads it.
    Values,
    /// A directory of references of both kinds (tools/mali/testdata): a
    /// stage is equal when it is equal under `--uniform-values` or with
    /// every uniform 0 ([`assert_matches_unset`]). The words, the facts and
    /// the FAU layout are the same under both; only the uniform values
    /// differ.
    Either,
}

impl Rule {
    pub fn parse(s: &str) -> Option<Rule> {
        match s {
            "values" => Some(Rule::Values),
            "either" => Some(Rule::Either),
            _ => None,
        }
    }
}

/// The stage a difference of [`differences`] is in: the vertex program's
/// regions and the secondary offset, or the fragment program.
fn stage_of_difference(d: &str) -> &'static str {
    if d.starts_with("fs") {
        "fs"
    } else {
        "vs"
    }
}

/// The first line of a panic's message.
fn panic_line(e: &(dyn std::any::Any + Send)) -> String {
    let m = e.downcast_ref::<String>().cloned().or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string()));
    m.unwrap_or_default().lines().next().unwrap_or("").to_string()
}

/// One stage's verdict in a measure row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StageVerdict {
    /// The program compiled and this stage is the reference's.
    Equal,
    /// The program compiled and this stage is not the reference's: its
    /// first difference.
    Differ(String),
    /// This stage was refused (its own compile, or the link step whose log
    /// names it): the step, the return code and the log.
    Refused(String),
    /// This stage compiled; the program was refused at the other's step.
    Blocked,
    /// The program compiled; there is no reference to compare it with.
    NoRef,
    /// The comparison itself failed (a reference this module cannot read):
    /// the panic's first line.
    Error(String),
}

impl StageVerdict {
    fn fields(&self) -> (&'static str, String) {
        match self {
            StageVerdict::Equal => ("EQUAL", "-".into()),
            StageVerdict::Differ(d) => ("DIFFER", d.clone()),
            StageVerdict::Refused(d) => ("REFUSED", d.clone()),
            StageVerdict::Blocked => ("BLOCKED", "-".into()),
            StageVerdict::NoRef => ("NOREF", "-".into()),
            StageVerdict::Error(d) => ("ERROR", d.clone()),
        }
    }
}

/// The measure of one program: the route's verdict (EQUAL, DIFFER,
/// REFUSED or NOREF, as tools/mali/route.sh prints it), the digest of
/// what it compiled, the route's detail, and each stage's verdict.
pub struct Measure {
    pub verdict: &'static str,
    pub digest: Option<String>,
    pub detail: String,
    pub vs: StageVerdict,
    pub fs: StageVerdict,
}

/// Program `vs`, `fs` on the route ([`route`]) against the reference
/// `json` (None: no reference) under `rule`.
pub fn measure(vs: &str, fs: &str, json: Option<&str>, rule: Rule) -> Measure {
    let r = route(vs, fs);
    let refused = |step: Step, rc: i32, log: &str| format!("{} ({rc}): {log}", step.name());
    let pair = match r.program {
        Err(e) => {
            let text = refused(e.step, e.rc, &e.log);
            /* a link step's log names the stage the code generator refused */
            let stage = match e.step {
                Step::Vertex => "vs",
                Step::Fragment => "fs",
                _ if e.log.contains("refused this fragment shader") => "fs",
                _ if e.log.contains("refused this vertex shader") => "vs",
                Step::VsLinked => "vs",
                Step::Link => "both",
            };
            let own = |o: &Result<(), (i32, String)>, step: Step, me: &str| match o {
                Err((rc, log)) => StageVerdict::Refused(refused(step, *rc, log)),
                Ok(()) if stage == me || stage == "both" => StageVerdict::Refused(text.clone()),
                Ok(()) => StageVerdict::Blocked,
            };
            let (v, f) = (own(&r.vs, Step::Vertex, "vs"), own(&r.fs, Step::Fragment, "fs"));
            return Measure { verdict: "REFUSED", digest: None, detail: text, vs: v, fs: f };
        }
        Ok(p) => p,
    };
    let digest = Some(pair.digest());
    let Some(json) = json else {
        return Measure {
            verdict: "NOREF",
            digest,
            detail: "-".into(),
            vs: StageVerdict::NoRef,
            fs: StageVerdict::NoRef,
        };
    };
    let reference = |uniform_values| Reference {
        name: String::new(),
        vs: vs.to_string(),
        fs: fs.to_string(),
        json: json.to_string(),
        uniform_values,
    };
    let diff = |uniform_values| {
        let r = reference(uniform_values);
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| differences_with(&r, &pair, true)))
            .map_err(|e| panic_line(&*e))
    };
    let first = diff(true);
    let second = if rule == Rule::Either { Some(diff(false)) } else { None };
    let stage = |me: &str| -> StageVerdict {
        let of = |d: &Result<Vec<String>, String>| -> Result<Vec<String>, String> {
            d.clone().map(|v| v.into_iter().filter(|x| stage_of_difference(x) == me).collect())
        };
        let a = of(&first);
        if matches!(&a, Ok(v) if v.is_empty()) {
            return StageVerdict::Equal;
        }
        if let Some(b) = second.as_ref().map(of) {
            if matches!(&b, Ok(v) if v.is_empty()) {
                return StageVerdict::Equal;
            }
        }
        match a {
            Ok(v) => {
                /* the first word diff of the stage when there is one (a
                 * region's FAU table is the stage's, so a varying
                 * program's other words show as the position region's
                 * FAU first): its count line, and the index of the first
                 * word that differs (`*[ i]`) */
                let first = v.iter().find(|d| d.lines().next().is_some_and(|l| l.contains(" words ours, ")));
                let mut lines = first.unwrap_or(&v[0]).lines();
                let head = lines.next().unwrap_or("").to_string();
                match lines.find(|l| l.starts_with('*')).and_then(|l| l.split(']').next()) {
                    Some(at) => StageVerdict::Differ(format!("{head}, first at word {}", at[2..].trim())),
                    None => StageVerdict::Differ(head),
                }
            }
            Err(e) => StageVerdict::Error(e),
        }
    };
    let (v, f) = (stage("vs"), stage("fs"));
    let (verdict, detail) = match (&v, &f) {
        (StageVerdict::Equal, StageVerdict::Equal) => ("EQUAL", "-".to_string()),
        (StageVerdict::Equal, d) => ("DIFFER", format!("fs: {}", d.fields().1)),
        (d, _) => ("DIFFER", format!("vs: {}", d.fields().1)),
    };
    Measure { verdict, digest, detail, vs: v, fs: f }
}

/// One measure row, tab-separated: the key, the route's verdict, the
/// digest (`-` when refused), the route's detail, then the vertex and the
/// fragment stage's verdict and detail. The first four fields are
/// tools/mali/route.sh's row.
pub fn measure_row(key: &str, m: &Measure) -> String {
    let flat = |s: &str| s.replace(['\t', '\n', '\r'], " ");
    let (vv, vd) = m.vs.fields();
    let (fv, fd) = m.fs.fields();
    format!(
        "{key}\t{}\t{}\t{}\t{vv}\t{}\t{fv}\t{}",
        m.verdict,
        m.digest.as_deref().unwrap_or("-"),
        flat(&m.detail),
        flat(&vd),
        flat(&fd)
    )
}

/// The measure of a list file, as tools/mali/route.sh and
/// tools/mali/corpus_check.py run it: each line `key<TAB>vs path<TAB>fs
/// path<TAB>expect.json path or -<TAB>values|either`; per line `begin<TAB>key`
/// before the compile (so that a driver can name a program that crashed
/// the process) and `measure<TAB>`[`measure_row`] after it, on stdout.
/// `shard` = (k, n) takes every n-th line from the k-th.
pub fn measure_list(list: &std::path::Path, shard: (usize, usize)) {
    /* a deep shader (dEQP's generated ones) recurses past a test thread's
     * 2 MiB in a debug build: the measure runs on a thread of its own */
    let list = list.to_path_buf();
    std::thread::Builder::new()
        .stack_size(256 << 20)
        .spawn(move || measure_list_on(&list, shard))
        .expect("the measure's thread")
        .join()
        .unwrap_or_else(|e| std::panic::resume_unwind(e));
}

fn measure_list_on(list: &std::path::Path, shard: (usize, usize)) {
    use std::io::Write as _;
    let text = std::fs::read_to_string(list).unwrap_or_else(|e| panic!("{}: {e}", list.display()));
    let out = std::io::stdout();
    for line in text.lines().filter(|l| !l.is_empty()).skip(shard.0).step_by(shard.1.max(1)) {
        let f: Vec<&str> = line.split('\t').collect();
        assert!(f.len() == 5, "{}: not a measure line: {line}", list.display());
        let rule = Rule::parse(f[4]).unwrap_or_else(|| panic!("{}: not a rule: {}", list.display(), f[4]));
        let read = |p: &str| std::fs::read_to_string(p).unwrap_or_else(|e| panic!("{p}: {e}"));
        let (vs, fs) = (read(f[1]), read(f[2]));
        let json = (f[3] != "-").then(|| read(f[3]));
        {
            let mut o = out.lock();
            let _ = writeln!(o, "begin\t{}", f[0]);
            let _ = o.flush();
        }
        let m = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| measure(&vs, &fs, json.as_deref(), rule)))
            .unwrap_or_else(|e| {
                let text = format!("panic: {}", panic_line(&*e));
                Measure {
                    verdict: "REFUSED",
                    digest: None,
                    detail: text.clone(),
                    vs: StageVerdict::Refused(text.clone()),
                    fs: StageVerdict::Refused(text),
                }
            });
        let mut o = out.lock();
        let _ = writeln!(o, "measure\t{}", measure_row(f[0], &m));
        let _ = o.flush();
    }
}
