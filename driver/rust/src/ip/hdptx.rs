// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The RK3588 Samsung HDMI/DP combo PHY ("HDPTX", PHY0 0xfed60000, PHY1
//! 0xfed70000) in HDMI TMDS mode: its register file, its GRF words, the
//! ROPLL plan for a TMDS character rate, the TMDS bring-up in two stages
//! (PLL, then lanes), the ready and lock polls, power-off and the status
//! read-back. Step D5 of `driver/docs/nanopc-t6-display.md` (section 7); the
//! facts are its section 1 "HDPTX PHY0", corrected where Linux says
//! otherwise (listed in the commit that added this file).
//!
//! Sources: Linux v6.18, GPL-2.0, read for facts only (address, bit, value,
//! order, formula; `docs/source-form.md`):
//! `drivers/phy/rockchip/phy-rockchip-samsung-hdptx.c` ("hdptx.c"),
//! `arch/arm64/boot/dts/rockchip/rk3588-base.dtsi` ("dtsi"),
//! `rk3588-extra.dtsi` ("extra"). Facts:
//!
//! 1. The register file is 32-bit words, register n at byte 4·n
//!    (hdptx.c:35-40). Linux reads and writes only byte offsets 0x0000-0x029c
//!    (common, "CMN", registers 0x00-0xa7), 0x0400-0x04a4 (sideband, "SB",
//!    0x100-0x129), 0x0800-0x08a4 (lane top, 0x200-0x229) and, for lanes
//!    0-3, 0x0c00-0x0cb4, 0x1000-0x10b4, 0x1400-0x14b4, 0x1800-0x18b4
//!    (0x300-0x32d, 0x400-0x42d, 0x500-0x52d, 0x600-0x62d) (hdptx.c:776-799).
//!    Here an access outside them is refused before the bus sees it.
//! 2. The PHY's GRF (dtsi:724-727, extra:110-113; 0x100 bytes): `CON0` 0x00
//!    with `PLL_EN` bit 7, `BIAS_EN` 6, `BGR_EN` 5, `MODE_SEL` 0 (0 HDMI,
//!    1 DP); `STATUS` 0x80 with `PLL_LOCK_DONE` 3, `PHY_CLK_RDY` 2,
//!    `PHY_RDY` 1, `SB_RDY` 0 (hdptx.c:24-33). `CON0` is written with the
//!    write mask in bits 31:16 (hdptx.c:816, :827-828, :858).
//! 3. Linux drives four of the node's seven resets: "apb", "init", "cmn",
//!    "lane" (hdptx.c:375-381, :2033-2036). The node also names "phy",
//!    "ropll" and "lcpll" (dtsi:3125-3130); Linux v6.18 never asserts or
//!    releases them. Probe releases apb, cmn and init (hdptx.c:2067-2069).
//!    The resets are the CRU's (D4, `soc::rk3588_display::reset`), so this
//!    module asks its caller for them through [`Host`].
//! 4. Stage 1, before any PLL write (hdptx.c:804-818): assert apb, wait 20
//!    µs, release apb; assert lane, cmn, init; `CON0` `PLL_EN`, `BIAS_EN`,
//!    `BGR_EN` = 0. Before that Linux reads `STATUS` and warns when
//!    `PLL_LOCK_DONE` is already set (hdptx.c:1112-1117).
//! 5. Stage 2, inside that reset window (hdptx.c:1001-1036): the two CMN
//!    init tables (hdptx.c:459-589), then the ROPLL words of fact 8.
//! 6. Stage 3 (hdptx.c:845-874): `CON0` `BIAS_EN`, `BGR_EN` = 1; 10 µs;
//!    release init; 10 µs; `PLL_EN` = 1; 10 µs; release cmn; poll `STATUS`
//!    for `PHY_CLK_RDY`, 20 µs steps, 400 µs bound. Linux runs stages 1-3
//!    when the PHY's clock is prepared or its rate set (hdptx.c:1103-1125,
//!    :1849-1853, :1901-1920): the PLL then feeds "clk_hdmiphy_pixel0"
//!    (hdptx.c:1931-1958) at the TMDS character rate (hdptx.c:1040, :1868).
//! 7. Stage 4, at the PHY's power-on (hdptx.c:1466-1469, :1045-1066,
//!    :820-843): `CON0` `MODE_SEL` = 0 (HDMI); the SB init table
//!    (hdptx.c:591-596); lane top register 0x200 = 0x06; the lane-top table
//!    for TMDS up to 340 MHz (hdptx.c:606-612; above it another table,
//!    hdptx.c:598-604, :1051-1057, which this module does not take); lane top
//!    0x206 = 0x07, 0x207 = 0x0f; the two lane tables (hdptx.c:614-690);
//!    release lane; `CON0` `BIAS_EN`, `BGR_EN` = 1 again; poll `STATUS` for
//!    `PHY_RDY` and `PLL_LOCK_DONE`, 100 µs steps, 5000 µs bound. Lane top
//!    0x200 bit 2 is the HDMI protocol select and bit 1 the TMDS/FRL select;
//!    0x206 bits 2:1 the data bus width and bit 0 its 36/40 select; 0x207
//!    bits 3:0 the lane enables (hdptx.c:244-254).
//! 8. The ROPLL words (hdptx.c:80-151 for the fields, :1006-1036 for the
//!    order): 0x51 = M; 0x55 = M (the AFC copy); 0x59 = P in 7:4 and the
//!    reference divider in 3:0 (Linux writes 1 and 1); 0x5a = S - 1 in 7:4;
//!    0x5e bit 6 = SDM on, and when it is off bits 3:0 = 0; 0x64 bit 3 =
//!    the SDM numerator's sign; 0x60 = the SDM denominator; 0x65 = the SDM
//!    numerator; 0x69 bits 2:0 = the SDC divider N - 3; 0x6c = the SDC
//!    numerator (5:0); 0x70 = the SDC denominator (5:0); 0x86 bits 7:4 =
//!    S - 1 (post divider), 3:1 = (bpc - 8) / 2, bit 0 = pixel clock enable.
//!    0x5e, 0x64, 0x69 and 0x86 are read-modify-written; the CMN init tables
//!    set 0x5e's and 0x86's other bits.
//! 9. The PLL arithmetic (what hdptx.c:900-970 computes; the frequency
//!    form below reproduces the rate from every table row it was checked
//!    with, hdptx.c:427, :443, :445, :453): the reference is 24 MHz
//!    (hdptx.c:903); the PLL output is 5 x the TMDS character rate
//!    (hdptx.c:903: rate / 200 in kHz) and equals VCO / S, S even or 1, at
//!    most 16 (hdptx.c:911-913); VCO 2.0-4.0 GHz (:917); M 20-255 (:921).
//!    With the SDM off, VCO = 24 MHz x M. With it on, a sigma-delta clock
//!    Fsdc = 24 MHz x M / (N - a/b), N the SDC divider and a/b the SDC
//!    fraction, of 264-750 MHz (hdptx.c:925-942), and
//!    VCO = 24 MHz x M -/+ (k / lc) x Fsdc / 16, k the SDM numerator, lc its
//!    denominator, minus when the sign bit is 1 (hdptx.c:932-936, :958-965).
//!    Linux's calculator keeps k within 7 bits (at most 127) and lc within 8
//!    bits (at most 255) (hdptx.c:934-935); its registers 0x65 and 0x60 are 8
//!    bits wide (hdptx.c:106, :112), so a table row or a dump may hold a
//!    larger k.
//! 10. Power-off (hdptx.c:876-898), done by Linux only when `STATUS` shows
//!    `PLL_LOCK_DONE` (hdptx.c:1148-1155): assert apb, 20 µs, release apb;
//!    lane 0x300 = 0x82, SB 0x10f = 0xc1, SB 0x110 = 0x01, lane 0x301,
//!    0x401, 0x501, 0x601 = 0x80; assert lane, cmn, init; `CON0` `PLL_EN`,
//!    `BIAS_EN`, `BGR_EN` = 0.
//!
//! The init tables of facts 5 and 7 (212 writes of analog settings that no
//! public document explains) are not in this file: the board hands them in
//! as an [`Image`], the PHY's register file as the board dump (D0) reads it
//! under Linux with the same mode (note, question U1). An image holds the
//! final value of each register once, in ascending order; Linux's tables
//! write some registers twice and the lane tables out of order, so the
//! image reproduces Linux's end state, not each of its intermediate writes.

use crate::mmio::Mmio;

/// PHY bases (dtsi:3118-3120, extra:607-609; hdptx.c:2086-2092).
pub const PHY0_BASE: usize = 0xfed6_0000;
pub const PHY1_BASE: usize = 0xfed7_0000;
/// The PHY's register window (dtsi:3120).
pub const PHY_SIZE: usize = 0x2000;
/// The PHY's GRFs (dtsi:724-727, extra:110-113).
pub const GRF0_BASE: usize = 0xfd5e_0000;
pub const GRF1_BASE: usize = 0xfd5e_4000;

/// The register ranges Linux reads and writes, as register numbers
/// (fact 1). The byte offset of register n is 4·n.
pub const RW_RANGES: [(u16, u16); 7] = [
    (0x000, 0x0a7), // CMN
    (0x100, 0x129), // SB
    (0x200, 0x229), // lane top
    (0x300, 0x32d), // lane 0
    (0x400, 0x42d), // lane 1
    (0x500, 0x52d), // lane 2
    (0x600, 0x62d), // lane 3
];

/// Register n may be read and written (fact 1).
pub const fn is_rw(n: u16) -> bool {
    let mut i = 0;
    while i < RW_RANGES.len() {
        if n >= RW_RANGES[i].0 && n <= RW_RANGES[i].1 {
            return true;
        }
        i += 1;
    }
    false
}

/// Register numbers this module writes itself (facts 7, 8, 10).
pub mod reg {
    /// ROPLL M.
    pub const PLL_M: u16 = 0x51;
    /// ROPLL M, the AFC copy.
    pub const PLL_M_AFC: u16 = 0x55;
    /// ROPLL P (7:4), reference divider (3:0).
    pub const PLL_PDIV: u16 = 0x59;
    /// ROPLL S - 1 (7:4).
    pub const PLL_SDIV: u16 = 0x5a;
    /// SDM enable (bit 6) and the SDC/SDM fraction enables (3:0).
    pub const PLL_SDM_CTRL: u16 = 0x5e;
    /// SDM denominator.
    pub const PLL_SDM_DENO: u16 = 0x60;
    /// SDM numerator sign (bit 3).
    pub const PLL_SDM_SIGN: u16 = 0x64;
    /// SDM numerator.
    pub const PLL_SDM_NUM: u16 = 0x65;
    /// SDC divider N - 3 (2:0).
    pub const PLL_SDC_N: u16 = 0x69;
    /// SDC numerator (5:0).
    pub const PLL_SDC_NUM: u16 = 0x6c;
    /// SDC denominator (5:0).
    pub const PLL_SDC_DENO: u16 = 0x70;
    /// Post divider (7:4), pixel clock select (3:1) and enable (0).
    pub const PLL_PCG: u16 = 0x86;
    /// SB words written at power-off.
    pub const SB_VREG: u16 = 0x10f;
    pub const SB_VREG_SEL: u16 = 0x110;
    /// Lane top: protocol, data bus, lane enables.
    pub const LANE_TOP_MODE: u16 = 0x200;
    pub const LANE_TOP_BUS: u16 = 0x206;
    pub const LANE_TOP_LANES: u16 = 0x207;
    /// Lane 0's first register; lane n's is this + 0x100·n.
    pub const LANE0: u16 = 0x300;
    /// The lane register written at power-off in every lane (+0x100·n).
    pub const LANE0_TX_DRV: u16 = 0x301;
}

/// Field masks of the ROPLL words (fact 8).
pub mod field {
    pub const PDIV: u32 = 0xf0;
    pub const REFDIV: u32 = 0x0f;
    pub const SDIV: u32 = 0xf0;
    pub const SDM_EN: u32 = 1 << 6;
    pub const SDM_FRAC_ENS: u32 = 0x0f;
    pub const SDM_SIGN: u32 = 1 << 3;
    pub const SDC_N: u32 = 0x07;
    pub const SDC_FRAC: u32 = 0x3f;
    pub const PCG_POSTDIV: u32 = 0xf0;
    pub const PCG_CLK_SEL: u32 = 0x0e;
    pub const PCG_CLK_EN: u32 = 1 << 0;
}

/// The lane-top values of stage 4 (fact 7).
pub const LANE_TOP_MODE_TMDS: u32 = 0x06;
pub const LANE_TOP_BUS_TMDS: u32 = 0x07;
pub const LANE_TOP_ALL_LANES: u32 = 0x0f;

/// GRF offsets and bits (fact 2).
pub mod grf {
    pub const CON0: usize = 0x00;
    pub const STATUS: usize = 0x80;
    /// `CON0`
    pub const PLL_EN: u32 = 1 << 7;
    pub const BIAS_EN: u32 = 1 << 6;
    pub const BGR_EN: u32 = 1 << 5;
    pub const MODE_SEL: u32 = 1 << 0;
    /// `STATUS`
    pub const PLL_LOCK_DONE: u32 = 1 << 3;
    pub const PHY_CLK_RDY: u32 = 1 << 2;
    pub const PHY_RDY: u32 = 1 << 1;
    pub const SB_RDY: u32 = 1 << 0;
}

/// A GRF word: `bits` (inside `mask`) with the write mask in 31:16.
pub const fn hiword(mask: u32, bits: u32) -> u32 {
    (mask << 16) | (bits & mask)
}

/// The `CON0` words, in the order the stages write them.
pub const CON0_ALL_OFF: u32 = hiword(grf::PLL_EN | grf::BIAS_EN | grf::BGR_EN, 0);
pub const CON0_BIAS_ON: u32 = hiword(grf::BIAS_EN | grf::BGR_EN, grf::BIAS_EN | grf::BGR_EN);
pub const CON0_PLL_ON: u32 = hiword(grf::PLL_EN, grf::PLL_EN);
pub const CON0_MODE_HDMI: u32 = hiword(grf::MODE_SEL, 0);

/// The waits and poll bounds (facts 4, 6, 7, 10), in µs.
pub const APB_PULSE_US: u32 = 20;
pub const STAGE3_STEP_US: u32 = 10;
pub const CLK_RDY_STEP_US: u32 = 20;
pub const CLK_RDY_BOUND_US: u32 = 400;
pub const LOCK_STEP_US: u32 = 100;
pub const LOCK_BOUND_US: u32 = 5000;

/// The reference clock (fact 9).
pub const REF_HZ: u64 = 24_000_000;
/// The VCO range (fact 9).
pub const VCO_MIN_HZ: u64 = 2_000_000_000;
pub const VCO_MAX_HZ: u64 = 4_000_000_000;
/// The largest TMDS character rate this module takes: the lane-top table
/// it is handed is the one for rates up to 340 MHz (fact 7, hdptx.c:323).
pub const TMDS_MAX_HZ: u64 = 340_000_000;
/// The SDC divider this module's plans use (N = 8, as hdptx.c:906).
pub const SDC_N: u32 = 8;
/// The largest SDM numerator and denominator [`plan`] makes: 7 and 8 bits,
/// as Linux's calculator (hdptx.c:934-935).
pub const SDM_K_MAX: u64 = 127;
pub const SDM_LC_MAX: u64 = 255;
/// Fsdc's range (fact 9); a plan's Fsdc is a multiple of [`REF_HZ`] in it.
pub const FSDC_MIN_HZ: u64 = 264_000_000;
pub const FSDC_MAX_HZ: u64 = 750_000_000;

/// The four resets Linux drives (fact 3). The caller maps them to the CRU
/// (D4: `P_HDPTX0`, `HDPTX0_INIT`, `HDPTX0_CMN`, `HDPTX0_LANE` for PHY0).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PhyReset {
    Apb,
    Init,
    Cmn,
    Lane,
}

/// What the bring-up borrows from its caller: the PHY's resets (in the
/// CRU, which this module never touches) and a busy wait. Every call is
/// made in Linux's order, between this module's own register accesses.
pub trait Host {
    /// Assert (`true`) or release (`false`) one reset.
    fn reset(&self, r: PhyReset, asserted: bool);
    /// Wait at least `us` µs.
    fn wait_us(&self, us: u32);
}

/// The PHY's two windows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bases {
    pub phy: usize,
    pub grf: usize,
}

/// PHY0 and its GRF.
pub const PHY0: Bases = Bases { phy: PHY0_BASE, grf: GRF0_BASE };

/// Why a step refused. The value crosses to C as one number per cause.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum Refusal {
    /// A register outside [`RW_RANGES`]. Nothing was written.
    OutOfRange = 1,
    /// An image register outside its section's range. Nothing was written.
    ImageSection = 2,
    /// An image register not above the one before it. Nothing was written.
    ImageOrder = 3,
    /// An image register this module writes itself (the full-word ROPLL
    /// words and lane top 0x200, 0x206, 0x207). Nothing was written.
    ImageOwnWord = 4,
    /// No plan for the rate: 0, above [`TMDS_MAX_HZ`], or no S, M in range.
    Rate = 5,
    /// `PHY_CLK_RDY` not seen within [`CLK_RDY_BOUND_US`].
    ClkRdyTimeout = 6,
    /// `PHY_RDY` and `PLL_LOCK_DONE` not seen within [`LOCK_BOUND_US`].
    LockTimeout = 7,
}

/// A refusal and the value it is about: the register number, the rate
/// (cut to 32 bits), or the last `STATUS` read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Refused {
    pub why: Refusal,
    pub found: u32,
}

const fn refuse(why: Refusal, found: u32) -> Refused {
    Refused { why, found }
}

/// Write register `n`, refused outside [`RW_RANGES`].
pub fn write(m: &impl Mmio, phy: usize, n: u16, v: u32) -> Result<(), Refused> {
    if !is_rw(n) {
        return Err(refuse(Refusal::OutOfRange, n as u32));
    }
    m.write32(phy + 4 * n as usize, v);
    Ok(())
}

/// Read register `n`, refused outside [`RW_RANGES`].
pub fn read(m: &impl Mmio, phy: usize, n: u16) -> Result<u32, Refused> {
    if !is_rw(n) {
        return Err(refuse(Refusal::OutOfRange, n as u32));
    }
    Ok(m.read32(phy + 4 * n as usize))
}

fn modify(m: &impl Mmio, phy: usize, n: u16, mask: u32, v: u32) -> Result<(), Refused> {
    let old = read(m, phy, n)?;
    write(m, phy, n, (old & !mask) | (v & mask))
}

/// The register file's image for one mode: (register, value) pairs, each
/// section in ascending register order. The board builds it from the D0
/// dump; Linux's tables are not carried here (module text).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Image<'a> {
    /// CMN registers, written in stage 2 before the ROPLL words.
    pub cmn: &'a [(u16, u32)],
    /// SB registers, stage 4.
    pub sb: &'a [(u16, u32)],
    /// Lane-top registers, stage 4, between 0x200 and 0x206.
    pub lane_top: &'a [(u16, u32)],
    /// Lane 0-3 registers, stage 4, before the lane reset is released.
    pub lanes: &'a [(u16, u32)],
}

/// The ROPLL words a plan writes whole (fact 8): an image may not hold
/// them. 0x5e, 0x64, 0x69 and 0x86 are read-modify-written and may be in
/// the image (their other bits are the image's).
const PLAN_WHOLE_WORDS: [u16; 8] = [
    reg::PLL_M,
    reg::PLL_M_AFC,
    reg::PLL_PDIV,
    reg::PLL_SDIV,
    reg::PLL_SDM_DENO,
    reg::PLL_SDM_NUM,
    reg::PLL_SDC_NUM,
    reg::PLL_SDC_DENO,
];
const LANE_TOP_OWN: [u16; 3] = [reg::LANE_TOP_MODE, reg::LANE_TOP_BUS, reg::LANE_TOP_LANES];

fn check_section(words: &[(u16, u32)], ranges: &[(u16, u16)], own: &[u16]) -> Result<(), Refused> {
    let mut last: Option<u16> = None;
    for &(n, _) in words {
        if !ranges.iter().any(|&(lo, hi)| n >= lo && n <= hi) {
            return Err(refuse(Refusal::ImageSection, n as u32));
        }
        if last.is_some_and(|l| n <= l) {
            return Err(refuse(Refusal::ImageOrder, n as u32));
        }
        if own.contains(&n) {
            return Err(refuse(Refusal::ImageOwnWord, n as u32));
        }
        last = Some(n);
    }
    Ok(())
}

impl Image<'_> {
    /// Every register is inside its section's range (and so inside
    /// [`RW_RANGES`]), each section ascends, and no register is one this
    /// module writes itself.
    pub fn check(&self) -> Result<(), Refused> {
        check_section(self.cmn, &RW_RANGES[0..1], &PLAN_WHOLE_WORDS)?;
        check_section(self.sb, &RW_RANGES[1..2], &[])?;
        check_section(self.lane_top, &RW_RANGES[2..3], &LANE_TOP_OWN)?;
        check_section(self.lanes, &RW_RANGES[3..7], &[])
    }
}

fn write_all(m: &impl Mmio, phy: usize, words: &[(u16, u32)]) -> Result<(), Refused> {
    for &(n, v) in words {
        write(m, phy, n, v)?;
    }
    Ok(())
}

/// The sigma-delta part of a fractional plan (fact 9).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sdm {
    /// The SDM term is subtracted from 24 MHz x M (sign bit 1).
    pub minus: bool,
    /// SDM numerator k and denominator lc (8-bit registers; [`plan`] keeps
    /// k at most [`SDM_K_MAX`], as Linux's calculator does).
    pub k: u8,
    pub lc: u8,
    /// SDC divider N (3-10) and fraction a / b (6 bits each).
    pub n: u8,
    pub a: u8,
    pub b: u8,
}

/// The ROPLL dividers for one TMDS character rate (fact 9), P and the
/// reference divider 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Plan {
    /// The TMDS character rate asked for, Hz.
    pub rate_hz: u64,
    pub m: u8,
    /// S: even or 1, at most 16.
    pub s: u8,
    /// `None`: integer, VCO = 24 MHz x M.
    pub sdm: Option<Sdm>,
}

/// One ROPLL write of a plan: a whole word, or a field read-modify-written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlanWrite {
    Word(u16, u32),
    Field(u16, u32, u32),
}

/// The most [`PlanWrite`]s a plan makes (an integer plan; a fractional
/// one makes one fewer).
pub const PLAN_WRITES_MAX: usize = 15;

/// A plan's writes, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlanWrites {
    w: [PlanWrite; PLAN_WRITES_MAX],
    len: usize,
}

impl PlanWrites {
    pub fn as_slice(&self) -> &[PlanWrite] {
        &self.w[..self.len]
    }
}

impl Plan {
    /// The plan is one [`plan`] could have made: rate 1 Hz to
    /// [`TMDS_MAX_HZ`], S even or 1 up to 16, M 20-255, and for the SDM
    /// lc > 0, N 3-10, a and b within 6 bits and N·b > a.
    pub fn is_valid(&self) -> bool {
        let s_ok = self.s == 1 || (self.s.is_multiple_of(2) && self.s >= 2 && self.s <= 16);
        let sdm_ok = match self.sdm {
            None => true,
            Some(d) => {
                let f = field::SDC_FRAC as u8;
                d.lc > 0
                    && (3..=10).contains(&d.n)
                    && d.a <= f
                    && d.b <= f
                    && u16::from(d.n) * u16::from(d.b) > u16::from(d.a)
            }
        };
        self.rate_hz > 0 && self.rate_hz <= TMDS_MAX_HZ && s_ok && self.m >= 20 && sdm_ok
    }

    /// The ROPLL writes in Linux's order (fact 8), for 8 bits per
    /// component (pixel clock select 0). With the SDM off its terms are
    /// written as 0 and the four fraction enables cleared; with it on that
    /// clear is not made, as in Linux.
    pub fn writes(&self) -> PlanWrites {
        use PlanWrite::{Field, Word};
        let z = Sdm { minus: false, k: 0, lc: 0, n: 3, a: 0, b: 0 };
        let d = self.sdm.unwrap_or(z);
        let s1 = u32::from(self.s.saturating_sub(1));
        let sdm_on = self.sdm.is_some();
        let mut w = [Word(0, 0); PLAN_WRITES_MAX];
        let mut len = 0;
        let mut push = |x: PlanWrite| {
            w[len] = x;
            len += 1;
        };
        push(Word(reg::PLL_M, u32::from(self.m)));
        push(Word(reg::PLL_M_AFC, u32::from(self.m)));
        push(Word(reg::PLL_PDIV, (1 << 4) | 1));
        push(Word(reg::PLL_SDIV, s1 << 4));
        push(Field(reg::PLL_SDM_CTRL, field::SDM_EN, if sdm_on { field::SDM_EN } else { 0 }));
        if !sdm_on {
            push(Field(reg::PLL_SDM_CTRL, field::SDM_FRAC_ENS, 0));
        }
        push(Field(reg::PLL_SDM_SIGN, field::SDM_SIGN, if d.minus { field::SDM_SIGN } else { 0 }));
        push(Word(reg::PLL_SDM_DENO, u32::from(d.lc)));
        push(Word(reg::PLL_SDM_NUM, u32::from(d.k)));
        push(Field(reg::PLL_SDC_N, field::SDC_N, u32::from(d.n.saturating_sub(3))));
        push(Word(reg::PLL_SDC_NUM, u32::from(d.a)));
        push(Word(reg::PLL_SDC_DENO, u32::from(d.b)));
        push(Field(reg::PLL_PCG, field::PCG_POSTDIV, s1 << 4));
        push(Field(reg::PLL_PCG, field::PCG_CLK_SEL, 0));
        push(Field(reg::PLL_PCG, field::PCG_CLK_EN, field::PCG_CLK_EN));
        PlanWrites { w, len }
    }

    /// The rate the dividers give, in mHz (fact 9's frequency form).
    pub fn achieved_mhz(&self) -> u64 {
        rate_mhz(u32::from(self.m), u32::from(self.s), self.sdm)
    }

    /// Achieved minus asked for, in mHz.
    pub fn error_mhz(&self) -> i64 {
        self.achieved_mhz() as i64 - (self.rate_hz * 1000) as i64
    }
}

/// The TMDS character rate in mHz of ROPLL dividers (fact 9), rounded.
/// Returns 0 for dividers that give no rate (S or M 0, an SDM with lc 0 or
/// N·b <= a).
pub fn rate_mhz(m: u32, s: u32, sdm: Option<Sdm>) -> u64 {
    if m == 0 || s == 0 {
        return 0;
    }
    let (m, s) = (u128::from(m), u128::from(s));
    let fref = u128::from(REF_HZ);
    // VCO = fref·m·num / den
    let (num, den) = match sdm {
        None => (1u128, 1u128),
        Some(d) => {
            let (k, lc, n, a, b) =
                (u128::from(d.k), u128::from(d.lc), u128::from(d.n), u128::from(d.a), u128::from(d.b));
            if lc == 0 || n * b <= a {
                return 0;
            }
            // Fsdc = fref·m·b / (n·b - a); VCO = fref·m ± (k/lc)·Fsdc/16
            let base = 16 * lc * (n * b - a);
            let term = k * b;
            if d.minus {
                if term >= base {
                    return 0;
                }
                (base - term, base)
            } else {
                (base + term, base)
            }
        }
    };
    let n = 1000 * fref * m * num;
    let d = den * 5 * s;
    ((n + d / 2) / d) as u64
}

const fn gcd(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        let t = a % b;
        a = b;
        b = t;
    }
    a
}

/// The fraction p/q closest to `num/den` with p <= `max_p` and q <= `max_q`
/// (continued fractions: the last convergent within the bounds or the best
/// semiconvergent after it). `num/den` must be at most `max_p`.
fn best_fraction(num: u64, den: u64, max_p: u64, max_q: u64) -> (u64, u64) {
    let g = gcd(num, den);
    let (num, den) = (num / g, den / g);
    if num <= max_p && den <= max_q {
        return (num, den);
    }
    // convergents h/k: (h0, k0) the one before (h1, k1)
    let (mut h0, mut k0, mut h1, mut k1) = (0u64, 1u64, 1u64, 0u64);
    let (mut n, mut d) = (num, den);
    while d != 0 {
        let a = n / d;
        let h2 = a * h1 + h0;
        let k2 = a * k1 + k0;
        if h2 > max_p || k2 > max_q {
            // the largest t < a that keeps t·h1 + h0 and t·k1 + k0 in bounds
            let tp = (max_p - h0).checked_div(h1).unwrap_or(a);
            let tq = (max_q - k0).checked_div(k1).unwrap_or(a);
            let t = tp.min(tq).min(a);
            let (hs, ks) = (t * h1 + h0, t * k1 + k0);
            // keep the closer of h1/k1 and hs/ks to num/den
            if k1 == 0 {
                return (hs, ks);
            }
            if ks != 0 {
                let e1 = (u128::from(h1) * u128::from(den)).abs_diff(u128::from(num) * u128::from(k1)) * u128::from(ks);
                let es = (u128::from(hs) * u128::from(den)).abs_diff(u128::from(num) * u128::from(ks)) * u128::from(k1);
                if es < e1 {
                    return (hs, ks);
                }
            }
            return (h1, k1);
        }
        (h0, k0, h1, k1) = (h1, k1, h2, k2);
        (n, d) = (d, n - a * d);
    }
    (h1, k1)
}

/// The ROPLL plan for a TMDS character rate (fact 9), 8 bits per component.
///
/// S is the largest even S (or 1) that puts the VCO (5 x rate x S) in
/// 2.0-4.0 GHz. When the VCO is a multiple of 24 MHz the plan is integer.
/// Otherwise M = VCO / 24 MHz rounded up, the SDM term is subtracted, N =
/// [`SDC_N`], Fsdc is the smallest multiple of 24 MHz from 264 MHz with
/// N x Fsdc > 24 MHz x M, so a/b = (N x Fsdc - 24 MHz x M) / Fsdc exactly,
/// and k/lc is 16 x (24 MHz x M - VCO) / Fsdc, exact or the closest
/// fraction with k at most [`SDM_K_MAX`] and lc at most [`SDM_LC_MAX`]
/// ([`Plan::error_mhz`] says how close).
pub fn plan(rate_hz: u64) -> Result<Plan, Refused> {
    let no = refuse(Refusal::Rate, rate_hz.min(u64::from(u32::MAX)) as u32);
    if rate_hz == 0 || rate_hz > TMDS_MAX_HZ {
        return Err(no);
    }
    let fout = 5 * rate_hz;
    let s = (1..=16u64)
        .rev()
        .filter(|s| s.is_multiple_of(2) || *s == 1)
        .find(|s| (VCO_MIN_HZ..=VCO_MAX_HZ).contains(&(fout * s)))
        .ok_or(no)?;
    let vco = fout * s;
    let m = vco.div_ceil(REF_HZ);
    if !(20..=255).contains(&m) {
        return Err(no);
    }
    let delta = REF_HZ * m - vco;
    if delta == 0 {
        return Ok(Plan { rate_hz, m: m as u8, s: s as u8, sdm: None });
    }
    let n = u64::from(SDC_N);
    // Fsdc = REF_HZ·q, q the smallest from 11 (264 MHz) with n·q > m
    let q = (m / n + 1).max(FSDC_MIN_HZ / REF_HZ);
    if q * REF_HZ > FSDC_MAX_HZ {
        return Err(no);
    }
    let (a, b) = {
        let (a, b) = (n * q - m, q);
        let g = gcd(a, b);
        (a / g, b / g)
    };
    if a > u64::from(field::SDC_FRAC) || b > u64::from(field::SDC_FRAC) {
        return Err(no);
    }
    // k/lc = 16·delta / (REF_HZ·q)
    let (k, lc) = best_fraction(16 * delta, REF_HZ * q, SDM_K_MAX, SDM_LC_MAX);
    if k == 0 {
        // the SDM term rounds to nothing: an integer plan, its error stated
        return Ok(Plan { rate_hz, m: m as u8, s: s as u8, sdm: None });
    }
    let sdm = Sdm { minus: true, k: k as u8, lc: lc as u8, n: SDC_N as u8, a: a as u8, b: b as u8 };
    Ok(Plan { rate_hz, m: m as u8, s: s as u8, sdm: Some(sdm) })
}

/// The ROPLL words as read back (fact 8), for the status read-back and the
/// comparison with the dump (note, U3).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PllWords {
    pub m: u32,
    pub m_afc: u32,
    pub pdiv: u32,
    pub sdiv: u32,
    pub sdm_ctrl: u32,
    pub sdm_deno: u32,
    pub sdm_sign: u32,
    pub sdm_num: u32,
    pub sdc_n: u32,
    pub sdc_num: u32,
    pub sdc_deno: u32,
    pub pcg: u32,
}

impl PllWords {
    /// The dividers these words hold; `None` when P or the reference
    /// divider is not 1 (Linux sets neither otherwise, so their meaning is
    /// not known here).
    fn dividers(&self) -> Option<(u8, u8, Option<Sdm>)> {
        if self.pdiv != 0x11 {
            return None;
        }
        let s = (((self.sdiv & field::SDIV) >> 4) + 1) as u8;
        let sdm = (self.sdm_ctrl & field::SDM_EN != 0).then(|| Sdm {
            minus: self.sdm_sign & field::SDM_SIGN != 0,
            k: self.sdm_num as u8,
            lc: self.sdm_deno as u8,
            n: ((self.sdc_n & field::SDC_N) + 3) as u8,
            a: (self.sdc_num & field::SDC_FRAC) as u8,
            b: (self.sdc_deno & field::SDC_FRAC) as u8,
        });
        Some((self.m as u8, s, sdm))
    }

    /// The TMDS character rate these words give, in mHz; `None` when the
    /// dividers are not known or give no rate.
    pub fn rate_mhz(&self) -> Option<u64> {
        let (m, s, sdm) = self.dividers()?;
        match rate_mhz(u32::from(m), u32::from(s), sdm) {
            0 => None,
            r => Some(r),
        }
    }

    /// These words as a [`Plan`] for `rate_hz`: the way a board runs a rate
    /// with the dividers Linux chose, read from the dump (D0), instead of
    /// [`plan`]'s. For 65 MHz Linux takes its own table's row
    /// (hdptx.c:445, :981-985), whose dividers need not be [`plan`]'s.
    /// `None` when the dividers are not known, the plan is not valid
    /// ([`Plan::is_valid`]), or the words are more than 1 Hz from `rate_hz`.
    pub fn plan(&self, rate_hz: u64) -> Option<Plan> {
        let (m, s, sdm) = self.dividers()?;
        let p = Plan { rate_hz, m, s, sdm };
        (p.is_valid() && p.error_mhz().abs() <= 1000).then_some(p)
    }
}

/// Read the ROPLL words.
pub fn pll_words(m: &impl Mmio, phy: usize) -> PllWords {
    // every register here is inside the CMN range (layout test)
    let r = |n: u16| m.read32(phy + 4 * n as usize);
    PllWords {
        m: r(reg::PLL_M),
        m_afc: r(reg::PLL_M_AFC),
        pdiv: r(reg::PLL_PDIV),
        sdiv: r(reg::PLL_SDIV),
        sdm_ctrl: r(reg::PLL_SDM_CTRL),
        sdm_deno: r(reg::PLL_SDM_DENO),
        sdm_sign: r(reg::PLL_SDM_SIGN),
        sdm_num: r(reg::PLL_SDM_NUM),
        sdc_n: r(reg::PLL_SDC_N),
        sdc_num: r(reg::PLL_SDC_NUM),
        sdc_deno: r(reg::PLL_SDC_DENO),
        pcg: r(reg::PLL_PCG),
    }
}

/// Poll `STATUS` until every bit of `want` is set: a read at 0, `step`,
/// 2·`step`, ... up to and including `bound` µs. Returns the µs waited, or
/// the last `STATUS` read.
fn poll(m: &impl Mmio, host: &impl Host, grf: usize, want: u32, step: u32, bound: u32) -> Result<u32, u32> {
    let mut waited = 0;
    loop {
        let st = m.read32(grf + grf::STATUS);
        if st & want == want {
            return Ok(waited);
        }
        if waited >= bound {
            return Err(st);
        }
        host.wait_us(step);
        waited += step;
    }
}

/// What [`pll_on`] saw.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PllOn {
    /// `STATUS` had `PLL_LOCK_DONE` before stage 1: something else left
    /// the PLL running (Linux warns, hdptx.c:1116-1117; note U4).
    pub was_locked: bool,
    /// µs until `PHY_CLK_RDY`.
    pub clk_rdy_us: u32,
}

/// Stages 1-3 (facts 4-6): the PLL at `plan`'s rate with `image`'s CMN
/// words. The PHY's clocks must be on and its APB reset released (D4).
/// The image and the plan ([`Plan::is_valid`]) are checked before anything
/// is touched.
/// On a timeout the PLL is left as it stands (Linux does the same,
/// hdptx.c:1038-1042, :1122-1124): [`power_off`] or the next [`pll_on`]
/// starts over.
pub fn pll_on(m: &impl Mmio, host: &impl Host, b: &Bases, plan: &Plan, image: &Image) -> Result<PllOn, Refused> {
    image.check()?;
    if !plan.is_valid() {
        return Err(refuse(Refusal::Rate, plan.rate_hz.min(u64::from(u32::MAX)) as u32));
    }
    let was_locked = m.read32(b.grf + grf::STATUS) & grf::PLL_LOCK_DONE != 0;

    // stage 1
    host.reset(PhyReset::Apb, true);
    host.wait_us(APB_PULSE_US);
    host.reset(PhyReset::Apb, false);
    host.reset(PhyReset::Lane, true);
    host.reset(PhyReset::Cmn, true);
    host.reset(PhyReset::Init, true);
    m.write32(b.grf + grf::CON0, CON0_ALL_OFF);

    // stage 2
    write_all(m, b.phy, image.cmn)?;
    for w in plan.writes().as_slice() {
        match *w {
            PlanWrite::Word(n, v) => write(m, b.phy, n, v)?,
            PlanWrite::Field(n, mask, v) => modify(m, b.phy, n, mask, v)?,
        }
    }

    // stage 3
    m.write32(b.grf + grf::CON0, CON0_BIAS_ON);
    host.wait_us(STAGE3_STEP_US);
    host.reset(PhyReset::Init, false);
    host.wait_us(STAGE3_STEP_US);
    m.write32(b.grf + grf::CON0, CON0_PLL_ON);
    host.wait_us(STAGE3_STEP_US);
    host.reset(PhyReset::Cmn, false);
    let clk_rdy_us = poll(m, host, b.grf, grf::PHY_CLK_RDY, CLK_RDY_STEP_US, CLK_RDY_BOUND_US)
        .map_err(|st| refuse(Refusal::ClkRdyTimeout, st))?;
    Ok(PllOn { was_locked, clk_rdy_us })
}

/// Stage 4 (fact 7): HDMI mode, `image`'s SB, lane-top and lane words, the
/// lane reset released, then the lock poll. Returns the µs until
/// `PHY_RDY` and `PLL_LOCK_DONE`. Runs after [`pll_on`]; the image is
/// checked again before anything is touched.
pub fn lanes_on(m: &impl Mmio, host: &impl Host, b: &Bases, image: &Image) -> Result<u32, Refused> {
    image.check()?;
    m.write32(b.grf + grf::CON0, CON0_MODE_HDMI);
    write_all(m, b.phy, image.sb)?;
    write(m, b.phy, reg::LANE_TOP_MODE, LANE_TOP_MODE_TMDS)?;
    write_all(m, b.phy, image.lane_top)?;
    write(m, b.phy, reg::LANE_TOP_BUS, LANE_TOP_BUS_TMDS)?;
    write(m, b.phy, reg::LANE_TOP_LANES, LANE_TOP_ALL_LANES)?;
    write_all(m, b.phy, image.lanes)?;
    host.reset(PhyReset::Lane, false);
    m.write32(b.grf + grf::CON0, CON0_BIAS_ON);
    poll(m, host, b.grf, grf::PHY_RDY | grf::PLL_LOCK_DONE, LOCK_STEP_US, LOCK_BOUND_US)
        .map_err(|st| refuse(Refusal::LockTimeout, st))
}

/// What [`power_on`] saw.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PowerOn {
    pub pll: PllOn,
    /// µs until `PHY_RDY` and `PLL_LOCK_DONE`.
    pub lock_us: u32,
}

/// [`pll_on`] then [`lanes_on`], with nothing between them. Linux runs the
/// two at different times (fact 6, 7); the display chain (D7) may do the
/// same, for example to move DCLK_VOP0 onto the PHY's clock in between.
pub fn power_on(m: &impl Mmio, host: &impl Host, b: &Bases, plan: &Plan, image: &Image) -> Result<PowerOn, Refused> {
    let pll = pll_on(m, host, b, plan, image)?;
    let lock_us = lanes_on(m, host, b, image)?;
    Ok(PowerOn { pll, lock_us })
}

/// The register writes of power-off (fact 10), in order.
pub const OFF_WRITES: [(u16, u32); 7] = [
    (reg::LANE0, 0x82),
    (reg::SB_VREG, 0xc1),
    (reg::SB_VREG_SEL, 0x01),
    (reg::LANE0_TX_DRV, 0x80),
    (reg::LANE0_TX_DRV + 0x100, 0x80),
    (reg::LANE0_TX_DRV + 0x200, 0x80),
    (reg::LANE0_TX_DRV + 0x300, 0x80),
];

/// Power-off (fact 10), as Linux does it: only when `STATUS` shows
/// `PLL_LOCK_DONE`. Returns whether it ran. The PHY's clocks must be on.
pub fn power_off(m: &impl Mmio, host: &impl Host, b: &Bases) -> bool {
    if m.read32(b.grf + grf::STATUS) & grf::PLL_LOCK_DONE == 0 {
        return false;
    }
    host.reset(PhyReset::Apb, true);
    host.wait_us(APB_PULSE_US);
    host.reset(PhyReset::Apb, false);
    for (n, v) in OFF_WRITES {
        m.write32(b.phy + 4 * n as usize, v);
    }
    host.reset(PhyReset::Lane, true);
    host.reset(PhyReset::Cmn, true);
    host.reset(PhyReset::Init, true);
    m.write32(b.grf + grf::CON0, CON0_ALL_OFF);
    true
}

/// The PHY's part of the note's section 5 read-backs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Status {
    pub con0: u32,
    pub status: u32,
    pub pll: PllWords,
}

impl Status {
    pub const fn pll_locked(&self) -> bool {
        self.status & grf::PLL_LOCK_DONE != 0
    }
    pub const fn clk_ready(&self) -> bool {
        self.status & grf::PHY_CLK_RDY != 0
    }
    pub const fn phy_ready(&self) -> bool {
        self.status & grf::PHY_RDY != 0
    }
    /// Locked, clock ready and lanes ready: the note's "PHY PLL" row.
    pub const fn up(&self) -> bool {
        let all = grf::PLL_LOCK_DONE | grf::PHY_CLK_RDY | grf::PHY_RDY;
        self.status & all == all
    }
    /// HDMI mode with bias, band gap and PLL on.
    pub const fn hdmi_on(&self) -> bool {
        let on = grf::PLL_EN | grf::BIAS_EN | grf::BGR_EN;
        self.con0 & (on | grf::MODE_SEL) == on
    }
}

/// Read the status, read only. The ROPLL words need the APB clock and
/// reset released (D4).
pub fn status(m: &impl Mmio, b: &Bases) -> Status {
    Status { con0: m.read32(b.grf + grf::CON0), status: m.read32(b.grf + grf::STATUS), pll: pll_words(m, b.phy) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};
    use std::collections::BTreeMap;

    // The model's own addresses, restated from the sources (not shared
    // with the driver, so a wrong constant cannot agree with itself).
    const PHY: usize = 0xfed6_0000;
    const GRF: usize = 0xfd5e_0000;
    const B: Bases = Bases { phy: PHY, grf: GRF };

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Op {
        /// PHY register (number), value
        R(u16, u32),
        W(u16, u32),
        GrfR(usize),
        GrfW(usize, u32),
        Rst(PhyReset, bool),
        Wait(u32),
    }
    use Op::*;

    /// The PHY register file, its GRF and its four resets. It fails the test
    /// on a PHY access outside Linux's ranges, unaligned or past the window,
    /// with the APB reset asserted, a CMN-image or ROPLL write outside the
    /// reset window, a lane write with the lane reset released, and a GRF
    /// write with a value bit outside its mask. `STATUS` follows the state:
    /// clock ready once init and cmn are released with bias, band gap and
    /// PLL on (after `clk_after` reads), lane ready and locked once lane is
    /// released too (after `lock_after` reads); `None` never.
    struct Soc {
        phy: RefCell<BTreeMap<u16, u32>>,
        grf: RefCell<BTreeMap<usize, u32>>,
        asserted: RefCell<[bool; 4]>,
        ops: RefCell<Vec<Op>>,
        clk_after: Option<u32>,
        lock_after: Option<u32>,
        clk_reads: Cell<u32>,
        lock_reads: Cell<u32>,
        /// `STATUS` bits forced on (a PLL some earlier stage left locked).
        stale: u32,
    }

    fn idx(r: PhyReset) -> usize {
        r as usize
    }

    impl Soc {
        fn new() -> Soc {
            Soc {
                phy: RefCell::new(BTreeMap::new()),
                grf: RefCell::new(BTreeMap::new()),
                // probe left apb, cmn, init released; lane asserted
                asserted: RefCell::new([false, false, false, true]),
                ops: RefCell::new(Vec::new()),
                clk_after: Some(0),
                lock_after: Some(0),
                clk_reads: Cell::new(0),
                lock_reads: Cell::new(0),
                stale: 0,
            }
        }
        fn get(&self, n: u16) -> u32 {
            *self.phy.borrow().get(&n).unwrap_or(&0)
        }
        fn con0(&self) -> u32 {
            *self.grf.borrow().get(&0).unwrap_or(&0)
        }
        fn ops(&self) -> Vec<Op> {
            self.ops.borrow().clone()
        }
        fn phy_writes(&self) -> Vec<(u16, u32)> {
            self.ops().iter().filter_map(|o| if let W(n, v) = *o { Some((n, v)) } else { None }).collect()
        }
        fn waited(&self) -> u32 {
            self.ops().iter().map(|o| if let Wait(us) = *o { us } else { 0 }).sum()
        }
        fn rst(&self, r: PhyReset) -> bool {
            self.asserted.borrow()[idx(r)]
        }
        fn reg_of(a: usize) -> u16 {
            assert_eq!(a % 4, 0, "unaligned {a:#x}");
            let n = (a - PHY) / 4;
            let ok = [
                (0x000, 0x0a7),
                (0x100, 0x129),
                (0x200, 0x229),
                (0x300, 0x32d),
                (0x400, 0x42d),
                (0x500, 0x52d),
                (0x600, 0x62d),
            ]
            .iter()
            .any(|&(lo, hi)| (lo..=hi).contains(&n));
            assert!(ok, "PHY register {n:#x} outside Linux's ranges");
            n as u16
        }
        fn status(&self) -> u32 {
            let c = self.con0();
            let on = c & 0xe0 == 0xe0;
            let mut st = self.stale;
            let clk = on && !self.rst(PhyReset::Init) && !self.rst(PhyReset::Cmn);
            if clk {
                let k = self.clk_reads.get();
                self.clk_reads.set(k + 1);
                if self.clk_after.is_some_and(|a| k >= a) {
                    st |= 0x4;
                }
            }
            if st & 0x4 != 0 && !self.rst(PhyReset::Lane) {
                let k = self.lock_reads.get();
                self.lock_reads.set(k + 1);
                if self.lock_after.is_some_and(|a| k >= a) {
                    st |= 0xa;
                }
            }
            st
        }
    }

    impl Mmio for Soc {
        fn read32(&self, a: usize) -> u32 {
            if (GRF..GRF + 0x100).contains(&a) {
                self.ops.borrow_mut().push(GrfR(a - GRF));
                return if a - GRF == 0x80 { self.status() } else { *self.grf.borrow().get(&(a - GRF)).unwrap_or(&0) };
            }
            assert!((PHY..PHY + 0x2000).contains(&a), "outside the blocks: {a:#x}");
            let n = Soc::reg_of(a);
            assert!(!self.rst(PhyReset::Apb), "PHY read {n:#x} with apb in reset");
            let v = self.get(n);
            self.ops.borrow_mut().push(R(n, v));
            v
        }
        fn write32(&self, a: usize, v: u32) {
            if (GRF..GRF + 0x100).contains(&a) {
                let o = a - GRF;
                assert_eq!(o, 0, "GRF write to {o:#x}: only CON0 is written");
                let mask = v >> 16;
                assert_eq!(v & 0xffff & !mask, 0, "GRF write {v:#x}: a value bit outside its mask");
                self.ops.borrow_mut().push(GrfW(o, v));
                let old = self.con0();
                self.grf.borrow_mut().insert(o, (old & !mask) | (v & mask));
                return;
            }
            assert!((PHY..PHY + 0x2000).contains(&a), "outside the blocks: {a:#x}");
            let n = Soc::reg_of(a);
            assert!(!self.rst(PhyReset::Apb), "PHY write {n:#x} with apb in reset");
            self.ops.borrow_mut().push(W(n, v));
            self.phy.borrow_mut().insert(n, v);
        }
        fn read8(&self, _: usize) -> u8 {
            unreachable!("the PHY and its GRF are accessed 32 bits wide")
        }
        fn write8(&self, _: usize, _: u8) {
            unreachable!("the PHY and its GRF are accessed 32 bits wide")
        }
    }

    impl Host for Soc {
        fn reset(&self, r: PhyReset, asserted: bool) {
            self.ops.borrow_mut().push(Rst(r, asserted));
            self.asserted.borrow_mut()[idx(r)] = asserted;
        }
        fn wait_us(&self, us: u32) {
            self.ops.borrow_mut().push(Wait(us));
        }
    }

    // An invented image: test data only, no value is Linux's or the
    // board's. The real one is the D0 dump's register file.
    const CMN: [(u16, u32); 4] = [(0x08, 0x11), (0x5e, 0x0f), (0x86, 0x01), (0xa7, 0x22)];
    const SB: [(u16, u32); 2] = [(0x100, 0x33), (0x129, 0x44)];
    const LTOP: [(u16, u32); 2] = [(0x201, 0x55), (0x229, 0x66)];
    const LANES: [(u16, u32); 4] = [(0x300, 0x77), (0x32d, 0x78), (0x400, 0x79), (0x62d, 0x7a)];
    const IMG: Image = Image { cmn: &CMN, sb: &SB, lane_top: &LTOP, lanes: &LANES };

    #[test]
    fn layout_is_linux_v6_18() {
        // hdptx.c:24-33
        assert_eq!((grf::CON0, grf::STATUS), (0x00, 0x80));
        assert_eq!((grf::PLL_EN, grf::BIAS_EN, grf::BGR_EN, grf::MODE_SEL), (0x80, 0x40, 0x20, 0x01));
        assert_eq!((grf::PLL_LOCK_DONE, grf::PHY_CLK_RDY, grf::PHY_RDY, grf::SB_RDY), (8, 4, 2, 1));
        // hdptx.c:776-799 (byte offsets)
        let bytes: Vec<(usize, usize)> = RW_RANGES.iter().map(|&(lo, hi)| (4 * lo as usize, 4 * hi as usize)).collect();
        assert_eq!(
            bytes,
            vec![
                (0x0000, 0x029c),
                (0x0400, 0x04a4),
                (0x0800, 0x08a4),
                (0x0c00, 0x0cb4),
                (0x1000, 0x10b4),
                (0x1400, 0x14b4),
                (0x1800, 0x18b4)
            ]
        );
        assert!(4 * RW_RANGES[6].1 as usize + 4 <= PHY_SIZE);
        // hdptx.c:80-151
        assert_eq!(
            (reg::PLL_M, reg::PLL_M_AFC, reg::PLL_PDIV, reg::PLL_SDIV, reg::PLL_SDM_CTRL, reg::PLL_SDM_DENO),
            (0x51, 0x55, 0x59, 0x5a, 0x5e, 0x60)
        );
        assert_eq!(
            (reg::PLL_SDM_SIGN, reg::PLL_SDM_NUM, reg::PLL_SDC_N, reg::PLL_SDC_NUM, reg::PLL_SDC_DENO, reg::PLL_PCG),
            (0x64, 0x65, 0x69, 0x6c, 0x70, 0x86)
        );
        assert_eq!((field::SDM_EN, field::SDM_SIGN, field::SDC_N, field::SDC_FRAC), (0x40, 0x08, 0x07, 0x3f));
        assert_eq!((field::PCG_POSTDIV, field::PCG_CLK_SEL, field::PCG_CLK_EN), (0xf0, 0x0e, 0x01));
        // hdptx.c:1049, :1059-1060, :884-890
        assert_eq!((reg::LANE_TOP_MODE, reg::LANE_TOP_BUS, reg::LANE_TOP_LANES), (0x200, 0x206, 0x207));
        assert_eq!((LANE_TOP_MODE_TMDS, LANE_TOP_BUS_TMDS, LANE_TOP_ALL_LANES), (0x06, 0x07, 0x0f));
        assert_eq!(
            OFF_WRITES,
            [(0x300, 0x82), (0x10f, 0xc1), (0x110, 0x01), (0x301, 0x80), (0x401, 0x80), (0x501, 0x80), (0x601, 0x80)]
        );
        // hdptx.c:809, :854-865, :831-834; :323
        assert_eq!((APB_PULSE_US, STAGE3_STEP_US), (20, 10));
        assert_eq!((CLK_RDY_STEP_US, CLK_RDY_BOUND_US, LOCK_STEP_US, LOCK_BOUND_US), (20, 400, 100, 5000));
        assert_eq!(TMDS_MAX_HZ, 340_000_000);
        // dtsi:724-727, :3118-3120; extra:110-113, :607-609
        assert_eq!(
            (PHY0_BASE, PHY1_BASE, PHY_SIZE, GRF0_BASE, GRF1_BASE),
            (0xfed6_0000, 0xfed7_0000, 0x2000, 0xfd5e_0000, 0xfd5e_4000)
        );
        // every ROPLL word is a CMN register (pll_words reads them unchecked)
        let ws = Plan { rate_hz: 1, m: 20, s: 1, sdm: None }.writes();
        for w in ws.as_slice() {
            let (PlanWrite::Word(n, _) | PlanWrite::Field(n, _, _)) = *w;
            assert!(n <= 0xa7, "{n:#x}");
        }
    }

    #[test]
    fn con0_words_are_their_bits_under_their_own_mask() {
        assert_eq!(CON0_ALL_OFF, 0x00e0_0000);
        assert_eq!(CON0_BIAS_ON, 0x0060_0060);
        assert_eq!(CON0_PLL_ON, 0x0080_0080);
        assert_eq!(CON0_MODE_HDMI, 0x0001_0000);
    }

    #[test]
    fn an_access_outside_the_ranges_is_refused_with_nothing_written() {
        for n in [0x0a8u16, 0x0ff, 0x12a, 0x1ff, 0x22a, 0x2ff, 0x32e, 0x3ff, 0x62e, 0x7ff] {
            let s = Soc::new();
            assert_eq!(write(&s, PHY, n, 1), Err(Refused { why: Refusal::OutOfRange, found: n as u32 }));
            assert_eq!(read(&s, PHY, n), Err(Refused { why: Refusal::OutOfRange, found: n as u32 }));
            assert!(s.ops().is_empty(), "{n:#x}: {:?}", s.ops());
        }
        for n in [0x000u16, 0x0a7, 0x100, 0x129, 0x200, 0x229, 0x300, 0x32d, 0x600, 0x62d] {
            let s = Soc::new();
            write(&s, PHY, n, 0x5a).unwrap();
            assert_eq!(read(&s, PHY, n), Ok(0x5a));
        }
    }

    #[test]
    fn a_bad_image_is_refused_before_any_access() {
        let p = plan(65_000_000).unwrap();
        let cases: [(Image, Refusal, u16); 8] = [
            (Image { cmn: &[(0x0a8, 0)], ..IMG }, Refusal::ImageSection, 0x0a8),
            (Image { cmn: &[(0x100, 0)], ..IMG }, Refusal::ImageSection, 0x100),
            (Image { sb: &[(0x200, 0)], ..IMG }, Refusal::ImageSection, 0x200),
            (Image { lanes: &[(0x32e, 0)], ..IMG }, Refusal::ImageSection, 0x32e),
            (Image { cmn: &[(0x10, 0), (0x10, 1)], ..IMG }, Refusal::ImageOrder, 0x10),
            (Image { lanes: &[(0x400, 0), (0x300, 1)], ..IMG }, Refusal::ImageOrder, 0x300),
            (Image { cmn: &[(0x51, 0)], ..IMG }, Refusal::ImageOwnWord, 0x51),
            (Image { lane_top: &[(0x206, 0)], ..IMG }, Refusal::ImageOwnWord, 0x206),
        ];
        for (img, why, n) in cases {
            assert_eq!(img.check(), Err(Refused { why, found: n as u32 }));
            let s = Soc::new();
            assert_eq!(power_on(&s, &s, &B, &p, &img), Err(Refused { why, found: n as u32 }));
            assert!(s.ops().is_empty(), "{:?}", s.ops());
            let s = Soc::new();
            assert_eq!(lanes_on(&s, &s, &B, &img), Err(Refused { why, found: n as u32 }));
            assert!(s.ops().is_empty(), "{:?}", s.ops());
        }
        // every plan word an image may not hold, and the three lane-top words
        for n in [0x51u16, 0x55, 0x59, 0x5a, 0x60, 0x65, 0x6c, 0x70] {
            assert_eq!(Image { cmn: &[(n, 0)], ..IMG }.check().unwrap_err().why, Refusal::ImageOwnWord);
        }
        for n in [0x200u16, 0x207] {
            assert_eq!(Image { lane_top: &[(n, 0)], ..IMG }.check().unwrap_err().why, Refusal::ImageOwnWord);
        }
        // the read-modify-written ROPLL words may be in it
        assert_eq!(Image { cmn: &[(0x5e, 0), (0x64, 0), (0x69, 0), (0x86, 0)], ..IMG }.check(), Ok(()));
    }

    #[test]
    fn power_on_runs_linux_order() {
        let s = Soc::new();
        let p = plan(65_000_000).unwrap();
        let on = power_on(&s, &s, &B, &p, &IMG).unwrap();
        assert_eq!(on, PowerOn { pll: PllOn { was_locked: false, clk_rdy_us: 0 }, lock_us: 0 });
        let d = p.sdm.unwrap();
        let s1 = u32::from(p.s - 1);
        let mut want = vec![
            GrfR(0x80),
            // stage 1
            Rst(PhyReset::Apb, true),
            Wait(20),
            Rst(PhyReset::Apb, false),
            Rst(PhyReset::Lane, true),
            Rst(PhyReset::Cmn, true),
            Rst(PhyReset::Init, true),
            GrfW(0, 0x00e0_0000),
        ];
        // stage 2: the image's CMN words, then the ROPLL words
        want.extend(CMN.iter().map(|&(n, v)| W(n, v)));
        want.extend([
            W(0x51, u32::from(p.m)),
            W(0x55, u32::from(p.m)),
            W(0x59, 0x11),
            W(0x5a, s1 << 4),
            R(0x5e, 0x0f),
            W(0x5e, 0x4f),
            R(0x64, 0),
            W(0x64, 0x08),
            W(0x60, u32::from(d.lc)),
            W(0x65, u32::from(d.k)),
            R(0x69, 0),
            W(0x69, u32::from(d.n - 3)),
            W(0x6c, u32::from(d.a)),
            W(0x70, u32::from(d.b)),
            R(0x86, 0x01),
            W(0x86, (s1 << 4) | 1),
            R(0x86, (s1 << 4) | 1),
            W(0x86, (s1 << 4) | 1),
            R(0x86, (s1 << 4) | 1),
            W(0x86, (s1 << 4) | 1),
            // stage 3
            GrfW(0, 0x0060_0060),
            Wait(10),
            Rst(PhyReset::Init, false),
            Wait(10),
            GrfW(0, 0x0080_0080),
            Wait(10),
            Rst(PhyReset::Cmn, false),
            GrfR(0x80),
            // stage 4
            GrfW(0, 0x0001_0000),
        ]);
        want.extend(SB.iter().map(|&(n, v)| W(n, v)));
        want.push(W(0x200, 0x06));
        want.extend(LTOP.iter().map(|&(n, v)| W(n, v)));
        want.extend([W(0x206, 0x07), W(0x207, 0x0f)]);
        want.extend(LANES.iter().map(|&(n, v)| W(n, v)));
        want.extend([Rst(PhyReset::Lane, false), GrfW(0, 0x0060_0060), GrfR(0x80)]);
        assert_eq!(s.ops(), want);
        // HDMI mode, bias, band gap and PLL on; every reset released
        assert_eq!(s.con0() & 0xe1, 0xe0);
        assert!(!s.asserted.borrow().iter().any(|&a| a));
        let st = status(&s, &B);
        assert!(st.up() && st.hdmi_on() && st.pll_locked() && st.clk_ready() && st.phy_ready());
    }

    #[test]
    fn the_image_and_the_plan_are_written_inside_the_reset_window() {
        // replay the order and check the resets each PHY write saw
        let s = Soc::new();
        let p = plan(25_200_000).unwrap();
        power_on(&s, &s, &B, &p, &IMG).unwrap();
        let mut asserted = [false, false, false, true];
        for o in s.ops() {
            match o {
                Rst(r, a) => asserted[idx(r)] = a,
                W(n, _) if n <= 0xa7 => {
                    assert!(asserted[1] && asserted[2] && asserted[3], "CMN {n:#x} outside the window: {asserted:?}");
                }
                W(n, _) if n >= 0x100 => {
                    assert!(asserted[3], "{n:#x} after the lane reset was released");
                    assert!(!asserted[1] && !asserted[2], "{n:#x} before the PLL was out of reset");
                }
                _ => {}
            }
        }
    }

    #[test]
    fn the_end_state_holds_the_image_and_the_plan() {
        // the D7 golden test in small: every image word reads back as given,
        // except the bits the plan read-modify-writes
        let s = Soc::new();
        let p = plan(65_000_000).unwrap();
        power_on(&s, &s, &B, &p, &IMG).unwrap();
        for &(n, v) in CMN.iter().chain(&SB).chain(&LTOP).chain(&LANES) {
            let mask = match n {
                0x5e => !0x40,
                0x86 => !0xff,
                _ => !0,
            };
            assert_eq!(s.get(n) & mask, v & mask, "{n:#x}");
        }
        let w = status(&s, &B).pll;
        assert_eq!(w.rate_mhz(), Some(p.achieved_mhz()));
        assert_eq!(w.m, u32::from(p.m));
        assert_eq!(w.m_afc, u32::from(p.m));
    }

    #[test]
    fn no_clock_ready_is_a_named_timeout_and_the_lanes_are_not_touched() {
        let s = Soc { clk_after: None, ..Soc::new() };
        let p = plan(65_000_000).unwrap();
        assert_eq!(power_on(&s, &s, &B, &p, &IMG), Err(Refused { why: Refusal::ClkRdyTimeout, found: 0 }));
        let reads = s.ops().iter().filter(|o| **o == GrfR(0x80)).count();
        assert_eq!(reads, 1 + 400 / 20 + 1, "the lock check, then a read at 0, 20, .., 400 µs");
        let polls: Vec<u32> =
            s.ops().iter().filter_map(|o| if let Wait(us) = *o { Some(us) } else { None }).skip(4).collect();
        assert_eq!(polls, vec![20; 20]);
        assert!(s.phy_writes().iter().all(|&(n, _)| n <= 0xa7), "a lane write after the timeout");
        assert!(s.rst(PhyReset::Lane));
    }

    #[test]
    fn no_lock_is_a_named_timeout() {
        let s = Soc { lock_after: None, ..Soc::new() };
        let p = plan(148_500_000).unwrap();
        // the last STATUS read carries clock ready only
        assert_eq!(power_on(&s, &s, &B, &p, &IMG), Err(Refused { why: Refusal::LockTimeout, found: 0x4 }));
        assert_eq!(s.lock_reads.get(), 5000 / 100 + 1);
        assert_eq!(s.waited(), 20 + 3 * 10 + 5000);
    }

    #[test]
    fn late_ready_and_lock_count_their_wait() {
        let s = Soc { clk_after: Some(3), lock_after: Some(7), ..Soc::new() };
        let p = plan(27_000_000).unwrap();
        let on = power_on(&s, &s, &B, &p, &IMG).unwrap();
        assert_eq!((on.pll.clk_rdy_us, on.lock_us), (60, 700));
        // the last step inside the bound still counts
        let s = Soc { clk_after: Some(20), lock_after: Some(50), ..Soc::new() };
        let on = power_on(&s, &s, &B, &p, &IMG).unwrap();
        assert_eq!((on.pll.clk_rdy_us, on.lock_us), (400, 5000));
    }

    #[test]
    fn a_pll_left_locked_is_reported() {
        let s = Soc { stale: 0x8, ..Soc::new() };
        let on = power_on(&s, &s, &B, &plan(65_000_000).unwrap(), &IMG).unwrap();
        assert!(on.pll.was_locked);
    }

    #[test]
    fn pll_on_then_lanes_on_equals_power_on() {
        let p = plan(74_250_000).unwrap();
        let a = Soc::new();
        power_on(&a, &a, &B, &p, &IMG).unwrap();
        let b = Soc::new();
        pll_on(&b, &b, &B, &p, &IMG).unwrap();
        lanes_on(&b, &b, &B, &IMG).unwrap();
        assert_eq!(a.ops(), b.ops());
    }

    /// The five rates of the brief: (rate, S, M, integer?, error in mHz).
    /// Integer plans reproduce the rate within 1 Hz; the fractional ones
    /// here are exact (error 0), which the fraction arithmetic makes so.
    #[test]
    fn plans_reproduce_their_rates() {
        let cases = [
            (25_200_000u64, 16u8, 84u8, true),
            (27_000_000, 16, 90, true),
            (65_000_000, 12, 163, false),
            (74_250_000, 10, 155, false),
            (148_500_000, 4, 124, false),
        ];
        for (rate, s, m, integer) in cases {
            let p = plan(rate).unwrap();
            assert_eq!((p.s, p.m, p.sdm.is_none()), (s, m, integer), "{rate}");
            let vco = 5 * rate * u64::from(s);
            assert!((2_000_000_000..=4_000_000_000).contains(&vco));
            // the rate from the plan's own dividers, recomputed here from
            // the frequency form (ref 24 MHz), not through rate_mhz
            let got_mhz: u128 = match p.sdm {
                None => 24_000_000_000u128 * u128::from(p.m) / (5 * u128::from(p.s)),
                Some(d) => {
                    let fsdc_num = 24_000_000u128 * u128::from(p.m) * u128::from(d.b);
                    let fsdc_den = u128::from(d.n) * u128::from(d.b) - u128::from(d.a);
                    // VCO·16·lc·fsdc_den = 24e6·M·16·lc·fsdc_den - k·fsdc_num
                    let den = 16 * u128::from(d.lc) * fsdc_den;
                    let vco_num = 24_000_000u128 * u128::from(p.m) * den - u128::from(d.k) * fsdc_num;
                    assert!(d.minus);
                    1000 * vco_num / (den * 5 * u128::from(p.s))
                }
            };
            assert_eq!(got_mhz as u64, p.achieved_mhz(), "{rate}");
            if integer {
                assert!(p.error_mhz().abs() <= 1000, "{rate}: {} mHz", p.error_mhz());
            } else {
                assert_eq!(p.error_mhz(), 0, "{rate}");
                let d = p.sdm.unwrap();
                let fsdc =
                    24_000_000 * u64::from(p.m) * u64::from(d.b) / (u64::from(d.n) * u64::from(d.b) - u64::from(d.a));
                assert!((264_000_000..=750_000_000).contains(&fsdc), "{rate}: Fsdc {fsdc}");
                assert!(d.a <= 0x3f && d.b <= 0x3f && d.a < d.b);
            }
        }
        // our mode (1024x768@60, 65 MHz): the fraction in full
        let d = plan(65_000_000).unwrap().sdm.unwrap();
        assert_eq!((d.minus, d.k, d.lc, d.n, d.a, d.b), (true, 8, 21, 8, 5, 21));
        // TODO(D0): compare plan(65 MHz).writes() with the dump's CMN 0x51,
        // 0x55, 0x59, 0x5a, 0x5e, 0x60, 0x64, 0x65, 0x69, 0x6c, 0x70, 0x86
        // once the dump is committed; until then no expected word is
        // written here (Linux takes 65 MHz from its table, hdptx.c:445,
        // :981-985, not from its calculator, so the two may differ).
    }

    #[test]
    fn a_rate_that_needs_an_approximation_states_its_error() {
        // 65.000001 MHz: 16·delta/Fsdc = 11999940/31500000 has no exact
        // form with k <= 127 and lc <= 255; the closest such fraction is
        // 8/21, the one of 65 MHz, so the plan makes 65 MHz and says it is
        // 1 Hz short
        let p = plan(65_000_001).unwrap();
        let d = p.sdm.unwrap();
        assert_eq!((d.k, d.lc), (8, 21));
        assert_eq!(p.error_mhz(), -1000);
        // the closest fraction within the bounds: no other lc gives a smaller error
        let fsdc = 24_000_000u64 * u64::from(d.b) * u64::from(p.m) / (u64::from(d.n) * u64::from(d.b) - u64::from(d.a));
        let delta = 24_000_000u128 * u128::from(p.m) - 5 * 65_000_001u128 * u128::from(p.s);
        let target = (16 * delta, u128::from(fsdc));
        let err = |k: u128, lc: u128| (k * target.1).abs_diff(target.0 * lc) * 1_000_000 / (lc * target.1);
        let best = err(u128::from(d.k), u128::from(d.lc));
        for lc in 1..=255u128 {
            let k = (target.0 * lc + target.1 / 2) / target.1;
            if k <= 127 {
                assert!(err(k, lc) >= best, "{k}/{lc} beats {}/{}", d.k, d.lc);
            }
        }
    }

    #[test]
    fn plan_refuses_rates_it_cannot_make() {
        for rate in [0u64, 340_000_001, 24_999_999, 1_000_000] {
            assert_eq!(plan(rate).unwrap_err().why, Refusal::Rate, "{rate}");
        }
        assert!(plan(25_000_000).is_ok());
        assert!(plan(340_000_000).is_ok());
        // above 340 MHz pll_on refuses too, before any access
        let s = Soc::new();
        let p = Plan { rate_hz: 594_000_000, m: 124, s: 1, sdm: None };
        assert_eq!(pll_on(&s, &s, &B, &p, &IMG).unwrap_err().why, Refusal::Rate);
        assert!(s.ops().is_empty());
    }
    #[test]
    fn the_sdm_numerator_stays_within_7_bits() {
        // hdptx.c:932-936 bounds k by GENMASK(6, 0) = 127 and lc by
        // GENMASK(7, 0) = 255. At 25.001 MHz the closest fraction with an
        // 8-bit k would be 192/199; within the bounds it is 55/57.
        let p = plan(25_001_000).unwrap();
        let d = p.sdm.unwrap();
        assert_eq!((p.s, p.m, d.k, d.lc), (16, 84, 55, 57));
        // no fraction within the bounds is closer to 16·delta/Fsdc
        let fsdc =
            24_000_000u128 * u128::from(p.m) * u128::from(d.b) / (u128::from(d.n) * u128::from(d.b) - u128::from(d.a));
        let delta = 24_000_000u128 * u128::from(p.m) - 5 * 25_001_000u128 * u128::from(p.s);
        let (tn, td) = (16 * delta, fsdc);
        let err = |k: u128, lc: u128| (k * td).abs_diff(tn * lc) * 1_000_000 / (lc * td);
        let best = err(u128::from(d.k), u128::from(d.lc));
        for lc in 1..=255u128 {
            let k = (tn * lc + td / 2) / td;
            if k <= 127 {
                assert!(err(k, lc) >= best, "{k}/{lc} beats {}/{}", d.k, d.lc);
            }
        }
        // every rate from 25 to 340 MHz in 100 kHz steps keeps k within 7 bits
        for rate in (25_000_000u64..=340_000_000).step_by(100_000) {
            let p = plan(rate).unwrap();
            if let Some(d) = p.sdm {
                assert!(d.k <= 127, "{rate}: k {}", d.k);
            }
        }
        // our mode is unchanged
        let d = plan(65_000_000).unwrap().sdm.unwrap();
        assert_eq!((d.k, d.lc), (8, 21));
    }

    #[test]
    fn an_integer_plan_turns_the_sdm_off() {
        let s = Soc::new();
        let p = plan(27_000_000).unwrap();
        power_on(&s, &s, &B, &p, &IMG).unwrap();
        // the image set 0x5e bits 3:0; an integer plan clears them and bit 6
        assert_eq!(s.get(0x5e), 0);
        assert_eq!((s.get(0x60), s.get(0x65), s.get(0x6c), s.get(0x70)), (0, 0, 0, 0));
        assert_eq!(s.get(0x64) & 0x08, 0);
        assert_eq!(status(&s, &B).pll.rate_mhz(), Some(27_000_000_000));
    }

    #[test]
    fn read_back_words_decode_to_the_rate() {
        for rate in [25_200_000u64, 27_000_000, 65_000_000, 74_250_000, 148_500_000] {
            let s = Soc::new();
            let p = plan(rate).unwrap();
            pll_on(&s, &s, &B, &p, &IMG).unwrap();
            assert_eq!(status(&s, &B).pll.rate_mhz(), Some(p.achieved_mhz()), "{rate}");
        }
        // P or the reference divider other than 1: not decoded
        let w = PllWords { pdiv: 0x12, m: 90, sdiv: 0xf0, ..Default::default() };
        assert_eq!(w.rate_mhz(), None);
        assert_eq!(PllWords { pdiv: 0x11, ..w }.rate_mhz(), Some(27_000_000_000));
        // a plus sign adds the term (hdptx.c's 148.5 MHz row has sign 0)
        let plus = Sdm { minus: false, k: 3, lc: 4, n: 8, a: 5, b: 16 };
        assert_eq!(rate_mhz(123, 4, Some(plus)), 148_500_000_000);
    }

    #[test]
    fn read_back_words_give_the_plan_back() {
        // pll_on writes the plan; the words read back are the same plan
        for rate in [25_200_000u64, 27_000_000, 65_000_000, 74_250_000, 148_500_000] {
            let s = Soc::new();
            let p = plan(rate).unwrap();
            pll_on(&s, &s, &B, &p, &IMG).unwrap();
            assert_eq!(status(&s, &B).pll.plan(rate), Some(p), "{rate}");
            assert_eq!(status(&s, &B).pll.plan(rate + 2), None, "{rate}: 2 Hz off");
        }
        // other dividers for 65 MHz (invented test words): M 162, S 12, a
        // plus term k/lc x Fsdc/16 with Fsdc = 24 MHz x 162 / (8 - 0/1)
        // = 486 MHz
        let w = PllWords {
            m: 162,
            pdiv: 0x11,
            sdiv: 11 << 4,
            sdm_ctrl: 0x40,
            sdm_deno: 2,
            sdm_num: 1,
            sdc_n: 5,
            sdc_num: 0,
            sdc_deno: 1,
            ..Default::default()
        };
        // 1/2: VCO 3888 + 15.1875 MHz, not 65 MHz
        assert_eq!(w.plan(65_000_000), None);
        // 16/81: VCO 3888 + 6 = 3894 MHz, 64.9 MHz
        let w = PllWords { sdm_num: 16, sdm_deno: 81, ..w };
        assert_eq!(w.rate_mhz(), Some(64_900_000_000));
        // 32/81: VCO 3888 + 12 = 3900 MHz, 65 MHz
        let w = PllWords { sdm_num: 32, sdm_deno: 81, ..w };
        let p = w.plan(65_000_000).unwrap();
        assert_eq!((p.m, p.s, p.sdm.unwrap().minus, p.error_mhz()), (162, 12, false, 0));
        let s = Soc::new();
        pll_on(&s, &s, &B, &p, &IMG).unwrap();
        assert_eq!(status(&s, &B).pll.plan(65_000_000), Some(p));
    }

    #[test]
    fn power_off_runs_linux_order_only_when_locked() {
        let s = Soc::new();
        assert!(!power_off(&s, &s, &B));
        assert_eq!(s.ops(), vec![GrfR(0x80)]);

        let s = Soc::new();
        power_on(&s, &s, &B, &plan(65_000_000).unwrap(), &IMG).unwrap();
        let n = s.ops().len();
        assert!(power_off(&s, &s, &B));
        let mut want = vec![GrfR(0x80), Rst(PhyReset::Apb, true), Wait(20), Rst(PhyReset::Apb, false)];
        want.extend(OFF_WRITES.iter().map(|&(n, v)| W(n, v)));
        want.extend([
            Rst(PhyReset::Lane, true),
            Rst(PhyReset::Cmn, true),
            Rst(PhyReset::Init, true),
            GrfW(0, 0x00e0_0000),
        ]);
        assert_eq!(s.ops()[n..].to_vec(), want);
        let st = status(&s, &B);
        assert!(!st.up() && !st.hdmi_on() && st.status == 0);
    }

    #[test]
    fn status_bits() {
        let st = Status { con0: 0xe0, status: 0xe, pll: PllWords::default() };
        assert!(st.up() && st.hdmi_on());
        assert!(!Status { status: 0x6, ..st }.up());
        assert!(!Status { con0: 0xe1, ..st }.hdmi_on(), "DP mode");
        assert!(!Status { con0: 0x60, ..st }.hdmi_on(), "PLL off");
    }
}
