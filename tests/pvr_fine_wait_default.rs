// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The PowerVR fence poll of 20 us (`CONFIG_YAKOGL_DRV_GPU_PVR_FINE_WAIT`,
//! yakogl-drivers feature `pvr-fine-wait`) is on by default in the AM62P's
//! images and nowhere else (docs/notes/pvr-fence-wait.md; fix candidate 7 of
//! docs/notes/am62p-slow-terrain.md: terrain 83.08 -> 81.67 ms at
//! `ce3354ac`).
//!
//! The chain it pins, read from the files the image's build reads:
//!
//! * driver/zephyr/Kconfig: the symbol sits inside `if YAKOGL_DRV_GPU_PVR`
//!   and has one default, `default y if SOC_AM62P54_A53`;
//! * the SoC symbol: only the Verdin AM62P board selects it, for both of its
//!   A53 targets (the U-Boot one and `_direct`); every other board's build
//!   keeps the old default n;
//! * the GPU driver: `YAKOGL_DRV_GPU_PVR` is default y on an enabled
//!   `img,img-rogue` node, and the AM62P's device tree has one;
//! * no defconfig or sample `.conf` an AM62P image is built from sets the
//!   symbol (so the default is what the image gets);
//! * driver/zephyr/CMakeLists.txt turns the symbol into the cargo feature
//!   `pvr-fine-wait`, driver/rust/Cargo.toml has it, and render.rs makes
//!   `WAIT_POLL_US` 20 under it.
//!
//! The files are compiled in (`include_str!`); the test opens nothing.

const KCONFIG: &str = include_str!("../driver/zephyr/Kconfig");
const CMAKE: &str = include_str!("../driver/zephyr/CMakeLists.txt");
const DRIVERS_CARGO: &str = include_str!("../driver/rust/Cargo.toml");
const RENDER_RS: &str = include_str!("../driver/rust/src/gpu/pvr/render.rs");
const BOARD_KCONFIG: &str = include_str!("../driver/boards/toradex/verdin_am62p/Kconfig.verdin_am62p");
const SOC_DTSI: &str = include_str!("../driver/dts/arm64/ti/ti_am62p54_a53.dtsi");
const BOARD_DTS: &str = include_str!("../driver/boards/toradex/verdin_am62p/verdin_am62p_am62p54_a53.dts");

const SYMBOL: &str = "YAKOGL_DRV_GPU_PVR_FINE_WAIT";

/// Every configuration fragment an AM62P image is built from: the board's
/// defconfigs, each sample's prj.conf and its AM62P board file.
const AM62P_CONFS: &[(&str, &str)] = &[
    (
        "verdin_am62p_am62p54_a53_defconfig",
        include_str!("../driver/boards/toradex/verdin_am62p/verdin_am62p_am62p54_a53_defconfig"),
    ),
    (
        "verdin_am62p_am62p54_a53_direct_defconfig",
        include_str!("../driver/boards/toradex/verdin_am62p/verdin_am62p_am62p54_a53_direct_defconfig"),
    ),
    ("glmark2/prj.conf", include_str!("../driver/samples/zephyr/glmark2/prj.conf")),
    (
        "glmark2/boards/verdin_am62p_am62p54_a53.conf",
        include_str!("../driver/samples/zephyr/glmark2/boards/verdin_am62p_am62p54_a53.conf"),
    ),
    ("gl_test/prj.conf", include_str!("../driver/samples/zephyr/gl_test/prj.conf")),
    ("gl_test/prj_sweep.conf", include_str!("../driver/samples/zephyr/gl_test/prj_sweep.conf")),
    (
        "gl_test/boards/verdin_am62p_am62p54_a53.conf",
        include_str!("../driver/samples/zephyr/gl_test/boards/verdin_am62p_am62p54_a53.conf"),
    ),
    ("deqp/prj.conf", include_str!("../driver/samples/zephyr/deqp/prj.conf")),
    ("deqp/texture-2048.conf", include_str!("../driver/samples/zephyr/deqp/texture-2048.conf")),
    (
        "deqp/boards/verdin_am62p_am62p54_a53.conf",
        include_str!("../driver/samples/zephyr/deqp/boards/verdin_am62p_am62p54_a53.conf"),
    ),
    ("examples/triangle/prj.conf", include_str!("../examples/triangle/prj.conf")),
    ("examples/triangle-rust/prj.conf", include_str!("../examples/triangle-rust/prj.conf")),
    ("examples/femtovg/prj.conf", include_str!("../examples/femtovg/prj.conf")),
    ("examples/slint/prj.conf", include_str!("../examples/slint/prj.conf")),
];

/// The lines of the `config <name>` entry: from its header to the next
/// top-level line that is not part of it (another entry, `if`, `endif`, ...).
fn entry<'a>(kconfig: &'a str, name: &str) -> Vec<&'a str> {
    let header = format!("config {name}");
    let mut lines = kconfig.lines().skip_while(|l| l.trim_end() != header);
    let first = lines.next().unwrap_or_else(|| panic!("no `{header}` in driver/zephyr/Kconfig"));
    let mut out = vec![first];
    out.extend(lines.take_while(|l| l.is_empty() || l.starts_with('\t') || l.starts_with(' ')));
    out
}

/// The `default` lines of an entry, trimmed.
fn defaults<'a>(lines: &[&'a str]) -> Vec<&'a str> {
    lines.iter().map(|l| l.trim()).filter(|l| l.starts_with("default")).collect()
}

/// The `if` blocks open at the line that starts with `at` (top-level `if`
/// and `endif` only, as driver/zephyr/Kconfig writes them).
fn open_ifs(kconfig: &str, at: &str) -> Vec<String> {
    let mut stack = Vec::new();
    for l in kconfig.lines() {
        if l.trim_end() == at {
            return stack;
        }
        if let Some(cond) = l.strip_prefix("if ") {
            stack.push(cond.trim().to_string());
        } else if l.starts_with("endif") {
            stack.pop().expect("an endif without an if in driver/zephyr/Kconfig");
        }
    }
    panic!("no `{at}` in driver/zephyr/Kconfig");
}

#[test]
fn kconfig_default_is_y_on_the_am62p_only() {
    let e = entry(KCONFIG, SYMBOL);
    assert!(e.iter().any(|l| l.trim().starts_with("bool ")), "{SYMBOL} is a bool: {e:?}");
    assert_eq!(
        defaults(&e),
        ["default y if SOC_AM62P54_A53"],
        "{SYMBOL}: one default, y on the AM62P's A53 and n (no default) on every other SoC"
    );
    assert_eq!(
        open_ifs(KCONFIG, &format!("config {SYMBOL}")),
        ["YAKOGL_DRV_GPU_PVR"],
        "{SYMBOL} is visible only with the PowerVR driver"
    );
}

#[test]
fn only_the_am62p_board_selects_its_soc() {
    // Both A53 targets of the board (U-Boot and direct boot) select the SoC.
    let sel =
        BOARD_KCONFIG.split("select SOC_AM62P54_A53").nth(1).expect("Kconfig.verdin_am62p selects SOC_AM62P54_A53");
    let sel = sel.split("\n\n").next().unwrap();
    for board in ["BOARD_VERDIN_AM62P_AM62P54_A53 ", "BOARD_VERDIN_AM62P_AM62P54_A53_DIRECT"] {
        assert!(sel.contains(board), "SOC_AM62P54_A53 is selected for {board}: {sel}");
    }
    // The other boards of this repository (the i.MX8MP ones) do not, and
    // the SoC's own Kconfig.soc defines it without a default.
    for (name, text) in [
        ("verdin_imx8mp", include_str!("../driver/boards/toradex/verdin_imx8mp/Kconfig.verdin_imx8mp")),
        ("am62px Kconfig.soc", include_str!("../driver/soc/ti/k3/am62px/Kconfig.soc")),
    ] {
        assert!(!text.contains("select SOC_AM62P54_A53"), "{name} selects SOC_AM62P54_A53");
    }
    let soc = entry(include_str!("../driver/soc/ti/k3/am62px/Kconfig.soc"), "SOC_AM62P54_A53");
    assert!(defaults(&soc).is_empty(), "SOC_AM62P54_A53 has no default: {soc:?}");
}

#[test]
fn the_am62p_image_has_the_powervr_driver() {
    let pvr = entry(KCONFIG, "YAKOGL_DRV_GPU_PVR");
    assert_eq!(defaults(&pvr), ["default y"], "YAKOGL_DRV_GPU_PVR: {pvr:?}");
    assert!(
        pvr.iter().any(|l| l.trim() == "depends on DT_HAS_IMG_IMG_ROGUE_ENABLED"),
        "YAKOGL_DRV_GPU_PVR turns on from the img,img-rogue node: {pvr:?}"
    );
    assert!(SOC_DTSI.contains("gpu: gpu@fd80000 {"), "the AM62P dtsi has the GPU node");
    assert!(SOC_DTSI.contains("compatible = \"img,img-rogue\""), "the GPU node is img,img-rogue");
    let gpu = BOARD_DTS.split("&gpu {").nth(1).expect("the board's dts enables &gpu");
    let gpu = gpu.split("};").next().unwrap();
    assert!(gpu.contains("status = \"okay\";"), "&gpu is okay in the board's dts: {gpu}");
}

#[test]
fn no_am62p_config_fragment_overrides_the_default() {
    for (name, text) in AM62P_CONFS {
        for l in text.lines() {
            assert!(!l.contains(SYMBOL), "{name} sets {SYMBOL} ({l}); the AM62P image takes the Kconfig default");
            assert!(
                !l.contains("YAKOGL_DRV_GPU_PVR=n") && !l.contains("YAKOGL_DRV_GPU_PVR is not set"),
                "{name} turns the PowerVR driver off ({l})"
            );
        }
    }
}

#[test]
fn the_symbol_builds_the_crate_with_the_20_us_poll() {
    let at = CMAKE.find(&format!("if(CONFIG_{SYMBOL})")).expect("CMakeLists.txt tests the symbol");
    let block = CMAKE[at..].split("endif()").next().unwrap();
    assert!(
        block.contains("list(APPEND _yd_features pvr-fine-wait)"),
        "CONFIG_{SYMBOL} adds the cargo feature pvr-fine-wait: {block}"
    );
    assert!(
        DRIVERS_CARGO.lines().any(|l| l.trim() == "pvr-fine-wait = [\"pvr\"]"),
        "driver/rust/Cargo.toml has the feature pvr-fine-wait"
    );
    let fine = "#[cfg(feature = \"pvr-fine-wait\")]\npub const WAIT_POLL_US: u32 = 20;";
    let coarse = "#[cfg(not(feature = \"pvr-fine-wait\"))]\npub const WAIT_POLL_US: u32 = 1000;";
    assert!(RENDER_RS.contains(fine), "render.rs: WAIT_POLL_US is 20 us with pvr-fine-wait");
    assert!(RENDER_RS.contains(coarse), "render.rs: WAIT_POLL_US is 1 ms without it");
}
