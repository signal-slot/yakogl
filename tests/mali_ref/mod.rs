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
//!   for gl_ref.py's draw, a 64x64 target, and its `--uniform-values`;
//!   [`assert_matches_unset`] for a reference drawn without that option,
//!   every uniform word 0), the secondary offset and the zero padding.
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
    /// (`drm_shim_ref.sh run` as the `deqp_intval_*` references were, whose
    /// `bool` uniforms the option cannot set): every uniform word is 0.
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

    /// [`Reference::load`] of a reference drawn without `--uniform-values`.
    pub fn load_unset(name: &str) -> Reference {
        Reference { uniform_values: false, ..Reference::load(name) }
    }
}

/// One stage as the tests read it: its words and facts, copied out.
pub struct Stage {
    pub code: Vec<u64>,
    pub work_reg_count: u8,
    pub preload: u64,
    /// (offset in bytes, preload, work_reg_count) of the varying program.
    pub secondary: Option<(u32, u64, u8)>,
    /// The offset in bytes of the position program without the point size
    /// store (`MaliShader::no_psiz_offset`), which a draw of triangles runs.
    pub no_psiz: Option<u32>,
    pub fau: Vec<FauWord>,
    /// The uniform rows (name, first dword, dwords).
    pub uniforms: Vec<(String, u16, u16)>,
}

pub struct Pair {
    pub vs: Stage,
    pub fs: Stage,
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
    let mut out = ShcShader::ZERO;
    let mut log = [0 as c_char; LOG];
    let z = format!("{src}\0");
    let f = yakogl::shaderc::online::SHC_ONLINE.compile.expect("the online back end compiles");
    // SAFETY: a NUL-terminated source, a real output, and a log of the size announced.
    let rc = unsafe { f(stage, z.as_ptr() as *const c_char, &mut out, log.as_mut_ptr(), log.len()) };
    if rc == 0 {
        Ok(out)
    } else {
        Err(message(&log_text(&log)))
    }
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
                (String::from_utf8_lossy(&b).into_owned(), u.dword_off, u.dwords)
            })
            .collect()
    };
    let m = MaliShader::of(sh).expect("a Mali record");
    Stage {
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
    /* the compiler's scratch arena and record cache are process-wide, and
     * the test harness runs tests on threads: one pair at a time */
    static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());
    let _one = ONE_AT_A_TIME.lock().unwrap_or_else(|p| p.into_inner());
    yakogl::shaderc::online::backend::forget_compiled();
    let vs = compile(SHC_STAGE_VERTEX, vs_src)?;
    let mut fs = compile(SHC_STAGE_FRAGMENT, fs_src)?;
    let mut log = [0 as c_char; LOG];
    let relink = yakogl::shaderc::online::SHC_ONLINE.compile_vs_linked.expect("the online back end relinks");
    let vs_z = format!("{vs_src}\0");
    let mut out = ShcShader::ZERO;
    // SAFETY: a NUL-terminated source, no map, a valid fragment shader, a
    // real output and a log of the size announced.
    let rc = unsafe {
        relink(vs_z.as_ptr() as *const c_char, core::ptr::null(), &fs, &mut out, log.as_mut_ptr(), log.len())
    };
    let vs = match rc {
        0 => out,
        1 => vs,
        _ => return Err(message(&log_text(&log))),
    };
    let link = yakogl::shaderc::online::SHC_ONLINE.link.expect("the online back end links");
    // SAFETY: two distinct valid shaders and a log of the size announced.
    let rc = unsafe { link(&vs, &mut fs, log.as_mut_ptr(), log.len()) };
    if rc != 0 {
        return Err(log_text(&log));
    }
    Ok(Pair { vs: stage_of(&vs), fs: stage_of(&fs) })
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

/// The uniforms a program declares, in gl_ref.py's `uniforms()` order: the
/// vertex shader's, then the fragment shader's not already declared.
/// Each uniform declared, by name in declaration order, and whether its
/// type is an `int`/`ivecN` (gl_ref.py's `INT_UNIFORM_COMPONENTS`, set
/// with glUniform*iv: the value is the integer, not its float's bits).
fn uniform_names(vs: &str, fs: &str) -> Vec<(String, bool)> {
    let mut out: Vec<(String, bool)> = Vec::new();
    for text in [vs, fs] {
        let mut t = String::new();
        let mut rest = text;
        while let Some(a) = rest.find("/*") {
            t.push_str(&rest[..a]);
            rest = rest[a..].find("*/").map_or("", |b| &rest[a + b + 2..]);
        }
        t.push_str(rest);
        /* a directive (`#version 100`) is no part of the next statement */
        let t: String = t.lines().filter(|l| !l.trim_start().starts_with('#')).collect::<Vec<_>>().join("\n");
        for stmt in t.split([';', '{', '}']) {
            let w: Vec<&str> = stmt.split_whitespace().collect();
            if w.first() != Some(&"uniform") {
                continue;
            }
            let skip = if matches!(w.get(1), Some(&("lowp" | "mediump" | "highp"))) { 3 } else { 2 };
            let int = matches!(w.get(skip - 1), Some(&("int" | "ivec2" | "ivec3" | "ivec4")));
            let names = w.get(skip..).unwrap_or(&[]).join(" ");
            for name in names.split(',').map(str::trim).filter(|n| !n.is_empty()) {
                let name = name.to_string();
                if !out.iter().any(|o| o.0 == name) {
                    out.push((name, int));
                }
            }
        }
    }
    out
}

/// The value gl_ref.py's `--uniform-values` gave uniform dword `d`
/// (component c of uniform k: 16 (k + 1) + c), as fp32 bits; an array
/// declarator gets none (its location lookup is -1), so 0.
fn uniform_value(names: &[(String, bool)], rows: &[(String, u16, u16)], d: u16) -> u32 {
    let (name, first, _) = rows.iter().find(|r| d >= r.1 && d < r.1 + r.2).expect("a uniform row holds the dword");
    /* the built-in's state, glDepthRangef(0, 1) by default: near, far, diff */
    match name.as_str() {
        "gl_DepthRange.near" => return 0f32.to_bits(),
        "gl_DepthRange.far" | "gl_DepthRange.diff" => return 1f32.to_bits(),
        _ => {}
    }
    if names.iter().any(|(n, _)| n.strip_prefix(name.as_str()).is_some_and(|t| t.starts_with('['))) {
        return 0;
    }
    let k = names.iter().position(|n| &n.0 == name).expect("the uniform is declared");
    /* gl_ref.py counts a matrix's components packed; the front end strides
     * a mat3's columns to 4 dwords (its 12 dwords, no array here) */
    let (_, _, count) = rows.iter().find(|r| &r.0 == name).expect("a uniform row holds the dword");
    let c = if *count == 12 { (d - first) / 4 * 3 + (d - first) % 4 } else { d - first };
    if names[k].1 {
        return 16 * (k as u32 + 1) + c as u32;
    }
    (16.0 * (k as f32 + 1.0) + c as f32).to_bits()
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
    let mut d = Vec::new();
    let names = uniform_names(&r.vs, &r.fs);
    /* gl_ref.py sets no struct uniform: such a reference is drawn without
     * `--uniform-values`, every uniform 0 (deqp_unitype_vsstruct_*); so is
     * one loaded with [`Reference::load_unset`] (`uniform_values` false) */
    let member = |r: &(String, u16, u16)| r.0.contains('.') && !r.0.starts_with("gl_");
    let values = r.uniform_values && !pair.vs.uniforms.iter().chain(pair.fs.uniforms.iter()).any(member);
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
