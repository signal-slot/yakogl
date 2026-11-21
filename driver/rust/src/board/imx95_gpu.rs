// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The i.MX 95's Z1 (Verdin iMX95, Mali-G310): the SoC's part of the Mali
//! bring-up, as [`super::nanopc_t6_fw::Power`] takes it. Everything after it
//! (Z2's AS0, the CSF firmware's boot, the group) is the same code as on the
//! NanoPC-T6 ([`super::nanopc_t6_fw::fw_up_on`],
//! [`super::nanopc_t6_osal::Osal::up_on`]) over the GPU's registers alone.
//!
//! The run:
//! 1. [`gpu_power::power_on`] over the board's SCMI transport: the protocol
//!    versions, the GPU's performance levels, power domain 14 on, the level
//!    for [`gpu_power::Config::rate_hz`], clock gate 174 on and its rate read
//!    (soc::imx95::gpu_power has the messages and docs/notes/imx95-gpu-power.md
//!    the facts). The System Manager owns the domain, the clock root, the
//!    GPU mix's blk-ctrl and its reset: no SoC register is written from the
//!    A55. A refusal stops the run before any GPU register is read
//!    ([`Stop::Scmi`]); a read of the GPU with its domain off is an external
//!    abort.
//! 2. The hand-over hook, with what the System Manager answered.
//! 3. [`mali::probe`]: `GPU_ID` (the G310 the model table names), the soft
//!    reset, the identity again, the L2.
//!
//! What the T6 does and this does not: no take-over of a GPU found running
//! (the SM's POWER_STATE_SET of an already-on domain changes nothing, so a
//! GPU left running by a kernel before us would still run; the U-Boot route
//! hands over with the domain off, docs/standalone-boot.md), no QoS words,
//! no supply (the PMICs are the SM's).

use crate::firmware::scmi::Transport;
use crate::gpu::mali::{self, Ident, L2On, ProbeStop, Reset};
use crate::mmio::Mmio;
use crate::soc::imx95::gpu_power::{self, Config, Error, Up};

use super::nanopc_t6_fw::{fw_up_on, FwUp, Power, STEP_CLEAN, STEP_NONE, STEP_VERSION};
use super::nanopc_t6_sections::{Backing, McuMem, Placement};
use crate::gpu::mali::pgtable::Table;
use crate::gpu::mali::{csf, fw};

/// The GPU's register block (imx95.dtsi `gpu@4d900000`, NXP's
/// `GPU_BASE`), and its size there. The doorbells are at 0x80000 and up, as
/// on the RK3588.
pub const GPU_BASE: usize = 0x4d90_0000;
pub const GPU_SIZE: usize = 0x48_0000;
/// The least GPU mapping the bring-up and the OSAL need: the register pages
/// and the doorbells up to the one group's (CSG slot 0 rings doorbell 1,
/// `csf::doorbell(1)` = 0x90000). `yd_imx95_osal_up` refuses a shorter
/// one before it sends anything.
pub const GPU_MAP_MIN: usize = crate::gpu::mali::csf::doorbell(2);

/// The GIC SPIs of the GPU node (imx95.dtsi: "job", "mmu", "gpu"). The
/// OSAL polls; nothing here takes them.
pub const IRQ_JOB: u32 = 289;
pub const IRQ_MMU: u32 = 290;
pub const IRQ_GPU: u32 = 288;

/// Which step stopped the run. The GPU's own stops keep the NanoPC-T6's
/// numbers (`YD_T6_GPU_STOP_UNSUPPORTED_GPU`, `_RESET`, `_L2`), so the
/// OSAL's line names them alike; the SCMI power-on is
/// `YD_IMX95_GPU_STOP_SCMI`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u32)]
pub enum Stop {
    /// Every step ran: the GPU on, `GPU_ID` read, reset done, L2 ready.
    #[default]
    None = 0,
    /// `GPU_ID` names no GPU the Mali kernel side takes (the detail).
    UnsupportedGpu = 6,
    /// The soft reset did not complete (`refusal` the [`mali::MaliRefusal`]).
    Reset = 7,
    /// The L2 did not come up (`refusal` the [`mali::MaliRefusal`]).
    L2 = 8,
    /// The SCMI power-on stopped: `refusal` and `detail` are
    /// [`scmi_error_code`]'s; [`GpuUp::power_error`] is the error.
    Scmi = 15,
}

/// [`gpu_power::Error`] as a refusal number and the value it is about:
/// 1 transport (detail: the step << 32 | the transport's code), 2 the SM
/// answered a non-zero status (the step << 32 | the status), 3 the reply's
/// header is not the request's (the step << 32 | the header read), 4 a short
/// reply (the step << 32 | its length), 5 a protocol major this code was not
/// written for (the step << 32 | the version), 6 a GET that does not read
/// back the SET (the step << 32 | the value read), 7 the levels' reply
/// (returned << 32 | remaining), 8 no level at or below the rate (the rate
/// asked for, Hz), 9 the gate's rate is 0 or above the level's (the rate
/// read, Hz). The step is [`step_code`]'s.
pub fn scmi_error_code(e: &Error) -> (u32, u64) {
    let at = |s: gpu_power::Step, v: u32| u64::from(step_code(s)) << 32 | u64::from(v);
    match *e {
        Error::Transport { step, code } => (1, at(step, code as u32)),
        Error::Refused { step, status } => (2, at(step, status.code() as u32)),
        Error::Echo { step, got, .. } => (3, at(step, got)),
        Error::Length { step, len } => (4, at(step, len as u32)),
        Error::Version { step, version } => (5, at(step, version)),
        Error::Readback { step, got, .. } => (6, at(step, got)),
        Error::Levels { returned, remaining } => (7, u64::from(returned) << 32 | u64::from(remaining)),
        Error::NoLevel { rate_hz } => (8, rate_hz),
        Error::Rate { got_hz, .. } => (9, got_hz),
    }
}

/// The SCMI message a step sends, as one number: protocol << 8 | message.
pub const fn step_code(s: gpu_power::Step) -> u32 {
    let (p, m) = s.ids();
    (p as u32) << 8 | m as u32
}

/// Everything the run read, in the order it read it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GpuUp {
    /// What the System Manager did: the level set and the gate's rate.
    pub power: Option<Up>,
    /// Where the SCMI power-on stopped ([`Stop::Scmi`]).
    pub power_error: Option<Error>,
    pub ident: Ident,
    pub reset: Reset,
    /// After the reset.
    pub ident_after_reset: Ident,
    pub l2: L2On,
    pub stop: Stop,
    /// The refusal number of the step that stopped, 0 otherwise.
    pub refusal: u32,
    /// The value the refusal is about.
    pub detail: u64,
}

impl GpuUp {
    fn stopped(&mut self, stop: Stop, refusal: u32, detail: u64) {
        self.stop = stop;
        self.refusal = refusal;
        self.detail = detail;
    }
}

/// The i.MX 95's power step over the System Manager's SCMI channel `t`.
pub struct Imx95Power<T: Transport> {
    pub scmi: T,
    pub cfg: Config,
}

impl<T: Transport> Imx95Power<T> {
    pub const fn new(scmi: T, cfg: Config) -> Self {
        Imx95Power { scmi, cfg }
    }
}

impl<T: Transport> Power for Imx95Power<T> {
    type Record = GpuUp;

    fn up(
        &mut self,
        m: &impl Mmio,
        gpu: usize,
        delay_us: &mut impl FnMut(u32),
        on_handover: &mut impl FnMut(&GpuUp),
    ) -> GpuUp {
        let mut o = GpuUp::default();
        match gpu_power::power_on(&mut self.scmi, &self.cfg) {
            Ok(up) => o.power = Some(up),
            Err(e) => {
                let (refusal, detail) = scmi_error_code(&e);
                o.power_error = Some(e);
                o.stopped(Stop::Scmi, refusal, detail);
                return o;
            }
        }
        on_handover(&o);

        // the GPU's own part: the GPU's registers only from here on
        let mut p = mali::Probe::default();
        let r = mali::probe(m, gpu, delay_us, &mut p);
        (o.ident, o.reset, o.ident_after_reset, o.l2) = (p.ident, p.reset, p.ident_after_reset, p.l2);
        match r {
            Ok(_) => {}
            Err(ProbeStop::Unsupported(_)) => o.stopped(Stop::UnsupportedGpu, 0, u64::from(o.ident.gpu_id)),
            Err(ProbeStop::Reset(why, raw)) => o.stopped(Stop::Reset, why as u32, u64::from(raw)),
            Err(ProbeStop::L2(why, v)) => o.stopped(Stop::L2, why as u32, v),
        }
        o
    }

    fn ident(g: &GpuUp) -> Ident {
        g.ident_after_reset
    }

    fn stopped(g: &GpuUp) -> Option<(u32, u32, u64)> {
        (g.stop != Stop::None).then_some((g.stop as u32, g.refusal, g.detail))
    }
}

/// CSG 0's control block as the firmware wrote it (the pre-study's B3:
/// the suspend sizes the group's buffers need), read back after
/// [`fw_up`]'s boot through the interface ([`csf::discover`], reads only).
/// The C side's `struct yd_imx95_fw_csg0`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Csg0 {
    /// 1 when the words below were read.
    pub read: u32,
    pub features: u32,
    pub suspend_size: u32,
    pub protm_suspend_size: u32,
    pub stream_num: u32,
    /// When not read: the [`csf::CsfRefusal`] of the discovery (or 0 when
    /// the boot had not run) and the value it is about.
    pub refusal: u32,
    pub detail: u32,
}

/// The pre-study's B3 on the i.MX 95: Z1 over SCMI ([`Imx95Power`]), Z2's
/// AS0 and the firmware's boot, as [`fw_up_on`] runs them on the NanoPC-T6
/// (the arguments are its), then CSG 0's control block read back
/// ([`Csg0`]) once the boot ran. Nothing is started after the `PING`: no
/// group, no queue.
#[allow(clippy::too_many_arguments)]
pub fn fw_up<T: Transport>(
    m: &impl Mmio,
    gpu: usize,
    power: &mut Imx95Power<T>,
    blob: &[u8],
    backings: &[Backing],
    pool: &mut [Table],
    tables_pa: u64,
    timer_hz: u32,
    clean: &impl Fn(usize, usize) -> i32,
    delay_us: &mut impl FnMut(u32),
    on_handover: &mut impl FnMut(&GpuUp),
    z1: &mut GpuUp,
) -> (FwUp, Csg0) {
    let o = fw_up_on(m, gpu, power, blob, backings, pool, tables_pa, timer_hz, clean, delay_us, on_handover, z1);
    let booted = matches!(o.step, STEP_NONE | STEP_CLEAN | STEP_VERSION);
    (o, if booted { csg0(m, blob, backings, clean) } else { Csg0::default() })
}

/// CSG 0's control block, through the interface the boot found.
fn csg0(m: &impl Mmio, blob: &[u8], backings: &[Backing], clean: &impl Fn(usize, usize) -> i32) -> Csg0 {
    let Ok(img) = fw::parse(blob) else { return Csg0::default() };
    let Ok(placement) = Placement::new(img.sections(), backings) else { return Csg0::default() };
    let mem = McuMem::new(&placement, clean);
    match csf::discover(m, &mem, &img).and_then(|i| i.csg(m, 0)) {
        Ok(g) => Csg0 {
            read: 1,
            features: g.features,
            suspend_size: g.suspend_size,
            protm_suspend_size: g.protm_suspend_size,
            stream_num: g.stream_num,
            ..Csg0::default()
        },
        Err((why, v)) => Csg0 { refusal: why as u32, detail: v, ..Csg0::default() },
    }
}

#[cfg(test)]
mod tests;
