// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The Raspberry Pi 5 deqp image's configuration and markers, pinned: the
//! board's conf (ES 2.0 reported, no limit override: docs/notes/rpi5-deqp-
//! glmark2.md section 8), its overlay against the gl_test rpi_5 overlay, the
//! build and end lines of `src/v3d71_board.c` against what
//! `tools/cts/deqp-uart-tally.py` and `tools/board.sh pi5`'s `END_RE_DEFAULT`
//! expect, and the watchdog's lifetime (`V3D_DEQP_LIFETIME_S`).
//!
//! The ES 2.0 setting reaches the conf through the Kconfig default of
//! `DEQP_REPORT_ES2` (`default y`, `select YAKOGL_REPORT_ES2`), so the test
//! pins that the conf turns neither symbol off and that the Kconfig selects.
//!
//! The sources are read with `include_str!`, so cargo rebuilds this test when
//! one of them changes and the gate's tree key sees it.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d71_deqp_rpi5_conf
//! ```

const CONF: &str = include_str!("../driver/samples/zephyr/deqp/boards/rpi_5.conf");
const OVERLAY: &str = include_str!("../driver/samples/zephyr/deqp/boards/rpi_5.overlay");
const GL_OVERLAY: &str = include_str!("../driver/samples/zephyr/gl_test/boards/rpi_5.overlay");
const KCONFIG: &str = include_str!("../driver/samples/zephyr/deqp/Kconfig");
const CMAKE: &str = include_str!("../driver/samples/zephyr/deqp/CMakeLists.txt");
const BOARD_C: &str = include_str!("../driver/samples/zephyr/deqp/src/v3d71_board.c");
const V3D_BOARD_C: &str = include_str!("../driver/samples/zephyr/deqp/src/v3d_board.c");
const BOARD_SH: &str = include_str!("../tools/board.sh");
const TALLY_PY: &str = include_str!("../tools/cts/deqp-uart-tally.py");

/// The active `CONFIG_X=value` lines of the conf (comments dropped).
fn settings() -> Vec<(&'static str, &'static str)> {
    CONF.lines().map(str::trim).filter(|l| !l.starts_with('#')).filter_map(|l| l.split_once('=')).collect()
}

fn setting(name: &str) -> Option<&'static str> {
    settings().into_iter().find(|(k, _)| *k == name).map(|(_, v)| v)
}

/// The `sram0` reg cells of an overlay: `<0x0 base size>`.
fn sram_reg(overlay: &str) -> Vec<String> {
    let at = overlay.find("&sram0").expect("&sram0 node");
    let rest = &overlay[at..];
    let reg = rest.split_once("reg = <").expect("reg").1;
    reg.split_once('>').expect("reg end").0.split_whitespace().map(String::from).collect()
}

#[test]
fn conf_reports_es2_and_overrides_no_limit() {
    // the ES 2.0 report is the Kconfig default, not turned off here
    assert_ne!(setting("CONFIG_YAKOGL_REPORT_ES2"), Some("n"));
    assert_ne!(setting("CONFIG_DEQP_REPORT_ES2"), Some("n"));
    let sym = KCONFIG.split_once("config DEQP_REPORT_ES2").expect("symbol").1;
    let sym = sym.split_once("\nconfig ").map_or(sym, |s| s.0);
    assert!(sym.contains("default y"), "DEQP_REPORT_ES2 default");
    assert!(sym.contains("select YAKOGL_REPORT_ES2"), "DEQP_REPORT_ES2 select");
    // 7.1 table on the V3D, and no limit override of any kind
    assert_eq!(setting("CONFIG_YAKOGL_GPU_V3D"), Some("y"));
    assert_eq!(setting("CONFIG_YAKOGL_GPU_V3D71"), Some("y"));
    for (k, _) in settings() {
        for bad in ["LIMIT", "MAX_TEXTURE", "VARYING", "UNIFORM", "DRAW_BUFFERS", "ATTRIB"] {
            assert!(!k.contains(bad), "limit override {k}");
        }
    }
}

#[test]
fn overlay_matches_gl_test_where_it_must() {
    let (d, g) = (sram_reg(OVERLAY), sram_reg(GL_OVERLAY));
    // two address cells, the same load address; the deqp window is larger
    assert_eq!(d.len(), 3, "{d:?}");
    assert_eq!(d[..2], g[..2], "deqp {d:?} gl_test {g:?}");
    assert_eq!(d[1], "0x200000");
    assert_eq!(d[2], "0x10000000", "256 MiB, the Pi 4 deqp window");
    assert_eq!(g[2], "0x6000000", "gl_test's 96 MiB");
    // the console is the board defconfig's uart10 in both: neither overlay
    // names a uart or a chosen node
    for (name, o) in [("deqp", OVERLAY), ("gl_test", GL_OVERLAY)] {
        let code: String = o.lines().filter(|l| !l.trim_start().starts_with(['/', '*'])).collect();
        for node in ["uart", "chosen", "zephyr,console", "zephyr,shell-uart"] {
            assert!(!code.contains(node), "{name} overlay sets {node}");
        }
    }
}

#[test]
fn board_prints_build_line_and_end_format() {
    assert!(BOARD_C.contains("printk(\"v3d_deqp: build %s\\n\", V3D_BUILD);"));
    // the end line comes from v3d_board.c, which this file includes unedited
    assert!(BOARD_C.contains("#include \"v3d_board.c\""));
    let end = "printk(\"v3d_deqp: build %s %s: %u cases, P %u F %u R %u N %u other %u\\n\"";
    assert!(V3D_BOARD_C.contains(end), "end line format");
    for word in ["\"DONE", "\"INCOMPLETE", "\"STOPPED"] {
        assert!(V3D_BOARD_C.contains(word), "ending {word}");
    }
    // the consumers
    assert!(BOARD_SH.contains("v3d_deqp: build [^ ]+ (DONE|INCOMPLETE|STOPPED)"));
    assert!(
        TALLY_PY.contains(r"^v3d_deqp: build \S+ DONE[^:]*: (\d+) cases, P (\d+) F (\d+) R (\d+) N (\d+) other (\d+)")
    );
}

#[test]
fn watchdog_lifetime_is_the_deqp_lifetime() {
    // the record's deadline is lifetime minus photo, in the board's own file
    assert!(BOARD_C.contains("CONFIG_V3D_DEQP_LIFETIME_S - CONFIG_V3D_DEQP_PHOTO_S"));
    assert!(BOARD_C.contains("rpi_wdog_announce();"));
    // the hold equals the lifetime: the conf's value is the Kconfig default,
    // the CMake branch refuses an image where they differ, and passes the
    // lifetime to the watchdog
    let kc = KCONFIG.split_once("config V3D_DEQP_LIFETIME_S").expect("symbol").1;
    let default = kc.lines().find_map(|l| l.trim().strip_prefix("default ")).expect("default");
    assert_eq!(setting("CONFIG_YAKOGL_RPI5_WDOG_HOLD_S"), Some(default));
    assert!(CMAKE.contains("NOT CONFIG_YAKOGL_RPI5_WDOG_HOLD_S EQUAL CONFIG_V3D_DEQP_LIFETIME_S"));
    assert!(CMAKE.contains("RPI_WDOG_HOLD_S=$"));
    assert!(CMAKE.contains("CONFIG_V3D_DEQP_LIFETIME_S"));
}
