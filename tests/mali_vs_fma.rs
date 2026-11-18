// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! glmark2's vertex fused multiply-adds (the vertex FMA family of plan
//! #367: ideas' matrix products, jellyfish 08a2's `smoothstep`, and the
//! built-ins `mod`, `mix` and `reflect`) on the Mali, against the Mesa
//! 26.2.1 drm-shim references `tools/mali/testdata/gm2_fma_1..8`
//! (docs/notes/mali-vs-fma.md, "glmark2's built-in multiply-adds"), built
//! as glCompileShader and glLinkProgram build them (`tests/mali_ref/mod.rs`)
//! and compared byte for byte. The references were drawn without
//! gl_ref.py's `--uniform-values`: every uniform word of their FAU is 0
//! ([`mali_ref::assert_matches_unset`]).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_vs_fma
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

/// ideas' `projection * modelview * curVertex`, `curVertex` a local
/// `vec4(vertex, 1.0)`: a mat4 times a mat4 times a vec4.
#[test]
fn gm2_fma_1_the_ideas_product_matches_its_reference() {
    mali_ref::assert_matches_unset("gm2_fma_1");
}

/// ideas' text shape: the same product of `vec4(vertex.xy, 0.0, 1.0)`.
#[test]
fn gm2_fma_2_the_ideas_text_product_matches_its_reference() {
    mali_ref::assert_matches_unset("gm2_fma_2");
}

/// `smoothstep(e0, e1, x)` of uniform edges on a vec4: the saturate on the
/// product (`FMA.f32.clamp_0_1`), `3.0 - 2.0 * t` one `FMA.f32 r, 0x40000000
/// .neg, t, 3.0` and the tail `t * (t * (3.0 - 2.0 * t))`.
#[test]
fn gm2_fma_5_smoothstep_matches_its_reference() {
    mali_ref::assert_matches_unset("gm2_fma_5");
}

/// `mix(aP, aC, k)`: every lane's `FADD.f32 d, c, p.neg`, then every lane's
/// `FMA.f32 r, k, d, p`.
#[test]
fn gm2_fma_7_mix_matches_its_reference() {
    mali_ref::assert_matches_unset("gm2_fma_7");
}

/// `reflect(aI, aN)`: the dot product from the `w` lane, `-2.0 * d` as
/// `FADD.f32 r4, r4^, r4^` read through `.neg` by each lane's `FMA.f32 r,
/// r4.neg, N, I`.
#[test]
fn gm2_fma_8_reflect_matches_its_reference() {
    mali_ref::assert_matches_unset("gm2_fma_8");
}

/// `smoothstep(0.0, 1.0, k)` and `mod(k, t)` of uniforms pass the vertex
/// walk and stop at the varying link (the four lanes of `vec4(..)` one
/// value, which Mesa's link merges): another family's refusal.
#[test]
fn gm2_fma_3_and_6_stop_at_the_varying_link() {
    for name in ["gm2_fma_3", "gm2_fma_6"] {
        mali_ref::assert_refused_ref(name, "two varying lanes of equal value");
    }
}

/// The hex words of the list after `key` in a `differences` line.
fn fau_list(line: &str, key: &str) -> Vec<String> {
    let at = line.find(key).unwrap_or_else(|| panic!("{key} in {line}")) + key.len();
    let list = &line[at..];
    let list = &list[list.find('[').expect("a list") + 1..list.find(']').expect("a list's end")];
    list.split(',').map(|w| w.trim().to_string()).collect()
}

/// jellyfish 08a2's `offset` (`smoothstep(0.0, 1.0, max(0.0, -y - 0.8) /
/// 10.0)`, then `(1.0 - offset)`) compiles, and differs from its reference
/// in one thing only: the two FAU constants 3.0 and 1.0 / 12.0 are pushed
/// in the other order (ours 3.0 first, as pan_shc's SPIR-V path does). In
/// Mesa's GLSL path `offset` is assigned once and read once, so
/// `opt_tree_grafting` moves its `t * (t * (3.0 - 2.0 * t))` into the read,
/// after `aCol / 12.0 * k`; the vertex walk takes the IR in statement order
/// (no grafting order). Every other word and fact is Mesa's.
#[test]
fn gm2_fma_4_differs_only_in_the_grafted_constant_order() {
    let r = mali_ref::Reference::load_unset("gm2_fma_4");
    let pair = mali_ref::compile_pair(&r.vs, &r.fs).unwrap_or_else(|e| panic!("gm2_fma_4: refused: {e}"));
    let d = mali_ref::differences(&r, &pair);
    assert_eq!(d.len(), 3, "gm2_fma_4: differences:\n{}", d.join("\n"));
    for line in d.iter().filter(|l| l.contains(": FAU words ours")) {
        let (mut ours, theirs) = (fau_list(line, "ours"), fau_list(line, "reference"));
        assert_eq!((ours[13].as_str(), ours[14].as_str()), ("40400000", "3daaaaab"), "{line}");
        ours.swap(13, 14);
        assert_eq!(ours, theirs, "{line}");
    }
    let words = d.iter().find(|l| l.starts_with("vs.varying: 28 words")).expect("a varying word difference");
    let lines: Vec<&str> = words.lines().collect();
    /* a line's disassembly: after "ours " or "ref ", past the word's hex */
    let text = |l: &str, key: &str| {
        let at = l.find(key).unwrap_or_else(|| panic!("{key} in {l}")) + key.len();
        l[at..].split_whitespace().skip(1).collect::<Vec<_>>().join(" ")
    };
    let mut n = 0;
    for (i, l) in lines.iter().enumerate().filter(|(_, l)| l.starts_with("*[")) {
        let theirs = text(lines[i + 1], "ref ").replace("u6.w1", "#").replace("u7.w0", "u6.w1").replace('#', "u7.w0");
        assert_eq!(text(l, "ours "), theirs, "gm2_fma_4: {l}");
        n += 1;
    }
    assert_eq!(n, 4, "gm2_fma_4: the instructions that read the two constants:\n{words}");
}

/// A built-in multiply-add with no reference stays refused: `reflect` of a
/// constant normal, whose dot and scale fold before the walk.
#[test]
fn a_reflect_of_a_constant_normal_stays_refused() {
    let vs = "attribute vec4 p; varying vec4 v; void main() { v = reflect(p, vec4(0.0)); gl_Position = p; }";
    let fs = "precision mediump float; varying vec4 v; void main() { gl_FragColor = v; }";
    mali_ref::assert_refused(vs, fs, "a fused multiply-add other than a matrix product's");
}
