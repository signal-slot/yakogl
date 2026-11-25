// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The start and end lines the Pi 5 images print (gl_test's `v3d_gl_test
//! RECORD end`, deqp's `v3d_deqp: build <id> DONE|INCOMPLETE|STOPPED`) against
//! `tools/board.sh pi5`'s `END_RE_DEFAULT`: if either side is renamed, the
//! board run would no longer end on the image's own end and would sit until
//! its bound.
//!
//! The sources are read with `include_str!`, so cargo rebuilds this test when
//! one of them changes and the gate's tree key sees it.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d71_rpi5_markers
//! ```

const BOARD_SH: &str = include_str!("../tools/board.sh");
const GL_TEST_C: &str = include_str!("../driver/samples/zephyr/gl_test/src/main_rpi4.c");
const DEQP_C: &str = include_str!("../driver/samples/zephyr/deqp/src/v3d_board.c");

/// The alternatives of the pi5 `END_RE_DEFAULT` (the one with the gl_test end).
fn end_alternatives() -> Vec<&'static str> {
    let line = BOARD_SH
        .lines()
        .find(|l| l.contains("END_RE_DEFAULT='") && l.contains("v3d_gl_test RECORD end"))
        .expect("pi5 END_RE_DEFAULT line");
    let body = line.split_once("='^(").expect("regex start").1;
    let body = body.rsplit_once(")'").expect("regex end").0;
    // split at the top-level '|' (the groups in it are not split)
    let mut out = Vec::new();
    let (mut depth, mut start) = (0, 0);
    for (i, c) in body.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            '|' if depth == 0 => {
                out.push(&body[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&body[start..]);
    out
}

#[test]
fn gl_test_end_line_is_a_pi5_end() {
    let alts = end_alternatives();
    assert!(alts.contains(&"v3d_gl_test RECORD end"), "{alts:?}");
    assert!(
        GL_TEST_C.contains("printk(\"v3d_gl_test RECORD end, build %s\\n\", V3D_BUILD)"),
        "gl_test no longer prints 'v3d_gl_test RECORD end, build <id>'"
    );
    assert!(GL_TEST_C.contains("v3d_gl_test RECORD, build %s"), "gl_test record start line");
}

#[test]
fn deqp_end_line_is_a_pi5_end() {
    let alts = end_alternatives();
    assert!(alts.contains(&"v3d_deqp: build [^ ]+ (DONE|INCOMPLETE|STOPPED)"), "{alts:?}");
    // the summary line is "v3d_deqp: build %s %s: ..." with %s the ending
    assert!(DEQP_C.contains("\"v3d_deqp: build %s %s: %u cases"), "deqp summary line");
    assert!(DEQP_C.contains("printk(\"v3d_deqp: build %s\\n\", V3D_BUILD)"), "deqp start line");
    for word in ["\"DONE", "\"INCOMPLETE", "\"STOPPED"] {
        assert!(DEQP_C.contains(word), "deqp ending {word}");
    }
}

#[test]
fn end_regex_has_no_other_gl_test_or_deqp_form() {
    let alts = end_alternatives();
    let gl: Vec<_> = alts.iter().filter(|a| a.starts_with("v3d_gl_test")).collect();
    assert_eq!(gl, [&"v3d_gl_test RECORD end"]);
    let dq: Vec<_> = alts.iter().filter(|a| a.starts_with("v3d_deqp")).collect();
    assert_eq!(dq.len(), 1, "{dq:?}");
}
