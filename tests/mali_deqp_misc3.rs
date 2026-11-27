// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Byte-compare of the Mali online compiler against the Mesa 26.2.1
//! references `tools/mali/testdata/deqp_misc3_*` (plan #387, item
//! board-baseline-rows; `docs/notes/mali-deqp-misc2.md` section 6): the
//! `attribute_location.bind_aliasing` shapes, two attributes bound to one
//! location. `cond_{float,vec2,vec3,vec4,mat2,mat3,mat4}` (`a_0` of the type
//! and `a_1` a vec4, both bound to location 1, selected by `u_A`) and
//! `max_cond_mat4` (eight mat4 on four locations) are dEQP's shaders; the
//! `hp_*` ones are the 17 shapes of the group (the `_offset_1` ones and every
//! `max_cond_*` too) with the vertex shader highp, so that the words check
//! the attribute layout at the link's locations alone (#11388,
//! `docs/notes/mali-attribute-records.md`). Mesa draws each with the
//! `.bind` file's bindings (`gl_ref.py --attrib-bindings`), so each is
//! compiled the way glBindAttribLocation + glLinkProgram compile it
//! ([`compile_bound`]), as `tests/mali_deqp_defects.rs` does for its
//! aliasing references.
//!
//! All compile. The ones that differ are ignored with the difference quoted:
//! dEQP's mediump vertex arithmetic is fp32 here (#4536), and in 13 of the
//! highp ones the first `FCMP` comes after the first `LD_ATTR_IMM` (Mesa:
//! before it; the same with one attribute and no aliasing). The lowering
//! removes the `ignore` and moves the reference from `REFUSALS` to
//! `MATCHED`; `refusals_pinned` runs now and names every one that moved.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_misc3
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

use core::ffi::c_char;
use std::sync::Mutex;
use yakogl::shaderc::online::mali::MaliShader;
use yakogl::shaderc::online::SHC_ONLINE;
use yakogl::shaderc::{
    ShcAttribBinding, ShcAttribMap, ShcShader, SHC_MAX_DECLARED_ATTRIBS, SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX,
};

const LOG: usize = 4096;

#[test]
#[ignore = "differs: the mediump vertex arithmetic is fp32 here, fp16 in Mesa (#4536)"]
fn misc3_bind_cond_float() {
    assert_matches_bound("deqp_misc3_bind_cond_float");
}

#[test]
#[ignore = "differs: the mediump vertex arithmetic is fp32 here, fp16 in Mesa (#4536)"]
fn misc3_bind_cond_vec2() {
    assert_matches_bound("deqp_misc3_bind_cond_vec2");
}

#[test]
#[ignore = "differs: the mediump vertex arithmetic is fp32 here, fp16 in Mesa (#4536)"]
fn misc3_bind_cond_vec3() {
    assert_matches_bound("deqp_misc3_bind_cond_vec3");
}

#[test]
#[ignore = "differs: the mediump vertex arithmetic is fp32 here, fp16 in Mesa (#4536)"]
fn misc3_bind_cond_vec4() {
    assert_matches_bound("deqp_misc3_bind_cond_vec4");
}

#[test]
#[ignore = "differs: the mediump vertex arithmetic is fp32 here, fp16 in Mesa (#4536)"]
fn misc3_bind_cond_mat2() {
    assert_matches_bound("deqp_misc3_bind_cond_mat2");
}

#[test]
#[ignore = "differs: the mediump vertex arithmetic is fp32 here, fp16 in Mesa (#4536)"]
fn misc3_bind_cond_mat3() {
    assert_matches_bound("deqp_misc3_bind_cond_mat3");
}

#[test]
#[ignore = "differs: the mediump vertex arithmetic is fp32 here, fp16 in Mesa (#4536)"]
fn misc3_bind_cond_mat4() {
    assert_matches_bound("deqp_misc3_bind_cond_mat4");
}

#[test]
#[ignore = "differs: the mediump vertex arithmetic is fp32 here, fp16 in Mesa (#4536)"]
fn misc3_bind_max_cond_mat4() {
    assert_matches_bound("deqp_misc3_bind_max_cond_mat4");
}

/* the same shapes with the vertex shader highp: the attribute layout at
 * the link's locations, byte for byte (#11388) */

#[test]
#[ignore = "differs: the first FCMP comes after the first LD_ATTR_IMM here, before it in Mesa"]
fn misc3_bind_hp_cond_float() {
    assert_matches_bound("deqp_misc3_bind_hp_cond_float");
}

#[test]
#[ignore = "differs: the first FCMP comes after the first LD_ATTR_IMM here, before it in Mesa"]
fn misc3_bind_hp_cond_vec2() {
    assert_matches_bound("deqp_misc3_bind_hp_cond_vec2");
}

#[test]
#[ignore = "differs: the first FCMP comes after the first LD_ATTR_IMM here, before it in Mesa"]
fn misc3_bind_hp_cond_vec3() {
    assert_matches_bound("deqp_misc3_bind_hp_cond_vec3");
}

#[test]
fn misc3_bind_hp_cond_vec4() {
    assert_matches_bound("deqp_misc3_bind_hp_cond_vec4");
}

#[test]
#[ignore = "differs: the first FCMP comes after the first LD_ATTR_IMM here, before it in Mesa"]
fn misc3_bind_hp_cond_mat2() {
    assert_matches_bound("deqp_misc3_bind_hp_cond_mat2");
}

#[test]
#[ignore = "differs: the first FCMP comes after the first LD_ATTR_IMM here, before it in Mesa"]
fn misc3_bind_hp_cond_mat2_offset_1() {
    assert_matches_bound("deqp_misc3_bind_hp_cond_mat2_offset_1");
}

#[test]
#[ignore = "differs: the first FCMP comes after the first LD_ATTR_IMM here, before it in Mesa"]
fn misc3_bind_hp_cond_mat3() {
    assert_matches_bound("deqp_misc3_bind_hp_cond_mat3");
}

#[test]
#[ignore = "differs: the first FCMP comes after the first LD_ATTR_IMM here, before it in Mesa"]
fn misc3_bind_hp_cond_mat3_offset_1() {
    assert_matches_bound("deqp_misc3_bind_hp_cond_mat3_offset_1");
}

#[test]
fn misc3_bind_hp_cond_mat4() {
    assert_matches_bound("deqp_misc3_bind_hp_cond_mat4");
}

#[test]
#[ignore = "differs: the first FCMP comes after the first LD_ATTR_IMM here, before it in Mesa"]
fn misc3_bind_hp_cond_mat4_offset_1() {
    assert_matches_bound("deqp_misc3_bind_hp_cond_mat4_offset_1");
}

#[test]
#[ignore = "differs: the first FCMP comes after the first LD_ATTR_IMM here, before it in Mesa"]
fn misc3_bind_hp_max_cond_float() {
    assert_matches_bound("deqp_misc3_bind_hp_max_cond_float");
}

#[test]
fn misc3_bind_hp_max_cond_vec2() {
    assert_matches_bound("deqp_misc3_bind_hp_max_cond_vec2");
}

#[test]
#[ignore = "differs: the first FCMP comes after the first LD_ATTR_IMM here, before it in Mesa"]
fn misc3_bind_hp_max_cond_vec3() {
    assert_matches_bound("deqp_misc3_bind_hp_max_cond_vec3");
}

#[test]
#[ignore = "differs: the first FCMP comes after the first LD_ATTR_IMM here, before it in Mesa"]
fn misc3_bind_hp_max_cond_vec4() {
    assert_matches_bound("deqp_misc3_bind_hp_max_cond_vec4");
}

#[test]
#[ignore = "differs: the first FCMP comes after the first LD_ATTR_IMM here, before it in Mesa"]
fn misc3_bind_hp_max_cond_mat2() {
    assert_matches_bound("deqp_misc3_bind_hp_max_cond_mat2");
}

#[test]
fn misc3_bind_hp_max_cond_mat3() {
    assert_matches_bound("deqp_misc3_bind_hp_max_cond_mat3");
}

#[test]
#[ignore = "differs: the first FCMP comes after the first LD_ATTR_IMM here, before it in Mesa"]
fn misc3_bind_hp_max_cond_mat4() {
    assert_matches_bound("deqp_misc3_bind_hp_max_cond_mat4");
}

const MEDIUMP: &str = "the mediump vertex arithmetic is fp32 here, fp16 in Mesa (#4536)";

/// Every ignored reference and why it differs now.
const REFUSALS: &[(&str, &str)] = &[
    ("deqp_misc3_bind_cond_float", MEDIUMP),
    ("deqp_misc3_bind_cond_vec2", MEDIUMP),
    ("deqp_misc3_bind_cond_vec3", MEDIUMP),
    ("deqp_misc3_bind_cond_vec4", MEDIUMP),
    ("deqp_misc3_bind_cond_mat2", MEDIUMP),
    ("deqp_misc3_bind_cond_mat3", MEDIUMP),
    ("deqp_misc3_bind_cond_mat4", MEDIUMP),
    ("deqp_misc3_bind_max_cond_mat4", MEDIUMP),
    ("deqp_misc3_bind_hp_cond_float", FIRST_LOAD),
    ("deqp_misc3_bind_hp_cond_vec2", FIRST_LOAD),
    ("deqp_misc3_bind_hp_cond_vec3", FIRST_LOAD),
    ("deqp_misc3_bind_hp_cond_mat2", FIRST_LOAD),
    ("deqp_misc3_bind_hp_cond_mat2_offset_1", FIRST_LOAD),
    ("deqp_misc3_bind_hp_cond_mat3", FIRST_LOAD),
    ("deqp_misc3_bind_hp_cond_mat3_offset_1", FIRST_LOAD),
    ("deqp_misc3_bind_hp_cond_mat4_offset_1", FIRST_LOAD),
    ("deqp_misc3_bind_hp_max_cond_float", FIRST_LOAD),
    ("deqp_misc3_bind_hp_max_cond_vec3", FIRST_LOAD),
    ("deqp_misc3_bind_hp_max_cond_vec4", FIRST_LOAD),
    ("deqp_misc3_bind_hp_max_cond_mat2", FIRST_LOAD),
    ("deqp_misc3_bind_hp_max_cond_mat4", FIRST_LOAD),
];

const FIRST_LOAD: &str = "the first FCMP comes after the first LD_ATTR_IMM here, before it in Mesa";

/// The references that compile byte-equal (their tests run).
const MATCHED: &[&str] = &[
    "deqp_misc3_bind_hp_cond_vec4",
    "deqp_misc3_bind_hp_cond_mat4",
    "deqp_misc3_bind_hp_max_cond_vec2",
    "deqp_misc3_bind_hp_max_cond_mat3",
];

#[test]
fn refusals_pinned() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/mali/testdata");
    let mut on_disk: Vec<String> = std::fs::read_dir(dir)
        .expect("tools/mali/testdata")
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|f| f.starts_with("deqp_misc3_"))
        .filter_map(|f| f.strip_suffix(".expect.json").map(str::to_string))
        .collect();
    on_disk.sort();
    let mut named: Vec<&str> = REFUSALS.iter().map(|(n, _)| *n).chain(MATCHED.iter().copied()).collect();
    named.sort();
    assert_eq!(on_disk, named, "a deqp_misc3_* reference is neither pinned nor matched here");
    let moved: Vec<String> = REFUSALS
        .iter()
        .filter_map(|(name, text)| {
            let r = mali_ref::Reference::load(name);
            match compile_bound(&r.vs, &r.fs, &bindings(name)) {
                Ok(pair) if mali_ref::differences(&r, &pair).is_empty() => {
                    Some(format!("{name}: now matches; remove its #[ignore] and its pin"))
                }
                Ok(_) => None,
                Err(e) => Some(format!("{name}: expected it to compile ({text}), got: {e}")),
            }
        })
        .collect();
    assert!(moved.is_empty(), "{}", moved.join("\n"));
}

/// The compiler's process-wide state takes one pair at a time
/// (`mali_ref::compile_pair` holds its own lock; this one covers
/// [`compile_bound`] and the pin's calls).
fn one_at_a_time() -> std::sync::MutexGuard<'static, ()> {
    static COMPILING: Mutex<()> = Mutex::new(());
    COMPILING.lock().unwrap_or_else(|p| p.into_inner())
}

fn log_text(log: &[c_char]) -> String {
    let s: String = log.iter().take_while(|c| **c != 0).map(|c| *c as u8 as char).collect();
    s.trim().replace('\n', " / ")
}

fn compile(stage: u32, src: &str) -> Result<ShcShader, String> {
    let mut out = ShcShader::ZERO;
    let mut log = [0 as c_char; LOG];
    let z = format!("{src}\0");
    let f = SHC_ONLINE.compile.expect("the online back end compiles");
    // SAFETY: a NUL-terminated source, a real output, and a log of the size announced.
    let rc = unsafe { f(stage, z.as_ptr() as *const c_char, &mut out, log.as_mut_ptr(), log.len()) };
    if rc == 0 {
        Ok(out)
    } else {
        Err(log_text(&log))
    }
}

/// The `.bind` file of a reference: (name, location) per line, `#` lines
/// left out; two names at one location alias.
fn bindings(name: &str) -> Vec<(String, u8)> {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("tools/mali/testdata/{name}.bind"));
    let text = std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
    text.lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
        .map(|l| {
            let mut w = l.split_whitespace();
            let n = w.next().expect("a name").to_string();
            let loc = w.next().and_then(|v| v.parse().ok()).unwrap_or_else(|| panic!("{name}.bind: {l}"));
            (n, loc)
        })
        .collect()
}

/// The map `src/gl/program.rs` (`attrib_layout`) builds for these bindings:
/// every declared attribute bound and read, with the GL type and the columns
/// its declaration in `vs_src` gives (`attribute mediump mat2 a_0;`).
fn attrib_map(vs_src: &str, binds: &[(String, u8)]) -> ShcAttribMap {
    assert!(binds.len() <= SHC_MAX_DECLARED_ATTRIBS);
    let mut map = ShcAttribMap::ZERO;
    for (d, (n, loc)) in map.b.iter_mut().zip(binds) {
        let mut name = [0 as c_char; 32];
        for (c, b) in name.iter_mut().zip(n.bytes()) {
            *c = b as c_char;
        }
        let (gl_type, columns) = declared_type(vs_src, n);
        *d = ShcAttribBinding { name, location: *loc, columns, gl_type, active: 1 };
    }
    map.count = binds.len() as u8;
    map
}

/// (GL type, columns) of the attribute `name` as `vs_src` declares it.
fn declared_type(vs_src: &str, name: &str) -> (u16, u8) {
    let ty = vs_src
        .lines()
        .find_map(|l| {
            let w: Vec<&str> = l.trim().trim_end_matches(';').split_whitespace().collect();
            (w.len() == 4 && w[0] == "attribute" && w[3] == name).then(|| w[2].to_string())
        })
        .unwrap_or_else(|| panic!("no `attribute` declaration of {name}"));
    match ty.as_str() {
        "float" => (0x1406, 1),
        "vec2" => (0x8B50, 1),
        "vec3" => (0x8B51, 1),
        "vec4" => (0x8B52, 1),
        "mat2" => (0x8B5A, 2),
        "mat3" => (0x8B5B, 3),
        "mat4" => (0x8B5C, 4),
        _ => panic!("{name}: type {ty}"),
    }
}

fn stage_of(sh: &ShcShader) -> mali_ref::Stage {
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
    mali_ref::Stage {
        code,
        work_reg_count: m.work_reg_count,
        preload: m.preload,
        secondary: m.secondary.map(|s| (s.offset, s.preload, s.work_reg_count)),
        no_psiz: m.no_psiz_offset,
        fau: m.fau.to_vec(),
        uniforms,
    }
}

/// The pair as the GL core builds it with aliasing bindings
/// (`src/gl/program.rs`): both shaders compile (glCompileShader), the
/// vertex shader is compiled again with the program's attribute map and the
/// fragment shader (`compile_vs_linked`, `link_attrib_layout`), then the
/// link. Err is the first failure's log.
fn compile_bound(vs_src: &str, fs_src: &str, binds: &[(String, u8)]) -> Result<mali_ref::Pair, String> {
    let _one = one_at_a_time();
    yakogl::shaderc::online::backend::forget_compiled();
    compile(SHC_STAGE_VERTEX, vs_src)?;
    let mut fs = compile(SHC_STAGE_FRAGMENT, fs_src)?;
    let map = attrib_map(vs_src, binds);
    let relink = SHC_ONLINE.compile_vs_linked.expect("the online back end relinks");
    let vs_z = format!("{vs_src}\0");
    let mut vs = ShcShader::ZERO;
    let mut log = [0 as c_char; LOG];
    // SAFETY: a NUL-terminated source, a map and a fragment shader that live
    // across the call, a real output and a log of the size announced.
    let rc = unsafe { relink(vs_z.as_ptr() as *const c_char, &map, &fs, &mut vs, log.as_mut_ptr(), log.len()) };
    if rc != 0 {
        return Err(log_text(&log));
    }
    let link = SHC_ONLINE.link.expect("the online back end links");
    // SAFETY: two distinct valid shaders and a log of the size announced.
    let rc = unsafe { link(&vs, &mut fs, log.as_mut_ptr(), log.len()) };
    if rc != 0 {
        return Err(log_text(&log));
    }
    Ok(mali_ref::Pair { vs: stage_of(&vs), fs: stage_of(&fs) })
}

fn assert_matches_bound(name: &str) {
    let r = mali_ref::Reference::load(name);
    let pair = compile_bound(&r.vs, &r.fs, &bindings(name)).unwrap_or_else(|e| panic!("{name}: refused: {e}"));
    let d = mali_ref::differences(&r, &pair);
    assert!(d.is_empty(), "{name}: differs from its reference:\n{}", d.join("\n"));
}
