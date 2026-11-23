// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Our side of the corpus differential (`tools/mali/corpus_diff.py`): each
//! program of a directory built as glCompileShader and glLinkProgram build
//! it on the Mali ([`mali_ref::compile_pair`]) and compared with its Mesa
//! 26.2.1 reference ([`mali_ref::differences_members`]), one line each.
//!
//! ```text
//! YAKOGL_MALI_CORPUS_DIR=<dir> YAKOGL_MALI_CORPUS_LIST=<file> \
//!   cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table \
//!   --test mali_corpus_diff -- --nocapture --test-threads 1
//! ```
//!
//! The directory holds `<name>.vert` and `<name>.frag` per program, and
//! `<name>.expect.json` when Mesa gave a reference; `<name>.zero-uniforms`
//! marks a reference drawn without `--uniform-values` (every uniform word
//! 0). The list names the programs to run, one per line, in order. Each
//! line printed is `corpus\t<name>\t<WORD>\t<detail>`, WORD one of EQUAL,
//! DIFFER (the detail: every difference, `; `-joined), REFUSED (the
//! refusal: the code generator's text, or the log), NOREF (compiled, no
//! reference) and ERROR (the compare panicked: the panic's first line).
//! The script runs several processes on shares of the list, and reads a
//! process that died as dying in the program after its last line.
//!
//! This is `tests/mali_ref`'s compare, the one every `deqp_*` reference and
//! the audit (`docs/notes/mali-deqp-compiled-audit.md`) use: its uniform
//! words follow gl_ref.py's for every GL type, struct members and
//! `gl_DepthRange` included. Without the variables the test checks itself
//! on committed references: an equal program, one against another's
//! reference, a refused one, one with no reference, one drawn without
//! uniform values.
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

use std::path::Path;

/// One program's line: the word and the detail.
fn measure(dir: &Path, name: &str) -> (&'static str, String) {
    let read = |ext: &str| std::fs::read_to_string(dir.join(format!("{name}.{ext}")));
    let (vs, fs) = (read("vert").expect("the vertex shader"), read("frag").expect("the fragment shader"));
    let json = read("expect.json").ok();
    let zero = dir.join(format!("{name}.zero-uniforms")).exists();
    let flat = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
    let run = std::panic::catch_unwind(|| {
        let pair = match mali_ref::compile_pair(&vs, &fs) {
            Ok(p) => p,
            Err(e) => return ("REFUSED", flat(&e)),
        };
        let Some(json) = json.clone() else { return ("NOREF", "-".to_string()) };
        let r = mali_ref::Reference { name: name.into(), vs: vs.clone(), fs: fs.clone(), json, uniform_values: !zero };
        let d = mali_ref::differences_members(&r, &pair, !zero);
        if d.is_empty() {
            ("EQUAL", "-".to_string())
        } else {
            /* each difference's first line: the word lists stay out */
            ("DIFFER", d.iter().map(|m| flat(m.lines().next().unwrap_or(""))).collect::<Vec<_>>().join("; "))
        }
    });
    run.unwrap_or_else(|e| {
        let m = e.downcast_ref::<String>().cloned().or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string()));
        ("ERROR", flat(m.unwrap_or_default().lines().next().unwrap_or("a panic")))
    })
}

#[test]
fn the_corpus_programs_are_measured_against_mesa() {
    if let Some(dir) = std::env::var_os("YAKOGL_MALI_CORPUS_DIR") {
        let dir = std::path::PathBuf::from(dir);
        let list = std::env::var("YAKOGL_MALI_CORPUS_LIST").expect("YAKOGL_MALI_CORPUS_LIST names the programs");
        let names = std::fs::read_to_string(&list).unwrap_or_else(|e| panic!("{list}: {e}"));
        for name in names.lines().map(str::trim).filter(|n| !n.is_empty()) {
            let (word, detail) = measure(&dir, name);
            println!("corpus\t{name}\t{word}\t{detail}");
        }
        return;
    }
    let td = Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/mali/testdata");
    assert_eq!(measure(&td, "tri_varying"), ("EQUAL", "-".to_string()));
    let (word, _) = measure(&td, "fstex_cubeno_texcoord_allconst");
    assert_eq!(word, "REFUSED");
    /* a program against another program's reference, and one with none */
    let tmp = std::env::temp_dir().join(format!("mali_corpus_diff.{}", std::process::id()));
    std::fs::create_dir_all(&tmp).expect("a scratch directory");
    let copy = |from: &str, to: &str| std::fs::copy(td.join(from), tmp.join(to)).expect("a testdata file");
    copy("tri_varying.vert", "x.vert");
    copy("tri_varying.frag", "x.frag");
    copy("tri.expect.json", "x.expect.json");
    copy("tri_varying.vert", "y.vert");
    copy("tri_varying.frag", "y.frag");
    /* gm2_* references are drawn without --uniform-values */
    copy("gm2_fma_1.vert", "z.vert");
    copy("gm2_fma_1.frag", "z.frag");
    copy("gm2_fma_1.expect.json", "z.expect.json");
    let (x, y, z_values) = (measure(&tmp, "x"), measure(&tmp, "y"), measure(&tmp, "z"));
    std::fs::write(tmp.join("z.zero-uniforms"), "").expect("a marker");
    let z_zero = measure(&tmp, "z");
    std::fs::remove_dir_all(&tmp).expect("the scratch directory removed");
    assert_eq!(x.0, "DIFFER", "{x:?}");
    /* tri draws no varying: tri_varying's varying program is the difference */
    assert!(x.1.starts_with("vs.varying: we have a varying program"), "{x:?}");
    assert_eq!(y, ("NOREF", "-".to_string()));
    assert_eq!(z_zero, ("EQUAL", "-".to_string()));
    assert_eq!(z_values.0, "DIFFER", "a reference drawn with every uniform 0 read as drawn with values: {z_values:?}");
    assert!(z_values.1.contains("FAU words"), "{z_values:?}");
}
