// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The 20 us PowerVR fence poll is on by default for the AM62P build
//! (docs/notes/pvr-fence-wait.md): the Kconfig default is conditioned on the
//! AM62P SoC series, no AM62P image `.conf` turns it off, and the module's
//! CMake maps the symbol to the `pvr-fine-wait` cargo feature.

const KCONFIG: &str = include_str!("../driver/zephyr/Kconfig");
const CMAKE: &str = include_str!("../driver/zephyr/CMakeLists.txt");
const CARGO: &str = include_str!("../driver/rust/Cargo.toml");
const AM62P_CONFS: [(&str, &str); 2] = [
    ("app.conf", include_str!("../driver/zephyr/lib/board/am62p/app.conf")),
    ("app_direct.conf", include_str!("../driver/zephyr/lib/board/am62p/app_direct.conf")),
];

/// The lines of the `config YAKOGL_DRV_GPU_PVR_FINE_WAIT` block.
fn fine_wait_block() -> Vec<&'static str> {
    let at = KCONFIG.find("config YAKOGL_DRV_GPU_PVR_FINE_WAIT\n").expect("the Kconfig symbol");
    KCONFIG[at..].lines().skip(1).take_while(|l| !l.starts_with("config ")).collect()
}

#[test]
fn kconfig_default_is_y_for_the_am62p_only() {
    let defaults: Vec<&str> =
        fine_wait_block().into_iter().map(str::trim).filter(|l| l.starts_with("default ")).collect();
    assert_eq!(defaults, ["default y if SOC_SERIES_AM62PX_A53"], "one default, on for the AM62P series only");
}

#[test]
fn no_am62p_image_config_turns_it_off() {
    for (name, text) in AM62P_CONFS {
        assert!(
            !text.lines().any(|l| l.trim_start().starts_with("CONFIG_YAKOGL_DRV_GPU_PVR_FINE_WAIT")),
            "{name} overrides the default"
        );
    }
}

#[test]
fn the_symbol_selects_the_cargo_feature() {
    let at = CMAKE.find("if(CONFIG_YAKOGL_DRV_GPU_PVR_FINE_WAIT)").expect("the CMake condition");
    let next = CMAKE[at..].lines().nth(1).unwrap().trim();
    assert_eq!(next, "list(APPEND _yd_features pvr-fine-wait)");
    assert!(CARGO.lines().any(|l| l.trim_start().starts_with("pvr-fine-wait")), "the cargo feature exists");
}
