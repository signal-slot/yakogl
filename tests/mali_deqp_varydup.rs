// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Byte-compare of the Mali online compiler against the Mesa 26.2.1
//! references `tools/mali/testdata/deqp_varydup_*` (plan #387, item
//! varydupres-refs, `docs/notes/mali-varydup-lowering.md`): the shapes of the
//! `VaryingDuplicate`, `AttributeNumbering` and `LinkedVertexNotLowered`
//! first refusals of the host dEQP-GLES2 measure at 349f989f that no
//! `deqp_misc2_*`, `deqp_misc3_*` or `deqp_varylink2_*` reference covers. Each
//! pair is a dEQP case's pair verbatim from the measure's `.qpa` (the
//! `.license` beside it names the case); every reference was drawn with
//! `tools/mali/deqp_ref.sh --instrument shim` (Mesa 26.2.1's G610 drm-shim,
//! `--uniform-values`). `tests/mali_ref` does the compare.
//!
//! A reference the compiler refuses is `#[ignore]`d and `refusals_pinned`
//! holds the whole refusal text ([`REFUSED`]). The lowering step that lifts one
//! moves it to [`LOWERED`] (byte-equal) or [`DIFFERS`] (compiles with other
//! words than Mesa's) in the same commit, drops its `#[ignore]` and updates the
//! note. `references_parse` checks every file and the fragment word count the
//! note quotes ([`FS_WORDS`]).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_varydup
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

/// The references a lowering step made byte-equal (or that were byte-equal when
/// drawn), without the `deqp_varydup_` prefix.
const LOWERED: &[&str] = &[];

/// The references that compile with other words than Mesa's (hazards).
const DIFFERS: &[&str] = &[];

/// Every ignored reference with its whole refusal (the `MaliError` message,
/// as `mali_ref::compile_pair` returns it).
const REFUSED: &[(&str, &str)] = &[
    ("attrnum_dead_global", TEXT_ATTRNUM),
    ("attrnum_dead_output", TEXT_ATTRNUM),
    ("attrnum_uniform_zero", TEXT_ATTRNUM),
    ("const_cf", TEXT_DUP),
];

const TEXT_ATTRNUM: &str = "Valhall: a vertex shader whose attribute numbering waits for a fold Mesa's back end makes after it numbers the locations, of a form the walk does not model, is not lowered yet";
const TEXT_DUP: &str = "Valhall: two varying lanes of equal value (Mesa's link merges them) is not lowered yet";

/// The words of each reference's `fs.fragment` region (Mesa's fragment
/// program, the tail and the blend branch included).
const FS_WORDS: &[(&str, usize)] =
    &[("attrnum_dead_global", 7), ("attrnum_dead_output", 7), ("attrnum_uniform_zero", 7), ("const_cf", 7)];

/// `shaders.random.basic_expression.vertex.19`: `v_color = e * vec4(d, c, b,
/// a)` with `e.z == 0.0` and `c == -0.0` (`b` read only through a zero
/// product), and the attributes `g`, `h` read only by dead writes of a global.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn attrnum_dead_global() {
    mali_ref::assert_matches("deqp_varydup_attrnum_dead_global");
}

/// `shaders.random.scalar_conversion.vertex.61`: `v_color` written three
/// times, the last `vec4(-1.75, bool(c), b, -1.25) * a` with `a = (-0.5, 0,
/// 0, 0)`: every lane a constant; `h`, `g`, `d` (and the uniform `e`) are
/// read only by the overwritten writes.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn attrnum_dead_output() {
    mali_ref::assert_matches("deqp_varydup_attrnum_dead_output");
}

/// `shaders.random.scalar_conversion.vertex.63`: the last write `vec4(g, f,
/// e, 0.0) * vec4(d, c, b, a)` with `c == false` (a uniform times zero) and
/// `a` read only through `0.0 * a`; `k`, `i` read only by overwritten writes.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn attrnum_uniform_zero() {
    mali_ref::assert_matches("deqp_varydup_attrnum_uniform_zero");
}

/// `shaders.random.conditionals.combined.85`: constant conditions leave the
/// varying `j = vec4(1.0, 0.0, 1.0, 0.0)` (equal constant lanes); Mesa's link
/// propagates it into the fragment shader, which has no varying load.
#[test]
#[ignore = "refused; pinned by refusals_pinned"]
fn const_cf() {
    mali_ref::assert_matches("deqp_varydup_const_cf");
}

fn on_disk() -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir("tools/mali/testdata")
        .expect("tools/mali/testdata")
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter_map(|n| n.strip_prefix("deqp_varydup_")?.strip_suffix(".expect.json").map(str::to_string))
        .collect();
    v.sort();
    v
}

/// The words of `region` in an expect.json, or None when the region is
/// absent; a word that is not 16 hex digits is an error.
fn region_words(json: &str, region: &str) -> Option<Result<usize, String>> {
    let at = json.find(&format!("\"{region}\": {{"))?;
    let rest = &json[at..];
    let words = match rest.find("\"words\": [") {
        Some(w) => &rest[w + "\"words\": [".len()..],
        None => return Some(Err(format!("{region}: no words"))),
    };
    let list = &words[..words.find(']').unwrap_or(0)];
    let mut n = 0;
    for w in list.split(',').map(|w| w.trim().trim_matches('"')).filter(|w| !w.is_empty()) {
        if w.len() != 16 || !w.chars().all(|c| c.is_ascii_hexdigit()) {
            return Some(Err(format!("{region}: {w:?} is not a 64-bit hex word")));
        }
        n += 1;
    }
    Some(Ok(n))
}

/// Every reference on disk parses: both shaders are there and not empty,
/// each has its `.license`, the trace is the reference's own (`deqp_ref.sh`
/// names it after the prefix), `vs.position` and `fs.fragment` have 64-bit
/// hex words (`vs.varying` may be absent: `const_cf` has no varying
/// program), and `fs.fragment` has the count of [`FS_WORDS`].
#[test]
fn references_parse() {
    let names = on_disk();
    let mut listed: Vec<&str> = FS_WORDS.iter().map(|(n, _)| *n).collect();
    listed.sort_unstable();
    assert_eq!(names, listed, "FS_WORDS does not list every deqp_varydup_* reference");
    let mut bad = Vec::new();
    for (name, fs_words) in FS_WORDS {
        let r = mali_ref::Reference::load(&format!("deqp_varydup_{name}"));
        if !r.vs.contains("void main") || !r.fs.contains("void main") {
            bad.push(format!("{name}: a shader has no main"));
        }
        for ext in ["vert", "frag"] {
            let lic = format!("tools/mali/testdata/deqp_varydup_{name}.{ext}.license");
            match std::fs::read_to_string(&lic) {
                Ok(t) if t.contains("SPDX-License-Identifier: Apache-2.0") => {}
                _ => bad.push(format!("{name}: {lic} is missing or has no SPDX line")),
            }
        }
        if !r.json.contains(&format!("\"trace\": \"deqp_varydup_{name}.trace\"")) {
            bad.push(format!("{name}: the trace is not deqp_varydup_{name}.trace"));
        }
        for region in ["vs.position", "vs.varying", "fs.fragment"] {
            match region_words(&r.json, region) {
                None if region == "vs.varying" => {}
                None => bad.push(format!("{name}: no {region}")),
                Some(Err(e)) => bad.push(format!("{name}: {e}")),
                Some(Ok(0)) => bad.push(format!("{name}: {region} has no words")),
                Some(Ok(n)) => {
                    if region == "fs.fragment" && n != *fs_words {
                        bad.push(format!("{name}: fs.fragment has {n} words, FS_WORDS says {fs_words}"));
                    }
                }
            }
        }
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

/// Each refused reference is refused with exactly its text in [`REFUSED`]
/// (the whole `MaliError` message, not a substring), each of [`DIFFERS`]
/// compiles and differs, each of [`LOWERED`] is byte-equal, and the three
/// lists cover every `deqp_varydup_*` reference on disk.
#[test]
fn refusals_pinned() {
    let on_disk = on_disk();
    let mut listed: Vec<&str> = LOWERED.iter().chain(DIFFERS).copied().chain(REFUSED.iter().map(|(n, _)| *n)).collect();
    listed.sort_unstable();
    assert_eq!(on_disk, listed, "a deqp_varydup_* reference has no outcome here");
    let mut changed = Vec::new();
    for (name, refused) in REFUSED {
        let r = mali_ref::Reference::load(&format!("deqp_varydup_{name}"));
        match mali_ref::compile_pair(&r.vs, &r.fs) {
            Ok(pair) => {
                let to = if mali_ref::differences(&r, &pair).is_empty() { "LOWERED" } else { "DIFFERS" };
                changed.push(format!("{name}: now compiles; move it to {to}"));
            }
            Err(e) => {
                if e != *refused {
                    changed.push(format!("{name}: the refusal changed to {e:?}"));
                }
            }
        }
    }
    for name in DIFFERS {
        let r = mali_ref::Reference::load(&format!("deqp_varydup_{name}"));
        match mali_ref::compile_pair(&r.vs, &r.fs) {
            Ok(pair) => {
                if mali_ref::differences(&r, &pair).is_empty() {
                    changed.push(format!("{name}: now byte-equal; move it to LOWERED"));
                }
            }
            Err(e) => changed.push(format!("{name}: now refused: {e}")),
        }
    }
    for name in LOWERED {
        let r = mali_ref::Reference::load(&format!("deqp_varydup_{name}"));
        match mali_ref::compile_pair(&r.vs, &r.fs) {
            Ok(pair) => {
                let d = mali_ref::differences(&r, &pair);
                if !d.is_empty() {
                    changed.push(format!("{name}: no longer byte-equal: {}", d[0]));
                }
            }
            Err(e) => changed.push(format!("{name}: now refused: {e}")),
        }
    }
    assert!(changed.is_empty(), "{}", changed.join("\n"));
}
