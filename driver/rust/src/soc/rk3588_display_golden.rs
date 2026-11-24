// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The golden test of the T6's cold display path: the register image the
//! combined model (`model_rk3588_display`) holds after
//! [`super::rk3588_display_chain::bring_up`] with the T6's configuration
//! ([`super::rk3588_display_chain::t6`]), against the registers of the same
//! blocks read from the board while its Linux showed a picture
//! (driver/tools/kexec-zephyr/t6-display-dump.sh). There is one dump per
//! mode of the chain ([`CASES`]): driver/docs/nanopc-t6-display-dump-linux.txt
//! at 1024x768 (step D0) and nanopc-t6-display-dump-linux-1920x1080.txt.
//! Bits that may differ, and why, are listed in
//! driver/docs/nanopc-t6-display-allow.txt for both dumps and in
//! nanopc-t6-display-allow-1920x1080.txt for the second only; every other
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
use crate::ip::hdptx::{self, Plan, PlanWrite, PllWords};
use crate::model_rk3588_display::{chain_bases, Mapped, Rk3588Display, PHY};

/// A file of driver/docs, read when the test runs: a missing one fails the
/// test by name, never passes it.
fn doc(name: &str) -> String {
    let path = format!("{}/../docs/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{path}: {e} (a dump or an allow list of the T6's display)"))
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

/// One mode of the T6's chain and the Linux dump that holds it.
struct Case {
    mode: t6::Mode,
    plan: Plan,
    /// The dump, a file of driver/docs.
    dump: &'static str,
    /// The allow lists that hold for this dump (files of driver/docs).
    allow: &'static [&'static str],
}

/// D0's dump (1024x768, 2026-11-12) and the dump of Linux on the capture
/// unit (1920x1080, 2026-11-23): the same blocks, the same reader.
const CASES: [Case; 2] = [
    Case {
        mode: t6::Mode::M1024x768,
        plan: t6::PLAN,
        dump: "nanopc-t6-display-dump-linux.txt",
        allow: &["nanopc-t6-display-allow.txt"],
    },
    Case {
        mode: t6::Mode::M1920x1080,
        plan: t6::PLAN_1920X1080,
        dump: "nanopc-t6-display-dump-linux-1920x1080.txt",
        allow: &["nanopc-t6-display-allow.txt", "nanopc-t6-display-allow-1920x1080.txt"],
    },
];

/// (address, value)
fn dump(c: &Case) -> Vec<(usize, u32)> {
    lines(&doc(c.dump))
        .into_iter()
        .map(|f| {
            assert!(f.len() == 3, "dump line {f:?}");
            assert_eq!(f[1], "32", "dump line {f:?}: every block here is read 32 bits wide");
            (hex(&f[0]) as usize, hex(&f[2]) as u32)
        })
        .collect()
}

fn dumped(c: &Case, addr: usize) -> u32 {
    dump(c).iter().find(|(a, _)| *a == addr).unwrap_or_else(|| panic!("{addr:#x} not in {}", c.dump)).1
}

/// (address, mask, reason)
fn allow(c: &Case) -> Vec<(usize, u32, String)> {
    c.allow
        .iter()
        .flat_map(|name| lines(&doc(name)))
        .map(|f| {
            assert!(f.len() >= 3, "allow-list line {f:?} needs ADDRESS MASK REASON");
            (hex(&f[0]) as usize, hex(&f[1]) as u32, f[2..].join(" "))
        })
        .collect()
}

/// The bits of PHY register `n` that `plan` writes (a field of a word the
/// image may also hold, `hdptx::PLAN_WHOLE_WORDS`' text).
fn plan_bits(plan: &Plan, n: u16) -> u32 {
    plan.writes().as_slice().iter().fold(0, |bits, w| match *w {
        PlanWrite::Word(r, _) if r == n => u32::MAX,
        PlanWrite::Field(r, mask, _) if r == n => bits | mask,
        _ => bits,
    })
}

#[test]
fn the_dumps_and_the_allow_list_parse_and_name_modelled_registers() {
    for c in &CASES {
        let d = dump(c);
        assert!(!d.is_empty(), "{} is empty", c.dump);
        for (a, _) in &d {
            assert!(Rk3588Display::block_of(*a).is_some(), "{} {a:#x}: not in a modelled block", c.dump);
        }
        for (a, mask, why) in allow(c) {
            assert!(d.iter().any(|(x, _)| *x == a), "allow-list {a:#x}: not a register of {}", c.dump);
            assert!(mask != 0 && !why.is_empty(), "allow-list {a:#x}: empty mask or no reason");
        }
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

/// The image is one for both modes: outside the bits the mode's plan writes
/// (the ROPLL fields of 0x5e, 0x64, 0x69 and 0x86, which
/// [`each_plan_is_linuxs_row_read_back_from_its_dump`] holds), each of its
/// registers reads the same in both dumps.
#[test]
fn the_phy_image_is_each_dump_except_seven_registers_with_linuxs_value() {
    let img = t6::PHY_IMAGE;
    let all: Vec<_> = img.cmn.iter().chain(img.sb).chain(img.lane_top).chain(img.lanes).collect();
    assert_eq!(all.len(), 189, "the registers Linux's tables write (hdptx.c:459-690)");
    for c in &CASES {
        for &&(n, v) in &all {
            let d = dumped(c, PHY + 4 * n as usize);
            match PHY_READ_BACK_DIFFERS.iter().find(|x| x.0 == n) {
                Some(&(_, linux, line)) => {
                    assert_eq!(v, linux, "{n:#x}: Linux's value (hdptx.c:{line})");
                    assert_ne!(d, linux, "{n:#x}: {} now reads Linux's value; drop it from the list", c.dump);
                }
                None => {
                    let own = !plan_bits(&c.plan, n);
                    assert_eq!(v & own, d & own, "{n:#x}: the image holds the value of {}", c.dump);
                }
            }
        }
        // and the allow list masks exactly these seven in the PHY's image words
        for (n, _, _) in PHY_READ_BACK_DIFFERS {
            let a = PHY + 4 * n as usize;
            assert!(allow(c).iter().any(|x| x.0 == a), "{a:#x} ({n:#x}) is not in the allow list");
        }
    }
}

#[test]
fn each_plan_is_linuxs_row_read_back_from_its_dump() {
    use hdptx::reg::*;
    for c in &CASES {
        let r = |n: u16| dumped(c, PHY + 4 * n as usize);
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
        assert_eq!(w.plan(c.plan.rate_hz), Some(c.plan), "{}", c.dump);
        let cfg = t6::config(c.mode, false, false, 0);
        assert_eq!(cfg.plan, c.plan, "{:?}", c.mode);
        assert_eq!(u64::from(cfg.mode.pixel_clock_hz), c.plan.rate_hz, "{:?}", c.mode);
    }
}

#[test]
fn the_controller_id_is_the_dumps() {
    for c in &CASES {
        assert_eq!(t6::BIND.core_id, dumped(c, RK3588.hdmi), "{}", c.dump);
        assert_eq!(t6::BIND.timer_base_hz, dumped(c, RK3588.hdmi + 0x80), "{}", c.dump);
    }
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

/// The chain run on the cold model in the case's mode, then every dumped
/// register compared outside the allow list.
fn cold_image_differences(c: &Case) -> Vec<String> {
    let mut m = Rk3588Display::cold();
    m.core_id = dumped(c, RK3588.hdmi);
    // The CRU's PLL and CLKSEL words are the boot loader's: the chain reads
    // them and sets only its own fields in them (ACLK_VOP's mux and divider,
    // aclk_vop_sub_src, DCLK_VOP0's mux). The run starts from the dump's
    // values there, so a write that reaches beyond its field shows.
    for (addr, v) in dump(c) {
        let off = addr.wrapping_sub(RK3588.cru);
        if off < CRU_PLL_END || (CRU_CLKSEL..CRU_CLKGATE).contains(&off) {
            m.poke(addr, v);
        }
    }
    // the bases mapped, as the probe passes them
    let r = bring_up(&Mapped(&m), &chain_bases(), &m, &t6::config(c.mode, false, false, 0), FB, STRIDE);
    assert!(r.is_ok(), "{r:?}");
    assert_eq!(m.refused(), Vec::<String>::new());
    let allow = allow(c);
    let mut bad = Vec::new();
    for (addr, want) in dump(c) {
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
    bad
}

#[test]
fn cold_image_matches_the_linux_dump_outside_the_allow_list() {
    let bad = cold_image_differences(&CASES[0]);
    assert!(bad.is_empty(), "{} register(s) differ outside the allow list:\n{}", bad.len(), bad.join("\n"));
}

#[test]
fn cold_image_at_1920x1080_matches_its_linux_dump_outside_the_allow_list() {
    let bad = cold_image_differences(&CASES[1]);
    assert!(bad.is_empty(), "{} register(s) differ outside the allow list:\n{}", bad.len(), bad.join("\n"));
}
