// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The STM32MP257F-DK display path's SoC glue, run by our image on the A35
//! before the IP drivers (`ip::stm32_ltdc`, `ip::dw_mipi_dsi`,
//! `ip::stm32_dsi_phy`, `ip::stm32_i2c`, `ip::adv7535`): the RIF checks, the
//! SCMI requests to OP-TEE, the RCC gates and resets, the pixel-clock mux and
//! the pins. Plan #353, item soc-display-host; issues #7 and #1281.
//!
//! Every number is [`super::display_regs`]'s; every rule is
//! docs/notes/stm32mp257f-dk-display-bringup.md's (the "note"; its row
//! numbers are those of "Clocks, resets, supplies, RIF and pins: who sets
//! what", its citations `L:` ST's Linux, `O:` ST's OP-TEE, `U:` ST's U-Boot
//! at the r3.1 tags). RM0457 is unread: nothing here is confirmed against
//! the manual.
//!
//! [`power_on`], in this order, stopping at the first failure:
//! 1. **RIF** (read only): the RIFSC entries that guard the RCC registers
//!    written below, LTDC `cmn` 80, DSI `cmn` 81, I2C2 42 (`clk-stm32mp25.c`
//!    checks the same before it touches a gate, note rows 6, 7, 14). A secure
//!    entry or one filtered to another CID stops everything: no SCMI request,
//!    no write.
//! 2. **Supplies** (SCMI voltage 0x17): `CONFIG_GET` of vddcore 11, v1v8 14,
//!    v3v3 16 (always on in OP-TEE's DK file; recorded, not set, note row
//!    18); then the HDMI +5 V, LDO5, domain 21: `CONFIG_GET`, `CONFIG_SET`
//!    on (0x7), `CONFIG_GET` back (row 19). The `CONFIG_GET`s read OP-TEE's
//!    flag, not the PMIC ("The ADV7535").
//! 3. **Clocks** (SCMI clock 0x14), each `CONFIG_SET` enable, then read back
//!    by `CLOCK_ATTRIBUTES` (enabled) and `RATE_GET`: GPIOB 90, GPIOF 94
//!    (row 15), FLEXGEN_12 (I2C2 kernel clock, row 13, 100 MHz), FLEXGEN_28
//!    (DSI PHY reference, row 10, 27 MHz), FLEXGEN_27 (LTDC kernel clock,
//!    row 9) with `RATE_SET` [`Config::ltdc_rate_hz`] between the enable and
//!    the read back. Every exported clock starts disabled for the agent
//!    ("Default OFF for RIF issues", O:`scmi_clock_consumer.c:275-280`), so
//!    every one is enabled whatever U-Boot left running.
//! 4. **RCC**: I2C2, DSI, LTDC: gate on, reset asserted, released, read back
//!    (rows 6, 7, 14). `RCC_DSICFGR`'s two muxes (`DSIBLSEL`,
//!    `DSIPHYCKREFSEL`, OP-TEE's boot setting) are kept and checked unchanged.
//!    The gate comes before the reset pulse, as ST's drivers order their own
//!    probe (the clock enabled, then the reset pulsed: ltdc.c `ltdc_load`,
//!    dw-mipi-dsi's `apb` pulse with `pclk` on, L:`dw-mipi-dsi.c:1213-1238`;
//!    i2c-stm32f7's probe); the note's row 6 lists "assert reset, gate on"
//!    for the LTDC, which a gated block may not latch.
//! 5. **SYSCFG** `DISPLAYCLKCR` bits 1:0 = 2, `ck_ker_ltdc`, read back
//!    (row 8, the value Linux and U-Boot choose for a DSI sink).
//!
//! The SCMI rules (note, "SCMI request rules", from scmi-session-study):
//! enable, then `RATE_SET` only on clock 27, then read back; never a clock
//! `CONFIG_SET` disable or a voltage `CONFIG_SET` off; never `RATE_SET` on 28
//! or 12; an SCMI success is intent, the read back is the state. `call`
//! refuses a forbidden request before it is built ([`Error::Forbidden`]), so
//! no code path can send one. Every request and reply is one [`Log`] line,
//! the request before it is sent, as `power` does for the GPU.
//!
//! [`pins`]: I2C2's SCL PF2 and SDA PF0 (AF9, open drain, no bias, low
//! speed; L:`stm32mp25-pinctrl.dtsi:276-284`), the ADV7535's interrupt PB4 an
//! input without bias (read as a level, the first probe polls), its reset PB6
//! (active low, L:`stm32mp257f-dk.dts:438`) an output driven high, released.
//! [`adv_reset`] pulses PB6 as U-Boot's EV1 code does (low 1 ms, high, wait
//! 10 ms; U:`stm32mp2.c:186-203`): nothing in ST's chain drives PB6 on a DK.
//! The GPIO bank clocks are step 3's: [`pins`] runs after [`power_on`]. Every
//! pin is checked first ([`gpio::access`], read only) and a secure or
//! foreign pin stops the whole step with nothing written.
//!
//! [`rif_check`] reads the display's RIFSC entries (note rows 1 to 3) and the
//! unpaired RIMU entries 11 to 13 (row 4) and writes nothing; [`fb_check`]
//! is the framebuffer range against the DK's RISAF4 table (note,
//! "Framebuffer memory and bus-master access"), from the table, not the bus:
//! RISAF4's register page may be secure to us (the note's board item 4).
//! This module writes no RIF or RISAF register.

use super::display_regs::*;
use super::power::{Log, Status, TOKEN};
use crate::firmware::scmi::{self, Transport, MAX_MSG};
use crate::ip::stm32_gpio::{self as gpio, OType, Pull, Speed, CID_A35};
use crate::mmio::{get, Mmio};

// ---- RIFSC (O:`core/drivers/firewall/stm32_rifsc.c:25-31`, `:86-107`) ----

/// `RIFSC_RISC_PRIVCFGRx`: 0x30 + 4 x (id / 32) (`stm32_rifsc.c:26`).
pub const fn rifsc_privcfgr(id: u32) -> usize {
    0x30 + 4 * (id as usize / 32)
}
/// `RIFSC_RISC_PERx_SEMCR`: 0x104 + 8 x id (`stm32_rifsc.c:29`, `:84`).
pub const fn rifsc_semcr(id: u32) -> usize {
    0x104 + 8 * id as usize
}
/// `PERx_CIDCFGR` fields (`stm32_rifsc.c:86-95`): `CFEN` bit 0, `SEMEN`
/// bit 1, `SCID` bits 6:4, `SEMWL` bits 23:16.
pub const CIDCFGR_CFEN: u32 = 1 << 0;
pub const CIDCFGR_SEMEN: u32 = 1 << 1;
pub const CIDCFGR_SCID: u32 = 0x7 << 4;
pub const CIDCFGR_SEMWL: u32 = 0xff << 16;
/// `PERx_SEMCR`: `MUTEX` bit 0, `SEMCID` bits 6:4 (O:`stm32_rif.h:27-29`).
pub const SEMCR_MUTEX: u32 = 1 << 0;
pub const SEMCR_SEMCID: u32 = 0x7 << 4;
/// `RIFSC_RIMC_ATTRx` fields (`stm32_rifsc.c:102-107`).
pub const RIMC_CIDSEL: u32 = 1 << 2;
pub const RIMC_MCID: u32 = 0x7 << 4;
pub const RIMC_MSEC: u32 = 1 << 8;
pub const RIMC_MPRIV: u32 = 1 << 9;

/// The RIFSC entries the display needs (layer 1 only, the DSI host, I2C2).
pub const RIF_REQUIRED: [u32; 4] = [RIFSC_LTDC_CMN, RIFSC_LTDC_L1L2, RIFSC_DSI_CMN, RIFSC_I2C2];
/// Entries read and reported, not required: layer 3 and rotation (Linux
/// hides layer 3 when 120 is secure, L:`ltdc.c:2445-2465`), DSI `trig` and
/// `rdfifo` (DCS reads; the ADV7535's video mode sends none).
pub const RIF_OPTIONAL: [u32; 4] = [RIFSC_LTDC_L3, RIFSC_LTDC_ROT, RIFSC_DSI_TRIG, RIFSC_DSI_RDFIFO];
/// The entries guarding the RCC registers [`power_on`] writes.
pub const RIF_RCC: [u32; 3] = [RIFSC_LTDC_CMN, RIFSC_DSI_CMN, RIFSC_I2C2];

/// What a RIF entry says for our compartment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Ok,
    /// `SECCFGR` makes it secure; our image is non-secure.
    Secure,
    /// CID filtering gives it to another compartment, or its semaphore is
    /// held by one / does not list ours.
    ForeignCid,
}

/// One RIFSC peripheral entry as read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RifEntry {
    pub id: u32,
    pub secure: bool,
    /// `PRIVCFGR`: reported only; our image runs privileged.
    pub privileged: bool,
    pub cidcfgr: u32,
    /// Read only when `SEMEN` is set, else 0.
    pub semcr: u32,
    pub verdict: Verdict,
}

/// Read RIFSC entry `id` and judge it for non-secure compartment `cid`, as
/// OP-TEE's `stm32_rifsc_check_access` does (O:`stm32_rifsc.c:746-771`):
/// secure refused; `CFEN` clear any CID; `SEMEN` set `cid` in `SEMWL` and
/// the semaphore free or ours (not taken: reads only); else `SCID == cid`.
pub fn rifsc_entry<M: Mmio>(m: &M, rifsc: usize, id: u32, cid: u32) -> RifEntry {
    let bit = 1u32 << (id % 32);
    let secure = m.read32(rifsc + rifsc_seccfgr(id)) & bit != 0;
    let privileged = m.read32(rifsc + rifsc_privcfgr(id)) & bit != 0;
    let cidcfgr = m.read32(rifsc + rifsc_cidcfgr(id));
    let mut semcr = 0;
    let verdict = if secure {
        Verdict::Secure
    } else if cidcfgr & CIDCFGR_CFEN == 0 {
        Verdict::Ok
    } else if cidcfgr & CIDCFGR_SEMEN != 0 {
        semcr = m.read32(rifsc + rifsc_semcr(id));
        let listed = get(CIDCFGR_SEMWL, cidcfgr) >> cid & 1 != 0;
        let free = semcr & SEMCR_MUTEX == 0 || get(SEMCR_SEMCID, semcr) == cid;
        if listed && free {
            Verdict::Ok
        } else {
            Verdict::ForeignCid
        }
    } else if get(CIDCFGR_SCID, cidcfgr) == cid {
        Verdict::Ok
    } else {
        Verdict::ForeignCid
    };
    RifEntry { id, secure, privileged, cidcfgr, semcr, verdict }
}

/// What [`rif_check`] read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RifReport {
    /// [`RIF_REQUIRED`] then [`RIF_OPTIONAL`].
    pub entries: [RifEntry; 8],
    /// `RIFSC_RIMC_ATTRx` of [`RIMU_LTDC_CANDIDATES`], raw.
    pub rimc: [u32; 3],
}

impl RifReport {
    /// The first required entry refused, if any.
    pub fn refused(&self) -> Option<RifEntry> {
        self.entries[..RIF_REQUIRED.len()].iter().copied().find(|e| e.verdict != Verdict::Ok)
    }
}

/// The display's RIFSC entries and the LTDC's candidate RIMU entries, read
/// only, each logged.
pub fn rif_check<M: Mmio, L: Log>(m: &M, rifsc: usize, cid: u32, log: &mut L) -> RifReport {
    let ids: [u32; 8] = core::array::from_fn(|i| if i < 4 { RIF_REQUIRED[i] } else { RIF_OPTIONAL[i - 4] });
    let entries = ids.map(|id| {
        let e = rifsc_entry(m, rifsc, id, cid);
        log_entry(log, &e);
        e
    });
    let rimc = RIMU_LTDC_CANDIDATES.map(|n| {
        let v = m.read32(rifsc + rifsc_rimc_attr(n));
        log_fmt(
            log,
            format_args!(
                "rif rimu {} 0x{:08x} (cidsel {} mcid {} msec {} mpriv {})",
                n,
                v,
                (v & RIMC_CIDSEL != 0) as u32,
                get(RIMC_MCID, v),
                (v & RIMC_MSEC != 0) as u32,
                (v & RIMC_MPRIV != 0) as u32
            ),
        );
        v
    });
    RifReport { entries, rimc }
}

fn log_entry<L: Log>(log: &mut L, e: &RifEntry) {
    log_fmt(
        log,
        format_args!(
            "rif rifsc {} sec {} priv {} cidcfgr 0x{:08x} semcr 0x{:08x}: {:?}",
            e.id, e.secure as u32, e.privileged as u32, e.cidcfgr, e.semcr, e.verdict
        ),
    );
}

// ---- Framebuffer range (note, "Framebuffer memory and bus-master access") --

/// RISAF4 region 9, `linuxkernel1`: non-secure, read/write CID0 and CID1
/// (O:`stm32mp257f-dk-ca35tdcid-rif.dtsi:981`, area `-resmem.dtsi:122`).
pub const RISAF4_REGION9: (u64, u64) = (0x8400_0000, 0xfa80_0000);
/// RISAF4 region 10, `gpu_reserved`, the GPU carveout; the same access
/// (`-rif.dtsi:985`, `-resmem.dtsi:127`).
pub const RISAF4_REGION10: (u64, u64) = (0xfa80_0000, 0xfe80_0000);
/// Our image, `zephyr,sram`, 256 MiB at its link address (board `.dts:22-24`,
/// `:44-47`): cacheable and the image's own.
pub const IMAGE: (u64, u64) = (0xa000_0000, 0xb000_0000);
/// Base alignment the note proposes (4 KiB, "Stride and alignment"; the
/// LTDC's own rule is RM0457's, unread).
pub const FB_ALIGN: u64 = 0x1000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FbError {
    /// Size 0, or the end overflows.
    Empty,
    /// Base not [`FB_ALIGN`] aligned.
    Unaligned,
    /// Ends above 4 GiB: `LxCFBAR` is 32 bits.
    Above4G,
    /// Overlaps the image's own memory.
    Image,
    /// Overlaps the LTDC secure-layer areas at 0xfe800000 (note, "The
    /// secure layer": kept free though non-secure on this DK).
    SecureLayer,
    /// Not inside RISAF4 regions 9 and 10.
    NotAllowed,
}

/// May the LTDC scan out `[base, base + size)`? `Ok(region)`: the RISAF4
/// region (9 or 10) the base lies in. A range may span both (the same
/// access).
pub fn fb_check(base: u64, size: u64) -> Result<u32, FbError> {
    let end = base.checked_add(size).filter(|_| size != 0).ok_or(FbError::Empty)?;
    let overlaps = |r: (u64, u64)| base < r.1 && r.0 < end;
    if !base.is_multiple_of(FB_ALIGN) {
        return Err(FbError::Unaligned);
    }
    if end > 1 << 32 {
        return Err(FbError::Above4G);
    }
    if overlaps(IMAGE) {
        return Err(FbError::Image);
    }
    if overlaps((LTDC_SEC_LAYER_BASE, LTDC_SEC_ROTATION_BASE + LTDC_SEC_ROTATION_SIZE)) {
        return Err(FbError::SecureLayer);
    }
    if base < RISAF4_REGION9.0 || end > RISAF4_REGION10.1 {
        return Err(FbError::NotAllowed);
    }
    Ok(if base < RISAF4_REGION9.1 { 9 } else { 10 })
}

// ---- SCMI -----------------------------------------------------------------

/// The SCMI clocks, in the order they are enabled.
pub const CLOCKS: [u32; 5] =
    [SCMI_CLK_GPIOB, SCMI_CLK_GPIOF, SCMI_CLK_FLEXGEN_12, SCMI_CLK_FLEXGEN_28, SCMI_CLK_FLEXGEN_27];
/// The rates the path relies on, as OP-TEE's boot sets them: FLEXGEN_12
/// PLL4 / 12 (row 13; the I2C timing), FLEXGEN_28 PLL8 / 22 (row 10; the DSI
/// PHY PLL). A different `RATE_GET` stops the sequence.
pub const FLEXGEN_12_HZ: u64 = 100_000_000;
pub const FLEXGEN_28_HZ: u64 = 27_000_000;
/// The always-on supplies read, not set (row 18).
pub const SUPPLIES: [u32; 3] = [SCMI_VOLT_VDDCORE, SCMI_VOLT_V1V8, SCMI_VOLT_V3V3];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Config {
    /// The compartment our image runs as (ST's A35 non-secure: 1).
    pub cid: u32,
    /// FLEXGEN_27's rate for the mode: 74.25 MHz for 720p60 (PLL8 594 / 8),
    /// 148.5 MHz for 1080p60 (OP-TEE's boot rate). 0: no `RATE_SET`, the
    /// rate is read back only.
    pub ltdc_rate_hz: u64,
}

impl Config {
    /// The DK at 1280x720@60 (CTA-861 VIC 4, 74.25 MHz).
    pub const DK_720P: Config = Config { cid: CID_A35, ltdc_rate_hz: 74_250_000 };
    /// The DK at 1920x1080@60 (VIC 16, 148.5 MHz).
    pub const DK_1080P: Config = Config { cid: CID_A35, ltdc_rate_hz: 148_500_000 };
}

/// An SCMI step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    VoltageVersion,
    SupplyGet,
    HdmiGet,
    HdmiSet,
    HdmiCheck,
    ClockVersion,
    ClockEnable,
    ClockRateSet,
    ClockAttributes,
    ClockRateGet,
}

/// The step and the clock or voltage domain it names (0 for a version).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Step {
    pub op: Op,
    pub id: u32,
}

/// An RCC register [`power_on`] writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Block {
    I2c2,
    Dsi,
    Ltdc,
}

impl Block {
    pub const ALL: [Block; 3] = [Block::I2c2, Block::Dsi, Block::Ltdc];
    /// Its `RCC_xxxCFGR` offset.
    pub const fn cfgr(self) -> usize {
        match self {
            Block::I2c2 => RCC_I2C2CFGR,
            Block::Dsi => RCC_DSICFGR,
            Block::Ltdc => RCC_LTDCCFGR,
        }
    }
    const fn name(self) -> &'static str {
        match self {
            Block::I2c2 => "i2c2cfgr",
            Block::Dsi => "dsicfgr",
            Block::Ltdc => "ltdccfgr",
        }
    }
}

/// The pins: bank letter and number.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pin {
    pub bank: char,
    pub pin: u32,
}

/// I2C2 SCL and SDA (L:`stm32mp25-pinctrl.dtsi:276-284`, in that order).
pub const PIN_I2C2_SCL: Pin = Pin { bank: 'F', pin: 2 };
pub const PIN_I2C2_SDA: Pin = Pin { bank: 'F', pin: 0 };
/// The ADV7535's interrupt (L:`stm32mp257f-dk.dts:436-437`) and reset
/// (`:438`, active low).
pub const PIN_ADV_INT: Pin = Pin { bank: 'B', pin: 4 };
pub const PIN_ADV_RESET: Pin = Pin { bank: 'B', pin: 6 };
/// I2C2's alternate function on PF0/PF2.
pub const I2C2_AF: u32 = 9;
/// Every pin [`pins`] configures, in its order.
pub const PINS: [Pin; 4] = [PIN_I2C2_SCL, PIN_I2C2_SDA, PIN_ADV_INT, PIN_ADV_RESET];
/// PB6 held low, then the wait after its release (U:`stm32mp2.c:186-203`).
pub const ADV_RESET_LOW_US: u32 = 1_000;
pub const ADV_RESET_WAIT_US: u32 = 10_000;
/// The RCC reset held this long (dw-mipi-dsi's 10-20 us, the longest need).
pub const RCC_RESET_US: u32 = 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// A required RIFSC entry is not ours; nothing was sent or written.
    Rif {
        id: u32,
        verdict: Verdict,
    },
    Transport {
        step: Step,
        code: i32,
    },
    Refused {
        step: Step,
        status: Status,
    },
    Echo {
        step: Step,
        sent: u32,
        got: u32,
    },
    Length {
        step: Step,
        len: usize,
    },
    /// A request the SCMI rules forbid; never sent.
    Forbidden {
        step: Step,
    },
    /// A voltage domain not on, a clock not enabled (want 1, got 0), or a
    /// rate other than wanted (low 32 bits).
    Readback {
        step: Step,
        want: u32,
        got: u32,
    },
    /// An `RCC_xxxCFGR` with the gate off or the reset held after release.
    Rcc {
        block: Block,
        raw: u32,
    },
    /// `RCC_DSICFGR`'s mux bits changed.
    Mux {
        before: u32,
        after: u32,
    },
    /// `DISPLAYCLKCR` did not read back 2.
    Syscfg {
        raw: u32,
    },
    /// A pin refused by its RIF registers (or out of range).
    Pin {
        pin: Pin,
        why: gpio::Error,
    },
    /// [`adv_reset`] on a PB6 that is not an output ([`pins`] not run).
    PinMode {
        pin: Pin,
        moder: u32,
    },
}

/// What [`power_on`] found.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Powered {
    pub voltage_version: u32,
    pub clock_version: u32,
    /// `CONFIG_GET` of [`SUPPLIES`] read on.
    pub supplies_on: [bool; 3],
    /// LDO5's `CONFIG_GET` mode before and after the set.
    pub hdmi_before: u32,
    pub hdmi: u32,
    /// `RATE_GET` of [`CLOCKS`], in that order.
    pub rates: [u64; 5],
    /// [`Block::ALL`]'s registers before and after.
    pub rcc_before: [u32; 3],
    pub rcc: [u32; 3],
    pub displayclkcr: u32,
}

/// A line buffer: a prefix and up to [`MAX_MSG`] bytes in hex.
struct Line {
    buf: [u8; 32 + 2 * MAX_MSG],
    n: usize,
}

impl Line {
    fn new() -> Line {
        Line { buf: [0; 32 + 2 * MAX_MSG], n: 0 }
    }
    fn as_str(&self) -> &str {
        // Only ASCII is ever written.
        core::str::from_utf8(&self.buf[..self.n]).unwrap_or("")
    }
}

impl core::fmt::Write for Line {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let b = s.as_bytes();
        let end = (self.n + b.len()).min(self.buf.len());
        self.buf[self.n..end].copy_from_slice(&b[..end - self.n]);
        self.n = end;
        Ok(())
    }
}

fn log_fmt<L: Log>(log: &mut L, args: core::fmt::Arguments) {
    let mut l = Line::new();
    let _ = core::fmt::Write::write_fmt(&mut l, args);
    log.line(l.as_str());
}

fn log_hex<L: Log>(log: &mut L, prefix: &str, bytes: &[u8]) {
    let mut l = Line::new();
    let _ = core::fmt::Write::write_str(&mut l, prefix);
    for b in bytes {
        let _ = core::fmt::Write::write_fmt(&mut l, format_args!("{:02x}", b));
    }
    log.line(l.as_str());
}

/// The SCMI rules as a filter: a clock `CONFIG_SET` that does not enable, a
/// `RATE_SET` on any clock but FLEXGEN_27, a voltage `CONFIG_SET` other
/// than on.
pub fn forbidden(proto: u8, msg: u8, words: &[u32]) -> bool {
    let w = |i: usize| words.get(i).copied();
    match (proto, msg) {
        (scmi::PROTO_CLOCK, scmi::CLOCK_CONFIG_SET) => !w(1).is_some_and(|a| a & 3 == scmi::CLOCK_ENABLE),
        (scmi::PROTO_CLOCK, scmi::CLOCK_RATE_SET) => w(1) != Some(SCMI_CLK_FLEXGEN_27),
        (scmi::PROTO_VOLTAGE, scmi::VOLTAGE_CONFIG_SET) => w(1) != Some(scmi::VOLTAGE_CONFIG_ON),
        _ => false,
    }
}

/// One command with [`TOKEN`]; `want` return values at least on success.
/// Logs `scmi > <request>` before the transfer and `scmi < <reply>` (or
/// `scmi ! transport <code>`) after it.
fn call<T: Transport, L: Log>(
    t: &mut T,
    log: &mut L,
    step: Step,
    proto: u8,
    msg: u8,
    words: &[u32],
    want: usize,
) -> Result<[u32; 5], Error> {
    if forbidden(proto, msg, words) {
        log_fmt(log, format_args!("scmi ! forbidden request {:#x}/{:#x} {:?} not sent", proto, msg, words));
        return Err(Error::Forbidden { step });
    }
    let hdr = scmi::header(proto, msg, TOKEN);
    let mut tx = [0u8; MAX_MSG];
    // At most four payload words: always fits.
    let len = scmi::encode(&mut tx, hdr, words).map_err(|_| Error::Length { step, len: 0 })?;
    log_hex(log, "scmi > ", &tx[..len]);
    let mut rx = [0u8; MAX_MSG];
    let got = match t.transfer(&tx[..len], &mut rx) {
        Ok(n) => n,
        Err(code) => {
            log_fmt(log, format_args!("scmi ! transport {}", code));
            return Err(Error::Transport { step, code });
        }
    };
    log_hex(log, "scmi < ", &rx[..got.min(MAX_MSG)]);
    if !(4..=MAX_MSG).contains(&got) {
        return Err(Error::Length { step, len: got });
    }
    let echo = scmi::word(&rx, 0);
    if echo != hdr {
        return Err(Error::Echo { step, sent: hdr, got: echo });
    }
    if got < 8 {
        return Err(Error::Length { step, len: got });
    }
    let status = scmi::word(&rx, 1) as i32;
    if status != scmi::STATUS_SUCCESS {
        return Err(Error::Refused { step, status: Status::from_code(status) });
    }
    if (got - 8) / 4 < want {
        return Err(Error::Length { step, len: got });
    }
    let mut v = [0u32; 5];
    for (i, w) in v.iter_mut().enumerate().take(want) {
        *w = scmi::word(&rx, 2 + i);
    }
    Ok(v)
}

fn step(op: Op, id: u32) -> Step {
    Step { op, id }
}

/// Step 2 of [`power_on`]: the supplies.
fn supplies<T: Transport, L: Log>(t: &mut T, log: &mut L, p: &mut Powered) -> Result<(), Error> {
    use scmi::{PROTOCOL_VERSION, PROTO_VOLTAGE as V, VOLTAGE_CONFIG_GET as GET, VOLTAGE_CONFIG_ON as ON};
    p.voltage_version = call(t, log, step(Op::VoltageVersion, 0), V, PROTOCOL_VERSION, &[], 1)?[0];
    for (i, &d) in SUPPLIES.iter().enumerate() {
        let mode = call(t, log, step(Op::SupplyGet, d), V, GET, &[d], 1)?[0] & 0xf;
        p.supplies_on[i] = mode == ON;
        if mode != ON {
            log_fmt(log, format_args!("supply {} reads 0x{:x}, not on (OP-TEE's flag; not set)", d, mode));
        }
    }
    let h = SCMI_VOLT_V5V_HDMI;
    p.hdmi_before = call(t, log, step(Op::HdmiGet, h), V, GET, &[h], 1)?[0] & 0xf;
    call(t, log, step(Op::HdmiSet, h), V, scmi::VOLTAGE_CONFIG_SET, &[h, ON], 0)?;
    p.hdmi = call(t, log, step(Op::HdmiCheck, h), V, GET, &[h], 1)?[0] & 0xf;
    if p.hdmi != ON {
        return Err(Error::Readback { step: step(Op::HdmiCheck, h), want: ON, got: p.hdmi });
    }
    Ok(())
}

/// Step 3 of [`power_on`]: the clocks.
fn clocks<T: Transport, L: Log>(t: &mut T, log: &mut L, cfg: &Config, p: &mut Powered) -> Result<(), Error> {
    use scmi::PROTO_CLOCK as C;
    p.clock_version = call(t, log, step(Op::ClockVersion, 0), C, scmi::PROTOCOL_VERSION, &[], 1)?[0];
    for (i, &id) in CLOCKS.iter().enumerate() {
        call(t, log, step(Op::ClockEnable, id), C, scmi::CLOCK_CONFIG_SET, &[id, scmi::CLOCK_ENABLE], 0)?;
        let mut want = match id {
            SCMI_CLK_FLEXGEN_12 => Some(FLEXGEN_12_HZ),
            SCMI_CLK_FLEXGEN_28 => Some(FLEXGEN_28_HZ),
            _ => None,
        };
        if id == SCMI_CLK_FLEXGEN_27 && cfg.ltdc_rate_hz != 0 {
            let hz = cfg.ltdc_rate_hz;
            // Flags 0: synchronous, rounded by the agent.
            call(
                t,
                log,
                step(Op::ClockRateSet, id),
                C,
                scmi::CLOCK_RATE_SET,
                &[0, id, hz as u32, (hz >> 32) as u32],
                0,
            )?;
            want = Some(hz);
        }
        let attr = call(t, log, step(Op::ClockAttributes, id), C, scmi::CLOCK_ATTRIBUTES, &[id], 5)?[0];
        if attr & 1 == 0 {
            return Err(Error::Readback { step: step(Op::ClockAttributes, id), want: 1, got: attr });
        }
        let r = call(t, log, step(Op::ClockRateGet, id), C, scmi::CLOCK_RATE_GET, &[id], 2)?;
        let hz = r[0] as u64 | (r[1] as u64) << 32;
        p.rates[i] = hz;
        log_fmt(log, format_args!("clock {} on, {} Hz", id, hz));
        if let Some(w) = want.filter(|&w| w != hz) {
            return Err(Error::Readback { step: step(Op::ClockRateGet, id), want: w as u32, got: hz as u32 });
        }
    }
    Ok(())
}

/// Step 4 of [`power_on`] for one block: gate on, reset asserted, held
/// [`RCC_RESET_US`], released, read back.
fn rcc_block<M: Mmio, L: Log, D: FnMut(u32)>(
    m: &M,
    rcc: usize,
    b: Block,
    delay: &mut D,
    log: &mut L,
) -> Result<(u32, u32), Error> {
    let a = rcc + b.cfgr();
    let before = m.read32(a);
    m.modify32(a, CFGR_GATE, CFGR_GATE);
    m.modify32(a, CFGR_RESET, CFGR_RESET);
    delay(RCC_RESET_US);
    m.modify32(a, CFGR_RESET, 0);
    let after = m.read32(a);
    log_fmt(log, format_args!("rcc {} 0x{:08x} -> 0x{:08x}", b.name(), before, after));
    if after & CFGR_GATE == 0 || after & CFGR_RESET != 0 {
        return Err(Error::Rcc { block: b, raw: after });
    }
    if b == Block::Dsi {
        let mux = DSICFGR_DSIBLSEL | DSICFGR_DSIPHYCKREFSEL;
        if before & mux != after & mux {
            return Err(Error::Mux { before, after });
        }
        if after & DSICFGR_DSIPHYCKREFSEL == 0 {
            log_fmt(log, format_args!("rcc dsicfgr: PHY reference is not FLEXGEN_28 (kept as found)"));
        }
    }
    Ok((before, after))
}

/// Check the RIFSC entries in `ids` for `cid`; the first refused is the error.
fn rif_guard<M: Mmio, L: Log>(m: &M, rifsc: usize, ids: &[u32], cid: u32, log: &mut L) -> Result<(), Error> {
    for &id in ids {
        let e = rifsc_entry(m, rifsc, id, cid);
        log_entry(log, &e);
        if e.verdict != Verdict::Ok {
            return Err(Error::Rif { id, verdict: e.verdict });
        }
    }
    Ok(())
}

/// Where the SoC blocks are (mapped addresses).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bases {
    pub rcc: usize,
    pub syscfg: usize,
    pub rifsc: usize,
    pub gpiob: usize,
    pub gpiof: usize,
}

impl Bases {
    /// The physical addresses ([`super::display_regs`]).
    pub const DK: Bases =
        Bases { rcc: RCC_BASE, syscfg: SYSCFG_BASE, rifsc: RIFSC_BASE, gpiob: GPIOB_BASE, gpiof: GPIOF_BASE };

    fn bank(&self, p: Pin) -> usize {
        if p.bank == 'B' {
            self.gpiob
        } else {
            self.gpiof
        }
    }
}

/// The display path powered: RIF, supplies, clocks, RCC, SYSCFG (the module
/// text has the order). `m` reaches the RCC, SYSCFG and RIFSC; `delay` waits
/// microseconds. Stops at the first failure; `p` holds what was read so far.
pub fn power_on<T: Transport, M: Mmio, L: Log, D: FnMut(u32)>(
    scmi: &mut T,
    m: &M,
    bases: &Bases,
    cfg: &Config,
    mut delay: D,
    log: &mut L,
    p: &mut Powered,
) -> Result<(), Error> {
    *p = Powered::default();
    rif_guard(m, bases.rifsc, &RIF_RCC, cfg.cid, log)?;
    supplies(scmi, log, p)?;
    clocks(scmi, log, cfg, p)?;
    for (i, b) in Block::ALL.into_iter().enumerate() {
        let (before, after) = rcc_block(m, bases.rcc, b, &mut delay, log)?;
        p.rcc_before[i] = before;
        p.rcc[i] = after;
    }
    let a = bases.syscfg + SYSCFG_DISPLAYCLKCR;
    m.modify32(a, DISPLAYCLKCR_MASK, DISPLAYCLKCR_CK_KER_LTDC);
    p.displayclkcr = m.read32(a);
    log_fmt(log, format_args!("syscfg displayclkcr 0x{:08x}", p.displayclkcr));
    if p.displayclkcr & DISPLAYCLKCR_MASK != DISPLAYCLKCR_CK_KER_LTDC {
        return Err(Error::Syscfg { raw: p.displayclkcr });
    }
    Ok(())
}

/// What [`pins`] and [`adv_reset`] leave.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PinState {
    pub gpiob_moder: u32,
    pub gpiof_moder: u32,
    pub gpiof_afrl: u32,
    /// PB4's level (the ADV7535's interrupt, active low).
    pub adv_int: bool,
}

fn pin_state<M: Mmio>(m: &M, bases: &Bases) -> PinState {
    PinState {
        gpiob_moder: m.read32(bases.gpiob + gpio::r::MODER),
        gpiof_moder: m.read32(bases.gpiof + gpio::r::MODER),
        gpiof_afrl: m.read32(bases.gpiof + gpio::r::AFRL),
        adv_int: gpio::read(m, bases.gpiob, PIN_ADV_INT.pin),
    }
}

fn pin_guard<M: Mmio, L: Log>(m: &M, bases: &Bases, pins: &[Pin], cid: u32, log: &mut L) -> Result<(), Error> {
    for &p in pins {
        if let Err(why) = gpio::access(m, bases.bank(p), p.pin, cid) {
            log_fmt(log, format_args!("pin P{}{} refused: {:?}; nothing written", p.bank, p.pin, why));
            return Err(Error::Pin { pin: p, why });
        }
    }
    Ok(())
}

/// The display's pins (the module text): every pin checked, then I2C2's
/// SCL and SDA, PB4 an input, PB6 an output driven high. The GPIO bank
/// clocks must run ([`power_on`]).
pub fn pins<M: Mmio, L: Log>(m: &M, bases: &Bases, cid: u32, log: &mut L) -> Result<PinState, Error> {
    pin_guard(m, bases, &PINS, cid, log)?;
    let err = |pin: Pin| move |why| Error::Pin { pin, why };
    for p in [PIN_I2C2_SCL, PIN_I2C2_SDA] {
        gpio::configure_af(m, bases.gpiof, p.pin, I2C2_AF, OType::OpenDrain, Speed::Low, Pull::None, cid)
            .map_err(err(p))?;
    }
    gpio::configure_input(m, bases.gpiob, PIN_ADV_INT.pin, Pull::None, cid).map_err(err(PIN_ADV_INT))?;
    gpio::configure_output(m, bases.gpiob, PIN_ADV_RESET.pin, true, OType::PushPull, Speed::Low, Pull::None, cid)
        .map_err(err(PIN_ADV_RESET))?;
    let s = pin_state(m, bases);
    log_fmt(
        log,
        format_args!(
            "pins gpiob moder 0x{:08x} gpiof moder 0x{:08x} afrl 0x{:08x} pb4 {}",
            s.gpiob_moder, s.gpiof_moder, s.gpiof_afrl, s.adv_int as u32
        ),
    );
    Ok(s)
}

/// The ADV7535's reset pulse on PB6 (low [`ADV_RESET_LOW_US`], high, wait
/// [`ADV_RESET_WAIT_US`]); PB6 must be an output ([`pins`]).
pub fn adv_reset<M: Mmio, L: Log, D: FnMut(u32)>(
    m: &M,
    bases: &Bases,
    cid: u32,
    mut delay: D,
    log: &mut L,
) -> Result<PinState, Error> {
    let p = PIN_ADV_RESET;
    pin_guard(m, bases, &[p], cid, log)?;
    let moder = m.read32(bases.gpiob + gpio::r::MODER);
    if moder >> (2 * p.pin) & 3 != gpio::Mode::Output as u32 {
        return Err(Error::PinMode { pin: p, moder });
    }
    gpio::clear(m, bases.gpiob, p.pin);
    delay(ADV_RESET_LOW_US);
    gpio::set(m, bases.gpiob, p.pin);
    delay(ADV_RESET_WAIT_US);
    let s = pin_state(m, bases);
    log_fmt(log, format_args!("adv7535 reset pulsed on PB6; pb4 {}", s.adv_int as u32));
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model_stm32mp2::{Mp2Config, Mp2Model};
    use std::cell::RefCell;
    use std::collections::BTreeMap;

    #[derive(Debug, PartialEq, Clone)]
    enum Ev {
        R(usize, u32),
        W(usize, u32),
        Delay(u32),
    }

    /// A register file over every block, recording reads and writes.
    #[derive(Default)]
    struct Bus {
        regs: RefCell<BTreeMap<usize, u32>>,
        ev: RefCell<Vec<Ev>>,
        /// Bits a write cannot set (a stuck bit), per address.
        stuck_clear: RefCell<BTreeMap<usize, u32>>,
    }

    impl Bus {
        fn set(&self, a: usize, v: u32) {
            self.regs.borrow_mut().insert(a, v);
        }
        fn get(&self, a: usize) -> u32 {
            self.regs.borrow().get(&a).copied().unwrap_or(0)
        }
        fn writes(&self) -> Vec<(usize, u32)> {
            self.ev
                .borrow()
                .iter()
                .filter_map(|e| match e {
                    Ev::W(a, v) => Some((*a, *v)),
                    _ => None,
                })
                .collect()
        }
        fn delay(&self, us: u32) {
            self.ev.borrow_mut().push(Ev::Delay(us));
        }
    }

    impl Mmio for Bus {
        fn read32(&self, a: usize) -> u32 {
            let v = self.get(a);
            self.ev.borrow_mut().push(Ev::R(a, v));
            v
        }
        fn write32(&self, a: usize, v: u32) {
            self.ev.borrow_mut().push(Ev::W(a, v));
            let v = v & !self.stuck_clear.borrow().get(&a).copied().unwrap_or(0);
            // BSRR: bit n sets ODR n, bit n + 16 clears it; IDR follows.
            if a & 0xffff == gpio::r::BSRR {
                let base = a - gpio::r::BSRR;
                let odr = self.get(base + gpio::r::ODR);
                let odr = (odr | (v & 0xffff)) & !(v >> 16);
                self.set(base + gpio::r::ODR, odr);
                return;
            }
            self.set(a, v);
        }
        fn read8(&self, _: usize) -> u8 {
            unreachable!()
        }
        fn write8(&self, _: usize, _: u8) {
            unreachable!()
        }
    }

    #[derive(Default)]
    struct Lines(Vec<String>);
    impl Log for Lines {
        fn line(&mut self, s: &str) {
            self.0.push(s.to_string());
        }
    }

    const B: Bases = Bases::DK;

    fn model() -> Mp2Model {
        Mp2Model::with_config(Mp2Config { display_scmi: true, ..Default::default() })
    }

    fn hdr(proto: u8, msg: u8) -> u32 {
        scmi::header(proto, msg, TOKEN)
    }

    /// The request frames, as (header, payload words).
    fn requests(m: &Mp2Model) -> Vec<(u32, Vec<u32>)> {
        m.messages()
            .iter()
            .map(|(tx, _)| (scmi::word(tx, 0), (1..tx.len() / 4).map(|i| scmi::word(tx, i)).collect()))
            .collect()
    }

    /// No clock disable, no voltage off, no RATE_SET but on 27 among `m`'s
    /// requests (the note's "SCMI request rules").
    fn assert_rules(m: &Mp2Model) {
        for (tx, _) in m.messages() {
            let h = scmi::word(&tx, 0);
            let (msg, proto) = ((h & 0xff) as u8, ((h >> 10) & 0xff) as u8);
            let w: Vec<u32> = (1..tx.len() / 4).map(|i| scmi::word(&tx, i)).collect();
            assert!(!forbidden(proto, msg, &w), "forbidden request sent: {tx:02x?}");
            if (proto, msg) == (scmi::PROTO_CLOCK, scmi::CLOCK_CONFIG_SET) {
                assert_eq!(w[1], scmi::CLOCK_ENABLE, "CONFIG_SET(0)");
            }
            if (proto, msg) == (scmi::PROTO_CLOCK, scmi::CLOCK_RATE_SET) {
                assert_eq!(w[1], 27);
                assert!(![12, 28].contains(&w[1]));
            }
            if (proto, msg) == (scmi::PROTO_VOLTAGE, scmi::VOLTAGE_CONFIG_SET) {
                assert_eq!(w[1], scmi::VOLTAGE_CONFIG_ON);
            }
        }
    }

    /// The request sequence of a full power-on with `rate` (0: no RATE_SET).
    fn want_requests(rate: u64) -> Vec<(u32, Vec<u32>)> {
        use scmi::*;
        let (v, c) = (PROTO_VOLTAGE, PROTO_CLOCK);
        let mut w = vec![
            (hdr(v, PROTOCOL_VERSION), vec![]),
            (hdr(v, VOLTAGE_CONFIG_GET), vec![11]),
            (hdr(v, VOLTAGE_CONFIG_GET), vec![14]),
            (hdr(v, VOLTAGE_CONFIG_GET), vec![16]),
            (hdr(v, VOLTAGE_CONFIG_GET), vec![21]),
            (hdr(v, VOLTAGE_CONFIG_SET), vec![21, 7]),
            (hdr(v, VOLTAGE_CONFIG_GET), vec![21]),
            (hdr(c, PROTOCOL_VERSION), vec![]),
        ];
        for id in [90, 94, 12, 28, 27] {
            w.push((hdr(c, CLOCK_CONFIG_SET), vec![id, 1]));
            if id == 27 && rate != 0 {
                w.push((hdr(c, CLOCK_RATE_SET), vec![0, 27, rate as u32, (rate >> 32) as u32]));
            }
            w.push((hdr(c, CLOCK_ATTRIBUTES), vec![id]));
            w.push((hdr(c, CLOCK_RATE_GET), vec![id]));
        }
        w
    }

    fn run(m: &Mp2Model, bus: &Bus, cfg: &Config, log: &mut Lines) -> (Result<(), Error>, Powered) {
        let mut p = Powered::default();
        let r = power_on(&mut m.scmi(), bus, &B, cfg, |us| bus.delay(us), log, &mut p);
        (r, p)
    }

    #[test]
    fn display_ids_and_offsets() {
        // O:stm32_rifsc.c:26 PRIVCFGR0 0x30, :29 PER0_SEMCR 0x104 (+ 8 x id,
        // :84), :86-95 CFEN 0, SEMEN 1, SCID 6:4, SEML 23:16, :102-107 RIMC.
        assert_eq!((rifsc_privcfgr(42), rifsc_privcfgr(119)), (0x34, 0x3c));
        assert_eq!((rifsc_semcr(0), rifsc_semcr(80)), (0x104, 0x104 + 640));
        assert_eq!((CIDCFGR_CFEN, CIDCFGR_SEMEN, CIDCFGR_SCID, CIDCFGR_SEMWL), (1, 2, 0x70, 0x00ff_0000));
        assert_eq!((RIMC_CIDSEL, RIMC_MCID, RIMC_MSEC, RIMC_MPRIV), (4, 0x70, 0x100, 0x200));
        assert_eq!(CLOCKS, [90, 94, 12, 28, 27]);
        assert_eq!(SUPPLIES, [11, 14, 16]);
        assert_eq!(PINS.map(|p| (p.bank, p.pin)), [('F', 2), ('F', 0), ('B', 4), ('B', 6)]);
        assert_eq!((Config::DK_720P.ltdc_rate_hz, Config::DK_720P.cid), (74_250_000, 1));
        // PLL8 594 MHz / 8 and / 4 (note, "The LTDC pixel clock on the MP25").
        assert_eq!((594_000_000 / 8, 594_000_000 / 4), (Config::DK_720P.ltdc_rate_hz, Config::DK_1080P.ltdc_rate_hz));
    }

    #[test]
    fn display_power_on_720p_frames_rules_and_state() {
        let m = model();
        let bus = Bus::default();
        // OP-TEE's boot muxes in RCC_DSICFGR (note row 7), U-Boot's LTDC gate
        // (row 6) and DISPLAYCLKCR = 2 (row 8) as the note expects them.
        bus.set(RCC_BASE + RCC_DSICFGR, DSICFGR_DSIBLSEL | DSICFGR_DSIPHYCKREFSEL);
        bus.set(RCC_BASE + RCC_LTDCCFGR, CFGR_GATE | CFGR_RESET);
        bus.set(SYSCFG_BASE + SYSCFG_DISPLAYCLKCR, 0xf0 | 2);
        let mut log = Lines::default();
        let (r, p) = run(&m, &bus, &Config::DK_720P, &mut log);
        assert_eq!(r, Ok(()));
        assert_eq!(requests(&m), want_requests(74_250_000));
        assert_rules(&m);
        assert!(m.faults().is_empty(), "{:?}", m.faults());
        for id in CLOCKS {
            assert!(m.clock_enabled(id), "clock {id}");
        }
        assert_eq!(m.clock_rate(27), 74_250_000);
        assert_eq!((m.clock_rate(12), m.clock_rate(28)), (100_000_000, 27_000_000));
        assert!(m.voltage_on(21));
        assert_eq!(p.supplies_on, [true; 3]);
        assert_eq!((p.hdmi_before, p.hdmi), (0, 7));
        assert_eq!(p.rates, [0, 0, 100_000_000, 27_000_000, 74_250_000]);
        assert_eq!(p.rcc_before, [0, 0x9000, 3]);
        assert_eq!(p.rcc, [2, 0x9002, 2]);
        assert_eq!(p.displayclkcr, 0xf2);
        // RCC: per block gate, assert, hold, release; then SYSCFG.
        let mut want = Vec::new();
        for (b, before) in [(RCC_I2C2CFGR, 0), (RCC_DSICFGR, 0x9000), (RCC_LTDCCFGR, 3)] {
            let a = RCC_BASE + b;
            want.extend([
                Ev::R(a, before),
                Ev::R(a, before),
                Ev::W(a, before | 2),
                Ev::R(a, before | 2),
                Ev::W(a, before | 3),
                Ev::Delay(RCC_RESET_US),
                Ev::R(a, before | 3),
                Ev::W(a, (before | 2) & !1),
                Ev::R(a, (before | 2) & !1),
            ]);
        }
        let s = SYSCFG_BASE + SYSCFG_DISPLAYCLKCR;
        want.extend([Ev::R(s, 0xf2), Ev::W(s, 0xf2), Ev::R(s, 0xf2)]);
        let ev = bus.ev.borrow();
        // The RIF reads come first (3 entries x 3 registers), nothing else
        // before the SCMI requests.
        assert!(ev[..9].iter().all(|e| matches!(e, Ev::R(a, _) if *a >= RIFSC_BASE && *a < RIFSC_BASE + 0x1000)));
        assert_eq!(ev[9..].to_vec(), want);
        // Each request is logged before its reply, in order.
        let reqs: Vec<&String> = log.0.iter().filter(|l| l.starts_with("scmi > ")).collect();
        assert_eq!(reqs.len(), want_requests(74_250_000).len());
        assert_eq!(log.0[3], "scmi > 005c0400");
        assert_eq!(log.0.last().unwrap(), "syscfg displayclkcr 0x000000f2");
    }

    #[test]
    fn display_power_on_1080p_sets_the_boot_rate_and_rate_zero_sets_none() {
        let m = model();
        let bus = Bus::default();
        bus.set(RCC_BASE + RCC_DSICFGR, DSICFGR_DSIPHYCKREFSEL);
        let (r, p) = run(&m, &bus, &Config::DK_1080P, &mut Lines::default());
        assert_eq!(r, Ok(()));
        assert_eq!(requests(&m), want_requests(148_500_000));
        assert_eq!(p.rates[4], 148_500_000);
        assert_rules(&m);

        let m = model();
        let (r, _) = run(&m, &bus, &Config { ltdc_rate_hz: 0, ..Config::DK_720P }, &mut Lines::default());
        assert_eq!(r, Ok(()));
        assert_eq!(requests(&m), want_requests(0));
        assert_rules(&m);
    }

    #[test]
    fn display_forbidden_requests_are_never_built() {
        use scmi::*;
        assert!(forbidden(PROTO_CLOCK, CLOCK_CONFIG_SET, &[27, CLOCK_DISABLE]));
        assert!(forbidden(PROTO_CLOCK, CLOCK_CONFIG_SET, &[27]));
        assert!(!forbidden(PROTO_CLOCK, CLOCK_CONFIG_SET, &[27, CLOCK_ENABLE]));
        assert!(forbidden(PROTO_CLOCK, CLOCK_RATE_SET, &[0, 28, 27_000_000, 0]));
        assert!(forbidden(PROTO_CLOCK, CLOCK_RATE_SET, &[0, 12, 100_000_000, 0]));
        assert!(!forbidden(PROTO_CLOCK, CLOCK_RATE_SET, &[0, 27, 74_250_000, 0]));
        assert!(forbidden(PROTO_VOLTAGE, VOLTAGE_CONFIG_SET, &[21, VOLTAGE_CONFIG_OFF]));
        assert!(!forbidden(PROTO_VOLTAGE, VOLTAGE_CONFIG_SET, &[21, VOLTAGE_CONFIG_ON]));
        // A forbidden call stops before the transport and the log says so.
        let m = model();
        let mut log = Lines::default();
        let s = step(Op::ClockEnable, 27);
        let e = call(&mut m.scmi(), &mut log, s, PROTO_CLOCK, CLOCK_CONFIG_SET, &[27, 0], 0);
        assert_eq!(e, Err(super::Error::Forbidden { step: s }));
        assert!(m.messages().is_empty());
        assert!(log.0[0].starts_with("scmi ! forbidden"));
    }

    #[test]
    fn display_refused_rif_entry_stops_before_any_request_or_write() {
        for (seccfgr, cidcfgr, verdict) in [
            (1u32 << (RIFSC_DSI_CMN % 32), 0, Verdict::Secure),
            // CID filtering on, static SCID 2 (the M33's).
            (0, CIDCFGR_CFEN | 2 << 4, Verdict::ForeignCid),
        ] {
            let m = model();
            let bus = Bus::default();
            bus.set(RIFSC_BASE + rifsc_seccfgr(RIFSC_DSI_CMN), seccfgr);
            bus.set(RIFSC_BASE + rifsc_cidcfgr(RIFSC_DSI_CMN), cidcfgr);
            let (r, _) = run(&m, &bus, &Config::DK_720P, &mut Lines::default());
            assert_eq!(r, Err(Error::Rif { id: RIFSC_DSI_CMN, verdict }));
            assert!(m.messages().is_empty());
            assert!(bus.writes().is_empty());
        }
    }

    #[test]
    fn display_ldo5_set_refused_stops_before_the_clocks() {
        // OP-TEE's CONFIG_SET answers success; the read back is the state.
        let m = model();
        m.force_status(scmi::PROTO_VOLTAGE, scmi::VOLTAGE_CONFIG_SET, scmi::STATUS_DENIED);
        let bus = Bus::default();
        let (r, p) = run(&m, &bus, &Config::DK_720P, &mut Lines::default());
        assert_eq!(r, Err(Error::Refused { step: step(Op::HdmiSet, 21), status: Status::Denied }));
        assert_eq!(p.hdmi_before, 0);
        assert_eq!(requests(&m).len(), 6);
        assert!(bus.writes().is_empty());
        assert_rules(&m);
    }

    /// Rewrites the reply's first value of every request `pick` selects.
    struct Rewrite<'a, F: Fn(&[u8]) -> Option<u32>> {
        inner: crate::model_stm32mp2::Mp2Scmi<'a>,
        pick: F,
    }
    impl<F: Fn(&[u8]) -> Option<u32>> Transport for Rewrite<'_, F> {
        fn transfer(&mut self, tx: &[u8], rx: &mut [u8]) -> Result<usize, i32> {
            let n = self.inner.transfer(tx, rx)?;
            if let Some(v) = (self.pick)(tx) {
                rx[8..12].copy_from_slice(&v.to_le_bytes());
            }
            Ok(n)
        }
    }

    #[test]
    fn display_success_is_intent_the_read_back_is_the_state() {
        // LDO5: CONFIG_SET succeeds, CONFIG_GET still reads off.
        let m = model();
        let bus = Bus::default();
        let get = hdr(scmi::PROTO_VOLTAGE, scmi::VOLTAGE_CONFIG_GET);
        let pick = |tx: &[u8]| (scmi::word(tx, 0) == get && scmi::word(tx, 1) == 21).then_some(0);
        let mut p = Powered::default();
        let r = power_on(
            &mut Rewrite { inner: m.scmi(), pick },
            &bus,
            &B,
            &Config::DK_720P,
            |_| {},
            &mut Lines::default(),
            &mut p,
        );
        assert_eq!(r, Err(Error::Readback { step: step(Op::HdmiCheck, 21), want: 7, got: 0 }));
        assert!(bus.writes().is_empty());
        assert_eq!(m.messages().len(), 7);

        // FLEXGEN_28: CONFIG_SET succeeds, CLOCK_ATTRIBUTES says disabled.
        let m = model();
        let attr = hdr(scmi::PROTO_CLOCK, scmi::CLOCK_ATTRIBUTES);
        let pick = |tx: &[u8]| (scmi::word(tx, 0) == attr && scmi::word(tx, 1) == 28).then_some(0);
        let r = power_on(
            &mut Rewrite { inner: m.scmi(), pick },
            &bus,
            &B,
            &Config::DK_720P,
            |_| {},
            &mut Lines::default(),
            &mut p,
        );
        assert_eq!(r, Err(Error::Readback { step: step(Op::ClockAttributes, 28), want: 1, got: 0 }));
        assert!(bus.writes().is_empty());

        // FLEXGEN_27: RATE_SET succeeds, RATE_GET reads OP-TEE's boot rate
        // (its set_rate returns success without writing when it has no
        // access, O:clk-stm32mp25.c:3150-3151).
        let m = model();
        let rg = hdr(scmi::PROTO_CLOCK, scmi::CLOCK_RATE_GET);
        let pick = |tx: &[u8]| (scmi::word(tx, 0) == rg && scmi::word(tx, 1) == 27).then_some(148_500_000);
        let r = power_on(
            &mut Rewrite { inner: m.scmi(), pick },
            &bus,
            &B,
            &Config::DK_720P,
            |_| {},
            &mut Lines::default(),
            &mut p,
        );
        assert_eq!(r, Err(Error::Readback { step: step(Op::ClockRateGet, 27), want: 74_250_000, got: 148_500_000 }));
        assert!(bus.writes().is_empty());

        // FLEXGEN_28 at a rate the DSI PHY PLL plan does not assume.
        let m = model();
        let pick = |tx: &[u8]| (scmi::word(tx, 0) == rg && scmi::word(tx, 1) == 28).then_some(40_000_000);
        let r = power_on(
            &mut Rewrite { inner: m.scmi(), pick },
            &bus,
            &B,
            &Config::DK_720P,
            |_| {},
            &mut Lines::default(),
            &mut p,
        );
        assert_eq!(r, Err(Error::Readback { step: step(Op::ClockRateGet, 28), want: 27_000_000, got: 40_000_000 }));
        assert_rules(&m);
    }

    #[test]
    fn display_clock_refusals_and_wrong_rates_stop_before_the_rcc() {
        // RATE_SET refused (e.g. a rate OP-TEE cannot reach).
        let m = Mp2Model::with_config(Mp2Config {
            display_scmi: true,
            clock_rates: vec![(27, vec![148_500_000])],
            ..Default::default()
        });
        let bus = Bus::default();
        let (r, _) = run(&m, &bus, &Config::DK_720P, &mut Lines::default());
        assert_eq!(r, Err(Error::Refused { step: step(Op::ClockRateSet, 27), status: Status::OutOfRange }));
        assert!(bus.writes().is_empty());
        assert_rules(&m);

        // A transport failure on the first clock request.
        let m = model();
        let mut log = Lines::default();
        let mut p = Powered::default();
        let mut t = FailAt { inner: m.scmi(), n: 8, code: -7 };
        let r = power_on(&mut t, &bus, &B, &Config::DK_720P, |_| {}, &mut log, &mut p);
        assert_eq!(r, Err(Error::Transport { step: step(Op::ClockEnable, 90), code: -7 }));
        assert_eq!(log.0.last().unwrap(), "scmi ! transport -7");
        assert!(bus.writes().is_empty());
    }

    #[test]
    fn display_reply_echo_and_length_are_checked() {
        let m = model();
        let mut log = Lines::default();
        let s = step(Op::ClockVersion, 0);
        // A reply to another message: the model answers a RATE_GET header.
        struct Swap<'a>(crate::model_stm32mp2::Mp2Scmi<'a>);
        impl Transport for Swap<'_> {
            fn transfer(&mut self, tx: &[u8], rx: &mut [u8]) -> Result<usize, i32> {
                let n = self.0.transfer(tx, rx)?;
                rx[0] ^= 0x1;
                Ok(n)
            }
        }
        let e = call(&mut Swap(m.scmi()), &mut log, s, scmi::PROTO_CLOCK, scmi::PROTOCOL_VERSION, &[], 1);
        assert!(matches!(e, Err(Error::Echo { .. })), "{e:?}");
        struct Short;
        impl Transport for Short {
            fn transfer(&mut self, tx: &[u8], rx: &mut [u8]) -> Result<usize, i32> {
                rx[..4].copy_from_slice(&tx[..4]);
                rx[4..8].fill(0);
                Ok(8)
            }
        }
        let e = call(&mut Short, &mut log, s, scmi::PROTO_CLOCK, scmi::PROTOCOL_VERSION, &[], 1);
        assert_eq!(e, Err(Error::Length { step: s, len: 8 }));
    }

    /// Fails the `n`th transfer (0-based) with `code`.
    struct FailAt<T: Transport> {
        inner: T,
        n: usize,
        code: i32,
    }
    impl<T: Transport> Transport for FailAt<T> {
        fn transfer(&mut self, tx: &[u8], rx: &mut [u8]) -> Result<usize, i32> {
            if self.n == 0 {
                return Err(self.code);
            }
            self.n -= 1;
            self.inner.transfer(tx, rx)
        }
    }

    #[test]
    fn display_rcc_readback_and_mux_are_checked() {
        let m = model();
        let bus = Bus::default();
        // The I2C2 reset bit sticks set.
        struct StickyReset<'a>(&'a Bus);
        impl Mmio for StickyReset<'_> {
            fn read32(&self, a: usize) -> u32 {
                self.0.read32(a)
            }
            fn write32(&self, a: usize, v: u32) {
                let v = if a == RCC_BASE + RCC_I2C2CFGR { v | CFGR_RESET } else { v };
                self.0.write32(a, v)
            }
            fn read8(&self, _: usize) -> u8 {
                unreachable!()
            }
            fn write8(&self, _: usize, _: u8) {
                unreachable!()
            }
        }
        let mut p = Powered::default();
        let r =
            power_on(&mut m.scmi(), &StickyReset(&bus), &B, &Config::DK_720P, |_| {}, &mut Lines::default(), &mut p);
        assert_eq!(r, Err(Error::Rcc { block: Block::I2c2, raw: 3 }));

        // A write that loses the DSI muxes (a register that does not hold them).
        let m = model();
        let bus = Bus::default();
        bus.set(RCC_BASE + RCC_DSICFGR, DSICFGR_DSIPHYCKREFSEL);
        bus.stuck_clear.borrow_mut().insert(RCC_BASE + RCC_DSICFGR, DSICFGR_DSIPHYCKREFSEL);
        let (r, _) = run(&m, &bus, &Config::DK_720P, &mut Lines::default());
        assert_eq!(r, Err(Error::Mux { before: 0x8000, after: 2 }));
    }

    #[test]
    fn display_syscfg_that_does_not_hold_2_is_an_error() {
        let m = model();
        let bus = Bus::default();
        bus.stuck_clear.borrow_mut().insert(SYSCFG_BASE + SYSCFG_DISPLAYCLKCR, 2);
        let (r, _) = run(&m, &bus, &Config::DK_720P, &mut Lines::default());
        assert_eq!(r, Err(Error::Syscfg { raw: 0 }));
    }

    #[test]
    fn display_supplies_off_are_reported_not_set() {
        // vddcore's flag reads OFF (a server that kept it off for the agent):
        // recorded, and no CONFIG_SET goes to an always-on domain.
        struct Vddcore<'a>(crate::model_stm32mp2::Mp2Scmi<'a>);
        impl Transport for Vddcore<'_> {
            fn transfer(&mut self, tx: &[u8], rx: &mut [u8]) -> Result<usize, i32> {
                let n = self.0.transfer(tx, rx)?;
                let get = hdr(scmi::PROTO_VOLTAGE, scmi::VOLTAGE_CONFIG_GET);
                if scmi::word(tx, 0) == get && scmi::word(tx, 1) == SCMI_VOLT_VDDCORE {
                    rx[8..12].fill(0);
                }
                Ok(n)
            }
        }
        let m = model();
        let bus = Bus::default();
        let mut log = Lines::default();
        let mut p = Powered::default();
        let r = power_on(&mut Vddcore(m.scmi()), &bus, &B, &Config::DK_720P, |_| {}, &mut log, &mut p);
        assert_eq!(r, Ok(()));
        assert_eq!(p.supplies_on, [false, true, true]);
        assert!(log.0.contains(&"supply 11 reads 0x0, not on (OP-TEE's flag; not set)".to_string()));
        let set = hdr(scmi::PROTO_VOLTAGE, scmi::VOLTAGE_CONFIG_SET);
        let sets: Vec<_> = requests(&m).into_iter().filter(|r| r.0 == set).collect();
        assert_eq!(sets, [(set, vec![21, 7])]);
        assert_rules(&m);
    }

    #[test]
    fn display_rifsc_entry_follows_optee_check_access() {
        let bus = Bus::default();
        let r = RIFSC_BASE;
        // Filter off: any CID.
        assert_eq!(rifsc_entry(&bus, r, 80, 1).verdict, Verdict::Ok);
        // Privileged only: reported, not refused.
        bus.set(r + rifsc_privcfgr(80), 1 << 16);
        let e = rifsc_entry(&bus, r, 80, 1);
        assert!(e.privileged && e.verdict == Verdict::Ok);
        // Static SCID 1 vs 2.
        bus.set(r + rifsc_cidcfgr(81), CIDCFGR_CFEN | 1 << 4);
        assert_eq!(rifsc_entry(&bus, r, 81, 1).verdict, Verdict::Ok);
        assert_eq!(rifsc_entry(&bus, r, 81, 2).verdict, Verdict::ForeignCid);
        // Semaphore: CID1 listed, free -> ok; held by 2 -> foreign; not listed.
        bus.set(r + rifsc_cidcfgr(42), CIDCFGR_CFEN | CIDCFGR_SEMEN | 0b110 << 16);
        assert_eq!(rifsc_entry(&bus, r, 42, 1).verdict, Verdict::Ok);
        bus.set(r + rifsc_semcr(42), SEMCR_MUTEX | 2 << 4);
        let e = rifsc_entry(&bus, r, 42, 1);
        assert_eq!((e.verdict, e.semcr), (Verdict::ForeignCid, 0x21));
        bus.set(r + rifsc_semcr(42), SEMCR_MUTEX | 1 << 4);
        assert_eq!(rifsc_entry(&bus, r, 42, 1).verdict, Verdict::Ok);
        bus.set(r + rifsc_cidcfgr(42), CIDCFGR_CFEN | CIDCFGR_SEMEN | 0b100 << 16);
        assert_eq!(rifsc_entry(&bus, r, 42, 1).verdict, Verdict::ForeignCid);
        // Secure wins over everything.
        bus.set(r + rifsc_seccfgr(119), 1 << (119 % 32));
        assert_eq!(rifsc_entry(&bus, r, 119, 1).verdict, Verdict::Secure);
        // Read only.
        assert!(bus.writes().is_empty());
    }

    #[test]
    fn display_rif_check_reports_entries_and_rimu_and_writes_nothing() {
        let bus = Bus::default();
        // The DK file: RIMU 11-13 CID0, non-secure, privileged, CIDSEL_M
        // (O:-rif.dtsi:636-638); layer 3 made secure here.
        for n in RIMU_LTDC_CANDIDATES {
            bus.set(RIFSC_BASE + rifsc_rimc_attr(n), RIMC_MPRIV | RIMC_CIDSEL);
        }
        bus.set(RIFSC_BASE + rifsc_seccfgr(120), 1 << (120 % 32));
        let mut log = Lines::default();
        let rep = rif_check(&bus, RIFSC_BASE, CID_A35, &mut log);
        assert_eq!(rep.entries.map(|e| e.id), [80, 119, 81, 42, 120, 121, 122, 123]);
        assert_eq!(rep.refused(), None);
        assert_eq!(rep.entries[4].verdict, Verdict::Secure);
        assert_eq!(rep.rimc, [0x204; 3]);
        assert!(bus.writes().is_empty());
        assert_eq!(log.0[0], "rif rifsc 80 sec 0 priv 0 cidcfgr 0x00000000 semcr 0x00000000: Ok");
        assert_eq!(log.0[8], "rif rimu 11 0x00000204 (cidsel 1 mcid 0 msec 0 mpriv 1)");
        bus.set(RIFSC_BASE + rifsc_cidcfgr(42), CIDCFGR_CFEN | 2 << 4);
        let rep = rif_check(&bus, RIFSC_BASE, CID_A35, &mut log);
        assert_eq!(rep.refused().map(|e| (e.id, e.verdict)), Some((42, Verdict::ForeignCid)));
    }

    #[test]
    fn display_fb_check_against_the_risaf4_table() {
        // The note's proposals: 0xb0000000 (region 9), the GPU carveout
        // (region 10), two 1080p frames.
        assert_eq!(fb_check(0xb000_0000, 32 << 20), Ok(9));
        assert_eq!(fb_check(0xfa80_0000, 0x7e_9000), Ok(10));
        assert_eq!(fb_check(0xfa00_0000, 16 << 20), Ok(9));
        assert_eq!(fb_check(0xb000_0000, 0), Err(FbError::Empty));
        assert_eq!(fb_check(u64::MAX - 0xfff, 0x2000), Err(FbError::Empty));
        assert_eq!(fb_check(0xb000_0040, 0x1000), Err(FbError::Unaligned));
        assert_eq!(fb_check(0x1_0000_0000, 0x1000), Err(FbError::Above4G));
        assert_eq!(fb_check(0xaf00_0000, 32 << 20), Err(FbError::Image));
        assert_eq!(fb_check(0xfe80_0000, 0x1000), Err(FbError::SecureLayer));
        assert_eq!(fb_check(0xfe00_0000, 16 << 20), Err(FbError::SecureLayer));
        // OP-TEE's and the M33's regions below 0x84000000.
        assert_eq!(fb_check(0x8200_0000, 0x1000), Err(FbError::NotAllowed));
        assert_eq!(fb_check(0x83ff_f000, 0x2000), Err(FbError::NotAllowed));
    }

    #[test]
    fn display_pins_write_sequence_and_state() {
        let bus = Bus::default();
        let (b, f) = (GPIOB_BASE, GPIOF_BASE);
        bus.set(b + gpio::r::MODER, 0xffff_ffff);
        bus.set(f + gpio::r::MODER, 0xffff_ffff);
        bus.set(b + gpio::r::IDR, 1 << 4);
        let mut log = Lines::default();
        let s = pins(&bus, &B, CID_A35, &mut log).unwrap();
        use gpio::r::*;
        // pinctrl-stm32.c's order per pin (ip::stm32_gpio): AFR, MODER,
        // OTYPER, OSPEEDR, PUPDR; the output's level to BSRR first.
        assert_eq!(
            bus.writes(),
            [
                (f + AFRL, 9 << 8),
                (f + MODER, !(3 << 4) | 2 << 4),
                (f + OTYPER, 1 << 2),
                (f + OSPEEDR, 0),
                (f + PUPDR, 0),
                (f + AFRL, 9 << 8 | 9),
                (f + MODER, !(3 << 4) & !3 | 2 << 4 | 2),
                (f + OTYPER, 1 << 2 | 1),
                (f + OSPEEDR, 0),
                (f + PUPDR, 0),
                (b + AFRL, 0),
                (b + MODER, !(3 << 8)),
                (b + PUPDR, 0),
                (b + BSRR, 1 << 6),
                (b + AFRL, 0),
                (b + MODER, !(3 << 8) & !(3 << 12) | 1 << 12),
                (b + OTYPER, 0),
                (b + OSPEEDR, 0),
                (b + PUPDR, 0),
            ]
        );
        assert_eq!(s.gpiof_afrl, 0x909);
        assert!(s.adv_int);
        assert_eq!(bus.get(b + ODR), 1 << 6);
        assert_eq!(log.0.len(), 1);
    }

    #[test]
    fn display_a_refused_pin_stops_with_nothing_written() {
        let bus = Bus::default();
        bus.set(GPIOB_BASE + gpio::r::SECCFGR, 1 << 6);
        let mut log = Lines::default();
        assert_eq!(pins(&bus, &B, CID_A35, &mut log), Err(Error::Pin { pin: PIN_ADV_RESET, why: gpio::Error::Secure }));
        assert!(bus.writes().is_empty());
        assert_eq!(log.0[0], "pin PB6 refused: Secure; nothing written");
        let bus = Bus::default();
        bus.set(GPIOF_BASE + gpio::r::cidcfgr(0), gpio::r::CID_CFEN | 2 << 4);
        assert_eq!(
            pins(&bus, &B, CID_A35, &mut log),
            Err(Error::Pin { pin: PIN_I2C2_SDA, why: gpio::Error::ForeignCid })
        );
        assert!(bus.writes().is_empty());
    }

    #[test]
    fn display_adv_reset_pulses_pb6_as_uboot_does() {
        let bus = Bus::default();
        let mut log = Lines::default();
        // Not an output yet: refused, nothing written.
        assert_eq!(
            adv_reset(&bus, &B, CID_A35, |us| bus.delay(us), &mut log),
            Err(Error::PinMode { pin: PIN_ADV_RESET, moder: 0 })
        );
        assert!(bus.writes().is_empty());
        pins(&bus, &B, CID_A35, &mut log).unwrap();
        bus.ev.borrow_mut().clear();
        adv_reset(&bus, &B, CID_A35, |us| bus.delay(us), &mut log).unwrap();
        let bsrr = GPIOB_BASE + gpio::r::BSRR;
        let seq: Vec<Ev> = bus.ev.borrow().iter().filter(|e| !matches!(e, Ev::R(..))).cloned().collect();
        assert_eq!(
            seq,
            [Ev::W(bsrr, 1 << 22), Ev::Delay(ADV_RESET_LOW_US), Ev::W(bsrr, 1 << 6), Ev::Delay(ADV_RESET_WAIT_US)]
        );
        assert_eq!(bus.get(GPIOB_BASE + gpio::r::ODR), 1 << 6);
        assert_eq!((ADV_RESET_LOW_US, ADV_RESET_WAIT_US), (1000, 10_000));
    }
}
