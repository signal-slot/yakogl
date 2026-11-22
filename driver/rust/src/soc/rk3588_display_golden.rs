// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The golden test of the T6's cold display path: the register image the
//! combined model (`model_rk3588_display`) holds after
//! [`super::rk3588_display_chain::bring_up`] with the T6's configuration
//! ([`super::rk3588_display_chain::t6`]), against the registers of the same
//! blocks read from the board while its Linux showed a picture
//! (driver/docs/nanopc-t6-display-dump-linux.txt, step D0, read by
//! driver/tools/kexec-zephyr/t6-display-dump.sh). Bits that may differ, and
//! why, are listed in driver/docs/nanopc-t6-display-allow.txt; every other
//! bit of every dumped register must agree.
//!
//! The dump is test data and, for the PHY's analog words, the source of the
//! image (owner decision U1): a difference outside the allow list is a
//! question for Linux's source and the board, answered by a change in the
//! driver or by an allow-list line with its reason.
//!
//! File formats (both: `#` starts a comment, blank lines ignored):
//! - dump: `ADDRESS WIDTH VALUE`, hex address, width 32, hex value;
//! - allow list: `ADDRESS MASK REASON...`, the bits of MASK may differ.

use super::rk3588_display_chain::{bring_up, t6, RK3588};
use crate::ip::hdptx::{self, PllWords};
use crate::model_rk3588_display::{chain_bases, Mapped, Rk3588Display, PHY};

/// A file of driver/docs, read when the test runs. The two files are step
/// D0's (they land with its dump); a missing one fails the test by name,
/// never passes it.
fn doc(name: &str) -> String {
    let path = format!("{}/../docs/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e} (the D0 dump and its allow list)"))
}

fn hex(s: &str) -> u64 {
    u64::from_str_radix(s.trim_start_matches("0x"), 16).unwrap_or_else(|_| panic!("not hex: {s}"))
}

fn lines(text: &str) -> Vec<Vec<String>> {
    text.lines()
        .map(|l| l.split('#').next().unwrap_or("").trim())
        .filter(|l| !l.is_empty())
        .map(|l| l.split_whitespace().map(str::to_string).collect())
        .collect()
}

/// (address, value)
fn dump() -> Vec<(usize, u32)> {
    lines(&doc("nanopc-t6-display-dump-linux.txt"))
        .into_iter()
        .map(|f| {
            assert!(f.len() == 3, "dump line {f:?}");
            assert_eq!(f[1], "32", "dump line {f:?}: every block here is read 32 bits wide");
            (hex(&f[0]) as usize, hex(&f[2]) as u32)
        })
        .collect()
}

fn dumped(addr: usize) -> u32 {
    dump().iter().find(|(a, _)| *a == addr).unwrap_or_else(|| panic!("{addr:#x} not in the dump")).1
}

/// (address, mask, reason)
fn allow() -> Vec<(usize, u32, String)> {
    lines(&doc("nanopc-t6-display-allow.txt"))
        .into_iter()
        .map(|f| {
            assert!(f.len() >= 3, "allow-list line {f:?} needs ADDRESS MASK REASON");
            (hex(&f[0]) as usize, hex(&f[1]) as u32, f[2..].join(" "))
        })
        .collect()
}

#[test]
fn the_dump_and_the_allow_list_parse_and_name_modelled_registers() {
    let d = dump();
    assert!(!d.is_empty(), "the D0 dump is empty");
    for (a, _) in &d {
        assert!(Rk3588Display::block_of(*a).is_some(), "dump {a:#x}: not in a modelled block");
    }
    for (a, mask, why) in allow() {
        assert!(d.iter().any(|(x, _)| *x == a), "allow-list {a:#x}: not a dumped register");
        assert!(mask != 0 && !why.is_empty(), "allow-list {a:#x}: empty mask or no reason");
    }
}

/// The PHY registers Linux's TMDS tables write (hdptx.c:459-690, the
/// tables of 340 MHz and below) whose dump value is not the value written
/// last: the image writes Linux's (owner decision for D7). (register,
/// Linux's value, hdptx.c line)
const PHY_READ_BACK_DIFFERS: [(u16, u32, u32); 7] = [
    (0x30, 0x20, 571),
    (0x46, 0xff, 503),
    (0x48, 0x44, 505),
    (0x31e, 0x00, 671),
    (0x41e, 0x00, 673),
    (0x51e, 0x00, 675),
    (0x61e, 0x08, 677),
];

#[test]
fn the_phy_image_is_the_dump_except_seven_registers_with_linuxs_value() {
    let img = t6::PHY_IMAGE;
    let all: Vec<_> = img.cmn.iter().chain(img.sb).chain(img.lane_top).chain(img.lanes).collect();
    assert_eq!(all.len(), 189, "the registers Linux's tables write (hdptx.c:459-690)");
    for &&(n, v) in &all {
        let d = dumped(PHY + 4 * n as usize);
        match PHY_READ_BACK_DIFFERS.iter().find(|x| x.0 == n) {
            Some(&(_, linux, line)) => {
                assert_eq!(v, linux, "{n:#x}: Linux's value (hdptx.c:{line})");
                assert_ne!(d, linux, "{n:#x}: the dump now reads Linux's value; drop it from the list");
            }
            None => assert_eq!(v, d, "{n:#x}: the image holds the dump's value"),
        }
    }
    // and the allow list masks exactly these seven in the PHY's image words
    for (n, _, _) in PHY_READ_BACK_DIFFERS {
        let a = PHY + 4 * n as usize;
        assert!(allow().iter().any(|x| x.0 == a), "{a:#x} ({n:#x}) is not in the allow list");
    }
}

#[test]
fn the_plan_is_linuxs_row_read_back_from_the_dump() {
    let r = |n: u16| dumped(PHY + 4 * n as usize);
    use hdptx::reg::*;
    let w = PllWords {
        m: r(PLL_M),
        m_afc: r(PLL_M_AFC),
        pdiv: r(PLL_PDIV),
        sdiv: r(PLL_SDIV),
        sdm_ctrl: r(PLL_SDM_CTRL),
        sdm_deno: r(PLL_SDM_DENO),
        sdm_sign: r(PLL_SDM_SIGN),
        sdm_num: r(PLL_SDM_NUM),
        sdc_n: r(PLL_SDC_N),
        sdc_num: r(PLL_SDC_NUM),
        sdc_deno: r(PLL_SDC_DENO),
        pcg: r(PLL_PCG),
    };
    assert_eq!(w.plan(65_000_000), Some(t6::PLAN));
}

#[test]
fn the_controller_id_is_the_dumps() {
    assert_eq!(t6::BIND.core_id, dumped(RK3588.hdmi));
    assert_eq!(t6::BIND.timer_base_hz, dumped(RK3588.hdmi + 0x80));
}

/// The CRU's PLL words (`PLL_CON`, `MODE_CON0`) end below 0x300, its
/// `CLKSEL_CON` words are 0x300-0x7ff (clk.h:359-365, as `soc::rk3588`).
const CRU_PLL_END: usize = 0x300;
const CRU_CLKSEL: usize = 0x300;
const CRU_CLKGATE: usize = 0x800;

/// The buffer the golden run shows: a 64-byte step of the image window,
/// 1024 pixels of 4 bytes a line.
const FB: u64 = 0x1000_0000;
const STRIDE: u32 = 4096;

#[test]
fn cold_image_matches_the_linux_dump_outside_the_allow_list() {
    let mut m = Rk3588Display::cold();
    m.core_id = dumped(RK3588.hdmi);
    // The CRU's PLL and CLKSEL words are the boot loader's: the chain reads
    // them and sets only its own fields in them (ACLK_VOP's mux and divider,
    // aclk_vop_sub_src, DCLK_VOP0's mux). The run starts from the dump's
    // values there, so a write that reaches beyond its field shows.
    for (addr, v) in dump() {
        let off = addr.wrapping_sub(RK3588.cru);
        if off < CRU_PLL_END || (CRU_CLKSEL..CRU_CLKGATE).contains(&off) {
            m.poke(addr, v);
        }
    }
    // the bases mapped, as the probe passes them
    let r = bring_up(&Mapped(&m), &chain_bases(), &m, &t6::config(false, false, 0), FB, STRIDE);
    assert!(r.is_ok(), "{r:?}");
    assert_eq!(m.refused(), Vec::<String>::new());
    let allow = allow();
    let mut bad = Vec::new();
    for (addr, want) in dump() {
        let got = m.peek(addr);
        let may = allow.iter().filter(|x| x.0 == addr).fold(0, |a, x| a | x.1);
        if (got ^ want) & !may != 0 {
            let blk = Rk3588Display::block_of(addr).unwrap();
            bad.push(format!(
                "{addr:#010x} ({blk}): ours {got:#x}, Linux {want:#x}, differing {:#x}",
                (got ^ want) & !may
            ));
        }
    }
    assert!(bad.is_empty(), "{} register(s) differ outside the allow list:\n{}", bad.len(), bad.join("\n"));
}
