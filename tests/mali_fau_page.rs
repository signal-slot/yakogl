// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! FAU page 1 on the Mali online compiler (plan #367 lower-fau-page,
//! `docs/notes/mali-fau-pages.md`): a push word past word 63 is read
//! through FAU page 1. An instruction selects the page of its first FAU
//! source (`va_select_fau_page`, fau.rs `select_fau_page`), and a uniform
//! or special word on another page is read through a `MOV.i32` before it
//! (`va_repair_fau`, fau.rs `repair_fau`). The references are Mesa 26.2.1 on
//! drm-shim: `tools/mali/testdata/sel_u_fau65` and `gm2_faupage_1..4` (the
//! uniform layouts of glmark2 refract vs 7215 and terrain vs 18bc), compared
//! through `tests/mali_ref`.
//!
//! sel_u_fau65 is byte-equal, and so is gm2_faupage_4 since the vertex
//! layout joins the loads Mesa's `nir_opt_load_store_vectorize` joins
//! (select.rs `UboLayout::join_loads`, plan #377 lower-varylink). The other
//! three gm2_faupage pairs read their page-1 words as Mesa does (the page
//! rule below holds on both sides, and gm2_faupage_1 reads the same page-1
//! word as its reference), but their whole programs are not byte-equal
//! yet, for causes that are not the page's ([`CAUSES`]): their
//! byte-compares stay `#[ignore]`d and
//! `causes_pinned` pins each cause. The item that lifts a cause removes the
//! `#[ignore]` and the pin in the same commit.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_fau_page
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

use yakogl::shaderc::online::mali::isa;

/// The words of one FAU page: a source names `u<slot>.w<half>`, the slot
/// counted over all pages (32 slots a page), as Mesa's disassembler does.
const SLOTS_PER_PAGE: u32 = 32;

/// Why each gm2_faupage pair is not byte-equal yet: the refusal it gets, or
/// `None` for a pair that compiles and differs.
const CAUSES: &[(&str, Option<&str>)] = &[
    /* the varying program's order: Mesa adds `vec4(u_w, 1.0, 1.0)`'s 1.0
     * into the first column's product (`FMA ... 0x3F800000`) and sums the
     * columns two by two; we add it last (`FADD_IMM`) */
    ("gm2_faupage_1", None),
    /* the fragment shader `(v_nrm + v_eye) * v_light`, v_nrm.w the
     * constant 1.0 the link moves into it (FRAGCOLOR_LANE) */
    (
        "gm2_faupage_2",
        Some("Valhall: a gl_FragColor lane other than the varying's own lane or a constant is not lowered yet"),
    ),
    /* the mediump fragment shader's product of five varyings, two of them
     * with a constant w: since the constant operations were lowered
     * (0fd576ea) the constant lane is FRAGCOLOR_LANE's; with that shader
     * highp the pair compiles, and the vertex stage differs only in its
     * push ranges (IDVS_PUSH_RANGE: Mesa's varying program pushes all of
     * u_eye, the lanes its shared NIR loads read, where we push the lanes
     * the varying program reads) */
    (
        "gm2_faupage_3",
        Some("Valhall: a gl_FragColor lane other than the varying's own lane or a constant is not lowered yet"),
    ),
];

fn describe(w: u64) -> String {
    let mut s = String::new();
    isa::describe_word(w, &mut s).unwrap_or_else(|e| panic!("{w:016x}: {e:?}"));
    s
}

/// The FAU page an instruction word selects (`fau_page_index`, bit 57).
fn page(w: u64) -> u32 {
    ((w >> 57) & 3) as u32
}

/// The uniform words an instruction reads, as (slot, half) of `uN.wM`.
fn uniforms(text: &str) -> Vec<(u32, u32)> {
    text.split([' ', ',', '[', ']', '@'])
        .filter_map(|t| {
            let (slot, half) = t.strip_prefix('u')?.split_once(".w")?;
            let half = half.split(|c: char| !c.is_ascii_digit()).next()?;
            Some((slot.parse().ok()?, half.parse().ok()?))
        })
        .collect()
}

/// Every instruction of `code` reads its uniform words from its own page,
/// one 64-bit slot at most (va_validate.c `va_validate_fau`).
fn assert_page_rule(what: &str, code: &[u64]) {
    for &w in code.iter().filter(|w| **w != 0) {
        let text = describe(w);
        let u = uniforms(&text);
        for (slot, _) in &u {
            assert_eq!(slot / SLOTS_PER_PAGE, page(w), "{what}: {w:016x} {text}: a word of another page");
        }
        assert!(u.windows(2).all(|p| p[0].0 == p[1].0), "{what}: {w:016x} {text}: two slots");
    }
}

/// The uniform words read on page 1, in program order.
fn page1_reads(code: &[u64]) -> Vec<(u32, u32)> {
    code.iter().filter(|w| page(**w) == 1).flat_map(|w| uniforms(&describe(*w))).collect()
}

/// The reference's words of one region (the expect.json's `words`).
fn reference_words(json: &str, region: &str) -> Vec<u64> {
    let r = &json[json.find(&format!("\"{region}\"")).unwrap_or_else(|| panic!("no {region}"))..];
    let r = &r[r.find("\"words\"").expect("words")..];
    let r = &r[r.find('[').unwrap() + 1..r.find(']').unwrap()];
    r.split(',').filter_map(|x| u64::from_str_radix(x.trim().trim_matches('"'), 16).ok()).collect()
}

/// sel_u_fau65: one promoted constant at word 64, read on page 1 by one
/// instruction of the position program (drawn with tri.frag, as
/// `src/shaderc/online/mali/tests.rs` builds the sel_u_* references).
/// Byte-equal: words, facts and FAU words.
#[test]
fn sel_u_fau65_one_constant_on_page_1() {
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tools/mali/testdata");
    let read = |f: &str| std::fs::read_to_string(dir.join(f)).unwrap_or_else(|e| panic!("{f}: {e}"));
    let r = mali_ref::Reference {
        name: "sel_u_fau65".into(),
        vs: read("sel_u_fau65.vert"),
        fs: read("tri.frag"),
        json: read("sel_u_fau65.expect.json"),
        uniform_values: true,
    };
    let pair = mali_ref::compile_pair(&r.vs, &r.fs).expect("the pair compiles");
    let d = mali_ref::differences(&r, &pair);
    assert!(d.is_empty(), "sel_u_fau65 differs:\n{}", d.join("\n"));
    assert_eq!(pair.vs.fau.len(), 65, "one word on page 1");
    assert_eq!(page1_reads(&pair.vs.code), [(32, 0)], "word 64 read on page 1");
}

/// The page rule holds on Mesa's words of every reference (the rule
/// transcribed is the one its programs follow) and on ours of every pair
/// that compiles.
#[test]
fn every_instruction_reads_its_own_page() {
    for (name, refused) in CAUSES {
        let r = mali_ref::Reference::load(name);
        for region in ["vs.position", "vs.varying"] {
            assert_page_rule(&format!("{name} reference {region}"), &reference_words(&r.json, region));
        }
        if refused.is_none() {
            let pair = mali_ref::compile_pair(&r.vs, &r.fs).expect("the pair compiles");
            assert_page_rule(&format!("{name} ours"), &pair.vs.code);
        }
    }
}

/// gm2_faupage_1: the varying program reads word 64 (`u_w.y`) in one
/// `FADD.f32 ... u32.w0` on page 1, as its reference does.
#[test]
fn gm2_faupage_1_reads_word_64_on_page_1() {
    let r = mali_ref::Reference::load("gm2_faupage_1");
    let pair = mali_ref::compile_pair(&r.vs, &r.fs).expect("the pair compiles");
    let theirs = page1_reads(&reference_words(&r.json, "vs.varying"));
    assert_eq!(theirs, [(32, 0)]);
    assert_eq!(page1_reads(&pair.vs.code), theirs);
    let w = pair.vs.code.iter().find(|w| page(**w) == 1).unwrap();
    assert!(describe(*w).starts_with("FADD.f32 "), "{}", describe(*w));
}

/// The mixed-page case: one product-sum of gm2_faupage_4's varying program
/// would read a page-0 word (`u_near`, u19.w0) and a page-1 word (`u_v.w`).
/// The FMA keeps the page of its first FAU source, page 0, and the page-1
/// word is read by a `MOV.i32` on page 1 just before it, as the reference's
/// `MOV.i32 r1, u41.w1` / `FMA.f32 r0, r0^, u19.w0, r1^` (Mesa's
/// `va_repair_fau`).
#[test]
fn gm2_faupage_4_mixed_pages_in_one_instruction() {
    let r = mali_ref::Reference::load("gm2_faupage_4");
    let shape = |code: &[u64]| {
        code.windows(2).any(|p| {
            let (mov, fma) = (describe(p[0]), describe(p[1]));
            let dest = mov.strip_prefix("MOV.i32 ").and_then(|t| t.split(',').next()).unwrap_or("");
            page(p[0]) == 1
                && uniforms(&mov).iter().all(|(s, _)| *s >= SLOTS_PER_PAGE)
                && page(p[1]) == 0
                && fma.starts_with("FMA.f32 ")
                && uniforms(&fma) == [(19, 0)]
                && fma.contains(&format!("{dest}^"))
        })
    };
    assert!(shape(&reference_words(&r.json, "vs.varying")), "the reference's shape");
    let pair = mali_ref::compile_pair(&r.vs, &r.fs).expect("the pair compiles");
    assert!(shape(&pair.vs.code), "the page-1 word is moved before the page-0 FMA");
}

/// Each gm2_faupage pair's cause: the refusal pinned, or a pair that
/// compiles and still differs. None is refused for its FAU page.
#[test]
fn causes_pinned() {
    for (name, refused) in CAUSES {
        let r = mali_ref::Reference::load(name);
        match (mali_ref::compile_pair(&r.vs, &r.fs), refused) {
            (Err(e), Some(want)) => assert_eq!(e, *want, "{name}"),
            (Ok(pair), None) => assert!(!mali_ref::differences(&r, &pair).is_empty(), "{name}: now byte-equal"),
            (Err(e), None) => panic!("{name}: refused: {e}"),
            (Ok(_), Some(_)) => panic!("{name}: compiles now"),
        }
    }
}

#[test]
#[ignore = "the varying program's order (CAUSES)"]
fn gm2_faupage_1_matches_its_reference() {
    mali_ref::assert_matches("gm2_faupage_1");
}

#[test]
#[ignore = "FRAGCOLOR_LANE in the fragment shader (CAUSES)"]
fn gm2_faupage_2_matches_its_reference() {
    mali_ref::assert_matches("gm2_faupage_2");
}

#[test]
#[ignore = "FRAGCOLOR_LANE, then IDVS_PUSH_RANGE (CAUSES)"]
fn gm2_faupage_3_matches_its_reference() {
    mali_ref::assert_matches("gm2_faupage_3");
}

/// IDVS_PUSH_RANGE: Mesa's position program pushes `u_near` with the three
/// words after it (the shared NIR's vectorized load of u_near and u_v.xyz).
#[test]
fn gm2_faupage_4_matches_its_reference() {
    mali_ref::assert_matches("gm2_faupage_4");
}
