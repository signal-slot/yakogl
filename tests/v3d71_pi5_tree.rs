// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.

//! tools/cts/pi5-full-tree.txt lists the same dEQP-GLES2 groups as the Pi 4's
//! tools/cts/pi4-full-tree.txt (the table tools/cts/pi4-deqp-images.sh and
//! board-stage.sh build the Pi 4 tree images from). File parsing only.
//!
//! Pinned: every line parses in the form tools/cts/deqp-tree-images.sh reads
//! (`# image <LABEL>:` then exactly one filter line), labels are unique, and
//! the union of filter patterns equals the Pi 4's. The two intended
//! differences (T1 -> T1 + TV, MISC -> MISC + PO) are listed below and their
//! reasons are checked to be stated in the Pi 5 file's own comments.
//!
//! Run: `cargo test --test v3d71_pi5_tree`

use std::collections::{BTreeMap, BTreeSet};

const PI5: &str = include_str!("../tools/cts/pi5-full-tree.txt");
const PI4: &str = include_str!("../tools/cts/pi4-full-tree.txt");

/// Images in file order: (label, filter line). Mirrors the awk of
/// deqp-tree-images.sh `images()`: label from `# image [A-Za-z0-9]+:`, the
/// next non-comment, non-blank line is its filter, a filter without a label
/// is an error.
fn parse(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut label: Option<String> = None;
    for (n, line) in text.lines().enumerate() {
        if let Some(rest) = line.strip_prefix("# image ") {
            let tok = rest.split_whitespace().next().unwrap_or("");
            if let Some(l) = tok.strip_suffix(':') {
                if !l.is_empty() && l.chars().all(|c| c.is_ascii_alphanumeric()) {
                    label = Some(l.to_string());
                    continue;
                }
            }
        }
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        let l = label.take().unwrap_or_else(|| panic!("line {}: filter without a label", n + 1));
        assert!(!t.contains(char::is_whitespace), "line {}: whitespace in filter", n + 1);
        for p in t.split(',') {
            assert!(
                p.starts_with("dEQP-GLES2.") && p.len() > "dEQP-GLES2.".len() && !p.contains(".."),
                "line {}: bad pattern '{p}'",
                n + 1
            );
        }
        out.push((l, t.to_string()));
    }
    assert!(label.is_none(), "a label without a filter at the end");
    out
}

fn patterns(f: &str) -> BTreeSet<&str> {
    f.split(',').collect()
}

fn by_label(v: &[(String, String)]) -> BTreeMap<&str, &str> {
    v.iter().map(|(l, f)| (l.as_str(), f.as_str())).collect()
}

#[test]
fn labels_unique_and_counts_match_headers() {
    for (name, text, n) in [("pi4", PI4, 29), ("pi5", PI5, 31)] {
        let v = parse(text);
        let m = by_label(&v);
        assert_eq!(m.len(), v.len(), "{name}: duplicate label");
        assert_eq!(v.len(), n, "{name}: image count");
        assert!(text.contains(&format!("in {n} images")), "{name}: header states {n} images");
        let mut pats = BTreeSet::new();
        for (_, f) in &v {
            for p in f.split(',') {
                assert!(pats.insert(p), "{name}: pattern {p} selected twice");
            }
        }
    }
}

#[test]
fn pi5_covers_the_pi4_groups() {
    let (a, b) = (parse(PI4), parse(PI5));
    let all = |v: &[(String, String)]| -> BTreeSet<String> {
        v.iter().flat_map(|(_, f)| patterns(f).into_iter().map(String::from)).collect()
    };
    assert_eq!(all(&a), all(&b), "dEQP group sets differ between the Pi 4 and Pi 5 trees");
}

#[test]
fn intended_differences_are_exactly_the_stated_recuts() {
    let (a, b) = (parse(PI4), parse(PI5));
    let (ma, mb) = (by_label(&a), by_label(&b));
    let ka: BTreeSet<_> = ma.keys().copied().collect();
    let kb: BTreeSet<_> = mb.keys().copied().collect();
    let added: Vec<_> = kb.difference(&ka).copied().collect();
    assert_eq!(added, ["PO", "TV"], "labels only in the Pi 5 tree");
    assert!(ka.is_subset(&kb), "a Pi 4 label is missing from the Pi 5 tree");
    let changed: Vec<_> = ka.iter().copied().filter(|l| ma[l] != mb[l]).collect();
    assert_eq!(changed, ["MISC", "T1"], "images whose filter differs");
    // The re-cuts move patterns only between the old image and its new one.
    for (old, new) in [("T1", "TV"), ("MISC", "PO")] {
        let mut u = patterns(mb[old]);
        u.extend(patterns(mb[new]));
        assert_eq!(u, patterns(ma[old]), "{old}+{new} on the Pi 5 != {old} on the Pi 4");
        assert!(patterns(mb[old]).is_disjoint(&patterns(mb[new])));
    }
    assert_eq!(mb["TV"], "dEQP-GLES2.functional.texture.vertex.*");
    assert_eq!(mb["PO"], "dEQP-GLES2.functional.polygon_offset.*");
    // Reasons are in the Pi 5 file's own comments.
    for s in [
        "T1 (181 on the Pi 4) -> T1 91 + TV 90: texture.vertex (vertex texture",
        "MISC (70 on the Pi 4) -> MISC 60 + PO 10: polygon_offset waits for the",
    ] {
        assert!(PI5.contains(s), "reason missing from pi5-full-tree.txt: {s}");
    }
}
