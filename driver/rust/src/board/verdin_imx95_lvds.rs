// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The Verdin iMX95 on the Mallow, the whole picture on X12: the Toradex
//! Capacitive Touch Display 10.1" LVDS on the Mallow's native LVDS
//! connector (the owner's board since 2026-11-24), cold, in the order of
//! driver/docs/imx95-display.md section 4(b'):
//!
//! | note step | what | here |
//! |---|---|---|
//! | 1, 2, 4, 5 (configured) | DISPLAY on, LDBPLL_VCO 2,986.2 MHz and LDBPLL 497.7 MHz over SCMI; display CSR engine clock = LDB PLL / 7, interleaver bypass; interleaver channel 0; the DPU's one plane, not started; pixel link 0 (as Linux has it, U5) | `display::prepare` with [`Route::Lvds`] |
//! | 3 | the LDB: clock gates, pixel mapper (channel 0 from DI0, 24-bit SPWG), LVDS PHY 0 | `soc::imx95::ldb::up` |
//! | 5 (started) | FrameGen0 on, wait for `PrimSyncStat` | `display::start` |
//! | 6 | the backlight: TPM5's clock over SCMI, TPM5 channel 0 low-true at 119/255 of 150 Hz, then GPIO5_IO04 high | [`backlight_on`] |
//!
//! No bridge, no I2C, no CAMERA domain, no DSI host: the X12 path is the
//! DPU's stream on the SoC's own LVDS pins (Verdin pins 88-114, `LVDS_1_A`
//! on the Mallow's X12, `MAL:` Tables 16-17).
//!
//! The board's facts, from the devicetrees (Toradex's
//! `imx95-verdin-panel-cap-touch-10inch-lvds.dtso`, the Verdin module's
//! `imx95-verdin.dtsi`), hal_nxp's `fsl_iomuxc.h` for the i.MX 95 and the
//! board's Linux (2026-11-25):
//! - the panel: 1280x800 at 71.1 MHz, its timing as Linux's FrameGen0 holds
//!   it ([`PANEL`], `display::VERDIN_LVDS_10INCH`);
//! - the backlight PWM: Verdin `PWM_2` (SODIMM 16, Mallow X12 pin 35) is the
//!   pad GPIO_IO06, whose mux 1 is TPM5 channel 0
//!   (`IOMUXC_PAD_GPIO_IO06__TPM5_CH0`); TPM5 is 0x42500000
//!   (`nxp_mimx95_a55.dtsi` `tpm5`); Linux's pwm shows
//!   `platform/42500000.pwm` channel 0 "backlight-lvds-native", period
//!   6,666,667 ns, duty 3,111,111 ns, polarity inverse, brightness 4 of 7:
//!   [`BACKLIGHT`] (119 of 255 low-true, the duty Linux runs);
//! - the backlight enable: Verdin `I2S_2_D_OUT` (SODIMM 46, X12 pin 37) is
//!   the pad XSPI1_DATA4, mux 5 GPIO5_IO04
//!   (`IOMUXC_PAD_XSPI1_DATA4__GPIO5_IO_BIT4`); GPIO5 is the RGPIO at
//!   0x43850000 ([`GPIO5_BASE`], [`EN_PIN`] 4), active high;
//! - the TPM5 clock is SCMI clock 154 (`IMX95_CLK_TPM5`, 41 + 113).
//!
//! The pads' mux is the board devicetree's (pinctrl through the System
//! Manager, the C side applies it before [`display_up`]).
//!
//! The RGPIO's registers are hal_nxp's BSD-3 `PERI_RGPIO.h` (:425-430: PDOR
//! 0x40, PSOR 0x44, PCOR 0x48, PDDR 0x54; the generator does not parse that
//! header's byte array, so the four offsets are written here); the order is
//! `fsl_rgpio.c` `RGPIO_PinInit` for an output: the level first, then the
//! direction.

use crate::env::Env;
use crate::firmware::scmi::{self, Transport};
use crate::ip::tpm_pwm::{self, Pwm};
use crate::mmio::Mmio;
use crate::soc::imx95::display::{self, Stream0, StreamConfig};
use crate::soc::imx95::display_power;
use crate::soc::imx95::dpu::{self, Form, Framebuffer, Route};
use crate::soc::imx95::ldb;
use crate::soc::imx95::regs::display_seeris as r;
use crate::soc::imx95::sm::{self, Fault};
use crate::video::VideoMode;
use crate::ylog;

/// The Toradex 10.1" LVDS panel as Linux runs it.
pub const PANEL: VideoMode = display::VERDIN_LVDS_10INCH;

/// TPM5 (`nxp_mimx95_a55.dtsi` `tpm5: tpm@42500000`).
pub const TPM5_BASE: usize = 0x4250_0000;
/// `IMX95_CLK_TPM5` (imx95_clock.h:166, 41 + 113).
pub const CLK_TPM5: u32 = 154;
/// GPIO5 (`nxp_mimx95_a55.dtsi` `gpio5: gpio@43850000`, `nxp,imx-rgpio`).
pub const GPIO5_BASE: usize = 0x4385_0000;
/// The backlight enable: GPIO5_IO04.
pub const EN_PIN: u32 = 4;

/// RGPIO registers (`PERI_RGPIO.h`).
pub mod rgpio {
    pub const PDOR: u32 = 0x40;
    pub const PSOR: u32 = 0x44;
    pub const PCOR: u32 = 0x48;
    pub const PDDR: u32 = 0x54;
}

/// FrameGen0's sequence complete: display stream 0's `DisEngCfg` "sequence
/// complete 0", bit 20 of an interrupt block's group 0 words (NXP's
/// `dpu95.h` `DPU95_IRQ_DISENGCFG_SEQCOMPLETE0` = 20; hal_nxp's BSD-3
/// `drivers/dpu_1/fsl_dpu.h` `kDPU_Group0DiSengcfgSeqComplete0Interrupt` =
/// `1U << 20U`). It sets when FrameGen0 has finished the frame it was
/// running after `FGENABLE` is cleared: the disable is done and the pixel
/// clock may go. The FrameGen0 status word `FGENSTS` itself has only
/// `EnSts` (bit 0) and `PanicStat` (bit 1).
///
/// Linux takes this event from the display engine 0 interrupt block
/// ([`DISP_IRQ0`]), not from the common-control block at DPU + 0x1000
/// (`IRQ_INTERRUPTSTATUS0`, the one `dpu::wait_flip` polls): NXP's
/// `dpu95-core.c` maps `comctrl_irq_reg` at `dpu_base + 0x1000` for the
/// software interrupts only and `disp_irq0_reg` at `dpu_base + 0x381000`
/// for `DPU95_IRQ_DISENGCFG_SEQCOMPLETE0` (`dpu95_dec_seqcomplete0_irq_handler`
/// -> `dpu95_disp_irq0_handle`). The board check after 211ed16a polled the
/// common-control word for 1 s and never saw the bit.
pub const SEQ_COMPLETE0: u32 = 1 << 20;
/// How long [`display_down`] waits for [`SEQ_COMPLETE0`]: Linux's 1 s
/// (`HZ`, `DPU95_CRTC_WAIT_FOR_COMPLETION_TIMEOUT`), polled every
/// `dpu::POLL_STEP_US`.
pub const SEQ_COMPLETE_BUDGET_US: u32 = 1_000_000;

/// The display engine 0 interrupt block, from the DPU's base: hal_nxp's
/// BSD-3 `PERI_DISPLAY_SEERIS.h` for the i.MX 95 (`DISPIRQ0_LOCKUNLOCK`,
/// offset 0x381000) and NXP's `dpu95-core.c` (`disp_irq0_reg =
/// devm_ioremap(dev, dpu_base + 0x381000, SZ_64)`). The generated
/// `regs::display_seeris` does not carry this block, so its three words
/// are written here, in [`disp_irq`]. Nothing else of ours uses it.
pub const DISP_IRQ0: u32 = 0x38_1000;

/// The words of an interrupt block, group 0, from the block's base
/// (`PERI_DISPLAY_SEERIS.h` `DISPIRQ0_*`; NXP's `dpu95.h`
/// `INTERRUPTENABLE(n)` 0x8, `INTERRUPTCLEAR(n)` 0x20,
/// `INTERRUPTSTATUS(n)` 0x2c). `INTERRUPTCLEAR0` is write-only (`__O`),
/// `INTERRUPTSTATUS0` read-only (`__I`), `INTERRUPTENABLE0` read-write.
pub mod disp_irq {
    pub const INTERRUPTENABLE0: u32 = 0x08;
    pub const INTERRUPTCLEAR0: u32 = 0x20;
    pub const INTERRUPTSTATUS0: u32 = 0x2c;
}

/// The backlight as Linux runs it (module text): TPM5 channel 0, 150 Hz,
/// low-true, 119 of 255.
pub const BACKLIGHT: Pwm = Pwm { channel: 0, period_ns: 6_666_667, level: 119, max: 255, low_true: true };

/// The whole X12 picture's configuration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Config {
    pub display: display::Bases,
    pub stream: StreamConfig,
    pub ldb: usize,
    pub ldb_form: ldb::Form,
    pub tpm: usize,
    pub gpio: usize,
    pub backlight: Pwm,
}

impl Config {
    /// The 10.1" LVDS panel showing `fb`: the defaults the design note's
    /// section 4(b') names (pixel link on as Linux has it, `fsl_dpu.c`'s DPU
    /// words, the dump's LDB words).
    pub const fn verdin(fb: Framebuffer) -> Config {
        Config {
            display: display::Bases::PHYS,
            stream: StreamConfig {
                mode: PANEL,
                route: Route::Lvds { pixel_link: true },
                fb,
                power: display_power::Config::VERDIN_LVDS_10INCH,
                form: Form::Hal,
            },
            ldb: ldb::LDB_BASE,
            ldb_form: ldb::Form::Linux,
            tpm: TPM5_BASE,
            gpio: GPIO5_BASE,
            backlight: BACKLIGHT,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The DPU side: power and LDB PLL, routing, DPU, FrameGen0's sync.
    Display(display::Error),
    /// The down: FrameGen0's [`SEQ_COMPLETE0`] did not set within
    /// `waited_us` after `FGENABLE` was cleared; `irq` is the last
    /// [`DISP_IRQ0`] `INTERRUPTSTATUS0`, `comctrl` the common-control
    /// block's `IRQ_INTERRUPTSTATUS0` then (for the record), `fg`
    /// FrameGen0's state. The LDB's clocks and the display's power were left
    /// on.
    SeqTimeout { waited_us: u32, irq: u32, comctrl: u32, fg: dpu::FgStatus },
    /// TPM5's clock over SCMI: the message (CLOCK_CONFIG_SET 0x07 or
    /// CLOCK_RATE_GET 0x06) and what came back; the picture is up, dark.
    BacklightClock { msg: u8, fault: Fault },
    /// TPM5 reads 0 Hz; the picture is up, dark.
    BacklightRate,
    /// The PWM (errno); the picture is up, dark.
    Backlight(i32),
}

/// What [`display_up`] brought up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Up {
    pub stream: Stream0,
    /// The LVDS CSR's four words as they read after the start.
    pub ldb: [u32; 4],
    /// TPM5's clock as the SM reports it.
    pub tpm_hz: u64,
    pub backlight: tpm_pwm::Plan,
}

fn clock<T: Transport>(t: &mut T, msg: u8, words: &[u32], want: usize) -> Result<sm::Resp, Error> {
    sm::call(t, scmi::PROTO_CLOCK, msg, words, want).map_err(|fault| Error::BacklightClock { msg, fault })
}

/// Drive the RGPIO pin: the level, then the direction (output).
pub fn enable_pin(m: &impl Mmio, gpio: usize, high: bool) {
    let bit = 1 << EN_PIN;
    let reg = if high { rgpio::PSOR } else { rgpio::PCOR };
    m.write32(gpio + reg as usize, bit);
    let a = gpio + rgpio::PDDR as usize;
    m.write32(a, m.read32(a) | bit);
}

/// The backlight on (Linux's pwm-backlight order: the PWM, then the
/// enable): TPM5's clock enabled and read over SCMI, the PWM started, then
/// GPIO5_IO04 high.
pub fn backlight_on<T: Transport>(t: &mut T, m: &impl Mmio, cfg: &Config) -> Result<(u64, tpm_pwm::Plan), Error> {
    clock(t, scmi::CLOCK_CONFIG_SET, &[CLK_TPM5, scmi::CLOCK_ENABLE, 0], 0)?;
    let r = clock(t, scmi::CLOCK_RATE_GET, &[CLK_TPM5], 2)?;
    let hz = r.get(0) as u64 | (r.get(1) as u64) << 32;
    if hz == 0 {
        return Err(Error::BacklightRate);
    }
    let plan = tpm_pwm::start(m, cfg.tpm, &cfg.backlight, hz).map_err(Error::Backlight)?;
    enable_pin(m, cfg.gpio, true);
    Ok((hz, plan))
}

/// The backlight off: the enable low, the PWM stopped, TPM5's clock off.
pub fn backlight_off<T: Transport>(t: &mut T, m: &impl Mmio, cfg: &Config) -> Result<(), Error> {
    enable_pin(m, cfg.gpio, false);
    tpm_pwm::stop(m, cfg.tpm);
    clock(t, scmi::CLOCK_CONFIG_SET, &[CLK_TPM5, scmi::CLOCK_DISABLE, 0], 0).map(|_| ())
}

/// The picture on the 10.1" LVDS panel (the module documentation's order).
/// On an error what was already switched on stays on; [`display_down`]
/// takes it down.
pub fn display_up<T: Transport>(t: &mut T, m: &impl Mmio, env: &impl Env, cfg: &Config) -> Result<Up, Error> {
    let mut delay = |us: u32| env.busy_wait_us(us);
    let prepared = display::prepare(t, m, &cfg.display, &cfg.stream, &mut delay).map_err(Error::Display)?;
    let words = ldb::words(&prepared.mode, cfg.ldb_form);
    ldb::up(m, cfg.ldb, &words, &mut delay);
    let stream = display::start(m, &cfg.display, &prepared, &mut delay).map_err(Error::Display)?;
    let (tpm_hz, backlight) = backlight_on(t, m, cfg)?;
    let ldb = ldb::read(m, cfg.ldb);
    let mode = &prepared.mode;
    ylog!(
        env,
        Inf,
        "verdin-imx95 LVDS: {}x{} at {} kHz (LDB PLL / 7), LDB {:#010x} {:#010x} {:#010x} {:#010x}, backlight {}/{} low-true",
        mode.hactive,
        mode.vactive,
        prepared.pix_hz / 1000,
        ldb[0],
        ldb[1],
        ldb[2],
        ldb[3],
        cfg.backlight.level,
        cfg.backlight.max
    );
    Ok(Up { stream, ldb, tpm_hz, backlight })
}

/// The backlight off, FrameGen0 stopped, the LDB down, then stream 0 and the
/// display's power (the LDB PLL and DISPLAY).
///
/// FrameGen0 is stopped as Linux stops it (NXP's `dpu95-crtc.c`
/// `dpu95_crtc_disable`; upstream `dc-crtc.c` `dc_crtc_atomic_disable` has
/// the same steps for the i.MX 8QXP):
/// 1. `enable_irq(dec_seq_complete_irq)`: [`SEQ_COMPLETE0`] enabled in the
///    display engine 0 interrupt block ([`DISP_IRQ0`]), a stale one cleared
///    first;
/// 2. `dpu95_fg_disable`: `FGENABLE` = 0 and nothing else, no `FGSLR`
///    shadow token (Linux writes that token only in `atomic_flush`, with
///    the ExtDst and DomainBlend ones, never at the disable);
/// 3. `DPU95_CRTC_WAIT_FOR_COMPLETION_TIMEOUT(dec_seq_complete_done)`: the
///    bit polled in that block's `INTERRUPTSTATUS0`, up to
///    [`SEQ_COMPLETE_BUDGET_US`], then cleared;
/// 4. `disable_irq`: the bit masked again;
/// 5. `dpu95_fg_disable_clock`: only then the pixel clock (the LDB) goes.
///
/// FrameGen0 ends the frame it is running after `FGENABLE` is cleared, and
/// the pixel clock comes through the LDB, so the LDB's clocks stay until the
/// sequence completes and FrameGen0 reads stopped (`FGENSTS`). When the
/// sequence does not complete nothing more is touched: the LDB and the power
/// stay on and [`Error::SeqTimeout`] comes back. When it completes but
/// FrameGen0 still reads running, the LDB is still brought down and the
/// first error is returned; the power stays on then.
pub fn display_down<T: Transport>(
    t: &mut T,
    m: &impl Mmio,
    env: &impl Env,
    cfg: &Config,
    stream: &Stream0,
) -> Result<(), Error> {
    backlight_off(t, m, cfg)?;
    let mut delay = |us: u32| env.busy_wait_us(us);
    let base = cfg.display.dpu;
    let irq0 = base + DISP_IRQ0 as usize;
    let enable = irq0 + disp_irq::INTERRUPTENABLE0 as usize;
    let clear = irq0 + disp_irq::INTERRUPTCLEAR0 as usize;
    let status = irq0 + disp_irq::INTERRUPTSTATUS0 as usize;
    // 1: a sequence complete left from before is not this stop's; the event
    // enabled as Linux's enable_irq leaves it (nothing else of ours is
    // enabled in this block, so the word is written, not read back)
    m.write32(clear, SEQ_COMPLETE0);
    m.write32(enable, SEQ_COMPLETE0);
    // 2: FrameGen0 off, no shadow token
    m.write32(base + r::DISENG_FRAMEGEN0_FGENABLE as usize, 0);
    ylog!(
        env,
        Inf,
        "verdin-imx95 LVDS down: FGENABLE 0 (no FGSLR token), waiting up to {} us for SeqComplete0 in DISPIRQ0 (DPU+{:#x} bit 20)",
        SEQ_COMPLETE_BUDGET_US,
        DISP_IRQ0 + disp_irq::INTERRUPTSTATUS0
    );
    // 3: FrameGen0's sequence complete, polled as dpu's waits poll
    let mut waited = 0;
    loop {
        let irq = m.read32(status);
        if irq & SEQ_COMPLETE0 != 0 {
            m.write32(clear, SEQ_COMPLETE0);
            break;
        }
        if waited >= SEQ_COMPLETE_BUDGET_US {
            // 4 also on the way out
            m.write32(enable, 0);
            let comctrl = m.read32(base + r::IRQ_INTERRUPTSTATUS0 as usize);
            let fg = dpu::fg_status(m, base);
            ylog!(
                env,
                Err,
                "verdin-imx95 LVDS down: no SeqComplete0 in {} us (DISPIRQ0 INTERRUPTSTATUS0 {:#010x}, COMCTRL IRQ_INTERRUPTSTATUS0 {:#010x}, FGENABLE {} FGENSTS {:#x} FGCHSTAT {:#x}); the LDB and the power stay on",
                waited,
                irq,
                comctrl,
                fg.fgenable,
                fg.fgensts,
                fg.fgchstat
            );
            return Err(Error::SeqTimeout { waited_us: waited, irq, comctrl, fg });
        }
        delay(dpu::POLL_STEP_US);
        waited += dpu::POLL_STEP_US;
    }
    // 4: masked again
    m.write32(enable, 0);
    let budget = 2 * display::frame_us(&stream.prepared.mode, stream.prepared.pix_hz);
    let stopped = dpu::wait_stopped(m, base, budget, &mut delay).map_err(display::Error::NotStopped);
    let frame =
        m.read32(base + r::DISENG_FRAMEGEN0_FGTIMESTAMP as usize) >> r::diseng_framegen0_fgtimestamp::FRAMEINDEX_SHIFT;
    ylog!(
        env,
        Inf,
        "verdin-imx95 LVDS down: SeqComplete0 after {} us, FGENSTS {}, frame {}",
        waited,
        if stopped.is_ok() { "stopped" } else { "running" },
        frame
    );
    // 5: the pixel clock goes
    ldb::down(m, cfg.ldb);
    // the pixel link off, the power; no second wait when the first failed
    match stopped {
        Ok(_) => display::stop(t, m, &cfg.display, stream, &mut delay).map_err(Error::Display),
        Err(e) => Err(Error::Display(e)),
    }
}
