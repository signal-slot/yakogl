// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The glmark2 sample's `CMakeLists.txt` on the two V3D boards, pinned at the
//! text level. It has no V3D71 branch and no Pi 4 branch: it names no board
//! source of the Raspberry Pi 4 (`rpi4/rpi_wdog.c`, `rpi_mbox.c`, ...) and no
//! rpi board, so neither the rpi_5 (V3D 7.1) nor the rpi_4b (V3D 4.2) image
//! can link one of them twice from the sample. Both take their board code
//! from the board layer's CMakeLists, once per Kconfig choice: the Pi 5 from
//! the `YAKOGL_RPI5_*` libraries (`rpi_wdog.c`, `rpi_mbox.c` each once), the
//! Pi 4 from the `board_rpi4.c` group, which keeps its sources.
//!
//! If a V3D71 or rpi branch is added to the glmark2 CMakeLists, the
//! `sample_*` tests fail on purpose: replace them with the pins of that branch.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d71_glmark2_cmake
//! ```

const CMAKE: &str = include_str!("../driver/samples/zephyr/glmark2/CMakeLists.txt");
const BOARD_CMAKE: &str = include_str!("../driver/zephyr/lib/board/CMakeLists.txt");

/// The CMake lines that are not comments.
fn active(text: &str) -> Vec<&str> {
    text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).collect()
}

#[test]
fn sample_links_no_rpi4_board_source() {
    for l in active(CMAKE) {
        for s in ["rpi4/", "rpi_wdog", "rpi_mbox", "fbcon.c", "board_rpi4", "board_rpi5", "rpi5/"] {
            assert!(!l.contains(s), "glmark2 CMakeLists names a Pi board source ({s}): {l}");
        }
    }
}

#[test]
fn sample_has_no_v3d71_or_rpi_branch() {
    for l in active(CMAKE) {
        let u = l.to_ascii_uppercase();
        assert!(!u.contains("V3D71") && !u.contains("RPI") && !u.contains("BCM2712"), "{l}");
    }
    // its BOARD cases are the Mali/Vivante boards only
    let cases: Vec<&str> = active(CMAKE).into_iter().filter(|l| l.starts_with("if(BOARD MATCHES")).collect();
    assert_eq!(cases.len(), 3, "{cases:?}");
}

#[test]
fn sample_takes_the_board_layer_through_sample_cmake_only() {
    let a = active(CMAKE);
    assert!(a.iter().any(|l| l.contains("common/sample.cmake")));
    // the app's own sources are ours, none of a board
    let srcs: Vec<&&str> = a.iter().filter(|l| l.starts_with("target_sources(app")).collect();
    assert_eq!(srcs.len(), 1, "{srcs:?}");
    assert!(srcs[0].contains("src/main.cpp src/platform.c"), "{}", srcs[0]);
}

#[test]
fn board_layer_links_each_pi5_source_once() {
    let a = active(BOARD_CMAKE);
    let guard = |sym: &str, src: &str| {
        let i = a.iter().position(|l| *l == format!("if({sym})")).unwrap_or_else(|| panic!("{sym}"));
        let body: Vec<&&str> = a[i + 1..].iter().take_while(|l| **l != "endif()").collect();
        let n = body.iter().filter(|l| l.starts_with("zephyr_library_sources(") && l.contains(src)).count();
        assert_eq!(n, 1, "{sym}: {src} once in its own library");
    };
    guard("CONFIG_YAKOGL_RPI5_WDOG", "rpi4/rpi_wdog.c");
    guard("CONFIG_YAKOGL_RPI5_MBOX", "rpi4/rpi_mbox.c");
}

#[test]
fn board_layer_keeps_the_pi4_sources() {
    let l = active(BOARD_CMAKE).into_iter().find(|l| l.contains("rpi4/board_rpi4.c")).expect("the Pi 4 group");
    for s in ["rpi4/board_rpi4.c", "rpi4/rpi_mbox.c", "rpi4/fbcon.c", "rpi4/rpi_wdog.c"] {
        assert!(l.contains(s), "{s}: {l}");
    }
}

#[test]
fn board_layer_serves_the_glmark2_pi5_image_without_double_links() {
    // The glmark2 image sets neither YAKOGL_RPI5_WDOG nor _MBOX: the SOC_BCM2712
    // arm links the two sources itself, and only when the own libraries are off.
    let a = active(BOARD_CMAKE);
    let i = a.iter().position(|l| *l == "elseif(CONFIG_SOC_BCM2712)").expect("the SoC arm");
    let arm: Vec<&&str> =
        a[i + 1..].iter().take_while(|l| !l.starts_with("zephyr_library_include_directories(rpi4")).collect();
    let text = arm.iter().map(|l| **l).collect::<Vec<_>>().join("\n");
    assert!(text.contains("if(NOT CONFIG_YAKOGL_RPI5_WDOG)\nzephyr_library_sources(rpi4/rpi_wdog.c)"), "{text}");
    assert!(text.contains("if(NOT CONFIG_YAKOGL_RPI5_MBOX)\nzephyr_library_sources(rpi4/rpi_mbox.c)"), "{text}");
}
