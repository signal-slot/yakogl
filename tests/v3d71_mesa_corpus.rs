// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Mesa's V3D 7.1 output for the es100 corpus: `tools/v3d/es100_counts_71.tsv`,
//! the reference table the 7.1 back end is compared with.
//!
//! The table is made by `tools/v3d/es100_ref71.sh` (Mesa 26.2.1's `v3d_shc
//! --device 71`, one row per `.vert`/`.frag` source of `tools/shaders/es100`).
//!
//! * `tsv_lists_the_corpus_program_set` needs nothing outside the repository: the
//!   table's paths are exactly the es100 sources `tests/v3d_online_corpus.rs`
//!   walks (every `.vert`/`.frag` under `tools/shaders`, 126 of them in es100),
//!   each once, and its header counts match its rows.
//! * `mesa_reproduces_the_tsv` needs `V3D_SHC` (tools/v3d/README.md, H2), plus
//!   glslangValidator and python3: it reruns the script into a scratch file and
//!   requires the same bytes. Without `V3D_SHC` it prints `gap: V3D_SHC` and
//!   returns; that run is a named gap, not a pass of the comparison.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d71_mesa_corpus -- --nocapture
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-v3d"))]
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

const PROGRAMS: usize = 126;
const COMPILED: usize = 124;

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

fn tsv_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/v3d/es100_counts_71.tsv")
}

#[test]
fn tsv_lists_the_corpus_program_set() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/shaders");
    let mut files = Vec::new();
    sources(&root, &mut files);
    let want: BTreeSet<String> = files
        .iter()
        .map(|p| p.strip_prefix(&root).expect("under the root").to_string_lossy().into_owned())
        .filter(|r| r.starts_with("es100/"))
        .collect();
    assert_eq!(want.len(), PROGRAMS, "tools/shaders/es100 holds {PROGRAMS} sources; re-derive the pins if it changed");

    let text = std::fs::read_to_string(tsv_path()).expect("es100_counts_71.tsv exists");
    let mut have = Vec::new();
    let mut compiled = 0;
    for l in text.lines().filter(|l| !l.starts_with('#') && !l.trim().is_empty()) {
        let c: Vec<&str> = l.split('\t').collect();
        assert_eq!(c.len(), 7, "path, stage, compiled, reason, instructions, threads, uniforms: {l}");
        let stage = if c[0].ends_with(".vert") { "vs" } else { "fs" };
        assert_eq!(c[1], stage, "{}: stage", c[0]);
        match c[2] {
            "yes" => {
                compiled += 1;
                assert_eq!(c[3], "ok", "{}", c[0]);
                for n in &c[4..] {
                    n.parse::<u32>().unwrap_or_else(|_| panic!("{}: `{n}` is not a count", c[0]));
                }
            }
            "no" => {
                assert!(!c[3].is_empty() && c[3] != "ok", "{}: a refusal names its reason", c[0]);
                assert_eq!(&c[4..], ["-", "-", "-"], "{}", c[0]);
            }
            v => panic!("{}: compiled is yes or no, not `{v}`", c[0]),
        }
        have.push(c[0].to_string());
    }
    let set: BTreeSet<String> = have.iter().cloned().collect();
    assert_eq!(set.len(), have.len(), "a program is listed twice");
    assert_eq!(set, want, "the table's programs differ from tools/shaders/es100's sources");
    let mut sorted = have.clone();
    sorted.sort();
    assert_eq!(have, sorted, "rows are sorted by path");
    assert_eq!(compiled, COMPILED, "Mesa compiles {COMPILED} of {PROGRAMS} at 7.1");
    assert!(text.contains(&format!("# programs: {PROGRAMS}, compiled: {COMPILED}, refused: {}", PROGRAMS - COMPILED)));
    println!("v3d71 mesa corpus: {PROGRAMS} es100 programs in the table, Mesa compiles {compiled} at 7.1");
}

#[test]
fn mesa_reproduces_the_tsv() {
    let shc = std::env::var("V3D_SHC").unwrap_or_default();
    if shc.is_empty() || !Path::new(&shc).is_file() {
        println!("gap: V3D_SHC unset or not a file: Mesa's 7.1 output was not rerun; the table was not compared (not a pass)");
        return;
    }
    let tmp = std::env::temp_dir().join(format!("es100_counts_71-{}.tsv", std::process::id()));
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/v3d/es100_ref71.sh");
    let out = std::process::Command::new("bash")
        .arg(&script)
        .arg("--out")
        .arg(&tmp)
        .output()
        .expect("bash runs es100_ref71.sh");
    assert!(
        out.status.success(),
        "es100_ref71.sh failed: {}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let got = std::fs::read_to_string(&tmp).expect("the script wrote its table");
    let _ = std::fs::remove_file(&tmp);
    let want = std::fs::read_to_string(tsv_path()).expect("es100_counts_71.tsv exists");
    if got != want {
        let diff: Vec<String> = got
            .lines()
            .zip(want.lines())
            .filter(|(a, b)| a != b)
            .map(|(a, b)| format!("  now:    {a}\n  pinned: {b}"))
            .collect();
        panic!(
            "Mesa's 7.1 output moved from tools/v3d/es100_counts_71.tsv ({} rows differ):\n{}",
            diff.len(),
            diff.join("\n")
        );
    }
    println!("v3d71 mesa corpus: {}", String::from_utf8_lossy(&out.stdout).trim());
}
