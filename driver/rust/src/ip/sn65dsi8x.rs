// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! TI SN65DSI83 / SN65DSI84: a MIPI DSI to FlatLink LVDS bridge, configured
//! over I2C. One DSI input (channel A), one LVDS link (channel A) in use.
//!
//! Written from the TI data sheets and nothing else:
//!
//! - SN65DSI84, "MIPI DSI Bridge To FLATLINK LVDS, Single Channel DSI to
//!   Dual-Link LVDS Bridge", **SLLSEC2H**, September 2012, revised October 2020.
//! - SN65DSI83, "MIPI DSI Bridge to FlatLink LVDS, Single-Channel DSI to
//!   Single-Link LVDS Bridge", **SLLSEC1I**, September 2012, revised October
//!   2020.
//!
//! Both from ti.com (`/lit/ds/symlink/sn65dsi84.pdf`, `.../sn65dsi83.pdf`),
//! fetched 2026-10-01. Citations below are SLLSEC2H's; SLLSEC1I has the same
//! content under these numbers:
//!
//! | what                                        | SLLSEC2H        | SLLSEC1I        |
//! |---------------------------------------------|-----------------|-----------------|
//! | fHS_CLK 40..500 MHz; UI = half an HS clock  | 6.3             | 6.6             |
//! | t_reset >= 10 ms                            | 6.6             | 6.7             |
//! | clock source, divider, ranges, PLL_EN       | 7.3.1           | 7.3.1           |
//! | EN low >= 10 ms, DSI clock in HS before EN  | 7.4.2           | 7.4.1           |
//! | initialization sequence (Table 7-2)         | 7.4.3           | 7.4.2           |
//! | sync delay (CHA_SYNC_DELAY)                 | 7.4.7, Table 7-9| Table 7-8       |
//! | I2C protocol, read auto-increment           | 7.5.1           | 7.5.1           |
//! | CSR map                                     | Tables 7-5..7-10| Tables 7-4..7-9 |
//! | IRQ status notes                            | 8.1.3           | 8.1.3           |
//! | example register script                     | 8.2.2.1         | 8.2.2.1         |
//!
//! The register and field names here are ours. Where a field has a name in
//! the data sheet the constant carries it (`PLL_EN_STAT`, `LVDS_CLK_RANGE`,
//! ...), so a reader can find it in the CSR tables.
//!
//! **Reserved and undefined bits are never changed** ("Reserved or undefined
//! bit fields should not be modified", 7.6.1): every CSR this driver sets is
//! written as a read-modify-write of the bits it owns ([`Csr::mask`]), unless
//! it owns the whole byte. That also keeps the SN65DSI84's channel-B fields
//! (undefined on the SN65DSI83) at their reset values, whichever part is
//! fitted: the nine ID bytes are the same on both (Table 7-5 / 7-4), so the
//! driver cannot tell them apart and does not need to. Single-link LVDS
//! (`LVDS_LINK_CFG` = 1, reserved on the SN65DSI83) is the reset value of both
//! and is likewise left alone.
//!
//! Derivations, each with its source:
//!
//! 1. **DSI clock = lane bit rate / 2.** The HS clock lane is DDR: "The unit
//!    interval (UI) is one half of the period of the HS clock" (6.3, note 1),
//!    and the lanes run "up to 1 Gbps" with the clock "up to 500 MHz" (pin
//!    table). The caller gives the bit rate per lane the DSI host runs.
//! 2. **CHA_DSI_CLK_RANGE = floor(DSI clock / 5 MHz)**, 0x08 (40 MHz) to 0x64
//!    (500 MHz) (Table 7-7, 0x12: "0x08 - 40 <= frequency < 45 MHz ... 0x64 -
//!    500 MHz"); outside 40..500 MHz the input is out of specification (6.3).
//! 3. **DSI_CLK_DIVIDER**: LVDS clock = DSI clock / N, N = 1..25 (Table 7-6,
//!    0x0B, field value N - 1, with HS_CLK_SRC = 1). The driver takes the
//!    largest N whose LVDS clock is not below the pixel clock, N =
//!    floor(DSI clock / pixel clock): the LVDS link then carries every pixel
//!    the DSI host sends (7.4.7: "the LVDS output rate must be less than or
//!    equal to the DSI input rate"; in burst mode the DSI clock is a little
//!    above pixel x bpp / lanes / 2, and the integer divider absorbs it). The
//!    rule "not below the pixel clock" is our reading; datasheet citation
//!    pending (the data sheet states the divider, not how to choose it).
//! 4. **LVDS_CLK_RANGE** from the LVDS clock the divider produces (7.3.1:
//!    "must be set to the frequency range of the FlatLink LVDS output
//!    clock"), by Table 7-6's ranges: 25..37.5, ..62.5, ..87.5, ..112.5,
//!    ..137.5, ..154 MHz.
//! 5. **CHA_SYNC_DELAY = 33.** Table 7-9: "The Sync delay must be programmed
//!    to at least 32 pixel clocks to ensure proper operation." 32 is the
//!    datasheet minimum; the one clock above it is not from the data sheet:
//!    it is the value the i.MX8MP board's Linux runs (golden dump,
//!    `driver/src/display/golden-linux-2026-09-21.txt`, 0x28 = 0x21) and the
//!    value both boards have been run with. Datasheet citation pending for
//!    any reason to prefer 33 over 32.
//! 6. **LVDS output voltages: the reset values.** 0x19 = CHA_LVDS_VOCM 0
//!    (1.2 V) and CHA_LVDS_VOD_SWING 01 (Table 7-8 defaults) -- what the old
//!    drivers wrote as "LVDS_VCOM 0x05" (risk R6) is exactly the reset value
//!    of the whole register (VOD swing 01 on both channels). Written
//!    explicitly so a part that was not reset still gets them.
//! 7. **Termination and sync polarity are board facts** (devicetree): the
//!    near-end termination of channel A (CHA_LVDS_TERM: 100 or 200 ohm), the
//!    HS / VS / DE polarity driven on the LVDS side (DE/HS/VS_NEG_POLARITY),
//!    24 bpp (CHA_24BPP_MODE) and format 1 (CHA_24BPP_FORMAT1, "JEIDA").
//!
//! The sequence is Table 7-2's ([`Bridge::start`], [`Bridge::check_link`]).
//! What it adds, and why:
//!
//! - the ID read is retried for a second (`ID_TRIES` x `ID_RETRY_MS`): the
//!   Verdin display adapters sequence their own power after EN, measured on
//!   the AM62P (`bridge id ... (after 50 ms)`); the data sheet's t_en is 1 ms
//!   (6.6). Not a datasheet fact; a board fact.
//! - the PLL is disabled before 0x0A/0x0B are written (Table 7-6, 0x0D: "The
//!   PLL should be disabled before changing any of the settings in CSR 0x0A
//!   and CSR 0x0B").
//! - after PLL_EN the driver polls PLL_EN_STAT (0x0A.7) before the 10 ms wait
//!   of Init seq 6; the data sheet says PLL_EN_STAT means "PLL enabled" and
//!   asks for 3 ms more for lock (Table 7-6), which the 10 ms covers.
//!   "BRIDGE_PLL_LOCK" in the display health word is this bit.
//! - the written CSRs are read back before the PLL is enabled (Init seq 9
//!   asks for a read-back; done early, while a wrong value can still be
//!   reported before anything starts).

use crate::env::Env;
use crate::errno::{EINVAL, EIO, ENODEV, ETIMEDOUT};
use crate::ylog;

/// CSR addresses (7.6) and the fields this driver touches.
pub mod csr {
    /// 0x00..0x08: the ID bytes (Table 7-5), RO.
    pub const ID: u8 = 0x00;
    /// What the ID bytes read, 0x00 first (Table 7-5 lists them from 0x08
    /// down: {0x01, 0x20, 0x20, 0x20, 0x44, 0x53, 0x49, 0x38, 0x35}). The
    /// same on the SN65DSI83 and the SN65DSI84.
    pub const ID_BYTES: [u8; 9] = [0x35, 0x38, 0x49, 0x53, 0x44, 0x20, 0x20, 0x20, 0x01];
    /// How many ID bytes must match for "an SN65DSI8x answers": the eight
    /// characters; byte 0x08 (0x01 in the data sheet) is logged, not
    /// required, since the data sheet calls the range "Reserved".
    pub const ID_MATCH: usize = 8;

    /// 0x09, WO: SOFT_RESET (bit 0), self-clearing; "must be set after the
    /// CSR's are updated" (Table 7-6).
    pub const RESET: u8 = 0x09;
    pub const SOFT_RESET: u8 = 1 << 0;

    /// 0x0A: PLL status, LVDS clock range, clock source (Table 7-6).
    pub const CLK_SRC: u8 = 0x0a;
    pub const PLL_EN_STAT: u8 = 1 << 7; // RO
    pub const LVDS_CLK_RANGE_SHIFT: u8 = 1; // bits 3:1
    pub const LVDS_CLK_RANGE_MASK: u8 = 0b111 << 1;
    pub const HS_CLK_SRC: u8 = 1 << 0; // 1: from the DSI channel A HS clock

    /// 0x0B: DSI_CLK_DIVIDER (7:3), REFCLK_MULTIPLIER (1:0) (Table 7-6).
    pub const CLK_DIV: u8 = 0x0b;
    pub const DSI_CLK_DIVIDER_SHIFT: u8 = 3;
    pub const DSI_CLK_DIVIDER_MASK: u8 = 0b1_1111 << 3;
    pub const REFCLK_MULTIPLIER_MASK: u8 = 0b11;

    /// 0x0D: PLL_EN (bit 0) (Table 7-6).
    pub const PLL: u8 = 0x0d;
    pub const PLL_EN: u8 = 1 << 0;

    /// 0x10: CHA_DSI_LANES (4:3, 00 = four .. 11 = one), SOT_ERR_TOL_DIS (0)
    /// (Table 7-7). Bits 7 and 6:5 are reserved ("Do not write to this
    /// field"); 2:1 are not described.
    pub const DSI_LANES: u8 = 0x10;
    pub const CHA_DSI_LANES_SHIFT: u8 = 3;
    pub const CHA_DSI_LANES_MASK: u8 = 0b11 << 3;
    pub const SOT_ERR_TOL_DIS: u8 = 1 << 0;

    /// 0x11: CHA_DSI_DATA_EQ (7:6), CHA_DSI_CLK_EQ (3:2) (Table 7-7).
    pub const DSI_EQ: u8 = 0x11;
    pub const CHA_DSI_DATA_EQ_MASK: u8 = 0b11 << 6;
    pub const CHA_DSI_CLK_EQ_MASK: u8 = 0b11 << 2;

    /// 0x12: CHA_DSI_CLK_RANGE, the whole byte (Table 7-7).
    pub const DSI_CLK_RANGE: u8 = 0x12;

    /// 0x18: polarities and the 24 bpp format (Table 7-8). Bit 4
    /// (LVDS_LINK_CFG on the '84, reserved on the '83) and the channel-B bits
    /// 2 and 0 are not touched.
    pub const LVDS_FMT: u8 = 0x18;
    pub const DE_NEG_POLARITY: u8 = 1 << 7;
    pub const HS_NEG_POLARITY: u8 = 1 << 6;
    pub const VS_NEG_POLARITY: u8 = 1 << 5;
    pub const LVDS_LINK_CFG: u8 = 1 << 4; // read only for the health report
    pub const CHA_24BPP_MODE: u8 = 1 << 3;
    pub const CHA_24BPP_FORMAT1: u8 = 1 << 1;

    /// 0x19: CHA_LVDS_VOCM (6), CHA_LVDS_VOD_SWING (3:2) (Table 7-8).
    pub const LVDS_LEVELS: u8 = 0x19;
    pub const CHA_LVDS_VOCM: u8 = 1 << 6;
    pub const CHA_LVDS_VOD_SWING_SHIFT: u8 = 2;
    pub const CHA_LVDS_VOD_SWING_MASK: u8 = 0b11 << 2;

    /// 0x1A: CHA_REVERSE_LVDS (5), CHA_LVDS_TERM (1) (Table 7-8).
    pub const LVDS_PINS: u8 = 0x1a;
    pub const CHA_REVERSE_LVDS: u8 = 1 << 5;
    pub const CHA_LVDS_TERM: u8 = 1 << 1; // 1: 200 ohm, 0: 100 ohm

    /// 0x1B: CHA_LVDS_CM_ADJUST (5:4) (Table 7-8).
    pub const LVDS_CM: u8 = 0x1b;
    pub const CHA_LVDS_CM_ADJUST_MASK: u8 = 0b11 << 4;

    /// Video registers (Table 7-9): a low byte and a high byte whose low
    /// bits carry the rest of the value.
    pub const ACTIVE_LINE_LENGTH: u8 = 0x20; // 12 bits
    pub const VERTICAL_DISPLAY_SIZE: u8 = 0x24; // 12 bits, test pattern only
    pub const SYNC_DELAY: u8 = 0x28; // 12 bits
    pub const HSYNC_PULSE_WIDTH: u8 = 0x2c; // 10 bits
    pub const VSYNC_PULSE_WIDTH: u8 = 0x30; // 10 bits
    pub const HORIZONTAL_BACK_PORCH: u8 = 0x34; // 8 bits
    pub const VERTICAL_BACK_PORCH: u8 = 0x36; // 8 bits, test pattern only
    pub const HORIZONTAL_FRONT_PORCH: u8 = 0x38; // 8 bits, test pattern only
    pub const VERTICAL_FRONT_PORCH: u8 = 0x3a; // 8 bits, test pattern only
    /// 0x3C: CHA_TEST_PATTERN (bit 4).
    pub const TEST_PATTERN: u8 = 0x3c;
    pub const CHA_TEST_PATTERN: u8 = 1 << 4;

    /// 0xE5: the error status, RW1C (Table 7-10): CHA_SYNCH_ERR (7),
    /// CHA_CRC_ERR (6), CHA_UNC_ECC_ERR (5), CHA_COR_ECC_ERR (4), CHA_LLP_ERR
    /// (3), CHA_SOT_BIT_ERR (2), PLL_UNLOCK (0, set at reset).
    pub const IRQ_STAT: u8 = 0xe5;
    /// Init seq 10 writes this to clear every status bit.
    pub const IRQ_STAT_CLEAR_ALL: u8 = 0xff;
}

/// The I2C bus index of the bridge in the shim's [`Env`].
pub const BUS: u32 = 0;
/// The GPIO lines of the shim's [`Env`]: the bridge's EN pin, and the
/// display adapter's own reset where the board has one (logical: 1 =
/// asserted, the shim's devicetree flags give the electrical level).
pub const LINE_EN: u32 = 0;
pub const LINE_RESET: u32 = 1;

/// Init seq 3/4 (Table 7-2) and 7.4.2: EN low at least 10 ms, then 10 ms
/// after EN high. Also t_reset (6.6).
pub const EN_WAIT_MS: u32 = 10;
/// Table 7-2: 10 ms after PLL_EN and after SOFT_RESET, 5 ms after the video
/// starts, 1 ms between clearing the error status and reading it.
pub const PLL_WAIT_MS: u32 = 10;
pub const SOFT_RESET_WAIT_MS: u32 = 10;
pub const VIDEO_WAIT_MS: u32 = 5;
pub const IRQ_WAIT_MS: u32 = 1;
/// Polls of PLL_EN_STAT, 1 ms apart, before the PLL counts as not started.
/// Ours (the data sheet gives no bound).
pub const PLL_POLLS: u32 = 100;
/// ID read attempts and their spacing (the adapter's power sequencing; see
/// the module documentation).
pub const ID_TRIES: u32 = 20;
pub const ID_RETRY_MS: u32 = 50;

/// CHA_SYNC_DELAY (derivation 5).
pub const SYNC_DELAY_CLOCKS: u16 = 33;
/// The DSI pixel format the driver configures for: RGB888 packets, 24 bits
/// a pixel (the hosts on both boards are attached with it).
pub const DSI_BPP: u32 = 24;

const MHZ: u32 = 1_000_000;
/// fHS_CLK, 6.3.
pub const DSI_CLK_MIN_HZ: u32 = 40 * MHZ;
pub const DSI_CLK_MAX_HZ: u32 = 500 * MHZ;
/// LVDS_CLK_RANGE 000..101 (Table 7-6): 25 MHz <= clock <= 154 MHz.
pub const LVDS_CLK_MIN_HZ: u32 = 25 * MHZ;
pub const LVDS_CLK_MAX_HZ: u32 = 154 * MHZ;
/// The upper edge of LVDS_CLK_RANGE 000..100; 101 runs to 154 MHz.
const LVDS_RANGE_EDGES_HZ: [u32; 5] = [37_500_000, 62_500_000, 87_500_000, 112_500_000, 137_500_000];
/// DSI_CLK_DIVIDER: "Divide by 25" is the last defined value.
pub const DIVIDER_MAX: u32 = 25;

/// What the devicetree says about the board (all LVDS-side facts).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Board {
    /// DSI data lanes wired, 1..=4.
    pub lanes: u8,
    /// CHA_24BPP_MODE: drive the fourth LVDS data lane (24 bpp panel).
    pub lvds_24bpp: bool,
    /// CHA_24BPP_FORMAT1: format 1 (the two LSB per colour on Y3, "JEIDA");
    /// clear is format 2 ("SPWG/VESA").
    pub format1: bool,
    /// CHA_LVDS_TERM = 0: 100 ohm near-end termination; clear is 200 ohm, the
    /// reset value.
    pub term_100_ohm: bool,
    /// The polarity driven on the LVDS side: set = negative (active low).
    pub hs_neg: bool,
    pub vs_neg: bool,
    pub de_neg: bool,
    /// The node has an EN line / a display-adapter reset line.
    pub has_en: bool,
    pub has_reset: bool,
}

/// The video mode, in the terms of the CSRs (Table 7-9).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mode {
    pub pixel_clock_hz: u32,
    pub hactive: u16,
    pub vactive: u16,
    pub hsync: u16,
    pub vsync: u16,
    pub hbp: u16,
    pub vbp: u16,
    pub hfp: u16,
    pub vfp: u16,
}

/// One CSR write: `reg <- (reg & !mask) | val`. A full mask is a plain write.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Csr {
    pub reg: u8,
    pub mask: u8,
    pub val: u8,
}

/// How many CSRs [`Plan::csrs`] holds.
pub const CSR_COUNT: usize = 24;

/// Everything derived from a mode before the bus is touched.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Plan {
    pub dsi_clk_hz: u32,
    pub lvds_clk_hz: u32,
    /// The divider N (1..=25); the field holds N - 1.
    pub divider: u8,
    pub lvds_clk_range: u8,
    pub dsi_clk_range: u8,
    pub lvds_fmt: u8,
    /// Init seq 5, in address order.
    pub csrs: [Csr; CSR_COUNT],
}

fn lo(v: u16) -> u8 {
    (v & 0xff) as u8
}
fn hi(v: u16) -> u8 {
    (v >> 8) as u8
}

/// Derive the CSR image for a mode. `lane_rate_hz` is the DSI bit rate per
/// lane the host runs; 0 takes pixel clock x 24 / lanes (no burst margin).
pub fn plan(board: &Board, mode: &Mode, lane_rate_hz: u32) -> Result<Plan, i32> {
    if !(1..=4).contains(&board.lanes) || mode.pixel_clock_hz == 0 {
        return Err(-EINVAL);
    }
    // 12-, 10- and 8-bit fields (Table 7-9).
    let fits = mode.hactive != 0
        && mode.hactive < 1 << 12
        && mode.vactive < 1 << 12
        && mode.hsync < 1 << 10
        && mode.vsync < 1 << 10
        && mode.hbp <= 0xff
        && mode.vbp <= 0xff
        && mode.hfp <= 0xff
        && mode.vfp <= 0xff;
    if !fits {
        return Err(-EINVAL);
    }
    let lane_rate = if lane_rate_hz != 0 {
        u64::from(lane_rate_hz)
    } else {
        u64::from(mode.pixel_clock_hz) * u64::from(DSI_BPP) / u64::from(board.lanes)
    };
    // Derivation 1: the clock lane is DDR.
    let dsi_clk = lane_rate / 2;
    if dsi_clk < u64::from(DSI_CLK_MIN_HZ) || dsi_clk > u64::from(DSI_CLK_MAX_HZ) {
        return Err(-EINVAL);
    }
    let dsi_clk = dsi_clk as u32;
    // Derivation 2.
    let dsi_clk_range = (dsi_clk / (5 * MHZ)) as u8;
    // Derivation 3.
    let divider = dsi_clk / mode.pixel_clock_hz;
    if divider == 0 || divider > DIVIDER_MAX {
        return Err(-EINVAL);
    }
    let lvds_clk = dsi_clk / divider;
    if !(LVDS_CLK_MIN_HZ..=LVDS_CLK_MAX_HZ).contains(&lvds_clk) {
        return Err(-EINVAL);
    }
    // Derivation 4.
    let lvds_clk_range = LVDS_RANGE_EDGES_HZ.iter().take_while(|&&edge| lvds_clk >= edge).count() as u8;

    let mut fmt = 0u8;
    if board.de_neg {
        fmt |= csr::DE_NEG_POLARITY;
    }
    if board.hs_neg {
        fmt |= csr::HS_NEG_POLARITY;
    }
    if board.vs_neg {
        fmt |= csr::VS_NEG_POLARITY;
    }
    if board.lvds_24bpp {
        fmt |= csr::CHA_24BPP_MODE;
    }
    if board.format1 {
        fmt |= csr::CHA_24BPP_FORMAT1;
    }
    let fmt_mask = csr::DE_NEG_POLARITY
        | csr::HS_NEG_POLARITY
        | csr::VS_NEG_POLARITY
        | csr::CHA_24BPP_MODE
        | csr::CHA_24BPP_FORMAT1;
    // CHA_DSI_LANES: 00 four .. 11 one.
    let lanes_field = (4 - board.lanes) << csr::CHA_DSI_LANES_SHIFT;
    let term = if board.term_100_ohm { 0 } else { csr::CHA_LVDS_TERM };

    use csr::*;
    let full = |reg, val| Csr { reg, mask: 0xff, val };
    let part = |reg, mask, val| Csr { reg, mask, val };
    let csrs = [
        part(CLK_SRC, LVDS_CLK_RANGE_MASK | HS_CLK_SRC, (lvds_clk_range << LVDS_CLK_RANGE_SHIFT) | HS_CLK_SRC),
        part(
            CLK_DIV,
            DSI_CLK_DIVIDER_MASK | REFCLK_MULTIPLIER_MASK,
            ((divider - 1) as u8) << DSI_CLK_DIVIDER_SHIFT, // multiplier 00 with HS_CLK_SRC = 1
        ),
        part(DSI_LANES, CHA_DSI_LANES_MASK | SOT_ERR_TOL_DIS, lanes_field),
        part(DSI_EQ, CHA_DSI_DATA_EQ_MASK | CHA_DSI_CLK_EQ_MASK, 0),
        full(DSI_CLK_RANGE, dsi_clk_range),
        part(LVDS_FMT, fmt_mask, fmt),
        part(LVDS_LEVELS, CHA_LVDS_VOCM | CHA_LVDS_VOD_SWING_MASK, 0b01 << CHA_LVDS_VOD_SWING_SHIFT),
        part(LVDS_PINS, CHA_REVERSE_LVDS | CHA_LVDS_TERM, term),
        part(LVDS_CM, CHA_LVDS_CM_ADJUST_MASK, 0),
        full(ACTIVE_LINE_LENGTH, lo(mode.hactive)),
        part(ACTIVE_LINE_LENGTH + 1, 0x0f, hi(mode.hactive)),
        full(VERTICAL_DISPLAY_SIZE, lo(mode.vactive)),
        part(VERTICAL_DISPLAY_SIZE + 1, 0x0f, hi(mode.vactive)),
        full(SYNC_DELAY, lo(SYNC_DELAY_CLOCKS)),
        part(SYNC_DELAY + 1, 0x0f, hi(SYNC_DELAY_CLOCKS)),
        full(HSYNC_PULSE_WIDTH, lo(mode.hsync)),
        part(HSYNC_PULSE_WIDTH + 1, 0x03, hi(mode.hsync)),
        full(VSYNC_PULSE_WIDTH, lo(mode.vsync)),
        part(VSYNC_PULSE_WIDTH + 1, 0x03, hi(mode.vsync)),
        full(HORIZONTAL_BACK_PORCH, lo(mode.hbp)),
        full(VERTICAL_BACK_PORCH, lo(mode.vbp)),
        full(HORIZONTAL_FRONT_PORCH, lo(mode.hfp)),
        full(VERTICAL_FRONT_PORCH, lo(mode.vfp)),
        part(TEST_PATTERN, CHA_TEST_PATTERN, 0),
    ];
    Ok(Plan {
        dsi_clk_hz: dsi_clk,
        lvds_clk_hz: lvds_clk,
        divider: divider as u8,
        lvds_clk_range,
        dsi_clk_range,
        lvds_fmt: fmt,
        csrs,
    })
}

/// What a successful [`Bridge::start`] found and wrote.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Started {
    pub id: [u8; 9],
    pub plan: Plan,
    /// ID read attempts it took.
    pub id_tries: u32,
}

/// Health bits, read from the part ([`Bridge::health`]).
pub mod health {
    /// The eight ID characters read back (Table 7-5).
    pub const ANSWERS: u32 = 1 << 0;
    /// 0x0A.7 PLL_EN_STAT.
    pub const PLL_EN_STAT: u32 = 1 << 1;
    /// 0xE5 reads 0 (nothing latched since the last clear).
    pub const LINK_CLEAN: u32 = 1 << 2;
    /// 0x18.4 reads 1: single-link LVDS, as this driver assumes.
    pub const SINGLE_LINK: u32 = 1 << 3;
}

/// The bridge on bus [`BUS`] of an [`Env`].
pub struct Bridge<'a, E: Env> {
    env: &'a E,
}

impl<'a, E: Env> Bridge<'a, E> {
    pub fn new(env: &'a E) -> Self {
        Bridge { env }
    }

    fn read(&self, reg: u8) -> Result<u8, i32> {
        let mut b = [0u8; 1];
        self.env.i2c_write_read(BUS, &[reg], &mut b)?;
        Ok(b[0])
    }

    fn write(&self, reg: u8, val: u8) -> Result<(), i32> {
        // One byte per transaction (7.5.1 states the read auto-increment;
        // the data sheet's own example script writes one register at a time).
        self.env.i2c_write(BUS, &[reg, val])
    }

    fn update(&self, c: Csr) -> Result<(), i32> {
        if c.mask == 0xff {
            return self.write(c.reg, c.val);
        }
        let old = self.read(c.reg)?;
        self.write(c.reg, (old & !c.mask) | (c.val & c.mask))
    }

    fn read_id(&self) -> Result<[u8; 9], i32> {
        let mut id = [0u8; 9];
        self.env.i2c_write_read(BUS, &[csr::ID], &mut id)?;
        Ok(id)
    }

    fn id_matches(id: &[u8; 9]) -> bool {
        id[..csr::ID_MATCH] == csr::ID_BYTES[..csr::ID_MATCH]
    }

    /// Init seq 3 to 7 of Table 7-2, for a host whose DSI clock lane is
    /// already in HS (7.4.2: it must be before EN rises). Leaves the part
    /// configured, its PLL running and soft-reset; the caller then starts
    /// the video (Init seq 8) and calls [`Bridge::check_link`].
    pub fn start(&self, board: &Board, mode: &Mode, lane_rate_hz: u32) -> Result<Started, i32> {
        let plan = match plan(board, mode, lane_rate_hz) {
            Ok(p) => p,
            Err(e) => {
                ylog!(
                    self.env,
                    Err,
                    "sn65dsi8x: mode {}x{} at {} Hz, {} lanes at {} bit/s is outside the part",
                    mode.hactive,
                    mode.vactive,
                    mode.pixel_clock_hz,
                    board.lanes,
                    lane_rate_hz
                );
                return Err(e);
            }
        };
        if board.has_reset {
            self.env.gpio_set(LINE_RESET, false)?;
        }
        if board.has_en {
            // Init seq 3 and 4.
            self.env.gpio_set(LINE_EN, false)?;
            self.env.sleep_ms(EN_WAIT_MS);
            self.env.gpio_set(LINE_EN, true)?;
        }
        self.env.sleep_ms(EN_WAIT_MS);

        let mut tries = 0;
        let id = loop {
            tries += 1;
            match self.read_id() {
                Ok(id) => break id,
                Err(e) if tries >= ID_TRIES => {
                    ylog!(self.env, Err, "sn65dsi8x: no answer on I2C after {} tries ({})", tries, e);
                    return Err(-EIO);
                }
                Err(_) => self.env.sleep_ms(ID_RETRY_MS),
            }
        };
        if !Self::id_matches(&id) {
            ylog!(self.env, Err, "sn65dsi8x: ID {:02x?} is not an SN65DSI8x", id);
            return Err(-ENODEV);
        }
        ylog!(self.env, Inf, "sn65dsi8x: ID \"58ISD\" rev {:#04x} (attempt {})", id[8], tries);

        // Init seq 5, with the PLL off while 0x0A/0x0B change.
        self.update(Csr { reg: csr::PLL, mask: csr::PLL_EN, val: 0 })?;
        for c in plan.csrs {
            self.update(c)?;
        }
        for c in plan.csrs {
            let got = self.read(c.reg)?;
            // 0x0A.7 is PLL_EN_STAT, outside every mask.
            if got & c.mask != c.val & c.mask {
                ylog!(self.env, Err, "sn65dsi8x: CSR {:#04x} reads {:#04x}, wrote {:#04x}", c.reg, got, c.val);
                return Err(-EIO);
            }
        }

        // Init seq 6.
        self.update(Csr { reg: csr::PLL, mask: csr::PLL_EN, val: csr::PLL_EN })?;
        let mut polls = 0;
        loop {
            if self.read(csr::CLK_SRC)? & csr::PLL_EN_STAT != 0 {
                break;
            }
            polls += 1;
            if polls >= PLL_POLLS {
                let st = self.read(csr::CLK_SRC).unwrap_or(0);
                ylog!(self.env, Err, "sn65dsi8x: PLL_EN_STAT not set after {} ms (0x0a = {:#04x})", polls, st);
                let _ = self.update(Csr { reg: csr::PLL, mask: csr::PLL_EN, val: 0 });
                return Err(-ETIMEDOUT);
            }
            self.env.sleep_ms(1);
        }
        self.env.sleep_ms(PLL_WAIT_MS);

        // Init seq 7.
        self.write(csr::RESET, csr::SOFT_RESET)?;
        self.env.sleep_ms(SOFT_RESET_WAIT_MS);

        ylog!(
            self.env,
            Inf,
            "sn65dsi8x: up, DSI {} kHz (range {:#04x}), LVDS {} kHz = /{} (range {}), fmt {:#04x}",
            plan.dsi_clk_hz / 1000,
            plan.dsi_clk_range,
            plan.lvds_clk_hz / 1000,
            plan.divider,
            plan.lvds_clk_range,
            plan.lvds_fmt
        );
        Ok(Started { id, plan, id_tries: tries })
    }

    /// Init seq 8 (its wait) to 11 of Table 7-2, once the host streams
    /// video: 5 ms, write 0xFF to IRQ_STAT, 1 ms, read IRQ_STAT. Returns what
    /// is still set: 0 is a clean link. (8.1.3: bits latched while the video
    /// started before the part was configured are cleared by the 0xFF.)
    pub fn check_link(&self) -> Result<u8, i32> {
        self.env.sleep_ms(VIDEO_WAIT_MS);
        self.write(csr::IRQ_STAT, csr::IRQ_STAT_CLEAR_ALL)?;
        self.env.sleep_ms(IRQ_WAIT_MS);
        let st = self.read(csr::IRQ_STAT)?;
        if st == 0 {
            ylog!(self.env, Inf, "sn65dsi8x: link clean (IRQ_STAT 0x00)");
        } else {
            ylog!(self.env, Err, "sn65dsi8x: link errors, IRQ_STAT {:#04x}", st);
        }
        Ok(st)
    }

    /// EN low and the reset time (6.6 t_reset); the part forgets its CSRs.
    pub fn stop(&self, board: &Board) -> Result<(), i32> {
        if board.has_en {
            self.env.gpio_set(LINE_EN, false)?;
            self.env.sleep_ms(EN_WAIT_MS);
        }
        Ok(())
    }

    /// Read the part's state, writing nothing: a running bridge someone else
    /// configured can be asked too. Bits as [`health`]; a part that does not
    /// answer reports 0.
    pub fn health(&self) -> u32 {
        let mut h = 0;
        match self.read_id() {
            Ok(id) if Self::id_matches(&id) => h |= health::ANSWERS,
            _ => return 0,
        }
        if matches!(self.read(csr::CLK_SRC), Ok(v) if v & csr::PLL_EN_STAT != 0) {
            h |= health::PLL_EN_STAT;
        }
        if matches!(self.read(csr::IRQ_STAT), Ok(0)) {
            h |= health::LINK_CLEAN;
        }
        if matches!(self.read(csr::LVDS_FMT), Ok(v) if v & csr::LVDS_LINK_CFG != 0) {
            h |= health::SINGLE_LINK;
        }
        h
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{BridgeEvent as Ev, BridgeFaults, BridgeModel};

    /// The i.MX8MP Verdin + Mallow + Riverdi RVT101HVDWC00-B (the display
    /// node of samples/zephyr/gc_common/verdin_imx8mp_display.dtsi).
    const IMX8MP: (Board, Mode, u32) = (
        Board {
            lanes: 4,
            lvds_24bpp: true,
            format1: false,
            term_100_ohm: false,
            hs_neg: false,
            vs_neg: false,
            de_neg: false,
            has_en: true,
            has_reset: false,
        },
        Mode {
            pixel_clock_hz: 69_300_000,
            hactive: 1280,
            vactive: 800,
            hsync: 24,
            vsync: 8,
            hbp: 64,
            vbp: 24,
            hfp: 36,
            vfp: 8,
        },
        426_600_000, // burst-clock-frequency: the DSIM PLL's bit rate
    );

    /// The AM62P Verdin + DSI-LVDS adapter + 7" panel (the `panel` node of
    /// boards/toradex/verdin_am62p/verdin_am62p_am62p54_a53.dts).
    const AM62P: (Board, Mode, u32) = (
        Board {
            lanes: 4,
            lvds_24bpp: true,
            format1: false,
            term_100_ohm: true,
            hs_neg: true,
            vs_neg: true,
            de_neg: false,
            has_en: true,
            has_reset: true,
        },
        Mode {
            pixel_clock_hz: 51_200_000,
            hactive: 1024,
            vactive: 600,
            hsync: 5,
            vsync: 10,
            hbp: 160,
            vbp: 23,
            hfp: 160,
            vfp: 12,
        },
        0, // the shim does not know the Cadence D-PHY's rate: derived
    );

    fn start(m: &BridgeModel, cfg: (Board, Mode, u32)) -> Result<Started, i32> {
        Bridge::new(m).start(&cfg.0, &cfg.1, cfg.2)
    }

    #[test]
    fn plan_for_the_imx8mp_board() {
        let p = plan(&IMX8MP.0, &IMX8MP.1, IMX8MP.2).unwrap();
        assert_eq!(p.dsi_clk_hz, 213_300_000);
        assert_eq!(p.dsi_clk_range, 42); // 210 <= f < 215 MHz
        assert_eq!(p.divider, 3);
        assert_eq!(p.lvds_clk_hz, 71_100_000);
        assert_eq!(p.lvds_clk_range, 2); // 62.5 <= f < 87.5 MHz
        assert_eq!(p.lvds_fmt, 0x08);
    }

    #[test]
    fn plan_for_the_am62p_board() {
        let p = plan(&AM62P.0, &AM62P.1, AM62P.2).unwrap();
        assert_eq!(p.dsi_clk_hz, 153_600_000);
        assert_eq!(p.dsi_clk_range, 30);
        assert_eq!(p.divider, 3);
        assert_eq!(p.lvds_clk_range, 1);
        assert_eq!(p.lvds_fmt, 0x68);
        // The rate the Cadence D-PHY really runs (307.8125 Mbit/s, measured
        // in its log) lands in the same ranges.
        let q = plan(&AM62P.0, &AM62P.1, 307_812_500).unwrap();
        assert_eq!((q.dsi_clk_range, q.divider, q.lvds_clk_range), (30, 3, 1));
    }

    #[test]
    fn plan_edges() {
        let (b, m, _) = IMX8MP;
        // fHS_CLK 40..500 MHz.
        assert_eq!(plan(&b, &m, 79_999_998).err(), Some(-EINVAL));
        assert_eq!(plan(&b, &m, 1_000_000_002).err(), Some(-EINVAL));
        let slow = Mode { pixel_clock_hz: 26_000_000, ..m };
        let p = plan(&b, &slow, 80_000_000).unwrap();
        assert_eq!((p.dsi_clk_range, p.divider, p.lvds_clk_range), (8, 1, 1)); // 40 MHz / 1
        let fast = Mode { pixel_clock_hz: 150_000_000, ..m };
        let p = plan(&b, &fast, 900_000_000).unwrap();
        assert_eq!((p.dsi_clk_range, p.divider, p.lvds_clk_range), (90, 3, 5));
        // 500 MHz is 0x64; a divider that would put the LVDS clock over
        // 154 MHz is refused rather than run below the pixel clock
        let top = Mode { pixel_clock_hz: 125_000_000, ..m };
        assert_eq!(plan(&b, &top, 1_000_000_000).unwrap().dsi_clk_range, 0x64);
        let over = Mode { pixel_clock_hz: 154_000_000, ..m };
        assert_eq!(plan(&b, &over, 1_000_000_000).err(), Some(-EINVAL));
        // an LVDS clock under 25 MHz
        let too_slow = Mode { pixel_clock_hz: 20_000_000, ..m };
        assert_eq!(plan(&b, &too_slow, 0).err(), Some(-EINVAL));
        // range edges: 37.5 MHz is range 1
        let e = Mode { pixel_clock_hz: 37_500_000, ..m };
        assert_eq!(plan(&b, &e, 225_000_000).unwrap().lvds_clk_range, 1);
        // fields
        assert_eq!(plan(&b, &Mode { hactive: 4096, ..m }, 0).err(), Some(-EINVAL));
        assert_eq!(plan(&b, &Mode { hsync: 1024, ..m }, 0).err(), Some(-EINVAL));
        assert_eq!(plan(&b, &Mode { hbp: 256, ..m }, 0).err(), Some(-EINVAL));
        assert_eq!(plan(&Board { lanes: 5, ..b }, &m, 0).err(), Some(-EINVAL));
        // lanes: CHA_DSI_LANES 00 four .. 11 one
        for (lanes, field) in [(4u8, 0u8), (3, 1), (2, 2), (1, 3)] {
            let p = plan(&Board { lanes, ..b }, &m, 426_600_000).unwrap();
            let c = p.csrs.iter().find(|c| c.reg == csr::DSI_LANES).unwrap();
            assert_eq!(c.val, field << 3);
        }
    }

    #[test]
    fn the_sequence_is_table_7_2() {
        let m = BridgeModel::new();
        let s = start(&m, IMX8MP).unwrap();
        assert_eq!(s.id, csr::ID_BYTES);
        let ev = m.events();
        // EN low, >= 10 ms, EN high, >= 10 ms, then the first I2C access.
        assert_eq!(&ev[..4], &[Ev::Gpio(LINE_EN, false), Ev::Sleep(10), Ev::Gpio(LINE_EN, true), Ev::Sleep(10)]);
        let pos = |want: &Ev| ev.iter().position(|e| e == want).unwrap_or_else(|| panic!("{want:?} in {ev:?}"));
        let last_write_to = |reg: u8| ev.iter().rposition(|e| matches!(e, Ev::Write(r, _) if *r == reg)).unwrap();
        let pll_off = pos(&Ev::Write(csr::PLL, 0));
        let pll_on = pos(&Ev::Write(csr::PLL, 1));
        let soft_reset = pos(&Ev::Write(csr::RESET, 1));
        // the PLL is off before 0x0A / 0x0B change; every CSR before PLL_EN;
        // the soft reset after it, last.
        for c in plan(&IMX8MP.0, &IMX8MP.1, IMX8MP.2).unwrap().csrs {
            let w = last_write_to(c.reg);
            assert!(pll_off < w && w < pll_on, "CSR {:#04x} at {w}, PLL off {pll_off} on {pll_on}", c.reg);
        }
        assert!(pll_on < soft_reset);
        assert_eq!(ev.iter().rposition(|e| matches!(e, Ev::Write(..))), Some(soft_reset));
        // 10 ms after PLL_EN_STAT, 10 ms after the soft reset
        assert_eq!(ev[soft_reset - 1], Ev::Sleep(PLL_WAIT_MS));
        assert_eq!(ev.last(), Some(&Ev::Sleep(SOFT_RESET_WAIT_MS)));
        assert!(m.reg(csr::CLK_SRC) & csr::PLL_EN_STAT != 0);
        assert_eq!(m.soft_resets(), 1);
    }

    #[test]
    fn reserved_and_channel_b_bits_keep_their_reset_values() {
        let m = BridgeModel::new();
        let reset = m.image();
        start(&m, IMX8MP).unwrap();
        let after = m.image();
        let p = plan(&IMX8MP.0, &IMX8MP.1, IMX8MP.2).unwrap();
        for reg in 0..=0xffu8 {
            let owned = p.csrs.iter().find(|c| c.reg == reg).map_or(0, |c| c.mask);
            let changed = reset[reg as usize] ^ after[reg as usize];
            // PLL_EN (0x0D.0) and PLL_EN_STAT (0x0A.7) change too
            let extra = match reg {
                csr::PLL => csr::PLL_EN,
                csr::CLK_SRC => csr::PLL_EN_STAT,
                _ => 0,
            };
            assert_eq!(
                changed & !(owned | extra),
                0,
                "reg {reg:#04x}: {:#04x} -> {:#04x}",
                reset[reg as usize],
                after[reg as usize]
            );
        }
    }

    #[test]
    fn the_id_is_retried_while_the_adapter_powers_up() {
        let m = BridgeModel::with_faults(BridgeFaults { nack_reads_after_en: 3, ..Default::default() });
        let s = start(&m, AM62P).unwrap();
        assert_eq!(s.id_tries, 4);
        // the adapter's reset is released before EN rises
        let ev = m.events();
        assert_eq!(ev[0], Ev::Gpio(LINE_RESET, false));
    }

    #[test]
    fn silence_is_eio_and_nothing_is_written() {
        let m = BridgeModel::with_faults(BridgeFaults { nack_reads_after_en: u32::MAX, ..Default::default() });
        assert_eq!(start(&m, IMX8MP).err(), Some(-EIO));
        assert!(!m.events().iter().any(|e| matches!(e, Ev::Write(..))));
    }

    #[test]
    fn a_foreign_id_is_refused_before_any_write() {
        let m = BridgeModel::with_faults(BridgeFaults { foreign_id: true, ..Default::default() });
        assert_eq!(start(&m, IMX8MP).err(), Some(-ENODEV));
        assert!(!m.events().iter().any(|e| matches!(e, Ev::Write(..))));
    }

    #[test]
    fn no_pll_times_out_disables_the_pll_and_never_resets() {
        let m = BridgeModel::with_faults(BridgeFaults { pll_never_starts: true, ..Default::default() });
        assert_eq!(start(&m, IMX8MP).err(), Some(-ETIMEDOUT));
        assert_eq!(m.reg(csr::PLL) & csr::PLL_EN, 0);
        assert_eq!(m.soft_resets(), 0);
        assert!(m.lines().iter().any(|l| l.contains("PLL_EN_STAT not set")));
    }

    #[test]
    fn a_csr_that_does_not_stick_is_reported_before_the_pll() {
        let m = BridgeModel::with_faults(BridgeFaults { stuck_reg: Some(csr::DSI_CLK_RANGE), ..Default::default() });
        assert_eq!(start(&m, IMX8MP).err(), Some(-EIO));
        assert_eq!(m.reg(csr::PLL) & csr::PLL_EN, 0);
    }

    #[test]
    fn a_bad_mode_touches_nothing() {
        let m = BridgeModel::new();
        let (b, mode, _) = IMX8MP;
        assert_eq!(Bridge::new(&m).start(&b, &mode, 10).err(), Some(-EINVAL));
        assert!(m.events().is_empty());
    }

    #[test]
    fn check_link_clears_with_0xff_then_reads() {
        let m = BridgeModel::new();
        start(&m, IMX8MP).unwrap();
        m.clear_events();
        m.latch_irq(0x80 | 0x01); // a sync error and PLL_UNLOCK from before the video
        assert_eq!(Bridge::new(&m).check_link(), Ok(0));
        assert_eq!(
            m.events(),
            vec![
                Ev::Sleep(VIDEO_WAIT_MS),
                Ev::Write(csr::IRQ_STAT, 0xff),
                Ev::Sleep(IRQ_WAIT_MS),
                Ev::Read(csr::IRQ_STAT, 0)
            ]
        );
        // an error that keeps coming is reported
        m.set_irq_source(0x40);
        assert_eq!(Bridge::new(&m).check_link(), Ok(0x40));
    }

    #[test]
    fn health_reads_only() {
        let m = BridgeModel::new();
        assert_eq!(Bridge::new(&m).health(), 0); // EN low: no answer
        start(&m, IMX8MP).unwrap();
        Bridge::new(&m).check_link().unwrap();
        m.clear_events();
        let h = Bridge::new(&m).health();
        assert_eq!(h, health::ANSWERS | health::PLL_EN_STAT | health::LINK_CLEAN | health::SINGLE_LINK);
        assert!(!m.events().iter().any(|e| matches!(e, Ev::Write(..) | Ev::Gpio(..))));
    }

    #[test]
    fn stop_drops_en_for_the_reset_time() {
        let m = BridgeModel::new();
        start(&m, IMX8MP).unwrap();
        m.clear_events();
        Bridge::new(&m).stop(&IMX8MP.0).unwrap();
        assert_eq!(m.events(), vec![Ev::Gpio(LINE_EN, false), Ev::Sleep(10)]);
        assert_eq!(Bridge::new(&m).health(), 0);
    }

    /// The golden comparison: the register image the driver leaves for the
    /// i.MX8MP board's mode against `i2cdump -f -y 1 0x2c` of the same
    /// bridge under the board's Linux with the panel on
    /// (driver/src/display/golden-linux-2026-09-21.txt, "SN65DSI83 i2c-1
    /// 0x2c"). The model starts from the data sheet's reset values.
    #[test]
    fn imx8mp_image_matches_the_linux_golden_dump() {
        let golden = crate::model::golden_imx8mp_bridge();
        let m = BridgeModel::new();
        start(&m, IMX8MP).unwrap();
        Bridge::new(&m).check_link().unwrap();
        let ours = m.image();
        // Each expected difference, and why.
        const ALLOWED: &[(u8, &str)] = &[
            // Addresses 0xF0..0xFF are outside every CSR table of the data
            // sheet (Tables 7-5..7-10 end at 0xE5): the part's own, never
            // read or written by this driver, and the model holds 0 there.
            (0xf4, "undocumented register, not in the CSR map"),
            (0xf7, "undocumented register, not in the CSR map"),
        ];
        let mut diffs = Vec::new();
        for r in 0..=0xffusize {
            if ours[r] != golden[r] {
                diffs.push(r as u8);
                assert!(
                    ALLOWED.iter().any(|a| a.0 == r as u8),
                    "reg {r:#04x}: ours {:#04x}, Linux {:#04x}",
                    ours[r],
                    golden[r]
                );
            }
        }
        // the allow-list holds nothing that is not a difference
        assert_eq!(diffs, ALLOWED.iter().map(|a| a.0).collect::<Vec<_>>());
    }

    /// The same for the AM62P, against what the replaced C driver
    /// (drivers/display/panel_sn65dsi83_lvds.c, the board's working state
    /// up to 2026-10-01) wrote. There is no Linux dump of that bridge.
    #[test]
    fn am62p_image_matches_the_replaced_driver() {
        let m = BridgeModel::new();
        start(&m, AM62P).unwrap();
        let ours = m.image();
        // (reg, the old driver's value)
        let old: &[(u8, u8)] = &[
            (0x0a, 0x80 | (1 << 1) | 1),
            (0x0b, 2 << 3),
            (0x0d, 0x01),
            (0x10, 0x26),
            (0x11, 0x00),
            (0x12, 30),
            (0x18, 0x78),
            (0x19, 0x05),
            (0x1a, 0x00),
            (0x1b, 0x00),
            (0x20, 0x00),
            (0x21, 0x04),
            (0x24, 0x58),
            (0x25, 0x02),
            (0x28, 33),
            (0x29, 0),
            (0x2c, 5),
            (0x2d, 0),
            (0x30, 10),
            (0x31, 0),
            (0x34, 160),
            (0x36, 23),
            (0x38, 160),
            (0x3a, 12),
            (0x3c, 0),
        ];
        // Expected differences, and why.
        const ALLOWED: &[(u8, u8, &str)] = &[
            // CHB_LVDS_TERM (0x1A.0) belongs to channel B, which single-link
            // LVDS disables; on the SN65DSI83 the bit is undefined. The old
            // driver wrote the byte as 0x00; this driver sets only
            // CHA_LVDS_TERM and leaves bit 0 at its reset value (7.6.1).
            (0x1a, 0x01, "channel B termination left at reset"),
        ];
        for &(reg, v) in old {
            let want = ALLOWED.iter().find(|a| a.0 == reg).map_or(v, |a| a.1);
            assert_eq!(ours[reg as usize], want, "reg {reg:#04x}");
        }
    }
}
