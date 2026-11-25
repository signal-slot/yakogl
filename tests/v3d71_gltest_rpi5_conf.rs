// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Pins the Raspberry Pi 5's gl_test image configuration (plan #404, item
//! gltest71): the six cases of P9, the memory windows of the board overlay,
//! the console, and the PM watchdog's hold. A change to one of these without
//! the others (a case list that outgrows the hold, a carve-out that no longer
//! fits the SRAM window, a second screen) fails here, on the host, before a
//! board run of about four minutes finds it.
//!
//! The files are read with `include_str!`, so the compiler reads them and
//! tools/check.sh's TEST_TREE_PATHS needs no entry. The price: a change that
//! reaches only those files (driver/samples/zephyr/gl_test, driver/zephyr/
//! lib/board/Kconfig, zephyr/Kconfig, tools/board.sh) does not select this
//! test's row in a `--changed` run; the nightly run does.
//!
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d71_gltest_rpi5_conf

#![cfg(all(feature = "shaderc-online", feature = "gpu-v3d"))]

static CONF: &str = include_str!("../driver/samples/zephyr/gl_test/boards/rpi_5.conf");
static OVERLAY: &str = include_str!("../driver/samples/zephyr/gl_test/boards/rpi_5.overlay");
static GL_KCONFIG: &str = include_str!("../driver/samples/zephyr/gl_test/Kconfig");
static BOARD_KCONFIG: &str = include_str!("../driver/zephyr/lib/board/Kconfig");
static ZEPHYR_KCONFIG: &str = include_str!("../zephyr/Kconfig");
static BOARD_SH: &str = include_str!("../tools/board.sh");

/// The body of `config <name>` entries: from the entry's line to the next
/// line that is neither indented nor a comment nor empty (the next `config`,
/// `if`, `endif`, `menu`, `choice`, ...). One element per entry of that name.
fn entries<'a>(kconfig: &'a str, name: &str) -> Vec<Vec<&'a str>> {
    let head = format!("config {name}");
    let mut out: Vec<Vec<&str>> = Vec::new();
    let mut open = false;
    for line in kconfig.lines() {
        if line.trim_end() == head {
            out.push(Vec::new());
            open = true;
        } else if open {
            let t = line.trim();
            if !t.is_empty() && !line.starts_with(char::is_whitespace) && !t.starts_with('#') {
                open = false;
            } else if line.starts_with(char::is_whitespace) {
                // the help text is prose: it may say "default"
                if t == "help" {
                    open = false;
                } else {
                    out.last_mut().unwrap().push(t);
                }
            }
        }
    }
    out
}

/// The `default` lines of the entries of `name`, text after `default `.
fn defaults<'a>(kconfig: &'a str, name: &str) -> Vec<&'a str> {
    entries(kconfig, name).into_iter().flatten().filter_map(|l| l.strip_prefix("default ")).collect()
}

/// `CONFIG_X=value` lines of a .conf, value text.
fn conf(name: &str) -> Option<&'static str> {
    let key = format!("CONFIG_{name}=");
    CONF.lines().find_map(|l| l.strip_prefix(key.as_str()))
}

fn num(s: &str) -> u64 {
    match s.strip_prefix("0x") {
        Some(h) => u64::from_str_radix(h, 16).unwrap(),
        None => s.parse().unwrap(),
    }
}

#[test]
fn six_cases_of_p9_are_the_default_on_the_v3d71() {
    let d = defaults(GL_KCONFIG, "V3D_GL_TEST_CASES");
    assert_eq!(d.len(), 2, "one V3D71 default, one for the Pi 4: {d:?}");
    // the 7.1 default is read first (Kconfig takes the first true default)
    assert_eq!(d[0], "\"5,58,59,66,67,102\" if YAKOGL_GPU_V3D71");
    assert!(!d[1].contains(" if "), "the Pi 4's default is unconditional: {}", d[1]);
    // the image does not override the list
    assert!(conf("V3D_GL_TEST_CASES").is_none());
    assert!(conf("V3D_GL_TEST_CASE").is_none());
}

#[test]
fn the_image_selects_the_71_table_and_the_board_layer() {
    assert_eq!(conf("YAKOGL_GPU_V3D"), Some("y"));
    assert_eq!(conf("YAKOGL_GPU_V3D71"), Some("y"));
    let e = &entries(ZEPHYR_KCONFIG, "YAKOGL_GPU_V3D71")[0];
    assert!(e.contains(&"depends on YAKOGL_GPU_V3D && SOC_BCM2712"));
    assert!(e.contains(&"select YAKOGL_RPI5_WDOG"));
    assert!(e.contains(&"select YAKOGL_RPI5_MBOX"));
    // no screen: the board's framebuffer is not proven on the board
    assert!(conf("YAKOGL_BOARD_RPI5").is_none());
    assert!(CONF.contains("no screen"));
}

#[test]
fn memory_windows_fit_the_image() {
    // sram0: two address cells, base 0x200000 (the firmware's kernel= load
    // address), 96 MiB, below 4 GiB (the V3D's PTEs and the mailbox buffer)
    let reg = OVERLAY
        .lines()
        .find_map(|l| l.trim().strip_prefix("reg = <").and_then(|r| r.strip_suffix(">;")))
        .expect("the overlay's reg");
    let cells: Vec<u64> = reg.split_whitespace().map(num).collect();
    assert_eq!(cells, [0x0, 0x20_0000, 0x600_0000]);
    assert!(OVERLAY.contains("&sram0 {"));
    let (base, size) = (cells[1], cells[2]);
    assert_eq!(size, 96 << 20);
    assert!(base + size <= 1 << 32, "the window stays below 4 GiB");

    let carve = num(conf("YAKOGL_V3D_CARVEOUT_BYTES").expect("carve-out"));
    assert_eq!(carve, 48 << 20);
    // the carve-out is a static of the image: it leaves room for the code,
    // the compiler's 2 MiB scratch and the libc arena
    let arena = num(conf("COMMON_LIBC_MALLOC_ARENA_SIZE").expect("arena"));
    assert!(carve + (2 << 20) + arena < size);
    // the firmware puts the DTB at 0x2efec600, above the image's end (the
    // window's end, 0x6200000), so the window never covers it
    assert_eq!(base + size, 0x620_0000);
    assert!(0x2efe_c600 > base + size);
    // address space: the image, the carve-out's alias and the device blocks
    assert_eq!(num(conf("MAX_XLAT_TABLES").unwrap()), 64);
    assert_eq!(num(conf("KERNEL_VM_SIZE").unwrap()), 0x1000_0000);
    assert!(num(conf("KERNEL_VM_SIZE").unwrap()) >= size + carve);
}

#[test]
fn console_is_the_board_defconfigs_uart() {
    // no chosen/console override, no second uart: the overlay is the sram0
    // window alone
    assert!(!OVERLAY.contains("uart"));
    assert!(!OVERLAY.contains("chosen"));
    assert!(!CONF.lines().any(|l| l.starts_with("CONFIG_UART") || l.contains("CONSOLE")));
    assert_eq!(conf("PRINTK"), Some("y"));
    assert_eq!(conf("LOG_MODE_MINIMAL"), Some("y"));
    assert_eq!(conf("FPU_SHARING"), Some("y"));
    // tools/board.sh pi5 ends the run on the record's end line
    let end = BOARD_SH.lines().find(|l| l.trim_start().starts_with("END_RE_DEFAULT=")).expect("END_RE_DEFAULT");
    assert!(end.contains("v3d_gl_test RECORD end"), "{end}");
}

#[test]
fn watchdog_holds_for_the_run_and_the_record() {
    let hold = entries(GL_KCONFIG, "V3D_GL_TEST_HOLD_S");
    assert_eq!(hold.len(), 1);
    assert!(hold[0].contains(&"default 200"));
    assert!(hold[0].contains(&"range 30 300"));
    // the board layer's hold follows the sample's on the 7.1 only, and the
    // board layer's own default (the probes') stays
    assert_eq!(defaults(GL_KCONFIG, "YAKOGL_RPI5_WDOG_HOLD_S"), ["V3D_GL_TEST_HOLD_S if YAKOGL_GPU_V3D71"]);
    let b = &entries(BOARD_KCONFIG, "YAKOGL_RPI5_WDOG_HOLD_S")[0];
    assert!(b.contains(&"default 30"));
    assert!(b.contains(&"range 10 3600"));
    assert!(b.contains(&"depends on YAKOGL_RPI5_WDOG"));
    // the image does not set either hold itself
    assert!(conf("YAKOGL_RPI5_WDOG_HOLD_S").is_none());
    assert!(conf("V3D_GL_TEST_HOLD_S").is_none());
    // the default hold (200 s) is inside the board.sh window (a reboot takes
    // ~150 s to ssh; the run's wait is 420 s) and above the sample's range floor
}

#[test]
fn entry_parser_stops_at_the_next_block() {
    let k = "config A\n\tdefault 1\n# note\nif X\nconfig B\n\tdefault 2\nendif\n\tdefault 3\n";
    assert_eq!(defaults(k, "A"), ["1"]);
    assert_eq!(defaults(k, "B"), ["2"]);
}
