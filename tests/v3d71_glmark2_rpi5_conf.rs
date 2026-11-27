// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The glmark2 sample's Raspberry Pi 5 configuration, pinned: `boards/rpi_5.conf`
//! and `.overlay` (the memory window, the 7.1 table, the PM watchdog and
//! mailbox through the board layer, no screen), the board layer's
//! `YAKOGL_BOARD` depending on `SOC_BCM2712`, and the glmark2 `CMakeLists.txt`
//! as it stands: it has no V3D71 branch and no rpi_5 case, the 7.1 pieces
//! come from the board file's `CONFIG_YAKOGL_GPU_V3D71` alone. The gl_test
//! sample's rpi_5 files are read beside them, so the two images agree on the
//! SoC choices.
//!
//! If a V3D71 branch is added to the glmark2 CMakeLists, `cmake_has_no_v3d71_branch`
//! fails on purpose: replace it with the pins of that branch.
//!
//! The sources are read with `include_str!`, so cargo rebuilds this test when
//! one of them changes and the gate's tree key sees it.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d71_glmark2_rpi5_conf
//! ```

const CONF: &str = include_str!("../driver/samples/zephyr/glmark2/boards/rpi_5.conf");
const OVERLAY: &str = include_str!("../driver/samples/zephyr/glmark2/boards/rpi_5.overlay");
const CMAKE: &str = include_str!("../driver/samples/zephyr/glmark2/CMakeLists.txt");
const PRJ: &str = include_str!("../driver/samples/zephyr/glmark2/prj.conf");
const GT_CONF: &str = include_str!("../driver/samples/zephyr/gl_test/boards/rpi_5.conf");
const GT_OVERLAY: &str = include_str!("../driver/samples/zephyr/gl_test/boards/rpi_5.overlay");
const BOARD_KCONFIG: &str = include_str!("../driver/zephyr/lib/board/Kconfig");

/// The active (non-comment) lines of a Kconfig fragment or devicetree file.
fn active(text: &str) -> Vec<&str> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#') && !l.starts_with("/*") && !l.starts_with('*'))
        .collect()
}

fn conf_value<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    let prefix = format!("CONFIG_{key}=");
    active(text).into_iter().find_map(|l| l.strip_prefix(prefix.as_str()))
}

/// The `reg` cells of the sram0 override: (address, size).
fn sram_reg(text: &str) -> (u64, u64) {
    let l = active(text).into_iter().find(|l| l.starts_with("reg")).expect("reg line");
    let cells = l.split_once('<').unwrap().1.split_once('>').unwrap().0;
    let v: Vec<u64> =
        cells.split_whitespace().map(|c| u64::from_str_radix(c.trim_start_matches("0x"), 16).unwrap()).collect();
    assert_eq!(v.len(), 3, "two address cells and a size: {l}");
    assert_eq!(v[0], 0, "high address cell: {l}");
    (v[1], v[2])
}

fn check_conf(conf: &str) -> Result<(), String> {
    let want = |k: &str, v: &str| match conf_value(conf, k) {
        Some(x) if x == v => Ok(()),
        x => Err(format!("CONFIG_{k}: want {v}, have {x:?}")),
    };
    want("YAKOGL_GPU_V3D", "y")?;
    want("YAKOGL_GPU_V3D71", "y")?;
    // 96 MiB carve-out, the TFU for refract's tiled copies
    want("YAKOGL_V3D_CARVEOUT_BYTES", "100663296")?;
    want("YAKOGL_V3D_TFU", "y")?;
    want("FPU_SHARING", "y")?;
    // No screen: the mailbox framebuffer is not chosen here.
    if conf_value(conf, "YAKOGL_BOARD_RPI5").is_some() {
        return Err("CONFIG_YAKOGL_BOARD_RPI5 must not be set in rpi_5.conf".into());
    }
    Ok(())
}

fn check_overlay(ov: &str) -> Result<(), String> {
    if !active(ov).iter().any(|l| l.starts_with("&sram0")) {
        return Err("no &sram0 override".into());
    }
    let (addr, size) = sram_reg(ov);
    // From the firmware's kernel= address, below 4 GiB (V3D PTEs, mailbox).
    if addr != 0x200000 {
        return Err(format!("sram0 starts at {addr:#x}"));
    }
    if addr + size > 1 << 32 {
        return Err("sram0 reaches 4 GiB".into());
    }
    // 288 MiB: carve-out 96 + scratch + 96 MiB heap + assets.
    if size != 0x1200_0000 {
        return Err(format!("sram0 size {size:#x}"));
    }
    if active(ov).iter().any(|l| l.contains("okay") || l.contains("zephyr,console") || l.contains("chosen")) {
        return Err("the overlay enables a node or changes the console".into());
    }
    Ok(())
}

#[test]
fn conf_pins_the_v3d71_image() {
    check_conf(CONF).unwrap();
}

#[test]
fn overlay_pins_the_memory_window() {
    check_overlay(OVERLAY).unwrap();
}

#[test]
fn carveout_fits_the_window() {
    let num = |k: &str| -> u64 { conf_value(CONF, k).unwrap().parse().unwrap() };
    let carve = num("YAKOGL_V3D_CARVEOUT_BYTES");
    let tiled = num("YAKOGL_V3D_TILED_COPIES_BYTES");
    let (_, size) = sram_reg(OVERLAY);
    assert_eq!(carve % (2 << 20), 0, "carve-out in 2 MiB tables");
    assert!(carve + tiled < size);
    // the translation tables: a table per 2 MiB of the alias, plus the rest
    assert!(num("MAX_XLAT_TABLES") >= 64 + carve / (2 << 20));
    assert_eq!(conf_value(CONF, "KERNEL_VM_SIZE"), Some("0x20000000"));
}

#[test]
fn agrees_with_gl_test_rpi5_files() {
    for k in ["YAKOGL_GPU_V3D", "YAKOGL_GPU_V3D71", "FPU_SHARING"] {
        assert!(conf_value(GT_CONF, k).is_some(), "{k}");
        assert_eq!(conf_value(CONF, k), conf_value(GT_CONF, k), "{k}");
    }
    let (ga, gs) = sram_reg(GT_OVERLAY);
    let (a, s) = sram_reg(OVERLAY);
    assert_eq!(a, ga, "same start (the firmware's kernel= address)");
    assert!(s > gs, "glmark2 carries more: 96 MiB carve-out, heap, assets");
    let carve = |t| -> u64 { conf_value(t, "YAKOGL_V3D_CARVEOUT_BYTES").unwrap().parse().unwrap() };
    assert!(carve(CONF) > carve(GT_CONF));
    // neither image names a screen or a console node
    for t in [CONF, GT_CONF] {
        assert!(conf_value(t, "YAKOGL_BOARD_RPI5").is_none());
    }
    for t in [OVERLAY, GT_OVERLAY] {
        assert!(!t.contains("zephyr,console"));
    }
}

#[test]
fn watchdog_and_mailbox_come_from_the_board_layer() {
    // The conf names no watchdog symbol: YAKOGL_GPU_V3D71 selects them (the
    // conf's own comment says so) and prj.conf sets the lifetime.
    let flat = CONF.replace("\n# ", " ");
    assert!(flat.contains("YAKOGL_GPU_V3D71 selects the board layer's PM watchdog"));
    assert_eq!(conf_value(PRJ, "YAKOGL_BOARD"), Some("y"));
    assert!(conf_value(PRJ, "YAKOGL_BOARD_LIFETIME_S").is_some());
    assert!(conf_value(PRJ, "YAKOGL_BOARD_HOLD_S").is_some());
    assert!(BOARD_KCONFIG.contains("config YAKOGL_RPI5_WDOG\n"));
    assert!(BOARD_KCONFIG.contains("config YAKOGL_RPI5_MBOX\n"));
}

/// The body of one `config`/`menuconfig` entry up to the next top-level entry.
fn kconfig_entry<'a>(text: &'a str, head: &str) -> &'a str {
    let start = text.find(head).expect(head);
    let rest = &text[start + head.len()..];
    let end = [rest.find("\nconfig "), rest.find("\nmenuconfig ")].into_iter().flatten().min().unwrap_or(rest.len());
    &rest[..end]
}

/// `YAKOGL_BOARD`'s own `depends on` lines (continuations joined) name the SoC.
fn board_depends_on_bcm2712(kconfig: &str) -> bool {
    let e = kconfig_entry(kconfig, "menuconfig YAKOGL_BOARD\n").replace("\\\n", " ");
    e.lines().any(|l| l.trim_start().starts_with("depends on") && l.contains("SOC_BCM2712"))
}

#[test]
fn board_layer_depends_on_bcm2712() {
    assert!(board_depends_on_bcm2712(BOARD_KCONFIG));
    // the Pi 5's own symbols depend on it as well
    for head in ["config YAKOGL_RPI5_WDOG\n", "config YAKOGL_RPI5_MBOX\n"] {
        assert!(kconfig_entry(BOARD_KCONFIG, head).contains("depends on SOC_BCM2712"), "{head}");
    }
}

#[test]
fn cmake_has_no_v3d71_branch() {
    // glmark2's CMakeLists.txt is board-agnostic for the V3D: no V3D71, no
    // rpi_5 case, no BCM2712; a 7.1 build is the board file's Kconfig alone.
    let lower = CMAKE.to_ascii_lowercase();
    for s in ["v3d71", "rpi_5", "bcm2712", "rpi5"] {
        assert!(!lower.contains(s), "CMakeLists.txt now names {s}: pin that branch");
    }
    // the board-specific cases it does have
    assert!(CMAKE.contains("BOARD MATCHES \"^stm32mp257f_dk\""));
    assert!(CMAKE.contains("BOARD MATCHES \"^nanopc_t6\""));
}

/// Each mutation of the pinned text must be caught by its check.
#[test]
fn mutations_are_detected() {
    let conf_muts = [
        ("CONFIG_YAKOGL_GPU_V3D71=y", "CONFIG_YAKOGL_GPU_V3D71=n"),
        ("CONFIG_YAKOGL_V3D_CARVEOUT_BYTES=100663296", "CONFIG_YAKOGL_V3D_CARVEOUT_BYTES=1"),
        ("CONFIG_FPU_SHARING=y", "CONFIG_FPU_SHARING=n"),
        ("CONFIG_YAKOGL_V3D_TFU=y", "CONFIG_YAKOGL_V3D_TFU=y\nCONFIG_YAKOGL_BOARD_RPI5=y"),
    ];
    for (from, to) in conf_muts {
        assert!(CONF.contains(from), "{from}");
        assert!(check_conf(&CONF.replacen(from, to, 1)).is_err(), "{to}");
    }
    let ov_muts = [
        ("0x200000 0x12000000", "0x300000 0x12000000"),
        ("0x200000 0x12000000", "0x200000 0xf0000000"),
        ("0x200000 0x12000000", "0x200000 0x6000000"),
        ("&sram0", "&sram1"),
    ];
    for (from, to) in ov_muts {
        assert!(OVERLAY.contains(from), "{from}");
        assert!(check_overlay(&OVERLAY.replacen(from, to, 1)).is_err(), "{to}");
    }
    let k = BOARD_KCONFIG.replacen(" || SOC_BCM2712", "", 1);
    assert_ne!(k, BOARD_KCONFIG);
    assert!(!board_depends_on_bcm2712(&k));
}

fn n(s: &str) -> u64 {
    s.parse().unwrap()
}

/// The statics the overlay's comment lists (carve-out, tiled copies, compiler
/// scratch, C heap) fit the window, which starts at the firmware's kernel=
/// address, and the firmware's DTB at 0x2efec600 lies above its end.
#[test]
fn statics_fit_the_window_and_the_dtb_lies_above_it() {
    let (base, size) = sram_reg(OVERLAY);
    assert_eq!(base, 0x20_0000);
    assert_eq!(size, 288 << 20);
    assert_eq!(base + size, 0x1220_0000, "the end the overlay's comment names");
    assert!(base + size <= 1 << 32, "below 4 GiB (PTEs, mailbox buffer)");
    assert!(0x2efe_c600 > base + size, "the DTB is above the window");

    let carve = n(conf_value(CONF, "YAKOGL_V3D_CARVEOUT_BYTES").unwrap());
    let tiled = n(conf_value(CONF, "YAKOGL_V3D_TILED_COPIES_BYTES").unwrap());
    let heap = n(conf_value(PRJ, "COMMON_LIBC_MALLOC_ARENA_SIZE").unwrap());
    assert_eq!((carve, tiled, heap), (96 << 20, 24 << 20, 96 << 20));
    // the conf sets no scratch, so the Kconfig default (4 MiB) applies; the
    // overlay's comment says 2 MiB (reported, not fixed)
    assert!(conf_value(CONF, "YAKOGL_SHADERC_SCRATCH_BYTES").is_none());
    let scratch: u64 = 4 << 20;
    let data: u64 = 10 << 20; // glmark2's ~9.4 MB of data files, rounded up
    assert!(carve + tiled + scratch + heap + data < size, "room for code and data");
    assert!(n(conf_value(CONF, "MAX_XLAT_TABLES").unwrap()) >= 64 + carve / (2 << 20));
    assert!(0x2000_0000 >= size + carve, "KERNEL_VM_SIZE covers the image and the alias");
}

/// No screen and no console override: the conf and overlay name no uart,
/// display or board-screen symbol (the console is the board defconfig's).
#[test]
fn there_is_no_screen_and_no_console_override() {
    for l in active(CONF) {
        for bad in ["UART", "CONSOLE", "BOARD_RPI5", "DISPLAY", "FRAMEBUFFER"] {
            assert!(!l.contains(bad), "{l}");
        }
    }
    let ov = active(OVERLAY).join("\n");
    for bad in ["uart", "chosen", "display", "zephyr,console"] {
        assert!(!ov.contains(bad), "{bad}");
    }
}
