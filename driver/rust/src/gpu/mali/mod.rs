// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The Mali-G610 (Valhall, CSF) of the RK3588: the kernel side's first
//! steps (feature `mali`): take over a GPU another kernel left running
//! ([`takeover`]), read the identity, soft reset, set the coherency
//! mode, power the L2 cache on (driver/docs/nanopc-t6-mali.md, "Z1"); flush
//! the caches, and in [`mmu`] and [`pgtable`] an address space's tables and
//! its `AS_COMMAND` sequences ("Z2").
//!
//! Source: Linux v6.18's panthor, used under the MIT option of its
//! `GPL-2.0 or MIT` licence (the design page's decision 1): register
//! offsets from drivers/gpu/drm/panthor/panthor_regs.h, the sequences from
//! panthor_gpu.c (Copyright 2018 Marty E. Plummer, 2019 Linaro, Ltd,
//! 2023 Collabora ltd.; register definitions based on ARM's
//! mali_kbase_gpu_regmap.h). Each item names its line.
//!
//! The host never powers the shader cores or the tiler: the firmware does
//! (panthor_gpu.c:356-366, "The FW takes care of powering the other
//! blocks").
//!
//! Measured on the NanoPC-T6 under Armbian (2026-10-18/19): panthor's probe
//! line "Mali-G610 id 0xa867 major 0x0 minor 0x0 status 0x5", "Features:
//! L2:0x7120306 Tiler:0x809 Mem:0x301 MMU:0x2830 AS:0xff", "shader_present=
//! 0x50005 l2_present=0x1 tiler_present=0x1"; `GPU_ID` read with the GPU
//! active: 0xa8670005.
//!
//! Those are the T6's readings. The code reads them ([`ident`]) and takes
//! any arch-10 product of panthor's model table ([`identify`]: the G710,
//! G510, G310 and G610); [`probe`] is the GPU's own part of a power-on, with
//! no SoC register in it. What is read and the G310 mock's sources:
//! docs/notes/mali-gpu-model.md.
//!
//! Z3, the firmware (host side): [`fw`] parses `mali_csffw.bin`, [`csf`]
//! loads it, boots the MCU and talks to it through the shared interface.
//!
//! Z4, the first submit (host side): [`sched`] starts command stream group 0
//! and its stream 0 as panthor does and runs a job on the stream's ring;
//! [`heap`] the tiler heap the L1 frame's `HEAP_SET` names, its context and
//! chunks made as panthor's TILER_HEAP_CREATE makes them.

pub mod csf;
pub mod fw;
pub mod heap;
pub mod sched;
pub mod takeover;

use crate::mmio::Mmio;

pub mod mmu;
pub mod pgtable;

/// Registers (panthor_regs.h:13-126).
pub const GPU_ID: usize = 0x0;
pub const GPU_L2_FEATURES: usize = 0x4;
pub const GPU_TILER_FEATURES: usize = 0xc;
pub const GPU_MEM_FEATURES: usize = 0x10;
pub const GPU_MMU_FEATURES: usize = 0x14;
pub const GPU_AS_PRESENT: usize = 0x18;
pub const GPU_CSF_ID: usize = 0x1c;
pub const GPU_INT_RAWSTAT: usize = 0x20;
pub const GPU_INT_CLEAR: usize = 0x24;
pub const GPU_INT_MASK: usize = 0x28;
pub const GPU_CMD: usize = 0x30;
pub const GPU_STATUS: usize = 0x34;
pub const GPU_SHADER_PRESENT: usize = 0x100;
pub const GPU_TILER_PRESENT: usize = 0x110;
pub const GPU_L2_PRESENT: usize = 0x120;
pub const L2_READY: usize = 0x160;
pub const L2_PWRON: usize = 0x1a0;
pub const L2_PWRTRANS: usize = 0x220;
pub const GPU_COHERENCY_FEATURES: usize = 0x300;
pub const GPU_COHERENCY_PROTOCOL: usize = 0x304;
pub const MCU_STATUS: usize = 0x704;

/// `GPU_IRQ_RESET_COMPLETED` (panthor_regs.h:43).
pub const IRQ_RESET_COMPLETED: u32 = 1 << 8;
/// `GPU_SOFT_RESET` = `GPU_CMD_DEF(1, 1)` (panthor_regs.h:51-53).
pub const CMD_SOFT_RESET: u32 = 1 | (1 << 8);
/// `GPU_HARD_RESET` = `GPU_CMD_DEF(1, 2)` (panthor_regs.h:52).
pub const CMD_HARD_RESET: u32 = 1 | (2 << 8);
/// `GPU_IRQ_CLEAN_CACHES_COMPLETED` (panthor_regs.h, the `GPU_INT_*` bits).
pub const IRQ_CLEAN_CACHES_COMPLETED: u32 = 1 << 17;
/// The cache operations of `GPU_FLUSH_CACHES` (panthor_regs.h
/// `CACHE_CLEAN`, `CACHE_INV`).
pub const CACHE_CLEAN: u32 = 1 << 0;
pub const CACHE_INV: u32 = 1 << 1;
/// `GPU_FLUSH_CACHES(l2, lsc, oth)` = `GPU_CMD_DEF(4, l2 | lsc << 4 |
/// oth << 8)` (panthor_regs.h).
pub const fn cmd_flush_caches(l2: u32, lsc: u32, other: u32) -> u32 {
    4 | (l2 | lsc << 4 | other << 8) << 8
}
/// The flush `panthor_mmu_as_enable` asks for (`AS_COMMAND_FLUSH_MEM` in
/// `mmu_hw_do_operation_locked`): L2 and LSC cleaned and invalidated.
pub const CMD_FLUSH_L2_LSC: u32 = cmd_flush_caches(CACHE_CLEAN | CACHE_INV, CACHE_CLEAN | CACHE_INV, 0);
/// panthor's wait for the flush: 100 ms (`panthor_gpu_flush_caches`,
/// `wait_event_timeout(..., msecs_to_jiffies(100))`).
pub const FLUSH_WAIT_US: u32 = 100_000;

/// `GPU_COHERENCY_NONE` (panthor_regs.h:119): the RK3588's GPU node has no
/// `dma-coherent`, so panthor writes this (panthor_gpu.c:49-53).
pub const COHERENCY_NONE: u32 = 31;

/// The RK3588's Mali-G610 as the T6 reads it: arch 10.8, product 7. The
/// code takes the GPU it finds through [`identify`]; this value is the T6's
/// record (and its mocks').
pub const G610_GPU_ID: u32 = 0xa867_0005;

/// The fields of `GPU_ID` (panthor_regs.h `GPU_ARCH_MAJOR` .. `GPU_VER_STATUS`):
/// bits 31:28 arch major, 27:24 arch minor, 23:20 arch revision, 19:16
/// product major, 15:12 version major, 11:4 version minor, 3:0 status.
pub const fn arch_major(gpu_id: u32) -> u32 {
    gpu_id >> 28
}
pub const fn arch_minor(gpu_id: u32) -> u32 {
    (gpu_id >> 24) & 0xf
}
pub const fn arch_rev(gpu_id: u32) -> u32 {
    (gpu_id >> 20) & 0xf
}
pub const fn prod_major(gpu_id: u32) -> u32 {
    (gpu_id >> 16) & 0xf
}
pub const fn ver_major(gpu_id: u32) -> u32 {
    (gpu_id >> 12) & 0xf
}
pub const fn ver_minor(gpu_id: u32) -> u32 {
    (gpu_id >> 4) & 0xff
}
pub const fn ver_status(gpu_id: u32) -> u32 {
    gpu_id & 0xf
}

/// The GPUs this kernel side takes: the arch-10 (Valhall, CSF) products of
/// panthor's model table (panthor_hw.c `get_gpu_model_name`, which keys on
/// `GPU_PROD_ID_MAKE(GPU_ARCH_MAJOR, GPU_PROD_MAJOR)`). Every one runs the
/// same sequences here; what differs between them is read from the GPU
/// ([`Ident`]): the L2, tiler and shader masks, the address spaces, the MMU's
/// VA and PA widths ([`pgtable::Shape`]), the L2 line, and from the firmware
/// its group and stream counts ([`csf`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Model {
    /// Product 2.
    G710,
    /// Product 3.
    G510,
    /// Product 4: the i.MX 95's (Mesa's model table: arch 10.12, product 4).
    G310,
    /// Product 7: the RK3588's (arch 10.8).
    G610,
}

impl Model {
    /// panthor's name for it.
    pub const fn name(self) -> &'static str {
        match self {
            Model::G710 => "Mali-G710",
            Model::G510 => "Mali-G510",
            Model::G310 => "Mali-G310",
            Model::G610 => "Mali-G610",
        }
    }
}

/// A `GPU_ID` [`identify`] refuses, with what panthor would call it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unsupported {
    /// Arch 10 with a product panthor's table does not name.
    Arch10Product(u32),
    /// Arch 11 to 13: a CSF GPU panthor names (the G715, G615, G720, G620,
    /// G725, G925, G625 families) that this kernel side was not written for
    /// (arch 11 adds `GPU_FEATURES`, panthor_hw.c:87-88; later arches change
    /// the interfaces).
    LaterArch { arch: u32, product: u32 },
    /// Another arch: below 10 a job-manager GPU (panfrost's, no CSF), above
    /// 13 one panthor v6.18 does not name.
    Arch(u32),
}

impl Unsupported {
    /// What is refused, by name (panthor_hw.c:11-59 at v6.18,
    /// `get_gpu_model_name`'s names for arch 11-13; panthor's "(Unknown Mali
    /// GPU)" otherwise). panthor tells product 0 of arch 12 and 13 and
    /// product 2 of arch 11 apart by the shader core count and the ray
    /// tracing feature, which a refusal before the cores are counted does not
    /// read: those name the family with its other members.
    pub const fn name(self) -> &'static str {
        match self {
            Unsupported::LaterArch { arch: 11, product: 2 } => "Mali-G715 (G715-Immortalis or G615 by core count)",
            Unsupported::LaterArch { arch: 11, product: 3 } => "Mali-G615",
            Unsupported::LaterArch { arch: 12, product: 0 } => "Mali-G720 (G720-Immortalis or G620 by core count)",
            Unsupported::LaterArch { arch: 12, product: 1 } => "Mali-G620",
            Unsupported::LaterArch { arch: 13, product: 0 } => "Mali-G725 (G925-Immortalis or G625 by core count)",
            Unsupported::LaterArch { arch: 13, product: 1 } => "Mali-G625",
            Unsupported::Arch(a) if a < 10 => "job-manager Mali (not CSF)",
            _ => "(Unknown Mali GPU)",
        }
    }
}

/// The model of `gpu_id`, or what it is when this kernel side does not take
/// it. Only the arch major and the product major decide, as in panthor; the
/// arch minor and revision and the version fields do not (a G610 r0p0
/// status 1 is a G610).
pub const fn identify(gpu_id: u32) -> Result<Model, Unsupported> {
    let (arch, product) = (arch_major(gpu_id), prod_major(gpu_id));
    match (arch, product) {
        (10, 2) => Ok(Model::G710),
        (10, 3) => Ok(Model::G510),
        (10, 4) => Ok(Model::G310),
        (10, 7) => Ok(Model::G610),
        (10, p) => Err(Unsupported::Arch10Product(p)),
        (11..=13, p) => Err(Unsupported::LaterArch { arch, product: p }),
        (a, _) => Err(Unsupported::Arch(a)),
    }
}

/// The firmware's directory under /lib/firmware for `gpu_id`:
/// `arm/mali/arch<major>.<minor>/mali_csffw.bin` (panthor_fw.c
/// `panthor_fw_load`, from `GPU_ARCH_MAJOR` and `GPU_ARCH_MINOR`): (10, 8)
/// on the RK3588, (10, 12) for a G310 as Mesa's table gives it.
pub const fn fw_arch(gpu_id: u32) -> (u32, u32) {
    (arch_major(gpu_id), arch_minor(gpu_id))
}

/// panthor's waits: the soft reset 100 ms (panthor_gpu.c:318), the L2
/// 20 ms (:247).
pub const RESET_WAIT_US: u32 = 100_000;
pub const L2_WAIT_US: u32 = 20_000;

/// The identity block, read with no write.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Ident {
    pub gpu_id: u32,
    pub l2_features: u32,
    pub tiler_features: u32,
    pub mem_features: u32,
    pub mmu_features: u32,
    pub as_present: u32,
    pub csf_id: u32,
    pub shader_present: u64,
    pub tiler_present: u64,
    pub l2_present: u64,
    pub coherency_features: u32,
    pub status: u32,
    pub mcu_status: u32,
}

impl Ident {
    /// [`identify`] on `gpu_id`.
    pub const fn model(&self) -> Result<Model, Unsupported> {
        identify(self.gpu_id)
    }

    /// The page tables `mmu_features` asks for ([`pgtable::Shape`]); `None`
    /// for widths no table here serves.
    pub const fn mmu_shape(&self) -> Option<pgtable::Shape> {
        pgtable::Shape::from_mmu_features(self.mmu_features)
    }
}

fn read64(m: &impl Mmio, addr: usize) -> u64 {
    u64::from(m.read32(addr)) | u64::from(m.read32(addr + 4)) << 32
}

/// Read the identity registers of the GPU at `base`. The caller has powered
/// PD_GPU and opened its clocks: on the RK3588 a read of a GPU register
/// with the domain off is an external abort.
pub fn ident(m: &impl Mmio, base: usize) -> Ident {
    Ident {
        gpu_id: m.read32(base + GPU_ID),
        l2_features: m.read32(base + GPU_L2_FEATURES),
        tiler_features: m.read32(base + GPU_TILER_FEATURES),
        mem_features: m.read32(base + GPU_MEM_FEATURES),
        mmu_features: m.read32(base + GPU_MMU_FEATURES),
        as_present: m.read32(base + GPU_AS_PRESENT),
        csf_id: m.read32(base + GPU_CSF_ID),
        shader_present: read64(m, base + GPU_SHADER_PRESENT),
        tiler_present: read64(m, base + GPU_TILER_PRESENT),
        l2_present: read64(m, base + GPU_L2_PRESENT),
        coherency_features: m.read32(base + GPU_COHERENCY_FEATURES),
        status: m.read32(base + GPU_STATUS),
        mcu_status: m.read32(base + MCU_STATUS),
    }
}

/// Why a step stopped. The value crosses to C, one number per cause.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum MaliRefusal {
    /// `RESET_COMPLETED` did not rise within [`RESET_WAIT_US`].
    ResetTimeout = 1,
    /// `L2_PWRTRANS` stayed set for the L2 before the power-on.
    L2TransTimeout = 2,
    /// `L2_READY` did not show the L2 within [`L2_WAIT_US`].
    L2ReadyTimeout = 3,
    /// `L2_PRESENT` is 0: there is nothing to power.
    NoL2 = 4,
    /// `RESET_COMPLETED` still reads set after its clear: a completion
    /// could not be told from an old one.
    ResetBitStuck = 5,
    /// `CLEAN_CACHES_COMPLETED` did not rise within [`FLUSH_WAIT_US`].
    FlushTimeout = 6,
    /// `CLEAN_CACHES_COMPLETED` still reads set after its clear.
    FlushBitStuck = 7,
    /// `AS_STATUS.AS_ACTIVE` stayed set ([`mmu::READY_WAIT_US`]).
    AsActiveStuck = 8,
    /// An `*_INT_MASK` read back other than the 0 written
    /// ([`takeover::take_over`]).
    IrqMaskStuck = 9,
    // 10 is not used: an MCU that does not halt is recorded, not refused
    // (`takeover::TakeOver::mcu_halt_timed_out`).
    /// The soft reset timed out and `RESET_COMPLETED` did not rise within
    /// [`RESET_WAIT_US`] after the hard reset either.
    HardResetTimeout = 11,
}

/// Poll the 64-bit register at `addr` once per microsecond for at most
/// `budget_us`; the microseconds it took.
fn wait64(
    m: &impl Mmio,
    addr: usize,
    budget_us: u32,
    delay_us: &mut impl FnMut(u32),
    cond: impl Fn(u64) -> bool,
) -> Option<u32> {
    for t in 0..=budget_us {
        if cond(read64(m, addr)) {
            return Some(t);
        }
        delay_us(1);
    }
    None
}

/// Soft reset (panthor_gpu.c:302-334): clear `RESET_COMPLETED`, write
/// `GPU_SOFT_RESET`, wait for `RESET_COMPLETED` in `GPU_INT_RAWSTAT`
/// (panthor waits for the interrupt; with `GPU_INT_MASK` as found this
/// polls the raw status, which panthor's timeout path reads too, :319-323).
/// The microseconds it took.
///
/// The record says whether the completion is this reset's: `GPU_INT_RAWSTAT`
/// before the clear, after it (the command is written only when it reads
/// clear), and the polls it took.
pub fn soft_reset(m: &impl Mmio, base: usize, delay_us: &mut impl FnMut(u32)) -> Result<Reset, (MaliRefusal, u32)> {
    reset_with(m, base, CMD_SOFT_RESET, delay_us)
}

/// [`soft_reset`]'s sequence with the reset command `cmd`
/// ([`CMD_SOFT_RESET`] or [`CMD_HARD_RESET`]).
pub(crate) fn reset_with(
    m: &impl Mmio,
    base: usize,
    cmd: u32,
    delay_us: &mut impl FnMut(u32),
) -> Result<Reset, (MaliRefusal, u32)> {
    let raw_before = m.read32(base + GPU_INT_RAWSTAT);
    m.write32(base + GPU_INT_CLEAR, IRQ_RESET_COMPLETED);
    let raw_cleared = m.read32(base + GPU_INT_RAWSTAT);
    if raw_cleared & IRQ_RESET_COMPLETED != 0 {
        return Err((MaliRefusal::ResetBitStuck, raw_cleared));
    }
    m.write32(base + GPU_CMD, cmd);
    for t in 0..=RESET_WAIT_US {
        if m.read32(base + GPU_INT_RAWSTAT) & IRQ_RESET_COMPLETED != 0 {
            m.write32(base + GPU_INT_CLEAR, IRQ_RESET_COMPLETED);
            return Ok(Reset { raw_before, raw_cleared, waited_us: t });
        }
        delay_us(1);
    }
    Err((MaliRefusal::ResetTimeout, m.read32(base + GPU_INT_RAWSTAT)))
}

/// What [`soft_reset`] saw.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Reset {
    /// `GPU_INT_RAWSTAT` before the clear and after it.
    pub raw_before: u32,
    pub raw_cleared: u32,
    /// Polls (each followed by a 1 µs wait) before `RESET_COMPLETED` rose.
    pub waited_us: u32,
}

/// What [`flush_caches`] saw.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Flush {
    /// The `GPU_CMD` value written.
    pub cmd: u32,
    /// `GPU_INT_RAWSTAT` before the clear and after it.
    pub raw_before: u32,
    pub raw_cleared: u32,
    /// Polls (each followed by a 1 µs wait) before `CLEAN_CACHES_COMPLETED`
    /// rose.
    pub waited_us: u32,
}

/// A cache flush round trip (`panthor_gpu_flush_caches`, panthor_gpu.c):
/// write `GPU_FLUSH_CACHES(l2, lsc, other)` to `GPU_CMD`, wait for
/// `CLEAN_CACHES_COMPLETED`. panthor waits for the interrupt (its handler
/// clears the bit, `panthor_gpu_irq_handler`) and on its timeout reads
/// `GPU_INT_RAWSTAT`; this polls the raw status and clears the bit itself,
/// as [`soft_reset`] does: the bit is cleared first, and the command is
/// written only when it reads clear, so the completion seen is this one's.
pub fn flush_caches(
    m: &impl Mmio,
    base: usize,
    l2: u32,
    lsc: u32,
    other: u32,
    delay_us: &mut impl FnMut(u32),
) -> Result<Flush, (MaliRefusal, u32)> {
    let cmd = cmd_flush_caches(l2, lsc, other);
    let raw_before = m.read32(base + GPU_INT_RAWSTAT);
    m.write32(base + GPU_INT_CLEAR, IRQ_CLEAN_CACHES_COMPLETED);
    let raw_cleared = m.read32(base + GPU_INT_RAWSTAT);
    if raw_cleared & IRQ_CLEAN_CACHES_COMPLETED != 0 {
        return Err((MaliRefusal::FlushBitStuck, raw_cleared));
    }
    m.write32(base + GPU_CMD, cmd);
    for t in 0..=FLUSH_WAIT_US {
        if m.read32(base + GPU_INT_RAWSTAT) & IRQ_CLEAN_CACHES_COMPLETED != 0 {
            m.write32(base + GPU_INT_CLEAR, IRQ_CLEAN_CACHES_COMPLETED);
            return Ok(Flush { cmd, raw_before, raw_cleared, waited_us: t });
        }
        delay_us(1);
    }
    Err((MaliRefusal::FlushTimeout, m.read32(base + GPU_INT_RAWSTAT)))
}

/// What [`l2_power_on`] saw.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct L2On {
    pub mask: u64,
    /// `L2_READY` before the `L2_PWRON` write, and after the wait.
    pub ready_before: u64,
    pub ready: u64,
    /// Microseconds of the transition wait and of the ready wait.
    pub waited_us: [u32; 2],
}

/// The coherency mode, then the L2 on (panthor_gpu.c:226-248 and
/// `panthor_gpu_block_power_on`, :190-218): with only one L2 (the
/// G610's `l2_present` 1), its mask is 1; wait while `L2_PWRTRANS` has it,
/// write `L2_PWRON`, wait until `L2_READY` has it.
///
/// One difference: panthor's ready condition is `(mask & val) == val`
/// (:210), which an `L2_READY` of 0 meets at once; this waits for
/// `ready & mask == mask`, the L2 actually reported ready.
pub fn l2_power_on(m: &impl Mmio, base: usize, delay_us: &mut impl FnMut(u32)) -> Result<L2On, (MaliRefusal, u64)> {
    let present = read64(m, base + GPU_L2_PRESENT);
    if present == 0 {
        return Err((MaliRefusal::NoL2, 0));
    }
    // panthor powers the first core group only (:229-243); the G610 has one
    let mask = present & present.wrapping_neg();
    m.write32(base + GPU_COHERENCY_PROTOCOL, COHERENCY_NONE);
    let mut out = L2On { mask, ..L2On::default() };
    out.waited_us[0] = wait64(m, base + L2_PWRTRANS, L2_WAIT_US, delay_us, |v| v & mask == 0)
        .ok_or((MaliRefusal::L2TransTimeout, read64(m, base + L2_PWRTRANS)))?;
    out.ready_before = read64(m, base + L2_READY);
    m.write32(base + L2_PWRON, mask as u32);
    m.write32(base + L2_PWRON + 4, (mask >> 32) as u32);
    out.waited_us[1] = wait64(m, base + L2_READY, L2_WAIT_US, delay_us, |v| v & mask == mask)
        .ok_or((MaliRefusal::L2ReadyTimeout, read64(m, base + L2_READY)))?;
    out.ready = read64(m, base + L2_READY);
    Ok(out)
}

/// What [`probe`] read and did, in order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Probe {
    pub ident: Ident,
    pub reset: Reset,
    /// After the reset.
    pub ident_after_reset: Ident,
    pub l2: L2On,
}

/// Why [`probe`] stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProbeStop {
    /// `GPU_ID` names a GPU this kernel side does not take ([`identify`]).
    /// Nothing was written to the GPU.
    Unsupported(Unsupported),
    /// [`soft_reset`] refused (the refusal and `GPU_INT_RAWSTAT`).
    Reset(MaliRefusal, u32),
    /// [`l2_power_on`] refused (the refusal and the register it is about).
    L2(MaliRefusal, u64),
}

/// The GPU's own part of a power-on, once the SoC has powered and clocked
/// it (and quieted a GPU another kernel left running,
/// [`takeover::take_over`]): the identity, the model it names, the soft
/// reset, the identity again, the L2. No SoC register is touched: the
/// RK3588's supply, clock, power domain and QoS are the board's
/// (board::nanopc_t6_gpu), before this. panthor's order: `panthor_hw_init`
/// reads the identity (panthor_hw.c `panthor_gpu_info_init`), then
/// `panthor_gpu_init` resets (panthor_gpu.c `panthor_gpu_soft_reset`) and
/// `panthor_gpu_l2_power_on` powers the L2.
pub fn probe(m: &impl Mmio, base: usize, delay_us: &mut impl FnMut(u32), o: &mut Probe) -> Result<Model, ProbeStop> {
    o.ident = ident(m, base);
    let model = identify(o.ident.gpu_id).map_err(ProbeStop::Unsupported)?;
    o.reset = soft_reset(m, base, delay_us).map_err(|(why, raw)| ProbeStop::Reset(why, raw))?;
    o.ident_after_reset = ident(m, base);
    o.l2 = l2_power_on(m, base, delay_us).map_err(|(why, v)| ProbeStop::L2(why, v))?;
    Ok(model)
}

/// Register values of the GPUs the host tests model, one table per GPU:
/// the G610 of the T6 as measured, the G310 of the i.MX 95 as far as a
/// public source gives it.
#[cfg(test)]
pub(crate) mod mock;

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;

    const BASE: usize = 0xfb00_0000;

    /// A GPU whose identity registers are one of [`mock`]'s tables (the
    /// G610 as panthor's probe line describes it, or the G310); a soft reset
    /// completes after `lag` polls, the L2 is ready `lag` polls after its
    /// power-on.
    struct Mali {
        regs: RefCell<HashMap<usize, u32>>,
        writes: RefCell<Vec<(usize, u32)>>,
        lag: u32,
        polls: RefCell<u32>,
        reset_pending: RefCell<bool>,
        flush_pending: RefCell<bool>,
        l2_on: RefCell<bool>,
        /// The L2 never becomes ready.
        stuck_l2: bool,
        /// `GPU_INT_CLEAR` does not clear.
        sticky_raw: bool,
    }

    /// The T6's GPU.
    type G610 = Mali;

    impl Mali {
        fn new(lag: u32) -> Mali {
            Mali::with_regs(&mock::G610, lag)
        }
        fn with_regs(r: &[(usize, u32)], lag: u32) -> Mali {
            Mali {
                regs: RefCell::new(r.iter().copied().collect()),
                writes: RefCell::new(vec![]),
                lag,
                polls: RefCell::new(0),
                reset_pending: RefCell::new(false),
                flush_pending: RefCell::new(false),
                l2_on: RefCell::new(false),
                stuck_l2: false,
                sticky_raw: false,
            }
        }
    }

    impl Mmio for Mali {
        fn read32(&self, addr: usize) -> u32 {
            let off = addr - BASE;
            let mut p = self.polls.borrow_mut();
            *p += 1;
            match off {
                GPU_INT_RAWSTAT => {
                    if *self.reset_pending.borrow() && *p > self.lag {
                        *self.reset_pending.borrow_mut() = false;
                        let mut r = self.regs.borrow_mut();
                        *r.entry(GPU_INT_RAWSTAT).or_insert(0) |= IRQ_RESET_COMPLETED;
                    }
                    if *self.flush_pending.borrow() && *p > self.lag {
                        *self.flush_pending.borrow_mut() = false;
                        *self.regs.borrow_mut().entry(GPU_INT_RAWSTAT).or_insert(0) |= IRQ_CLEAN_CACHES_COMPLETED;
                    }
                }
                L2_READY if *self.l2_on.borrow() && *p > self.lag && !self.stuck_l2 => {
                    self.regs.borrow_mut().insert(L2_READY, 1);
                }
                _ => {}
            }
            *self.regs.borrow().get(&off).unwrap_or(&0)
        }
        fn write32(&self, addr: usize, val: u32) {
            let off = addr - BASE;
            self.writes.borrow_mut().push((off, val));
            *self.polls.borrow_mut() = 0;
            match off {
                GPU_INT_CLEAR if self.sticky_raw => {}
                GPU_INT_CLEAR => {
                    *self.regs.borrow_mut().entry(GPU_INT_RAWSTAT).or_insert(0) &= !val;
                }
                GPU_CMD if val == CMD_SOFT_RESET => {
                    *self.reset_pending.borrow_mut() = true;
                    self.regs.borrow_mut().insert(L2_READY, 0);
                    *self.l2_on.borrow_mut() = false;
                }
                GPU_CMD if val & 0xff == 4 => *self.flush_pending.borrow_mut() = true,
                L2_PWRON if val & 1 != 0 => *self.l2_on.borrow_mut() = true,
                _ => {
                    self.regs.borrow_mut().insert(off, val);
                }
            }
        }
        fn read8(&self, _: usize) -> u8 {
            unreachable!()
        }
        fn write8(&self, _: usize, _: u8) {
            unreachable!()
        }
    }

    #[test]
    fn ident_reads_what_panthor_printed() {
        let g = G610::new(0);
        let i = ident(&g, BASE);
        assert_eq!(i.gpu_id, 0xa867_0005);
        assert_eq!((i.gpu_id >> 16, (i.gpu_id >> 12) & 0xf, (i.gpu_id >> 4) & 0xff, i.gpu_id & 0xf), (0xa867, 0, 0, 5));
        assert_eq!(
            (i.l2_features, i.tiler_features, i.mem_features, i.mmu_features, i.as_present),
            (0x7120306, 0x809, 0x301, 0x2830, 0xff)
        );
        assert_eq!((i.shader_present, i.l2_present, i.tiler_present), (0x50005, 1, 1));
        assert!(g.writes.borrow().is_empty());
    }

    #[test]
    fn the_models_are_panthor_s_arch_10_products_and_the_rest_is_refused_by_name() {
        assert_eq!(identify(G610_GPU_ID), Ok(Model::G610));
        // the version fields do not decide: another G610 status is a G610
        assert_eq!(identify(0xa867_0001), Ok(Model::G610));
        assert_eq!(identify(mock::G310_GPU_ID), Ok(Model::G310));
        assert_eq!(identify(0xa002_0000), Ok(Model::G710));
        assert_eq!(identify(0xa003_0000), Ok(Model::G510));
        assert_eq!(
            [Model::G710, Model::G510, Model::G310, Model::G610].map(Model::name),
            ["Mali-G710", "Mali-G510", "Mali-G310", "Mali-G610"]
        );
        let refused = |id| identify(id).map_err(|u| (u, u.name()));
        assert_eq!(
            refused(0xb002_0000),
            Err((Unsupported::LaterArch { arch: 11, product: 2 }, "Mali-G715 (G715-Immortalis or G615 by core count)"))
        );
        assert_eq!(refused(0xc001_0000), Err((Unsupported::LaterArch { arch: 12, product: 1 }, "Mali-G620")));
        assert_eq!(
            refused(0xd000_0000),
            Err((Unsupported::LaterArch { arch: 13, product: 0 }, "Mali-G725 (G925-Immortalis or G625 by core count)"))
        );
        // Mesa's G57 (arch 9, product 1): a job-manager GPU
        assert_eq!(refused(0x9001_0000), Err((Unsupported::Arch(9), "job-manager Mali (not CSF)")));
        assert_eq!(refused(0xa86a_0000), Err((Unsupported::Arch10Product(10), "(Unknown Mali GPU)")));
        assert_eq!(refused(0xe000_0000), Err((Unsupported::Arch(14), "(Unknown Mali GPU)")));
    }

    #[test]
    fn the_gpu_id_fields_and_the_firmware_directory_follow_the_register() {
        let f = |id| {
            (arch_major(id), arch_minor(id), arch_rev(id), prod_major(id), ver_major(id), ver_minor(id), ver_status(id))
        };
        assert_eq!(f(G610_GPU_ID), (10, 8, 6, 7, 0, 0, 5));
        assert_eq!(f(mock::G310_GPU_ID), (10, 12, 7, 4, 0, 0, 1));
        assert_eq!(fw_arch(G610_GPU_ID), (10, 8));
        assert_eq!(fw_arch(mock::G310_GPU_ID), (10, 12));
    }

    #[test]
    fn the_g310_reads_its_own_registers() {
        let g = Mali::with_regs(&mock::G310, 0);
        let i = ident(&g, BASE);
        assert_eq!(i.gpu_id >> 16, 0xac74); // the thread's "id 0xac74"
        assert_eq!(i.model(), Ok(Model::G310));
        assert_eq!((i.shader_present, i.l2_present, i.tiler_present, i.as_present), (0x3, 1, 1, 0xff));
        assert_eq!(i.mmu_shape(), Some(pgtable::Shape { va_bits: 48, pa_bits: 40 }));
        assert!(g.writes.borrow().is_empty());
    }

    /// The probe's writes are the same on both GPUs (the sequences do not
    /// depend on the model); what it read is each GPU's.
    #[test]
    fn the_probe_runs_alike_on_the_g610_and_the_g310() {
        let mut seen = vec![];
        for (regs, model, shaders) in [(&mock::G610, Model::G610, 0x50005), (&mock::G310, Model::G310, 0x3)] {
            let g = Mali::with_regs(regs, 3);
            let mut p = Probe::default();
            assert_eq!(probe(&g, BASE, &mut |_| {}, &mut p), Ok(model));
            assert_eq!((p.ident.gpu_id, p.ident_after_reset.gpu_id), (regs[0].1, regs[0].1));
            assert_eq!(p.ident_after_reset.shader_present, shaders);
            assert_eq!(p.reset.waited_us, 3);
            assert_eq!(p.l2, L2On { mask: 1, ready_before: 0, ready: 1, waited_us: [0, 2] });
            seen.push(g.writes.borrow().clone());
        }
        assert_eq!(seen[0], seen[1]);
        assert_eq!(seen[0][..2], [(GPU_INT_CLEAR, IRQ_RESET_COMPLETED), (GPU_CMD, CMD_SOFT_RESET)]);
    }

    #[test]
    fn the_probe_refuses_another_gpu_before_any_write() {
        let g = Mali::new(0);
        g.regs.borrow_mut().insert(GPU_ID, 0xb002_0000);
        let mut p = Probe::default();
        let r = probe(&g, BASE, &mut |_| {}, &mut p);
        assert_eq!(r, Err(ProbeStop::Unsupported(Unsupported::LaterArch { arch: 11, product: 2 })));
        assert_eq!(p.ident.gpu_id, 0xb002_0000);
        assert!(g.writes.borrow().is_empty());
    }

    #[test]
    fn soft_reset_clears_then_commands_then_acknowledges() {
        let g = G610::new(4);
        assert_eq!(soft_reset(&g, BASE, &mut |_| {}), Ok(Reset { raw_before: 0, raw_cleared: 0, waited_us: 4 }));
        assert_eq!(*g.writes.borrow(), vec![(GPU_INT_CLEAR, 1 << 8), (GPU_CMD, 0x101), (GPU_INT_CLEAR, 1 << 8)]);
    }

    #[test]
    fn a_completion_that_cannot_be_cleared_is_refused_before_the_command() {
        let mut g = G610::new(0);
        g.sticky_raw = true;
        g.regs.borrow_mut().insert(GPU_INT_RAWSTAT, IRQ_RESET_COMPLETED);
        assert_eq!(soft_reset(&g, BASE, &mut |_| {}), Err((MaliRefusal::ResetBitStuck, IRQ_RESET_COMPLETED)));
        assert!(g.writes.borrow().iter().all(|w| w.0 != GPU_CMD));
    }

    #[test]
    fn a_reset_that_never_completes_times_out_after_100_ms() {
        let g = G610::new(u32::MAX);
        let mut slept = 0u64;
        assert_eq!(soft_reset(&g, BASE, &mut |us| slept += u64::from(us)), Err((MaliRefusal::ResetTimeout, 0)));
        assert_eq!(slept, u64::from(RESET_WAIT_US) + 1);
    }

    #[test]
    fn l2_power_on_sets_no_coherency_then_powers_the_one_l2() {
        let g = G610::new(3);
        let on = l2_power_on(&g, BASE, &mut |_| {}).unwrap();
        // 64-bit polls: two reads each, so a lag of 3 reads is two polls
        assert_eq!(on, L2On { mask: 1, ready_before: 0, ready: 1, waited_us: [0, 2] });
        assert_eq!(*g.writes.borrow(), vec![(GPU_COHERENCY_PROTOCOL, 31), (L2_PWRON, 1), (L2_PWRON + 4, 0)]);
    }

    #[test]
    fn an_l2_ready_of_0_is_not_taken_as_ready() {
        let mut g = G610::new(0);
        g.stuck_l2 = true;
        assert_eq!(l2_power_on(&g, BASE, &mut |_| {}), Err((MaliRefusal::L2ReadyTimeout, 0)));
        let g = G610::new(0);
        g.regs.borrow_mut().insert(GPU_L2_PRESENT, 0);
        assert_eq!(l2_power_on(&g, BASE, &mut |_| {}), Err((MaliRefusal::NoL2, 0)));
        assert!(g.writes.borrow().is_empty());
    }

    #[test]
    fn the_flush_command_is_panthor_s_encoding() {
        assert_eq!(CMD_FLUSH_L2_LSC, 0x3304);
        assert_eq!(cmd_flush_caches(CACHE_CLEAN | CACHE_INV, 0, 0), 0x0304);
    }

    #[test]
    fn a_flush_clears_then_commands_then_sees_its_completion() {
        let g = G610::new(5);
        let f = flush_caches(&g, BASE, CACHE_CLEAN | CACHE_INV, CACHE_CLEAN | CACHE_INV, 0, &mut |_| {}).unwrap();
        assert_eq!(f, Flush { cmd: 0x3304, raw_before: 0, raw_cleared: 0, waited_us: 5 });
        assert_eq!(*g.writes.borrow(), vec![(GPU_INT_CLEAR, 1 << 17), (GPU_CMD, 0x3304), (GPU_INT_CLEAR, 1 << 17)]);
        assert_eq!(g.read32(BASE + GPU_INT_RAWSTAT), 0);
    }

    #[test]
    fn a_flush_that_cannot_be_told_from_an_old_one_or_never_ends_is_refused() {
        let mut g = G610::new(0);
        g.sticky_raw = true;
        g.regs.borrow_mut().insert(GPU_INT_RAWSTAT, IRQ_CLEAN_CACHES_COMPLETED);
        assert_eq!(flush_caches(&g, BASE, 3, 3, 0, &mut |_| {}), Err((MaliRefusal::FlushBitStuck, 1 << 17)));
        assert!(g.writes.borrow().iter().all(|w| w.0 != GPU_CMD));
        let g = G610::new(u32::MAX);
        let mut slept = 0u64;
        assert_eq!(
            flush_caches(&g, BASE, 3, 3, 0, &mut |us| slept += u64::from(us)),
            Err((MaliRefusal::FlushTimeout, 0))
        );
        assert_eq!(slept, u64::from(FLUSH_WAIT_US) + 1);
    }
}
