// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.

//! The 7.1 table's identity check (src/v3d/ident71.rs): field positions
//! against driver sources by text, the board's recorded identity, and every
//! refusal with synthetic words.
//!
//! Run: `cargo test --no-default-features --features
//! host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d71_ident`

#![allow(dead_code)]

#[path = "../src/v3d/ident71.rs"]
mod ident71;

use ident71::*;

fn read(p: &str) -> String {
    std::fs::read_to_string(format!("{}/{p}", env!("CARGO_MANIFEST_DIR"))).unwrap()
}

fn words(tver: u32, rev: u32, iprev: u32, compat: u32) -> (u32, u32) {
    (tver | rev << 4 | 1 << 8, compat << 16 | iprev << 8)
}

#[test]
fn field_positions_match_driver_text() {
    let regs = read("driver/src/v3d/v3d_regs.h");
    assert!(regs.contains("V3D_HUB_CTL_IDENT1_TVER_GET(v)   ((v) & 0xf)"));
    assert!(regs.contains("V3D_HUB_CTL_IDENT1_REV_GET(v)    (((v) >> 4) & 0xf)"));
    let hw = read("driver/src/v3d/v3d_hw.c");
    assert!(hw.contains("V3D_HUB_CTL_IDENT1_TVER_GET(h1) * 10 + V3D_HUB_CTL_IDENT1_REV_GET(h1)"));
    let facts = read("driver/docs/rpi5-v3d71-facts.md");
    assert!(facts.contains("`TVER` 3:0, `REV` 7:4"));
    assert!(facts.contains("`IPREV` 15:8"));
    assert!(facts.contains("23:16 as `compat_rev`"));
}

#[test]
fn board_words_pass() {
    // "Revision: 7.1.10.16" is ver.iprev.compat_rev.
    let t = read("tools/v3d/pi5/v3d_ident.txt");
    let rev = t.lines().find_map(|l| l.strip_prefix("Revision:")).unwrap().trim();
    let p: Vec<u32> = rev.split('.').map(|x| x.parse().unwrap()).collect();
    assert_eq!(p.len(), 4);
    let (h1, h3) = words(p[0], p[1], p[2], p[3]);
    let id = check_words(h1, h3).unwrap();
    assert_eq!((id.ver, id.iprev, id.compat_rev), (71, 10, 16));
}

#[test]
fn pi4_refused() {
    let (h1, h3) = words(4, 2, 14, 0);
    let e = check_words(h1, h3).unwrap_err();
    assert_eq!(e.to_string(), "V3D 4.2: build gpu-v3d");
}

#[test]
fn old_71_refused() {
    for ip in [9, 6, 2] {
        let (h1, h3) = words(7, 1, ip, 0);
        let e = check_words(h1, h3).unwrap_err();
        assert_eq!(e.to_string(), format!("V3D 7.1.{ip}: this table is 7.1.10 (BCM2712 D0) only"));
    }
}

#[test]
fn newer_iprev_passes() {
    let (h1, h3) = words(7, 1, 11, 0);
    assert!(check_words(h1, h3).is_ok());
}

#[test]
fn unknown_refused() {
    let (h1, h3) = words(3, 3, 10, 0);
    let e = check_words(h1, h3).unwrap_err();
    assert_eq!(e, Refusal::UnknownVersion { tver: 3, rev: 3, ver: 33 });
    assert!(e.to_string().contains("ver 33"));
    let (h1, h3) = words(7, 2, 10, 0);
    assert!(matches!(check_words(h1, h3), Err(Refusal::UnknownVersion { ver: 72, .. })));
}
