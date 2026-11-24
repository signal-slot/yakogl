// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Host check of the Raspberry Pi 5's gl_test configuration (commit 1bb98759):
//! the inputs the first board session relies on, read by text from
//! `driver/samples/zephyr/gl_test/{Kconfig,boards/rpi_5.conf,boards/rpi_5.overlay}`,
//! `driver/zephyr/lib/board/Kconfig`, `zephyr/Kconfig` and `tools/board.sh`.
//!
//! Run: `cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d71_gltest_rpi5_conf`
//! (the features only let the crate build; the test needs none of it).
//!
//! Pinned (each failure names the file and the line):
//!  1. the V3D71 default of `V3D_GL_TEST_CASES` is 5,58,59,66,67,102;
//!  2. the overlay's `sram0` starts at 0x200000, is 96 MiB long, ends below
//!     the DTB address its comment cites (0x2efec600) and below 4 GiB;
//!  3. the carve-out (rpi_5.conf) and the compiler scratch (the Kconfig
//!     default: rpi_5.conf does not set it) fit in `sram0` with 44 MiB left;
//!  4. the console is uart10 and the image has no screen;
//!  5. `YAKOGL_RPI5_WDOG_HOLD_S` has a V3D71 default (the sample's
//!     `V3D_GL_TEST_HOLD_S`, 200 s) shorter than board.sh pi5's RUN_BOUND.
//!
//! Where the tree differs from the plan's description: the compiler scratch
//! is 4 MiB (`YAKOGL_SHADERC_SCRATCH_BYTES` default, zephyr/Kconfig), not the
//! 2 MiB the overlay's comment names; the test pins the tree's value.

const ROOT: &str = env!("CARGO_MANIFEST_DIR");

const MIB: u64 = 1024 * 1024;
const GIB4: u64 = 4 * 1024 * MIB;

const KCONFIG_GL: &str = "driver/samples/zephyr/gl_test/Kconfig";
const CONF: &str = "driver/samples/zephyr/gl_test/boards/rpi_5.conf";
const OVERLAY: &str = "driver/samples/zephyr/gl_test/boards/rpi_5.overlay";
const KCONFIG_BOARD: &str = "driver/zephyr/lib/board/Kconfig";
const KCONFIG_ZEPHYR: &str = "zephyr/Kconfig";
const BOARD_SH: &str = "tools/board.sh";

struct File {
    rel: &'static str,
    lines: Vec<String>,
}

impl File {
    fn new(rel: &'static str) -> File {
        let p = format!("{ROOT}/{rel}");
        let s = std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{p}: {e}"));
        File { rel, lines: s.lines().map(str::to_string).collect() }
    }
    /// 1-based line of the first line satisfying `f`, at or after `from`.
    fn find(&self, from: usize, what: &str, f: impl Fn(&str) -> bool) -> usize {
        for (i, l) in self.lines.iter().enumerate().skip(from.saturating_sub(1)) {
            if f(l) {
                return i + 1;
            }
        }
        panic!("{}: no line for {what} (from line {from})", self.rel);
    }
    fn at(&self, n: usize) -> &str {
        &self.lines[n - 1]
    }
    /// The Kconfig entry `config NAME`: its first line and the last line of
    /// its first block (the body is lines start+1 ..= end).
    fn config(&self, name: &str) -> (usize, usize) {
        let want = format!("config {name}");
        let start = self.find(1, &want, |l| l.trim() == want);
        let mut end = start;
        while end < self.lines.len() {
            let l = &self.lines[end];
            if l.starts_with("config ") || l.starts_with("menu ") || l.starts_with("endmenu") {
                break;
            }
            end += 1;
        }
        (start, end)
    }
    fn fail(&self, line: usize, msg: &str) -> ! {
        panic!("{}:{}: {msg}: `{}`", self.rel, line, self.at(line).trim())
    }
}

fn num(s: &str) -> Option<u64> {
    let s = s.trim().trim_matches('"');
    match s.strip_prefix("0x") {
        Some(h) => u64::from_str_radix(h, 16).ok(),
        None => s.parse().ok(),
    }
}

/// `default <v> [if COND]` lines of a config block: (line, value, cond).
fn defaults(f: &File, name: &str) -> Vec<(usize, String, Option<String>)> {
    let (s, e) = f.config(name);
    let mut v = Vec::new();
    for n in s + 1..=e {
        let l = f.at(n).trim();
        if let Some(r) = l.strip_prefix("default ") {
            let (val, cond) = match r.split_once(" if ") {
                Some((a, b)) => (a, Some(b.trim().to_string())),
                None => (r, None),
            };
            v.push((n, val.trim().trim_matches('"').to_string(), cond));
        }
    }
    v
}

/// The V3D71 default of `name` (`default .. if YAKOGL_GPU_V3D71`) and its line.
fn v3d71_default(f: &File, name: &str) -> (usize, String) {
    defaults(f, name)
        .into_iter()
        .find(|(_, _, c)| c.as_deref() == Some("YAKOGL_GPU_V3D71"))
        .map(|(n, v, _)| (n, v))
        .unwrap_or_else(|| panic!("{}: config {name} has no `default .. if YAKOGL_GPU_V3D71`", f.rel))
}

/// Value of an uncommented `CONFIG_NAME=value` of a .conf, with its line.
fn conf_value(f: &File, name: &str) -> Option<(usize, String)> {
    let key = format!("CONFIG_{name}=");
    for (i, l) in f.lines.iter().enumerate() {
        if let Some(v) = l.trim().strip_prefix(&key) {
            return Some((i + 1, v.trim().to_string()));
        }
    }
    None
}

/// (line, start, length) of the overlay's sram0 `reg = <hi lo len>;`.
fn sram0(f: &File) -> (usize, u64, u64) {
    let b = f.find(1, "&sram0", |l| l.trim_start().starts_with("&sram0"));
    let n = f.find(b, "reg = <", |l| l.trim_start().starts_with("reg = <"));
    let cells: Vec<u64> = f
        .at(n)
        .split(['<', '>'])
        .nth(1)
        .unwrap_or_else(|| f.fail(n, "no <..> in reg"))
        .split_whitespace()
        .map(|c| num(c).unwrap_or_else(|| f.fail(n, "reg cell is not a number")))
        .collect();
    if cells.len() != 3 {
        f.fail(n, "reg must be <hi lo len>: two address cells, one size cell");
    }
    (n, (cells[0] << 32) | cells[1], cells[2])
}

/// The scratch size of the rpi_5 image: rpi_5.conf's override or the Kconfig
/// default; the file, line and bytes.
fn scratch_bytes() -> (&'static str, usize, u64) {
    let conf = File::new(CONF);
    if let Some((n, v)) = conf_value(&conf, "YAKOGL_SHADERC_SCRATCH_BYTES") {
        let b = num(&v).unwrap_or_else(|| conf.fail(n, "not a number"));
        return (CONF, n, b);
    }
    let k = File::new(KCONFIG_ZEPHYR);
    let (n, v, c) = defaults(&k, "YAKOGL_SHADERC_SCRATCH_BYTES").into_iter().next().expect("scratch default");
    assert!(c.is_none(), "{KCONFIG_ZEPHYR}:{n}: scratch default is conditional");
    (KCONFIG_ZEPHYR, n, num(&v).unwrap())
}

#[test]
fn six_cases_default() {
    let f = File::new(KCONFIG_GL);
    let (n, v) = v3d71_default(&f, "V3D_GL_TEST_CASES");
    if v != "5,58,59,66,67,102" {
        f.fail(n, "the V3D71 default of V3D_GL_TEST_CASES must be exactly 5,58,59,66,67,102");
    }
    let d = defaults(&f, "V3D_GL_TEST_CASES");
    assert!(d.len() >= 2, "{KCONFIG_GL}: the Pi 4 default of V3D_GL_TEST_CASES is gone");
    assert!(d[0].0 == n, "{KCONFIG_GL}:{}: the V3D71 default must come first", d[0].0);
}

#[test]
fn sram0_window() {
    let f = File::new(OVERLAY);
    let (n, start, len) = sram0(&f);
    if start != 0x200000 {
        f.fail(n, "sram0 must start at 0x200000 (where the firmware loads kernel=)");
    }
    if len != 96 * MIB {
        f.fail(n, "sram0 must be 96 MiB (0x6000000) long");
    }
    let end = start + len;
    let c = f.find(1, "the DTB address", |l| l.contains("0x2efec600"));
    let dtb = 0x2efec600u64;
    if end > dtb {
        f.fail(c, &format!("sram0 ends at {end:#x}, above the DTB at {dtb:#x}"));
    }
    if end >= GIB4 {
        f.fail(n, &format!("sram0 ends at {end:#x}, not below 4 GiB"));
    }
    eprintln!(
        "sram0 {start:#x}..{end:#x}; {} MiB below the DTB, {} MiB below 4 GiB",
        (dtb - end) / MIB,
        (GIB4 - end) / MIB
    );
}

#[test]
fn carveout_and_scratch_fit() {
    let conf = File::new(CONF);
    let (cn, cv) = conf_value(&conf, "YAKOGL_V3D_CARVEOUT_BYTES")
        .unwrap_or_else(|| panic!("{CONF}: no CONFIG_YAKOGL_V3D_CARVEOUT_BYTES"));
    let carve = num(&cv).unwrap_or_else(|| conf.fail(cn, "not a number"));
    if carve != 48 * MIB {
        conf.fail(cn, "the carve-out must be 48 MiB (50331648)");
    }
    let (sf, sn, scratch) = scratch_bytes();
    assert_eq!(scratch, 4 * MIB, "{sf}:{sn}: the rpi_5 compiler scratch changed from the tree's 4 MiB");
    let ov = File::new(OVERLAY);
    let (on, _, len) = sram0(&ov);
    let used = carve + scratch;
    let margin = len
        .checked_sub(used)
        .unwrap_or_else(|| ov.fail(on, &format!("carve-out {carve} + scratch {scratch} exceed sram0 {len}")));
    eprintln!(
        "carve-out {} MiB + scratch {} MiB = {} MiB of {} MiB sram0: margin {} MiB",
        carve / MIB,
        scratch / MIB,
        used / MIB,
        len / MIB,
        margin / MIB
    );
    assert_eq!(margin, 44 * MIB, "{OVERLAY}:{on}: margin changed from the 44 MiB pinned here");
}

#[test]
fn console_uart10_and_no_screen() {
    let conf = File::new(CONF);
    let n = conf.find(1, "the uart10 console comment", |l| l.trim_start().starts_with('#') && l.contains("uart10"));
    assert!(conf.at(n).contains("board defconfig"), "{CONF}:{n}: uart10 is no longer said to be the board defconfig's");
    // The overlay leaves the console to the board file.
    let ov = File::new(OVERLAY);
    for (i, l) in ov.lines.iter().enumerate() {
        let code = l.split("//").next().unwrap_or("");
        if code.contains("zephyr,console") || code.contains("zephyr,shell-uart") || code.contains("chosen") {
            ov.fail(i + 1, "the overlay must not move the console off the board file's uart10");
        }
        if code.contains("&uart") {
            ov.fail(i + 1, "the overlay must not redefine a uart");
        }
    }
    // No screen.
    for sym in ["YAKOGL_BOARD_RPI5", "YAKOGL_BOARD_WINDOW", "DISPLAY"] {
        if let Some((n, v)) = conf_value(&conf, sym) {
            if v == "y" {
                conf.fail(n, &format!("CONFIG_{sym}=y: the rpi_5 image must have no screen"));
            }
        }
    }
    conf.find(1, "the no-screen statement", |l| l.contains("no screen"));
    let b = File::new(KCONFIG_BOARD);
    let (s, e) = b.config("YAKOGL_BOARD_RPI5");
    for n in s + 1..=e {
        if b.at(n).trim().starts_with("default y") {
            b.fail(n, "YAKOGL_BOARD_RPI5 must not default to y");
        }
    }
}

/// board.sh's `pi5)` arm: RUN_BOUND_DEFAULT in seconds, with its line.
fn pi5_run_bound() -> (usize, u64) {
    let f = File::new(BOARD_SH);
    let arm = f.find(1, "the pi5) arm", |l| l.trim() == "pi5)");
    let n = f.find(arm, "RUN_BOUND_DEFAULT of pi5", |l| l.contains("RUN_BOUND_DEFAULT="));
    let v = f
        .at(n)
        .split("RUN_BOUND_DEFAULT=")
        .nth(1)
        .and_then(|r| r.split(';').next())
        .and_then(num)
        .unwrap_or_else(|| f.fail(n, "RUN_BOUND_DEFAULT is not a number"));
    (n, v)
}

#[test]
fn watchdog_hold_shorter_than_run_bound() {
    let f = File::new(KCONFIG_GL);
    let (n, v) = v3d71_default(&f, "YAKOGL_RPI5_WDOG_HOLD_S");
    assert_eq!(v, "V3D_GL_TEST_HOLD_S", "{KCONFIG_GL}:{n}: the rpi_5 hold must follow V3D_GL_TEST_HOLD_S");
    let (hn, hv, _) = defaults(&f, "V3D_GL_TEST_HOLD_S").into_iter().next().expect("V3D_GL_TEST_HOLD_S default");
    let hold = num(&hv).unwrap_or_else(|| f.fail(hn, "not a number"));
    // The board layer's symbol must accept the value.
    let b = File::new(KCONFIG_BOARD);
    let (s, e) = b.config("YAKOGL_RPI5_WDOG_HOLD_S");
    let range = (s + 1..=e).find(|&n| b.at(n).trim().starts_with("range ")).unwrap_or_else(|| b.fail(s, "no range"));
    let r: Vec<u64> = b.at(range).trim()["range ".len()..].split_whitespace().filter_map(num).collect();
    assert!(r[0] <= hold && hold <= r[1], "{KCONFIG_BOARD}:{range}: hold {hold} outside {}..{}", r[0], r[1]);
    // The reset follows the hold by at most one 10 s watchdog period.
    let (bn, bound) = pi5_run_bound();
    assert!(
        hold + 10 < bound,
        "{KCONFIG_GL}:{hn}: hold {hold} s + one 10 s period is not shorter than \
         {BOARD_SH}:{bn} RUN_BOUND_DEFAULT {bound} s: the runner would give up before the reset"
    );
    assert_eq!(hold, 200, "{KCONFIG_GL}:{hn}: V3D_GL_TEST_HOLD_S default changed from 200 s");
    assert_eq!(bound, 420, "{BOARD_SH}:{bn}: pi5 RUN_BOUND_DEFAULT changed from 420 s");
}
