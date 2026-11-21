// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Byte-compare of the Mali online compiler against the Mesa 26.2.1
//! references `tools/mali/testdata/deqp_defects_*` (plan #387, item
//! defects-ref: the two back end defects of section 18.4 of
//! `docs/notes/mali-deqp-families.md` other than `Defect(IrIndex)`; see
//! `docs/notes/mali-deqp-defects.md`). One test per reference:
//!
//! - defect `encode`, `MaliError::Defect(Encode)` on a vertex shader that
//!   reads more than 16 attributes (`pack.rs` `Fill::done`: `LD_ATTR_IMM`'s
//!   4-bit `attribute_index` takes record 16):
//!   `encode_max_cond_float`
//!   (`attribute_location.bind_aliasing.max_cond_float`, 32 attributes on 16
//!   locations) and `encode_reduced` (18 on 9). Mesa draws them with the
//!   aliasing bindings of each reference's `.bind` file, so these two are
//!   compiled the way glLinkProgram compiles them after glBindAttribLocation
//!   ([`compile_bound`]: the attribute relink with the program's map), not by
//!   `mali_ref::compile_pair`;
//! - defect `lanecycle`, in the fragment compile `nir_fs::lane_tree` and
//!   `nir_fs::vectorise` call each other without end on a pair whose second
//!   lane is a constant (a stack overflow until `nir_fs::pair_node` refused
//!   the pair asked for again as `Defect(LaneCycle)`): `lanecycle_fs`
//!   (`shaders.random.scalar_conversion.fragment.96`, the fragment shader of
//!   `.combined.96` too) and `lanecycle_reduced`
//!   (`vec4(2.0) * b * vec4(0.0, 0.0, b, 0.0)`).
//!
//! None compiles at the tip: each test is ignored with its outcome quoted.
//! The fix removes the `ignore` and the entry in `PINNED` together.
//! `defects_pinned` runs now: it asserts each reference's exact refusal
//! (`Defect(Encode)`, `Defect(LaneCycle)`) and names every one that moved.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_defects
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
/// `GL_FLOAT`: every attribute of the `encode` references is a float.
const GL_FLOAT: u16 = 0x1406;

#[test]
#[ignore = "refused: Valhall back end defect: an invariant of the pipeline did not hold (Encode)"]
fn defects_encode_max_cond_float() {
    assert_matches_bound("deqp_defects_encode_max_cond_float");
}

#[test]
#[ignore = "refused: Valhall back end defect: an invariant of the pipeline did not hold (Encode)"]
fn defects_encode_reduced() {
    assert_matches_bound("deqp_defects_encode_reduced");
}

#[test]
#[ignore = "refused: Valhall back end defect: an invariant of the pipeline did not hold (LaneCycle)"]
fn defects_lanecycle_fs() {
    let _one = one_at_a_time();
    mali_ref::assert_matches("deqp_defects_lanecycle_fs");
}

#[test]
#[ignore = "refused: Valhall back end defect: an invariant of the pipeline did not hold (LaneCycle)"]
fn defects_lanecycle_reduced() {
    let _one = one_at_a_time();
    mali_ref::assert_matches("deqp_defects_lanecycle_reduced");
}

const ENCODE: &str = "Valhall back end defect: an invariant of the pipeline did not hold (Encode)";
const LANECYCLE: &str = "Valhall back end defect: an invariant of the pipeline did not hold (LaneCycle)";

/// Every ignored reference and the refusal its program's compile gives now
/// (the `encode` ones: the vertex shader's glCompileShader, no bindings
/// yet; the `lanecycle` ones: the fragment shader's).
const PINNED: &[(&str, &str)] = &[
    ("deqp_defects_encode_max_cond_float", ENCODE),
    ("deqp_defects_encode_reduced", ENCODE),
    ("deqp_defects_lanecycle_fs", LANECYCLE),
    ("deqp_defects_lanecycle_reduced", LANECYCLE),
];

/// The references that compile byte-equal (their tests run): none yet.
const MATCHED: &[&str] = &[];

#[test]
fn defects_pinned() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/mali/testdata");
    let mut on_disk: Vec<String> = std::fs::read_dir(dir)
        .expect("tools/mali/testdata")
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|f| f.starts_with("deqp_defects_"))
        .filter_map(|f| f.strip_suffix(".expect.json").map(str::to_string))
        .collect();
    on_disk.sort();
    let mut named: Vec<&str> = PINNED.iter().map(|(n, _)| *n).chain(MATCHED.iter().copied()).collect();
    named.sort();
    assert_eq!(on_disk, named, "a deqp_defects_* reference is neither pinned nor matched here");
    let moved: Vec<String> = PINNED
        .iter()
        .filter_map(|(name, text)| {
            let _one = one_at_a_time();
            mali_ref::refusal_change(name, text)
        })
        .collect();
    assert!(moved.is_empty(), "{}", moved.join("\n"));
}

/* ---- the program as glBindAttribLocation and glLinkProgram build it ---- */

/// The compiler's process-wide state takes one pair at a time, and
/// `forget_compiled()` resets the keep arena every handed-out shader points
/// into: every compile of this file, and the reads of its result, hold this
/// lock ([`compile_bound`], the `lanecycle` tests' `mali_ref::assert_matches`
/// and the pin's calls). `mali_ref::compile_pair` holds a lock of its own
/// inside, which alone does not keep it apart from [`compile_bound`].
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
/// every declared attribute bound, read, one float column each.
fn attrib_map(binds: &[(String, u8)]) -> ShcAttribMap {
    assert!(binds.len() <= SHC_MAX_DECLARED_ATTRIBS);
    let mut map = ShcAttribMap::ZERO;
    for (d, (n, loc)) in map.b.iter_mut().zip(binds) {
        let mut name = [0 as c_char; 32];
        for (c, b) in name.iter_mut().zip(n.bytes()) {
            *c = b as c_char;
        }
        *d = ShcAttribBinding { name, location: *loc, columns: 1, gl_type: GL_FLOAT, active: 1 };
    }
    map.count = binds.len() as u8;
    map
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
    let map = attrib_map(binds);
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
