// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The rpi_5 deqp image's build line and end formats, checked from source:
//! `src/v3d71_board.c` (the v3d71 board file) emits `v3d_deqp: build <id>`,
//! includes `src/v3d_board.c` whose end lines carry the state words DONE,
//! INCOMPLETE and STOPPED, and `tools/board.sh`'s pi5 verdict code matches
//! exactly those words. The watchdog announcement and the UART record path
//! are present too. Acceptance c3 can then be checked without a built ELF.
//!
//! std only; the sources are read with `include_str!`.
//!
//! ```text
//! cargo test --test v3d71_deqp_rpi5_formats
//! ```

const BOARD71: &str = include_str!("../driver/samples/zephyr/deqp/src/v3d71_board.c");
const BOARD: &str = include_str!("../driver/samples/zephyr/deqp/src/v3d_board.c");
const BOARD_SH: &str = include_str!("../tools/board.sh");

const STATES: [&str; 3] = ["DONE", "INCOMPLETE", "STOPPED"];

#[test]
fn board_file_emits_the_build_line() {
    assert!(BOARD71.contains(r#"printk("v3d_deqp: build %s\n", V3D_BUILD);"#));
    assert!(BOARD71.contains(r#"#include "v3d_board.c""#), "the end lines come from v3d_board.c");
}

#[test]
fn end_line_formats_are_emitted() {
    // the summary line: 'v3d_deqp: build <id> <state>: N cases, ...'
    assert!(BOARD.contains(r#"printk("v3d_deqp: build %s %s: %u cases, P %u F %u R %u N %u other %u\n""#));
    // the record header: 'v3d_deqp <ending>: build <id>, filter ...'
    assert!(BOARD.contains(r#""v3d_deqp %s: build %s, filter ""#));
    // every state word starts an ending string
    assert!(BOARD.contains(r#"take_record("INCOMPLETE (the photo deadline came first)")"#));
    assert!(BOARD.contains(r#""DONE, all passed""#));
    assert!(BOARD.contains(r#""STOPPED EARLY by dEQP"#));
}

#[test]
fn board_sh_pi5_matches_the_state_words() {
    let end_re = BOARD_SH
        .lines()
        .filter(|l| l.contains("END_RE_DEFAULT='") && l.contains("v3d_deqp: build"))
        .map(str::trim)
        .collect::<Vec<_>>();
    assert!(!end_re.is_empty());
    for l in &end_re {
        assert!(l.contains("v3d_deqp: build [^ ]+ (DONE|INCOMPLETE|STOPPED)"), "{l}");
    }
    // the verdict code: the state word after the build id, and the failing ones
    assert!(BOARD_SH.contains("s/^v3d_deqp: build [^ ]+ (DONE|INCOMPLETE|STOPPED)[: ].*/\\1/p"));
    assert!(BOARD_SH.contains("v3d_deqp_INCOMPLETE | v3d_deqp_STOPPED) verdict=failed"));
    for s in STATES {
        assert!(BOARD.contains(&format!("\"{s}")), "{s} is not an ending of v3d_board.c");
    }
}

#[test]
fn watchdog_and_uart_record_paths_are_present() {
    assert!(BOARD71.contains("rpi_wdog_announce();"));
    assert!(BOARD71.contains("CONFIG_YAKOGL_RPI5_WDOG"));
    assert!(BOARD71.contains("uart_out = __printk_get_hook();"));
    assert!(BOARD71.contains("__printk_hook_install(tap);"));
    assert!(BOARD71.contains("__stdout_hook_install(stdout_tap);"));
    assert!(BOARD.contains("uart_out((unsigned char)c);"));
}
