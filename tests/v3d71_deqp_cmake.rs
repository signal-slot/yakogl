// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The deqp sample's CMakeLists, read as text: its `CONFIG_YAKOGL_GPU_V3D71`
//! branch (Raspberry Pi 5) takes `src/v3d71_board.c` and links none of the
//! rpi4 board layer's sources (`rpi_mbox.c`, `rpi_wdog.c`, `fbcon.c`, which
//! the rpi5 board layer already carries: linked twice they fail the link),
//! and the Pi 4 branch (`CONFIG_YAKOGL_GPU_V3D`) keeps its sources. The V3D71
//! branch clears `CONFIG_YAKOGL_GPU_V3D` so the Pi 4 branch is skipped for
//! rpi_5, and it must come first for that to work.
//!
//! No feature is needed:
//!
//! ```text
//! cargo test --test v3d71_deqp_cmake
//! ```

const CMAKE: &str = include_str!("../driver/samples/zephyr/deqp/CMakeLists.txt");

/// The file without comment lines.
fn code() -> String {
    CMAKE.lines().filter(|l| !l.trim_start().starts_with('#')).collect::<Vec<_>>().join("\n")
}

/// Byte offset of the top-level `if(<cond>)` line and the body up to its
/// closing top-level `endif()`.
fn branch(code: &str, cond: &str) -> (usize, String) {
    let head = format!("\nif({cond})\n");
    let at = code.find(&head).unwrap_or_else(|| panic!("no top-level {head:?}"));
    let body = &code[at + head.len()..];
    let end = body.find("\nendif()").expect("closing endif()");
    (at, body[..end].to_string())
}

const RPI4_SOURCES: [&str; 3] = ["rpi_mbox.c", "rpi_wdog.c", "fbcon.c"];

#[test]
fn v3d71_branch_takes_its_board_file_and_no_rpi4_sources() {
    let (_, b) = branch(&code(), "CONFIG_YAKOGL_GPU_V3D71");
    assert!(b.contains("target_sources(app PRIVATE src/v3d71_board.c)"), "{b}");
    for s in RPI4_SOURCES {
        assert!(!b.contains(s), "the V3D71 branch names {s}");
    }
    assert!(!b.contains("src/v3d_board.c"), "the Pi 4 board file");
    assert!(!b.contains("RPI_WDOG_HOLD_S"), "rpi_wdog.c's define belongs to the Pi 4");
    // the headers of rpi4 stay on the include path: only the sources are not linked
    assert!(b.contains("lib/board/rpi4"), "rpi_mbox.h include");
}

#[test]
fn v3d71_branch_hides_the_symbol_from_the_pi4_branch_and_comes_first() {
    let c = code();
    let (at71, b) = branch(&c, "CONFIG_YAKOGL_GPU_V3D71");
    assert!(b.contains("set(CONFIG_YAKOGL_GPU_V3D OFF)"), "{b}");
    let (at42, _) = branch(&c, "CONFIG_YAKOGL_GPU_V3D");
    assert!(at71 < at42, "the V3D71 branch must precede the Pi 4 branch");
    assert!(!c[..at71].contains("src/v3d_board.c"), "Pi 4 sources before the V3D71 branch");
}

#[test]
fn pi4_branch_keeps_its_sources() {
    let (_, b) = branch(&code(), "CONFIG_YAKOGL_GPU_V3D");
    assert!(b.contains("src/v3d_board.c"), "{b}");
    for s in RPI4_SOURCES {
        assert!(b.contains(s), "the Pi 4 branch lost {s}");
    }
    assert!(b.contains("set(V3D_BOARD \"${PVR_REPO}/zephyr/lib/board/rpi4\")"), "{b}");
    assert!(b.contains("RPI_WDOG_HOLD_S=${CONFIG_V3D_DEQP_LIFETIME_S}"), "{b}");
    assert!(!b.contains("v3d71_board.c"), "the Pi 5 board file in the Pi 4 branch");
}

#[test]
fn each_board_source_is_named_once_in_the_file() {
    let c = code();
    for s in ["src/v3d71_board.c", "src/v3d_board.c", "rpi_mbox.c", "rpi_wdog.c", "fbcon.c"] {
        assert_eq!(c.matches(s).count(), 1, "{s}");
    }
}
