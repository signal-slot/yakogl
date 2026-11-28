// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The Pi 5 gl_test's end line and per-case verdict lines against what
//! `tools/board.sh pi5` reads: the end line `v3d_gl_test RECORD end, build
//! <id>` (main_rpi5.c) must start with the text of the pi5 `END_RE_DEFAULT`'s
//! gl_test alternative, and the runner's verdict line `gl_test: case <n>
//! <name>: PASS|FAIL|TIMEOUT` (driver/tools/pvrlab/runner, which gl_test.c
//! links) must match the pattern `single_case` greps (`gl_test: case $n [^:]*:
//! (PASS|FAIL|TIMEOUT)`) and the refusal reason must be told apart by the
//! `yakogl/v3d: draw refused: ` prefix that `tools/cts/pi5-gltest-verdicts.sh`
//! reads. The sources are read with `include_str!` as text; std only.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d71_gltest_endline
//! ```

const BOARD_SH: &str = include_str!("../tools/board.sh");
const MAIN_RPI5: &str = include_str!("../driver/samples/zephyr/gl_test/src/main_rpi5.c");
const RUNNER: &str = include_str!("../driver/tools/pvrlab/runner/src/lib.rs");
const BACKEND: &str = include_str!("../src/v3d/backend.rs");
const VERDICTS_SH: &str = include_str!("../tools/cts/pi5-gltest-verdicts.sh");

/// The gl_test alternative of the pi5 END_RE_DEFAULT, as literal text.
fn end_text() -> &'static str {
    let line = BOARD_SH
        .lines()
        .find(|l| l.contains("END_RE_DEFAULT='") && l.contains("v3d_gl_test: FAIL"))
        .expect("pi5 END_RE_DEFAULT line");
    assert!(line.contains("|v3d_gl_test RECORD end|"), "{line}");
    "v3d_gl_test RECORD end"
}

/// The C format string of the first `printk("<prefix>...")` in `src`.
fn printk_format(src: &str, prefix: &str) -> String {
    let at = src.find(&format!("printk(\"{prefix}")).unwrap_or_else(|| panic!("no printk {prefix}"));
    let rest = &src[at + "printk(\"".len()..];
    rest[..rest.find('"').expect("format end")].to_string()
}

#[test]
fn end_line_format_starts_with_the_board_end_text() {
    let fmt = printk_format(MAIN_RPI5, "v3d_gl_test RECORD end");
    assert_eq!(fmt, "v3d_gl_test RECORD end, build %s\\n");
    assert!(fmt.starts_with(end_text()));
    // board.sh's anchored regex: the line begins with the text (after the
    // console's optional stamp, which the end pattern does not see).
    let printed = fmt.replace("%s", "abc1234").replace("\\n", "");
    assert!(printed.starts_with(end_text()), "{printed}");
}

#[test]
fn per_case_record_line_names_the_verdicts_board_reads() {
    // main_rpi5.c's record: "v3d_gl_test: case %d %s" with rc_name().
    assert_eq!(printk_format(MAIN_RPI5, "v3d_gl_test: case "), "v3d_gl_test: case %d %s\\n");
    for w in ["\"PASS\"", "\"FAIL\"", "\"SKIP\"", "\"TIMEOUT\""] {
        assert!(MAIN_RPI5.contains(&format!("return {w};")), "rc_name lacks {w}");
    }
    assert!(VERDICTS_SH.contains("/^v3d_gl_test: case [0-9]+ /"));
}

/// `gl_test: case $n [^:]*: (PASS|FAIL|TIMEOUT)` as `single_case` greps it.
fn single_case_matches(line: &str, n: i32) -> bool {
    let pre = format!("gl_test: case {n} ");
    let Some(at) = line.find(&pre) else { return false };
    let rest = &line[at + pre.len()..];
    let Some((name, tail)) = rest.split_once(':') else { return false };
    let _ = name; // [^:]* is the name, whatever it holds but a colon
    let tail = tail.strip_prefix(' ').unwrap_or("\u{0}");
    ["PASS", "FAIL", "TIMEOUT"].iter().any(|v| tail.starts_with(v))
}

#[test]
fn runner_verdict_line_matches_the_board_pattern() {
    assert!(
        BOARD_SH.contains(r#"grep -aoE "gl_test: case $n [^:]*: (PASS|FAIL|TIMEOUT)""#),
        "board.sh single_case pattern changed"
    );
    assert!(BOARD_SH.contains(r#"grep -qE ': (FAIL|TIMEOUT)$'"#));
    assert!(RUNNER.contains(r#"writeln!(l, "gl_test: case {c} {name}: {verdict}")"#));
    for v in ["PASS", "FAIL", "TIMEOUT"] {
        assert!(RUNNER.contains(&format!("=> \"{v}\"")), "runner lacks verdict {v}");
        let line = format!("gl_test: case 59 clear: {v}");
        assert!(single_case_matches(&line, 59), "{line}");
    }
    // a skip is not a verdict of the board's pattern
    assert!(!single_case_matches("gl_test: case 59 clear: SKIP (no)", 59));
    assert!(!single_case_matches("gl_test: case 5 tri: PASS", 59));
}

#[test]
fn refusal_line_is_the_prefix_the_verdict_reader_reads() {
    assert!(BACKEND.contains(r#"glog!("yakogl/v3d: draw refused: {}", why)"#));
    assert!(VERDICTS_SH.contains("/^yakogl\\/v3d: draw refused: /"));
    assert!(VERDICTS_SH.contains("/^gl_test: case [0-9]+ [^ :]+: /"));
}
