// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Z3 on the NanoPC-T6 (driver/docs/nanopc-t6-mali.md, "The firmware
//! (Z3)"): the firmware booted from our side, in one run. The image is
//! parsed ([`fw::parse`]) and each loaded section placed on the caller's
//! backing ([`Placement`]) before anything is touched; then Z1's power-on
//! ([`gpu_up`]), Z2's AS0 over every section ([`mmu_up_sections`]), and
//! [`csf::boot`] through the board's [`McuMem`]: the load (data, zeroes,
//! the clean of each section), the MCU's start, the interface, the global
//! configuration and its acknowledge, one `PING`. The run then checks that
//! the firmware answers the interface version its own header names
//! (0x01050000, 1.5.0, on the board's file).
//!
//! What it read is one [`FwUp`], `#[repr(C)]` and of 32-bit words only, so
//! the C side takes it as it is; the first step that refuses ends the run
//! there and names itself in `step`. Z1's and Z2's own records stay with
//! their probes (`t6_gpu`, `t6_mmu`): here they give their stop, refusal
//! and value, and the few values Z3 depends on.

use super::nanopc_t6_gpu::{self, gpu_up, Bases, GpuUp};
use super::nanopc_t6_mmu;
use super::nanopc_t6_sections::{mmu_up_sections, Backing, McuMem, Placement};
use crate::gpu::mali::csf::{self, GlbInput, GlbOutput, GlobalConfig, GLB_INIT_TOGGLES, GLB_PING};
use crate::gpu::mali::fw::{self, FwError};
use crate::gpu::mali::pgtable::Table;
use crate::gpu::mali::MCU_STATUS;
use crate::mmio::Mmio;
use core::mem::offset_of;

/// Which step stopped the run ([`FwUp::step`]), in the run's order.
pub const STEP_NONE: u32 = 0;
/// [`fw::parse`] refused the image (`refusal`: [`fw_error_code`]).
pub const STEP_PARSE: u32 = 1;
/// [`Placement::new`] refused the backings (`refusal`: the
/// [`super::nanopc_t6_sections::SectionRefusal`]).
pub const STEP_PLACE: u32 = 2;
/// Z1's power-on stopped (`stop`: its [`nanopc_t6_gpu::Stop`]).
pub const STEP_GPU: u32 = 3;
/// Z2's AS0 stopped (`stop`: its [`nanopc_t6_mmu::Stop`]).
pub const STEP_MMU: u32 = 4;
/// [`csf::boot`] stopped (`refusal`: the [`csf::CsfRefusal`]).
pub const STEP_BOOT: u32 = 5;
/// The boot ran, but a clean of a loaded section failed (`detail`: the
/// first rc that was not 0, also in `clean_rc`).
pub const STEP_CLEAN: u32 = 6;
/// The boot ran, but the GLB version is not the image header's version
/// hash (`detail`: the version read; `image_version` the header's).
pub const STEP_VERSION: u32 = 7;

/// [`fw::parse`]'s refusal as a number, in [`FwError`]'s order, with the
/// value it carries (an offset, a size, a word; 0 for none).
pub fn fw_error_code(e: &FwError) -> (u32, u64) {
    match *e {
        FwError::TooSmall { len } => (1, len as u64),
        FwError::BadMagic(m) => (2, u64::from(m)),
        FwError::UnsupportedMajor(v) => (3, u64::from(v)),
        FwError::Truncated { size, .. } => (4, u64::from(size)),
        FwError::Unaligned { offset, .. } => (5, u64::from(offset)),
        FwError::BadEntrySize { offset, .. } => (6, u64::from(offset)),
        FwError::ShortSection { offset } => (7, u64::from(offset)),
        FwError::DataReversed { offset } => (8, u64::from(offset)),
        FwError::VaReversed { offset } => (9, u64::from(offset)),
        FwError::DataOutsideFile { offset, .. } => (10, u64::from(offset)),
        FwError::VaUnaligned { offset } => (11, u64::from(offset)),
        FwError::UnsupportedFlags { offset, .. } => (12, u64::from(offset)),
        FwError::SharedNotShared { offset } => (13, u64::from(offset)),
        FwError::DataLargerThanVa { offset } => (14, u64::from(offset)),
        FwError::Overlap { offset } => (15, u64::from(offset)),
        FwError::TooManySections => (16, 0),
        FwError::UnknownEntry { offset, .. } => (17, u64::from(offset)),
        FwError::NoSharedSection => (18, 0),
    }
}

fn lo_hi(v: u64) -> [u32; 2] {
    [v as u32, (v >> 32) as u32]
}

/// What [`fw_up`] read, in 32-bit words (a 64-bit value is lo, hi; an
/// interface address is the MCU address). The C side's `struct yd_t6_fw_up`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FwUp {
    /// `STEP_*` of the step that stopped, [`STEP_NONE`] when all ran.
    pub step: u32,
    /// For [`STEP_GPU`] and [`STEP_MMU`], that run's own `Stop`; 0 otherwise.
    pub stop: u32,
    /// The refusal number of the step (or of its `stop`), 0 for none.
    pub refusal: u32,
    /// The value the refusal is about (lo, hi).
    pub detail: [u32; 2],

    /// The image: the header's version hash, the sections loaded and the
    /// protected ones skipped.
    pub image_version: u32,
    pub sections: u32,
    pub protected_skipped: u32,

    /// Z1: `GPU_ID`, `SHADER_PRESENT` (lo, hi) and `MMU_FEATURES` after the
    /// reset.
    pub gpu_id: u32,
    pub shader_present: [u32; 2],
    pub mmu_features: u32,

    /// Z2: the table pages used, the flushes after the map (one per
    /// section), the shared section's backing (lo, hi).
    pub tables_used: u32,
    pub flushed: u32,
    pub shared_pa: [u32; 2],

    /// The load: 0, or the first failed clean's rc.
    pub clean_rc: i32,

    /// The start: `MCU_STATUS` before it, the polls until `JOB_INT` bit 31,
    /// `JOB_INT_RAWSTAT` then, and `MCU_STATUS` then.
    pub mcu_status_before: u32,
    pub boot_waited_us: u32,
    pub job_raw: u32,
    pub mcu_status: u32,

    /// The GLB control block: `version`, `features`, `instr_features`,
    /// `group_num`, `group_stride`, the input and output pages.
    pub version: u32,
    pub features: u32,
    pub instr_features: u32,
    pub group_num: u32,
    pub group_stride: u32,
    pub glb_input: u32,
    pub glb_output: u32,
    /// CSG 0: input, output, `stream_num`, `stream_stride`; its CS 0's
    /// input, output and `features`.
    pub csg0: [u32; 4],
    pub cs0: [u32; 3],

    /// The global configuration written: `core_en_mask` (lo, hi),
    /// `poweroff_timer`, `progress_timer`, `idle_timer`, `ack_irq_mask`,
    /// and the GLB `req` word after its toggles.
    pub core_en_mask: [u32; 2],
    pub poweroff_timer: u32,
    pub progress_timer: u32,
    pub idle_timer: u32,
    pub ack_irq_mask: u32,
    pub init_req: u32,

    /// The acknowledges: of `GLB_INIT_TOGGLES | GLB_PING`, the bits where
    /// `req` and `ack` agree; the polls until the configuration's toggles
    /// were acknowledged; the `PING` acknowledged (1) or not (0), and its
    /// polls; the GLB `req` and `ack` words at the end.
    pub acked: u32,
    pub init_ack_us: u32,
    pub ping_acked: u32,
    pub ping_ack_us: u32,
    pub glb_req: u32,
    pub glb_ack: u32,
}

const _: () = assert!(size_of::<FwUp>() == 4 * 48);
const _: () = assert!(offset_of!(FwUp, image_version) == 4 * 5);
const _: () = assert!(offset_of!(FwUp, gpu_id) == 4 * 8);
const _: () = assert!(offset_of!(FwUp, tables_used) == 4 * 12);
const _: () = assert!(offset_of!(FwUp, clean_rc) == 4 * 16);
const _: () = assert!(offset_of!(FwUp, version) == 4 * 21);
const _: () = assert!(offset_of!(FwUp, csg0) == 4 * 28);
const _: () = assert!(offset_of!(FwUp, core_en_mask) == 4 * 35);
const _: () = assert!(offset_of!(FwUp, acked) == 4 * 42);
const _: () = assert!(offset_of!(FwUp, glb_ack) == 4 * 47);

impl FwUp {
    fn stopped(&mut self, step: u32, stop: u32, refusal: u32, detail: u64) {
        self.step = step;
        self.stop = stop;
        self.refusal = refusal;
        self.detail = lo_hi(detail);
    }
}

/// The run (see the module). `m` reaches the registers at the bases of `b`
/// (the GPU's mapping covering its doorbells, 0x80000 and up) and the
/// backings at their CPU addresses. `blob` is `mali_csffw.bin`;
/// `backings[i]` is the memory of the `i`-th section [`fw::parse`] keeps,
/// in the file's order (on the board's file: 0x00400000, 0x00410000,
/// 0x00000000, 0x00800000, 0x02000000, 0x01000000, 0x04000000), the shared
/// section's mapped uncached for the CPU. `pool` is the image's table pages
/// at physical `tables_pa` (9 of them for the board's file). `timer_hz` is
/// the arch timer's rate ([`csf::RK3588_TIMER_HZ`]). `clean` cleans the data
/// cache over `(address, length)` to the point of coherency and returns 0
/// (`sys_cache_data_flush_range`), for the tables and for each loaded
/// section. `delay_us` waits; `on_handover` is Z1's, called before the first
/// PMU or GPU write.
#[allow(clippy::too_many_arguments)]
pub fn fw_up(
    m: &impl Mmio,
    b: Bases,
    blob: &[u8],
    backings: &[Backing],
    pool: &mut [Table],
    tables_pa: u64,
    timer_hz: u32,
    clean: &impl Fn(usize, usize) -> i32,
    delay_us: &mut impl FnMut(u32),
    on_handover: &mut impl FnMut(&GpuUp),
) -> FwUp {
    let mut o = FwUp::default();
    let img = match fw::parse(blob) {
        Ok(img) => img,
        Err(e) => {
            let (code, v) = fw_error_code(&e);
            o.stopped(STEP_PARSE, 0, code, v);
            return o;
        }
    };
    o.image_version = img.header.version_hash;
    o.sections = img.sections().len() as u32;
    o.protected_skipped = img.protected_skipped;
    let placement = match Placement::new(img.sections(), backings) {
        Ok(p) => p,
        Err((why, v)) => {
            o.stopped(STEP_PLACE, 0, why as u32, u64::from(v));
            return o;
        }
    };

    let g = gpu_up(m, b, delay_us, on_handover);
    o.gpu_id = g.ident_after_reset.gpu_id;
    o.shader_present = lo_hi(g.ident_after_reset.shader_present);
    o.mmu_features = g.ident_after_reset.mmu_features;
    if g.stop != nanopc_t6_gpu::Stop::None {
        o.stopped(STEP_GPU, g.stop as u32, g.refusal, g.detail);
        return o;
    }

    let s = mmu_up_sections(m, b.gpu, o.mmu_features, pool, tables_pa, &placement, &mut |a, n| clean(a, n), delay_us);
    o.tables_used = s.up.tables_used;
    o.flushed = s.flushed;
    o.shared_pa = lo_hi(s.up.backing_pa);
    if s.up.stop != nanopc_t6_mmu::Stop::None {
        o.stopped(STEP_MMU, s.up.stop as u32, s.up.refusal, s.up.detail);
        return o;
    }

    let mem = McuMem::new(&placement, clean);
    let cfg = GlobalConfig { shader_present: g.ident_after_reset.shader_present, timer_hz };
    let booted = csf::boot(m, &mem, b.gpu, &img, &cfg, delay_us);
    o.clean_rc = mem.clean_rc();
    let r = match booted {
        Ok(r) => r,
        Err((why, v)) => {
            o.stopped(STEP_BOOT, 0, why as u32, u64::from(v));
            // what the acknowledge waits saw
            o.acked = match why {
                csf::CsfRefusal::GlbConfigAckTimeout => v,
                csf::CsfRefusal::PingTimeout => GLB_INIT_TOGGLES | v,
                _ => 0,
            };
            o.mcu_status = m.read32(b.gpu + MCU_STATUS);
            return o;
        }
    };

    let i = &r.iface;
    let va = |cpu: usize| i.shared.va.wrapping_add((cpu - i.shared.cpu) as u32);
    o.mcu_status_before = r.start.mcu_status_before;
    o.boot_waited_us = r.start.waited_us;
    o.job_raw = r.start.job_raw;
    o.mcu_status = r.start.mcu_status;
    o.version = i.version;
    o.features = i.features;
    o.instr_features = i.instr_features;
    o.group_num = i.group_num;
    o.group_stride = i.group_stride;
    o.glb_input = va(i.glb_input);
    o.glb_output = va(i.glb_output);
    o.csg0 = [va(r.csg0.input), va(r.csg0.output), r.csg0.stream_num, r.csg0.stream_stride];
    o.cs0 = [va(r.cs0.input), va(r.cs0.output), r.cs0.features];
    o.core_en_mask = lo_hi(r.init.core_en_mask);
    o.poweroff_timer = r.init.poweroff_timer;
    o.progress_timer = r.init.progress_timer;
    o.idle_timer = r.init.idle_timer;
    o.ack_irq_mask = r.init.ack_irq_mask;
    o.init_req = r.init.req;
    let (req, ack) = (i.glb_input + offset_of!(GlbInput, req), i.glb_output + offset_of!(GlbOutput, ack));
    o.acked = csf::acked(m, req, ack, GLB_INIT_TOGGLES | GLB_PING);
    o.init_ack_us = r.init_ack_us;
    o.ping_acked = 1;
    o.ping_ack_us = r.ping_ack_us;
    o.glb_req = m.read32(req);
    o.glb_ack = m.read32(ack);

    if o.clean_rc != 0 {
        o.stopped(STEP_CLEAN, 0, 0, o.clean_rc as u64);
    } else if o.version != o.image_version {
        o.stopped(STEP_VERSION, 0, 0, u64::from(o.version));
    }
    o
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::gpu::mali::csf::tests::{Mcu, GPU, MEM};
    use crate::gpu::mali::csf::{
        cs_features_scoreboards, cs_features_work_regs, iface_version, CsfRefusal, GLB_CFG_ALLOC_EN,
        GLB_CFG_POWEROFF_TIMER, GLB_CFG_PROGRESS_TIMER, GLB_IDLE_EN, JOB_INT_GLOBAL_IF, MCU_CONTROL, MCU_CONTROL_AUTO,
        RK3588_TIMER_HZ,
    };
    use crate::gpu::mali::fw::{synth, MAGIC};
    use crate::gpu::mali::mmu::tests::Gpu;
    use crate::gpu::mali::mmu::{mmu_as, AS_COMMAND, AS_COMMAND_UNLOCK, AS_COMMAND_UPDATE};
    use crate::gpu::mali::pgtable::{PtRefusal, ENTRIES, PAGE};
    use crate::gpu::mali::{self, CMD_SOFT_RESET, GPU_CMD, GPU_INT_CLEAR, GPU_INT_RAWSTAT, IRQ_RESET_COMPLETED};
    use crate::ip::rockchip_spi;
    use crate::soc::rk3588::{self, clkgate_con, clksel_con, pd_gpu, pll_con, CRU_BASE, MODE_CON0, PMU_BASE};
    use std::cell::RefCell;
    use std::collections::{HashMap, VecDeque};

    const SPI: usize = rk3588::SPI2_BASE;
    const QOS: usize = rk3588::QOS_GPU[0];
    pub(crate) const B: Bases = Bases { cru: CRU_BASE, pmu: PMU_BASE, spi2: SPI, qos: QOS, gpu: GPU };
    pub(crate) const TABLES_PA: u64 = 0x1040_0000;

    /// What the board saw, in order: a register write, or a clean.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Ev {
        W(usize, u32),
        Clean(usize, usize),
    }

    /// The T6 for the whole of Z3: the SoC as Armbian hands it over with the
    /// GPU runtime-suspended (the CRU and PMU with their hiword masks, SPI2
    /// with the RK806 behind it, the GPU QoS blocks: nanopc_t6_gpu's model),
    /// the GPU's identity, soft reset and L2; its MMU and cache flush
    /// (`mmu::tests::Gpu`); and the MCU with the memory behind AS0
    /// (`csf::tests::Mcu`: an MCU address `va` is the CPU's `MEM + va`), which
    /// boots only from code that was loaded and cleaned.
    pub(crate) struct T6 {
        soc: RefCell<HashMap<usize, u32>>,
        rk806: HashMap<u16, u8>,
        frame: RefCell<Vec<u8>>,
        rxq: RefCell<VecDeque<u8>>,
        l2_on: RefCell<bool>,
        reset_done: RefCell<bool>,
        mmu: Gpu,
        mcu: Mcu,
        log: RefCell<Vec<Ev>>,
        /// The rc the clean gives for a range at this CPU address.
        clean_fails: Option<(usize, i32)>,
    }

    impl T6 {
        pub(crate) fn handed_over(blob: Vec<u8>) -> T6 {
            let mut r = HashMap::new();
            for (a, v) in [
                (CRU_BASE + clksel_con(158), 0x1f24),
                (CRU_BASE + clksel_con(159), 0),
                (CRU_BASE + clkgate_con(66), 0xd6),
                (CRU_BASE + clkgate_con(14), 0),
                (CRU_BASE + MODE_CON0, 0x545),
                (CRU_BASE + pll_con(104), 0xfa),
                (CRU_BASE + pll_con(105), 0x42),
                (CRU_BASE + pll_con(106), 0),
                (PMU_BASE + pd_gpu::PWR_GATE, 0xffff),
                (PMU_BASE + pd_gpu::STATUS, 0x2455_ffff),
                (PMU_BASE + pd_gpu::BUS_IDLE_REQ, 0x83cf),
                (PMU_BASE + pd_gpu::BUS_IDLE_ACK, 0x000a_9fff),
                (PMU_BASE + pd_gpu::BUS_IDLE_ST, 0x000a_9fff),
                (PMU_BASE + pd_gpu::REPAIR_STATUS, 0xfad5_0001),
                (GPU + mali::GPU_ID, mali::G610_GPU_ID),
                (GPU + mali::GPU_MMU_FEATURES, 0x2830),
                (GPU + mali::GPU_L2_PRESENT, 1),
                (GPU + mali::GPU_SHADER_PRESENT, 0x50005),
            ] {
                r.insert(a, v);
            }
            T6 {
                soc: RefCell::new(r),
                rk806: [(0x00, 0x0f), (0x06, 0xe0), (0x10, 0xa4), (0x1a, 0x1c)].into_iter().collect(),
                frame: RefCell::new(vec![]),
                rxq: RefCell::new(VecDeque::new()),
                l2_on: RefCell::new(false),
                reset_done: RefCell::new(false),
                mmu: Gpu::new(1),
                mcu: Mcu::new(blob),
                log: RefCell::new(vec![]),
                clean_fails: None,
            }
        }
        fn gpu_powered(&self) -> bool {
            let r = self.soc.borrow();
            r[&(PMU_BASE + pd_gpu::REPAIR_STATUS)] & 2 != 0 && r[&(PMU_BASE + pd_gpu::BUS_IDLE_ST)] & 1 == 0
        }
        /// The MCU's registers (its control and status, the job interrupt,
        /// the doorbells) and the sections' memory.
        fn is_mcu(a: usize) -> bool {
            let o = a.wrapping_sub(GPU);
            a >= MEM
                || Self::is_gpu(a)
                    && (o == MCU_CONTROL || o == mali::MCU_STATUS || (0x1000..0x1010).contains(&o) || o >= 0x8_0000)
        }
        /// The GPU's registers and its 32 doorbell pages.
        fn is_gpu(a: usize) -> bool {
            (GPU..GPU + 0x28_0000).contains(&a)
        }
        fn writes(&self) -> Vec<(usize, u32)> {
            self.log.borrow().iter().filter_map(|e| if let Ev::W(a, v) = *e { Some((a, v)) } else { None }).collect()
        }
        pub(crate) fn clean(&self, a: usize, n: usize) -> i32 {
            self.log.borrow_mut().push(Ev::Clean(a, n));
            // the model boots from code it saw cleaned, failed or not
            self.mcu.cleaned.borrow_mut().push((a, n));
            match self.clean_fails {
                Some((at, rc)) if at == a => rc,
                _ => 0,
            }
        }
    }

    impl Mmio for T6 {
        fn read32(&self, a: usize) -> u32 {
            if Self::is_gpu(a) || (QOS..QOS + 0x800).contains(&a) {
                assert!(self.gpu_powered(), "read of {a:#x} with PD_GPU off: an external abort on the board");
            }
            if Self::is_mcu(a) {
                return self.mcu.read32(a);
            }
            match a {
                _ if a == SPI + rockchip_spi::RXFLR => self.rxq.borrow().len() as u32,
                _ if a == SPI + rockchip_spi::RXDR => u32::from(self.rxq.borrow_mut().pop_front().unwrap()),
                _ if a == GPU + mali::L2_READY => u32::from(*self.l2_on.borrow()),
                _ if a == GPU + GPU_INT_RAWSTAT => {
                    self.mmu.read32(a) | if *self.reset_done.borrow() { IRQ_RESET_COMPLETED } else { 0 }
                }
                _ if (GPU + 0x2000..GPU + 0x3000).contains(&a) => self.mmu.read32(a),
                _ => *self.soc.borrow().get(&a).unwrap_or(&0),
            }
        }
        fn write32(&self, a: usize, v: u32) {
            if a < MEM {
                self.log.borrow_mut().push(Ev::W(a, v));
            }
            if Self::is_mcu(a) {
                return self.mcu.write32(a, v);
            }
            if a == SPI + rockchip_spi::TXDR {
                let mut f = self.frame.borrow_mut();
                f.push(v as u8);
                // the RK806 answers the value in the fourth byte of a 1-value read
                let out = if f.len() == 4 {
                    assert_eq!(f[0], 0, "only 1-value reads, never a write: {f:02x?}");
                    self.rk806[&u16::from_le_bytes([f[1], f[2]])]
                } else {
                    0
                };
                self.rxq.borrow_mut().push_back(out);
                return;
            }
            if a == SPI + rockchip_spi::SER && v == 0 {
                self.frame.borrow_mut().clear();
            }
            // receive only: the frames after the command, MOSI idle
            if a == SPI + rockchip_spi::SSIENR
                && v == 1
                && self.soc.borrow().get(&(SPI + rockchip_spi::CTRLR0)).copied().unwrap_or(0)
                    & rockchip_spi::CTRLR0_XFM_RO
                    != 0
            {
                let f = self.frame.borrow().clone();
                assert_eq!((f.len(), f[0]), (3, 0), "a 1-value read's command: {f:02x?}");
                self.rxq.borrow_mut().push_back(self.rk806[&u16::from_le_bytes([f[1], f[2]])]);
            }
            match a {
                _ if a == GPU + mali::L2_PWRON && v & 1 != 0 => *self.l2_on.borrow_mut() = true,
                // a soft reset completes at once
                _ if a == GPU + GPU_CMD && v == CMD_SOFT_RESET => *self.reset_done.borrow_mut() = true,
                _ if a == GPU + GPU_INT_CLEAR => {
                    if v & IRQ_RESET_COMPLETED != 0 {
                        *self.reset_done.borrow_mut() = false;
                    }
                    self.mmu.write32(a, v);
                }
                _ if a == GPU + GPU_CMD || (GPU + 0x2000..GPU + 0x3000).contains(&a) => self.mmu.write32(a, v),
                _ => {
                    let mut r = self.soc.borrow_mut();
                    if (CRU_BASE..CRU_BASE + 0x1000).contains(&a) || (PMU_BASE..PMU_BASE + 0x400).contains(&a) {
                        let e = r.entry(a).or_insert(0);
                        let mask = v >> 16;
                        *e = (*e & !mask) | (v & mask);
                        // PD_GPU follows its controls at once
                        if a == PMU_BASE + pd_gpu::PWR_GATE && v == 1 << 16 {
                            *r.get_mut(&(PMU_BASE + pd_gpu::REPAIR_STATUS)).unwrap() |= 2;
                        }
                        if a == PMU_BASE + pd_gpu::BUS_IDLE_REQ && v == 1 << 16 {
                            *r.get_mut(&(PMU_BASE + pd_gpu::BUS_IDLE_ACK)).unwrap() &= !1;
                            *r.get_mut(&(PMU_BASE + pd_gpu::BUS_IDLE_ST)).unwrap() &= !1;
                        }
                    } else {
                        r.insert(a, v);
                    }
                }
            }
        }
        fn read8(&self, _: usize) -> u8 {
            unreachable!()
        }
        fn write8(&self, a: usize, v: u8) {
            assert!(a >= MEM, "a byte write outside the sections: {a:#x}");
            self.mcu.write8(a, v)
        }
    }

    /// One backing per section, in the parse's order: physical memory one
    /// after the other from 0x10500000, at the CPU address the model's
    /// memory has for the section's MCU address.
    pub(crate) fn backings(blob: &[u8]) -> Vec<Backing> {
        let img = fw::parse(blob).unwrap();
        let mut pa = 0x1050_0000;
        img.sections()
            .iter()
            .map(|s| {
                let b = Backing { pa, cpu: MEM + s.va_start as usize, size: u64::from(s.va_size()) };
                pa += u64::from(s.va_size()).next_multiple_of(PAGE);
                b
            })
            .collect()
    }

    fn run_in(t6: &T6, blob: &[u8], backings: &[Backing], pool: &mut [Table]) -> FwUp {
        fw_up(t6, B, blob, backings, pool, TABLES_PA, RK3588_TIMER_HZ, &|a, n| t6.clean(a, n), &mut |_| {}, &mut |_| {})
    }

    fn run_with(t6: &T6, blob: &[u8], backings: &[Backing], pages: usize) -> FwUp {
        run_in(t6, blob, backings, &mut vec![[0u64; ENTRIES]; pages])
    }

    fn run(t6: &T6, blob: &[u8]) -> FwUp {
        run_with(t6, blob, &backings(blob), 16)
    }

    /// The design page's predictions for the boot, as the record holds them.
    fn assert_booted(o: &FwUp) {
        assert_eq!((o.step, o.stop, o.refusal, o.detail), (STEP_NONE, 0, 0, [0, 0]));
        assert_eq!((o.version, o.image_version), (0x0105_0000, 0x0105_0000));
        assert_eq!((o.features, o.instr_features), (0, 0x71));
        assert_eq!((o.group_num, o.group_stride, o.glb_input, o.glb_output), (8, 0xa0, 0x0400_2000, 0x0400_6000));
        assert_eq!(o.csg0, [0x0400_3000, 0x0400_7000, 8, 0xc]);
        assert_eq!(&o.cs0[..2], &[0x0400_3400, 0x0400_7200]);
        assert_eq!((cs_features_work_regs(o.cs0[2]), cs_features_scoreboards(o.cs0[2])), (96, 8));
        assert_eq!((o.mcu_status_before, o.job_raw, o.mcu_status), (0, JOB_INT_GLOBAL_IF, 1));
        assert_eq!(o.core_en_mask, [0x50005, 0]);
        assert_eq!((o.poweroff_timer, o.progress_timer, o.idle_timer), (235, 0x27_1000, 19));
        assert_eq!((o.ack_irq_mask, o.init_req), (0x0400_050e, 0x40e));
        // the configuration's three toggles and the PING, acknowledged
        assert_eq!(o.acked, GLB_CFG_ALLOC_EN | GLB_CFG_POWEROFF_TIMER | GLB_CFG_PROGRESS_TIMER | GLB_PING);
        assert_eq!(o.ping_acked, 1);
        assert_eq!((o.glb_req, o.glb_ack), (GLB_IDLE_EN | GLB_INIT_TOGGLES | GLB_PING, 0x50e));
        assert_eq!(o.clean_rc, 0);
    }

    #[test]
    fn z3_boots_version_1_5_0_and_the_ping_is_acknowledged() {
        let blob = synth::small();
        let t6 = T6::handed_over(blob.clone());
        let o = run(&t6, &blob);
        assert_booted(&o);
        assert_eq!((o.sections, o.protected_skipped), (4, 1));
        assert_eq!((o.gpu_id, o.shader_present, o.mmu_features), (0xa867_0005, [0x50005, 0], 0x2830));
        // 1 L0, 1 L1, 1 L2, an L3 for each of the four sections
        assert_eq!((o.tables_used, o.flushed), (7, 4));
        // the shared section, the last: after 128 + 64 + 256 KiB
        assert_eq!(o.shared_pa, [0x1057_0000, 0]);
        assert_eq!(o.boot_waited_us, 3);
    }

    #[test]
    fn the_steps_run_in_z3_s_order() {
        let blob = synth::small();
        let img = fw::parse(&blob).unwrap();
        let t6 = T6::handed_over(blob.clone());
        let mut pool = vec![[0u64; ENTRIES]; 16];
        assert_eq!(run_in(&t6, &blob, &backings(&blob), &mut pool).step, STEP_NONE);
        let log = t6.log.borrow().clone();
        let at = |e: &dyn Fn(&Ev) -> bool| log.iter().position(e).unwrap();
        // Z1: the L2 on; Z2: the tables cleaned, AS0's UPDATE, the last
        // FLUSH_PT's UNLOCK; the load: one clean per section; the start; the
        // configuration's doorbell
        let as0 = GPU + mmu_as(0) + AS_COMMAND;
        let l2 = at(&|e| matches!(e, Ev::W(a, _) if *a == GPU + mali::L2_PWRON));
        let tables = at(&|e| *e == Ev::Clean(pool.as_ptr() as usize, 7 * 4096));
        let update = at(&|e| *e == Ev::W(as0, AS_COMMAND_UPDATE));
        let unlock = log.iter().rposition(|e| *e == Ev::W(as0, AS_COMMAND_UNLOCK)).unwrap();
        let sections: Vec<usize> = img
            .sections()
            .iter()
            .map(|s| at(&|e| *e == Ev::Clean(MEM + s.va_start as usize, s.va_size() as usize)))
            .collect();
        let start = at(&|e| *e == Ev::W(GPU + MCU_CONTROL, MCU_CONTROL_AUTO));
        let ring = at(&|e| *e == Ev::W(GPU + csf::doorbell(0), 1));
        assert!(l2 < tables && tables < update && update < unlock, "{log:x?}");
        assert!(sections.iter().all(|&c| unlock < c && c < start), "{log:x?}");
        assert!(sections.is_sorted());
        assert!(start < ring);
        // between the last clean and the start only the job interrupt is
        // touched (its clear): no GPU flush, no MMU command
        let last_clean = sections[sections.len() - 1];
        assert!(
            log[last_clean + 1..start]
                .iter()
                .all(|e| matches!(e, Ev::W(a, _) if (0x1000..0x1010).contains(&(a - GPU)))),
            "{log:x?}"
        );
        // the sections' first words are the image's, where AS0 maps them
        // (the shared one's GLB control block the firmware has rewritten)
        for s in img.sections().iter().filter(|s| !s.shared()) {
            let d = s.data(&blob);
            for (k, w) in d.as_chunks::<4>().0.iter().enumerate().take(4) {
                let want = u32::from_le_bytes(*w);
                assert_eq!(t6.mcu.word(MEM + s.va_start as usize + 4 * k), want, "{s:x?}");
            }
        }
    }

    #[test]
    fn a_missing_acknowledge_stops_at_the_boot_with_the_bits_that_were() {
        let blob = synth::small();
        let mut t6 = T6::handed_over(blob.clone());
        t6.mcu.deaf = GLB_PING;
        let o = run(&t6, &blob);
        assert_eq!((o.step, o.stop, o.refusal, o.detail), (STEP_BOOT, 0, CsfRefusal::PingTimeout as u32, [0, 0]));
        assert_eq!((o.acked, o.ping_acked), (GLB_INIT_TOGGLES, 0));
        assert_eq!(o.mcu_status, 1);

        let mut t6 = T6::handed_over(blob.clone());
        t6.mcu.deaf = GLB_CFG_POWEROFF_TIMER;
        let o = run(&t6, &blob);
        let acked = GLB_CFG_ALLOC_EN | GLB_CFG_PROGRESS_TIMER;
        assert_eq!((o.step, o.refusal, o.detail), (STEP_BOOT, CsfRefusal::GlbConfigAckTimeout as u32, [acked, 0]));
        assert_eq!((o.acked, o.ping_acked), (acked, 0));
        // the PING is never asked: one ring of the global doorbell
        assert_eq!(t6.writes().iter().filter(|w| w.0 == GPU + csf::doorbell(0)).count(), 1);
    }

    #[test]
    fn a_version_other_than_the_header_s_is_refused() {
        let blob = synth::small();
        // the firmware answers, the PING too, but with another version
        let mut t6 = T6::handed_over(blob.clone());
        t6.mcu.version = iface_version(1, 4, 0);
        let o = run(&t6, &blob);
        assert_eq!((o.step, o.refusal, o.detail), (STEP_VERSION, 0, [0x0104_0000, 0]));
        assert_eq!((o.version, o.image_version, o.ping_acked), (0x0104_0000, 0x0105_0000, 1));
        // version 0: the boot failed (csf's own refusal)
        let mut t6 = T6::handed_over(blob.clone());
        t6.mcu.version = 0;
        let o = run(&t6, &blob);
        assert_eq!((o.step, o.refusal, o.detail), (STEP_BOOT, CsfRefusal::VersionZero as u32, [0, 0]));
        assert_eq!(o.version, 0);
    }

    #[test]
    fn a_bad_placement_or_image_is_refused_before_anything_is_written() {
        let blob = synth::small();
        let good = backings(&blob);
        // the code, 128 KiB at 0x00800000, the first section: a backing too small
        let mut b = good.clone();
        b[0].size = 0x1_f000;
        let t6 = T6::handed_over(blob.clone());
        let o = run_with(&t6, &blob, &b, 16);
        assert_eq!((o.step, o.stop, o.refusal, o.detail), (STEP_PLACE, 0, 3, [0x0080_0000, 0]));
        assert_eq!((o.sections, o.image_version), (4, 0x0105_0000));
        assert!(t6.log.borrow().is_empty(), "{:x?}", t6.log.borrow());
        // two backings over the same memory; one backing short
        let mut b = good.clone();
        b[1].pa = b[0].pa;
        assert_eq!(run_with(&t6, &blob, &b, 16).refusal, 5);
        assert_eq!(run_with(&t6, &blob, &good[..3], 16).detail, [3, 0]);
        // not a firmware file
        let mut bad = blob.clone();
        bad[0] ^= 1;
        let o = run_with(&t6, &bad, &good, 16);
        assert_eq!((o.step, o.refusal, o.detail), (STEP_PARSE, 2, [MAGIC ^ 1, 0]));
        assert_eq!(o.sections, 0);
        assert!(t6.log.borrow().is_empty());
    }

    #[test]
    fn z1_and_z2_stops_are_named_with_their_own_numbers() {
        let blob = synth::small();
        let t6 = T6::handed_over(blob.clone());
        t6.soc.borrow_mut().insert(GPU + mali::GPU_ID, 0xa867_0001);
        let o = run(&t6, &blob);
        let stop = nanopc_t6_gpu::Stop::NotG610 as u32;
        assert_eq!((o.step, o.stop, o.refusal, o.detail), (STEP_GPU, stop, 0, [0xa867_0001, 0]));
        assert!(t6.writes().iter().all(|w| w.0 != GPU + MCU_CONTROL && w.0 != GPU + GPU_CMD));

        // 6 table pages, 7 needed: AS0 is never enabled, the MCU never started
        let t6 = T6::handed_over(blob.clone());
        let o = run_with(&t6, &blob, &backings(&blob), 6);
        let stop = nanopc_t6_mmu::Stop::Tables as u32;
        assert_eq!((o.step, o.stop, o.refusal), (STEP_MMU, stop, PtRefusal::PoolFull as u32));
        assert_eq!((o.gpu_id, o.tables_used), (0xa867_0005, 6));
        assert!(t6.writes().iter().all(|w| w.0 != GPU + MCU_CONTROL));
        assert!(t6.mcu.mem.borrow().is_empty(), "a section was loaded");
    }

    #[test]
    fn a_failed_clean_of_a_section_is_named_after_the_boot() {
        let blob = synth::small();
        let mut t6 = T6::handed_over(blob.clone());
        t6.clean_fails = Some((MEM + 0x0200_0000, -5));
        let o = run(&t6, &blob);
        assert_eq!((o.step, o.refusal, o.clean_rc), (STEP_CLEAN, 0, -5));
        assert_eq!(o.detail, [-5i32 as u32, u32::MAX]);
        assert_eq!((o.version, o.ping_acked), (0x0105_0000, 1));
    }

    #[test]
    fn the_record_is_48_words_at_fixed_offsets() {
        assert_eq!((size_of::<FwUp>(), align_of::<FwUp>()), (4 * 48, 4));
        let words = [
            (offset_of!(FwUp, step), 0),
            (offset_of!(FwUp, stop), 1),
            (offset_of!(FwUp, refusal), 2),
            (offset_of!(FwUp, detail), 3),
            (offset_of!(FwUp, image_version), 5),
            (offset_of!(FwUp, sections), 6),
            (offset_of!(FwUp, protected_skipped), 7),
            (offset_of!(FwUp, gpu_id), 8),
            (offset_of!(FwUp, shader_present), 9),
            (offset_of!(FwUp, mmu_features), 11),
            (offset_of!(FwUp, tables_used), 12),
            (offset_of!(FwUp, flushed), 13),
            (offset_of!(FwUp, shared_pa), 14),
            (offset_of!(FwUp, clean_rc), 16),
            (offset_of!(FwUp, mcu_status_before), 17),
            (offset_of!(FwUp, boot_waited_us), 18),
            (offset_of!(FwUp, job_raw), 19),
            (offset_of!(FwUp, mcu_status), 20),
            (offset_of!(FwUp, version), 21),
            (offset_of!(FwUp, features), 22),
            (offset_of!(FwUp, instr_features), 23),
            (offset_of!(FwUp, group_num), 24),
            (offset_of!(FwUp, group_stride), 25),
            (offset_of!(FwUp, glb_input), 26),
            (offset_of!(FwUp, glb_output), 27),
            (offset_of!(FwUp, csg0), 28),
            (offset_of!(FwUp, cs0), 32),
            (offset_of!(FwUp, core_en_mask), 35),
            (offset_of!(FwUp, poweroff_timer), 37),
            (offset_of!(FwUp, progress_timer), 38),
            (offset_of!(FwUp, idle_timer), 39),
            (offset_of!(FwUp, ack_irq_mask), 40),
            (offset_of!(FwUp, init_req), 41),
            (offset_of!(FwUp, acked), 42),
            (offset_of!(FwUp, init_ack_us), 43),
            (offset_of!(FwUp, ping_acked), 44),
            (offset_of!(FwUp, ping_ack_us), 45),
            (offset_of!(FwUp, glb_req), 46),
            (offset_of!(FwUp, glb_ack), 47),
        ];
        for &(off, word) in &words {
            assert_eq!(off, 4 * word, "word {word}");
        }
        // the steps, as the C side will name them
        assert_eq!(
            [STEP_NONE, STEP_PARSE, STEP_PLACE, STEP_GPU, STEP_MMU, STEP_BOOT, STEP_CLEAN, STEP_VERSION],
            [0, 1, 2, 3, 4, 5, 6, 7]
        );
        assert_eq!(fw_error_code(&FwError::TooSmall { len: 7 }), (1, 7));
        assert_eq!(fw_error_code(&FwError::NoSharedSection), (18, 0));
    }

    /// The board's file (`MALI_CSFFW`, `tools/cts/deps-env.sh --build
    /// MALI_CSFFW` fetches it) through the whole run. Skipped, with a line on
    /// stderr, when `MALI_CSFFW` is not set or names no file.
    #[test]
    fn the_boards_file_boots_through_the_whole_run() {
        let Some(path) = std::env::var_os("MALI_CSFFW") else {
            eprintln!("skipped: MALI_CSFFW (the board's mali_csffw.bin) is not set");
            return;
        };
        if !std::path::Path::new(&path).is_file() {
            eprintln!("skipped: MALI_CSFFW={} is not a file", path.to_string_lossy());
            return;
        }
        let blob = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.to_string_lossy()));
        let t6 = T6::handed_over(blob.clone());
        let o = run(&t6, &blob);
        assert_booted(&o);
        assert_eq!((o.sections, o.protected_skipped, o.tables_used, o.flushed), (7, 1, 9, 7));
    }
}
