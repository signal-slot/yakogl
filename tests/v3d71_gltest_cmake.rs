// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Pins the source lists of the gl_test and deqp samples' CMakeLists.txt
//! (plan #404, item gtcmake71t). Before the V3D71 branch existed, the rpi_5
//! image took the Pi 4 branch and linked rpi4/rpi_wdog.c and rpi_mbox.c twice
//! (once from the branch, once from the board layer, which YAKOGL_GPU_V3D71
//! makes build them). So: the V3D71 branch adds the rpi_5 sources and no
//! rpi4/ `.c` file (headers on the include path are fine), and the 4.2 branch
//! keeps its own rpi4 sources. A merge of plan #403's STM32MP2 edits that
//! moves a line across a branch fails here, on the host, before a link error.
//!
//! The files are read with `include_str!`, so tools/check.sh's
//! TEST_TREE_PATHS needs no entry; a change reaching only the CMakeLists does
//! not select this test's row in a `--changed` run, the nightly run does.
//!
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d71_gltest_cmake

#![cfg(all(feature = "shaderc-online", feature = "gpu-v3d"))]

static GL_TEST: &str = include_str!("../driver/samples/zephyr/gl_test/CMakeLists.txt");
static DEQP: &str = include_str!("../driver/samples/zephyr/deqp/CMakeLists.txt");

const GL_PATH: &str = "gl_test/CMakeLists.txt";
const DEQP_PATH: &str = "deqp/CMakeLists.txt";

/// The lines of the branch that starts at the first line whose text (trimmed)
/// is `start`, up to the next line at column 0 that begins `elseif(`, `else(`
/// or `endif(` (the branch's own nested ifs are indented). Each line comes
/// with its 1-based number, comments cut off, empty lines dropped.
fn branch(text: &str, path: &str, start: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let mut open = false;
    let mut found = false;
    for (i, raw) in text.lines().enumerate() {
        let n = i + 1;
        if !open {
            if raw.trim() == start {
                open = true;
                found = true;
            }
            continue;
        }
        if raw.starts_with("elseif(") || raw.starts_with("else(") || raw.starts_with("endif(") {
            break;
        }
        let code = raw.split('#').next().unwrap_or("").trim();
        if !code.is_empty() {
            out.push((n, code.to_string()));
        }
    }
    assert!(found, "{path}: no line `{start}`");
    assert!(!out.is_empty(), "{path}: the branch `{start}` is empty");
    out
}

/// The `.c` source words of a branch: every word (split at blanks, quotes and
/// parentheses) that ends in `.c`, with its line number.
fn c_sources(b: &[(usize, String)]) -> Vec<(usize, String)> {
    let mut v = Vec::new();
    for (n, l) in b {
        for w in l.split(|c: char| c.is_whitespace() || "\"()".contains(c)) {
            if w.ends_with(".c") {
                v.push((*n, w.to_string()));
            }
        }
    }
    v
}

fn has_source(srcs: &[(usize, String)], tail: &str) -> bool {
    srcs.iter().any(|(_, w)| w.ends_with(tail))
}

/// No `.c` of the branch is the Pi 4 board layer's: not under `rpi4/`, not
/// through `${V3D_BOARD}` (the 4.2 branches' variable for that directory),
/// and none of the Pi 4 files by name.
fn assert_no_rpi4_source(path: &str, b: &[(usize, String)]) {
    for (n, w) in c_sources(b) {
        let name = w.rsplit('/').next().unwrap_or(&w);
        let bad = w.contains("rpi4/")
            || w.contains("V3D_BOARD")
            || ["rpi_wdog.c", "rpi_mbox.c", "fbcon.c", "main_rpi4.c", "v3d_board.c"].contains(&name);
        assert!(!bad, "{path}:{n}: the V3D71 branch lists the Pi 4 source `{w}`");
    }
}

#[test]
fn gl_test_v3d71_branch_has_rpi5_sources_and_no_rpi4_ones() {
    let b = branch(GL_TEST, GL_PATH, "if(CONFIG_YAKOGL_GPU_V3D71)");
    let srcs = c_sources(&b);
    assert!(has_source(&srcs, "/main_rpi5.c"), "{GL_PATH}: the V3D71 branch does not list main_rpi5.c");
    assert_no_rpi4_source(GL_PATH, &b);
    // The board layer's own symbols stay a Kconfig matter: the branch checks
    // they are on instead of adding the files.
    let text = b.iter().map(|(_, l)| l.as_str()).collect::<Vec<_>>().join("\n");
    for sym in ["CONFIG_YAKOGL_RPI5_WDOG", "CONFIG_YAKOGL_RPI5_MBOX"] {
        assert!(text.contains(sym), "{GL_PATH}: the V3D71 branch no longer checks {sym}");
    }
}

#[test]
fn gl_test_v3d_42_branch_keeps_its_rpi4_sources() {
    let b = branch(GL_TEST, GL_PATH, "elseif(CONFIG_YAKOGL_GPU_V3D)");
    let srcs = c_sources(&b);
    for tail in ["/main_rpi4.c", "/rpi_mbox.c", "/fbcon.c", "/rpi_wdog.c"] {
        assert!(has_source(&srcs, tail), "{GL_PATH}: the 4.2 branch no longer lists `{tail}`");
    }
    assert!(
        b.iter().any(|(_, l)| l.contains("V3D_BOARD") && l.contains("lib/board/rpi4")),
        "{GL_PATH}: the 4.2 branch's V3D_BOARD is no longer zephyr/lib/board/rpi4"
    );
}

#[test]
fn gl_test_branch_order_is_v3d71_first() {
    // The 4.2 branch's test is CONFIG_YAKOGL_GPU_V3D, which a 7.1 build also
    // sets; the V3D71 branch must come first or it is never taken.
    let p71 = GL_TEST.find("if(CONFIG_YAKOGL_GPU_V3D71)");
    let p42 = GL_TEST.find("elseif(CONFIG_YAKOGL_GPU_V3D)");
    assert!(
        matches!((p71, p42), (Some(a), Some(b)) if a < b),
        "{GL_PATH}: the V3D71 branch is not ahead of the 4.2 branch"
    );
}

#[test]
fn deqp_v3d71_branch_has_rpi5_sources_and_no_rpi4_ones() {
    let b = branch(DEQP, DEQP_PATH, "if(CONFIG_YAKOGL_GPU_V3D71)");
    let srcs = c_sources(&b);
    assert!(has_source(&srcs, "/v3d71_board.c"), "{DEQP_PATH}: the V3D71 branch does not list v3d71_board.c");
    assert_no_rpi4_source(DEQP_PATH, &b);
    // The 4.2 branch is switched off for a 7.1 build.
    assert!(
        b.iter().any(|(_, l)| l == "set(CONFIG_YAKOGL_GPU_V3D OFF)"),
        "{DEQP_PATH}: the V3D71 branch no longer hides CONFIG_YAKOGL_GPU_V3D from the 4.2 branch"
    );
}

#[test]
fn deqp_v3d_42_branch_keeps_its_rpi4_sources() {
    let b = branch(DEQP, DEQP_PATH, "if(CONFIG_YAKOGL_GPU_V3D)");
    let srcs = c_sources(&b);
    for tail in ["/v3d_board.c", "/rpi_mbox.c", "/fbcon.c", "/rpi_wdog.c"] {
        assert!(has_source(&srcs, tail), "{DEQP_PATH}: the 4.2 branch no longer lists `{tail}`");
    }
}
