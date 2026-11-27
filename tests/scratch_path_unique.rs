// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Every test that writes under `CARGO_TARGET_TMPDIR` or the system temp
//! directory names its files by process id.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test scratch_path_unique
//! ```
//!
//! tools/check.sh runs the test rows of several feature sets at once, and a
//! test binary built for one set is the same source built for another: all of
//! them share one `CARGO_TARGET_TMPDIR` (`target/tmp`). A fixed path such as
//! `.join("v3d71_cube_packed")` let one row rewrite or relink the executable
//! another row was running, and the test failed with "Text file busy",
//! "Permission denied" or "No such file" at a `spawn().unwrap()` -- on one
//! pull request in a few, at a different test each time, and unrelated to the
//! change under test (tests/v3d71_cube_packed.rs, tests/v3d71_texfill.rs and
//! tests/v3d71_ref/mod.rs, docs/ci.md section 3.9).
//!
//! The check is the lexical one: a file that names `CARGO_TARGET_TMPDIR` or
//! `temp_dir()` must also contain `process::id()` (or `tempfile`, which makes
//! a unique name itself). It cannot prove that every path uses it; it stops
//! the file that uses none, which is how all three were written.

use std::path::{Path, PathBuf};

fn rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).expect("tests/ is readable").flatten() {
        let p = e.path();
        if p.is_dir() {
            rs_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

#[test]
fn a_test_that_writes_scratch_files_names_them_by_process() {
    let mut files = Vec::new();
    rs_files(&Path::new(env!("CARGO_MANIFEST_DIR")).join("tests"), &mut files);
    files.sort();
    let uses = ["CARGO_TARGET_TMPDIR", "temp_dir()"];
    let unique = ["process::id()", "tempfile"];
    let mut bad = Vec::new();
    for f in &files {
        if f.ends_with("scratch_path_unique.rs") {
            continue;
        }
        let s = std::fs::read_to_string(f).expect("a UTF-8 source");
        if uses.iter().any(|u| s.contains(u)) && !unique.iter().any(|u| s.contains(u)) {
            bad.push(f.strip_prefix(env!("CARGO_MANIFEST_DIR")).unwrap().display().to_string());
        }
    }
    assert!(
        bad.is_empty(),
        "these tests write under a directory every row shares and do not name the path by \
         std::process::id(): {bad:?} (tests/scratch_path_unique.rs)"
    );
}
