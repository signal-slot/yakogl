// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Our implementation of the TI/Img Rogue DDK 25.2's `rgxlayer.h`
//! (`third_party/ti-img-rogue-ddk-25.2/services/server/devices/rgxlayer.h`),
//! the layer the DDK's `rgxstartstop.c` (`RGXStart`, `RGXStop`) and
//! `rgxfwimageutils.c` (`RGXGetFWImageAllocSize`, `RGXProcessFWImage`) are
//! written against. Both are compiled unmodified in the Zephyr build
//! (`driver/compat/pvr_rgxstartstop.c`, `pvr_rgxfwimageutils.c`); every
//! function they call that is not their own is exported from here with the
//! DDK's name and signature (C ABI), except the two variadic log functions,
//! `RGXCommentLog` and `RGXErrorLog`, which stable Rust cannot define and which
//! are therefore C (`driver/compat/pvr_rgxlayer_log.c`).
//!
//! `hPrivate` is a [`YdPvrLayer`] (`struct yd_pvr_layer`,
//! `include/yakogl_drivers.h`), filled by the C driver: the mapped register
//! base, the OS services ([`YdEnv`]), the kernel page catalogue, the firmware
//! code and data device addresses and the core clock. Register accesses go
//! through [`Mmio`]: on the board the hardware form ([`Hw`], Zephyr's
//! `sys_read32` / `sys_write32`), in the host check a recording bus the C side
//! hands in ([`YdPvrBus`]).
//!
//! What the DDK's own implementation (`rgxlayer_impl_common.c`, not vendored)
//! does and this one does differently is said at each function. The answers
//! about the core come from the vendored configuration of this GPU,
//! `hwdefs/rogue/km/configs/rgxconfig_km_36.V.104.796.h` (features) and
//! `hwdefs/rogue/km/cores/rgxcore_km_36.53.104.796.h` (errata), with the bit
//! positions of `hwdefs/rogue/km/rgx_bvnc_defs_km.h`; the host check
//! (`tools/pvr/rgxlayer_check.c`) asks every function below the same
//! questions the C preprocessor answers from those headers and requires the
//! same answers.
//!
//! The firmware is RISC-V: the META and MIPS entry points `rgxlayer.h`
//! declares (because the configuration's `rgx_bvnc_defs_km.h` defines
//! `RGX_FEATURE_MIPS_BIT_MASK` and `RGX_FEATURE_META_MAX_VALUE_IDX`) are
//! reached only on cores with those processors; here they answer "not
//! supported" and log it.

#![allow(non_snake_case)] // the DDK's names

use crate::env::{Env, YdEnv};
use crate::mmio::{Hw, Mmio};
use crate::ylog;
use core::ffi::c_void;
use core::mem::{offset_of, size_of};

// ---- the DDK types in the signatures ----------------------------------------

/// `IMG_BOOL` (img_types.h: `typedef bool IMG_BOOL`).
pub type ImgBool = bool;

/// `PVRSRV_ERROR` (pvrsrv_error.h: an enum, `PVRSRV_OK` first, then the
/// `PVRE()` list of pvrsrv_errors.h in order). The values used here are
/// pinned in `driver/compat/pvr_rgxlayer_check.c`.
pub type PvrsrvError = u32;
pub const PVRSRV_OK: PvrsrvError = 0;
pub const PVRSRV_ERROR_INVALID_PARAMS: PvrsrvError = 3;
pub const PVRSRV_ERROR_TIMEOUT: PvrsrvError = 9;
pub const PVRSRV_ERROR_NOT_SUPPORTED: PvrsrvError = 20;

/// `IMG_DEV_PHYADDR` (img_types.h).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImgDevPhyAddr {
    pub ui_addr: u64,
}

/// `IMG_DEV_VIRTADDR` (img_types.h).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImgDevVirtAddr {
    pub ui_addr: u64,
}

// ---- the core (BVNC 36.53.104.796) -------------------------------------------

/// The feature bits of `rgx_bvnc_defs_km.h` (every one has
/// `..._ARRAY_INDEX (0U)`) whose `RGX_FEATURE_<name>` the configuration
/// `rgxconfig_km_36.V.104.796.h` defines: AXI_ACELITE, COMPUTE,
/// COMPUTE_OVERLAP, COREID_PER_OS, GPU_MULTICORE_SUPPORT, GPU_VIRTUALISATION,
/// GS_RTA_SUPPORT, IRQ_PER_OS, PBE2_IN_XE, PBVNC_COREID_REG, PERFBUS,
/// RISCV_FW_PROCESSOR, ROGUEXE, SAFETY_IRQ, SIMPLE_INTERNAL_PARAMETER_FORMAT,
/// SIMPLE_INTERNAL_PARAMETER_FORMAT_V2, SLC_SIZE_CONFIGURABLE, SOC_TIMER,
/// SYS_BUS_SECURE_RESET, TILE_REGION_PROTECTION,
/// TPU_CEM_DATAMASTER_GLOBAL_REGISTERS, TPU_DM_GLOBAL_REGISTERS,
/// WATCHDOG_TIMER, WORKGROUP_PROTECTION, XE_MEMORY_HIERARCHY. Not MIPS, not
/// XT_TOP_INFRASTRUCTURE. `YD_PVR_RGX_FEATURES` in yakogl_drivers.h; the
/// board's build asserts it against the headers.
pub const FEATURES: u64 = 0x0000_039a_1d79_af35;

/// The erratum bits of `rgx_bvnc_defs_km.h` whose `FIX_HW_BRN_<n>` /
/// `HW_ERN_<n>` the core header `rgxcore_km_36.53.104.796.h` defines:
/// HW_ERN_42290, HW_ERN_42606, HW_ERN_47025, HW_ERN_57596, FIX_HW_BRN_71317.
/// `YD_PVR_RGX_ERNS_BRNS` in yakogl_drivers.h.
pub const ERNS_BRNS: u64 = 0x0000_0000_0008_005a;

/// The value of each feature of `RGX_FEATURE_WITH_VALUE_INDEX`
/// (rgx_bvnc_defs_km.h, in its order), as `rgxconfig_km_36.V.104.796.h`
/// defines it; -1 where it does not (META, META_COREMEM_SIZE, NUM_MEMBUS,
/// TB_GPU_COUNT), which is what the DDK's `RGXDeviceGetFeatureValue` returns
/// for a feature the core lacks.
pub const FEATURE_VALUES: [i32; 30] = [
    1,   // CDM_CONTROL_STREAM_FORMAT
    2,   // ECC_RAMS
    0,   // FAULT_DECODE_VERSION
    50,  // FBCDC
    50,  // FBCDC_ALGORITHM
    7,   // FBCDC_ARCHITECTURE
    0,   // FBC_MAX_DEFAULT_DESCRIPTORS
    0,   // FBC_MAX_LARGE_DESCRIPTORS
    1,   // LAYOUT_MARS
    -1,  // META
    -1,  // META_COREMEM_SIZE
    1,   // NUM_CLUSTERS
    6,   // NUM_ISP_IPP_PIPES
    -1,  // NUM_MEMBUS
    8,   // NUM_OSIDS
    1,   // NUM_RASTER_PIPES
    36,  // PHYS_BUS_WIDTH
    2,   // SIMPLE_PARAMETER_FORMAT_VERSION
    1,   // SLC_BANKS
    512, // SLC_CACHE_LINE_SIZE_BITS
    16,  // SLC_SIZE_IN_KILOBYTES
    -1,  // TB_GPU_COUNT
    10,  // TFBC_VERSION
    16,  // TILE_SIZE_X
    16,  // TILE_SIZE_Y
    40,  // VIRTUAL_ADDRESS_SPACE_BITS
    1,   // XE_ARCHITECTURE
    19,  // XPU_MAX_REGBANKS_ADDR_WIDTH
    3,   // XPU_MAX_SLAVES
    1,   // XPU_REGISTER_BROADCAST
];

const IDX_META_COREMEM_SIZE: usize = 10;
const IDX_PHYS_BUS_WIDTH: usize = 16;
const IDX_SLC_BANKS: usize = 18;
const IDX_SLC_CACHE_LINE_SIZE_BITS: usize = 19;

/// Registers of `rgx_cr_defs_km.h` this file touches itself (the DDK code
/// names its own): the OS0 interrupt status and clear (`RGXDeviceAckIrq`).
const RGX_CR_IRQ_OS0_EVENT_STATUS: u32 = 0x0BD8;
const RGX_CR_IRQ_OS0_EVENT_STATUS_SOURCE_EN: u32 = 0x1;
const RGX_CR_IRQ_OS0_EVENT_CLEAR: u32 = 0x0BE8;
const RGX_CR_IRQ_OS0_EVENT_CLEAR_SOURCE_EN: u32 = 0x1;

/// The DDK's poll bound on this SoC: TI's system layer
/// (`services/system/rgx_ti_linux/sysinfo.h`, not vendored) sets
/// `MAX_HW_TIME_US` 500000 and `WAIT_TRY_COUNT` 10000, i.e. a read every
/// 50 us for at most 0.5 s. Used when the context leaves them 0.
pub const POLL_TIMEOUT_US_DEFAULT: u32 = 500_000;
pub const POLL_STEP_US_DEFAULT: u32 = 50;

// ---- the context (hPrivate) -------------------------------------------------

/// `struct yd_pvr_bus`: register access hooks for a host check. Addresses are
/// absolute (`regs` + offset), as [`Mmio`] sees them. `barrier` is called
/// where `RGXWriteRegMemoryBarrier32/64` would issue the barrier.
#[repr(C)]
pub struct YdPvrBus {
    pub ctx: *mut c_void,
    pub read32: unsafe extern "C" fn(ctx: *mut c_void, addr: usize) -> u32,
    pub write32: unsafe extern "C" fn(ctx: *mut c_void, addr: usize, val: u32),
    pub barrier: unsafe extern "C" fn(ctx: *mut c_void, addr: usize),
}

/// `struct yd_pvr_layer`: what `hPrivate` points at. Filled by the C driver;
/// `powered` is written by `RGXSetPoweredState`, so the object must be
/// writable even though the DDK passes it as `const void *`.
#[repr(C)]
pub struct YdPvrLayer {
    /// The GPU register block, mapped (`device_map()`).
    pub regs: usize,
    /// The OS services (busy wait, log). Required.
    pub env: *const YdEnv,
    /// Null on the board (register access is [`Hw`]); a host check's
    /// recording bus otherwise.
    pub bus: *const YdPvrBus,
    /// `RGXAcquireKernelMMUPC`: the physical address of the kernel (firmware)
    /// MMU context's page catalogue.
    pub kernel_pc: u64,
    /// `RGXAcquireBootCodeAddr`: the firmware code allocation's device
    /// virtual address (the DDK requires it to be RGX_FIRMWARE_RAW_HEAP_BASE).
    pub fw_code_devva: u64,
    /// `RGXAcquireBootDataAddr`: the firmware data allocation's.
    pub fw_data_devva: u64,
    /// The GPU core clock (`RGXWaitCycles`); 0: wait the microseconds only.
    pub core_clock_hz: u32,
    /// `RGXPollReg32/64` bound and step; 0: [`POLL_TIMEOUT_US_DEFAULT`],
    /// [`POLL_STEP_US_DEFAULT`].
    pub poll_timeout_us: u32,
    pub poll_step_us: u32,
    /// `RGXSetPoweredState`: 1 powered, 0 not.
    pub powered: u32,
}

// The layout include/yakogl_drivers.h pins with _Static_assert (LP64).
const _: () = {
    assert!(size_of::<YdPvrBus>() == 32);
    assert!(offset_of!(YdPvrBus, ctx) == 0);
    assert!(offset_of!(YdPvrBus, read32) == 8);
    assert!(offset_of!(YdPvrBus, write32) == 16);
    assert!(offset_of!(YdPvrBus, barrier) == 24);
    assert!(size_of::<YdPvrLayer>() == 64);
    assert!(offset_of!(YdPvrLayer, regs) == 0);
    assert!(offset_of!(YdPvrLayer, env) == 8);
    assert!(offset_of!(YdPvrLayer, bus) == 16);
    assert!(offset_of!(YdPvrLayer, kernel_pc) == 24);
    assert!(offset_of!(YdPvrLayer, fw_code_devva) == 32);
    assert!(offset_of!(YdPvrLayer, fw_data_devva) == 40);
    assert!(offset_of!(YdPvrLayer, core_clock_hz) == 48);
    assert!(offset_of!(YdPvrLayer, poll_timeout_us) == 52);
    assert!(offset_of!(YdPvrLayer, poll_step_us) == 56);
    assert!(offset_of!(YdPvrLayer, powered) == 60);
    assert!(size_of::<ImgDevPhyAddr>() == 8);
    assert!(size_of::<ImgDevVirtAddr>() == 8);
    assert!(size_of::<ImgBool>() == 1);
};

// ---- register access ----------------------------------------------------------

/// The register bus of one call: the hardware, or the context's hooks.
pub struct Bus<'a>(Option<&'a YdPvrBus>);

impl<'a> Bus<'a> {
    /// The hardware (`None`) or a host check's hooks.
    pub fn new(hooks: Option<&'a YdPvrBus>) -> Bus<'a> {
        Bus(hooks)
    }
}

impl Mmio for Bus<'_> {
    fn read32(&self, addr: usize) -> u32 {
        match self.0 {
            // SAFETY: the C side's hooks, with their own context.
            Some(b) => unsafe { (b.read32)(b.ctx, addr) },
            // SAFETY: `YdPvrLayer::regs` is the mapped register block.
            None => unsafe { Hw::new() }.read32(addr),
        }
    }

    fn write32(&self, addr: usize, val: u32) {
        match self.0 {
            // SAFETY: as above.
            Some(b) => unsafe { (b.write32)(b.ctx, addr, val) },
            // SAFETY: as above.
            None => unsafe { Hw::new() }.write32(addr, val),
        }
    }

    // The DDK layer has no byte accesses; these exist for the trait. The hooks
    // are 32-bit only, so a byte goes through its aligned word.
    fn read8(&self, addr: usize) -> u8 {
        match self.0 {
            Some(_) => (self.read32(addr & !3) >> (8 * (addr & 3))) as u8,
            // SAFETY: as above.
            None => unsafe { Hw::new() }.read8(addr),
        }
    }

    fn write8(&self, addr: usize, val: u8) {
        match self.0 {
            Some(_) => {
                let sh = 8 * (addr & 3);
                let w = self.read32(addr & !3) & !(0xff << sh);
                self.write32(addr & !3, w | (u32::from(val) << sh));
            }
            // SAFETY: as above.
            None => unsafe { Hw::new() }.write8(addr, val),
        }
    }
}

impl Bus<'_> {
    /// `RGXWriteRegMemoryBarrier32/64`. The DDK's is `OSWriteMemoryBarrier`,
    /// which on Linux is `wmb()` (`dsb st` on arm64) and reads the register
    /// back only with `PVRSRV_ENABLE_READBACK_ON_WRITES`, which the AM62P
    /// configuration does not set (config_kernel.h). So: `dsb st`, no read.
    fn wmb(&self, addr: usize) {
        match self.0 {
            // SAFETY: as above.
            Some(b) => unsafe { (b.barrier)(b.ctx, addr) },
            None => wmb_hw(),
        }
    }
}

#[cfg(target_arch = "aarch64")]
fn wmb_hw() {
    // SAFETY: a barrier; no memory is accessed.
    unsafe { core::arch::asm!("dsb st", options(nostack, preserves_flags)) }
}

#[cfg(not(target_arch = "aarch64"))]
fn wmb_hw() {
    core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
}

/// A 64-bit register as two 32-bit accesses, low word first: what the driver
/// has always done on this GPU (`pvr_wr64` in driver/src/pvr_internal.h). The
/// DDK on Linux uses one 64-bit access (`writeq`).
pub fn write64(m: &impl Mmio, addr: usize, val: u64) {
    m.write32(addr, val as u32);
    m.write32(addr + 4, (val >> 32) as u32);
}

/// Low word, then high word (`pvr_rd64`).
pub fn read64(m: &impl Mmio, addr: usize) -> u64 {
    let lo = m.read32(addr);
    let hi = m.read32(addr + 4);
    (u64::from(hi) << 32) | u64::from(lo)
}

/// `RGXReadModifyWriteReg64`: keep the bits set in `keep`, take the others
/// from `new` (the DDK's: `new &= ~keep; reg = (reg & keep) | new`).
pub fn read_modify_write64(m: &impl Mmio, addr: usize, new: u64, keep: u64) {
    let v = read64(m, addr);
    write64(m, addr, (v & keep) | (new & !keep));
}

/// `PVRSRVPollForValueKM` as `RGXPollReg32` uses it: read until
/// `(reg & mask) == value`, a read every `step_us`, giving up after
/// `timeout_us`. The first read is immediate; the last is at or after the
/// bound. Returns whether the value was seen.
pub fn poll32(
    m: &impl Mmio,
    env: &impl Env,
    addr: usize,
    value: u32,
    mask: u32,
    timeout_us: u32,
    step_us: u32,
) -> bool {
    let step = step_us.max(1);
    let mut waited: u32 = 0;
    loop {
        if m.read32(addr) & mask == value {
            return true;
        }
        if waited >= timeout_us {
            return false;
        }
        env.busy_wait_us(step);
        waited = waited.saturating_add(step);
    }
}

/// The time `RGXWaitCycles` waits: the DDK's waits `ui32WaitUs` on silicon
/// (`OSWaitus`; the cycle count is for PDump). Here the longer of the two,
/// the cycles at the core clock rounded up, so a slow clock is still covered.
pub fn wait_cycles_us(core_clock_hz: u32, cycles: u32, us: u32) -> u32 {
    if core_clock_hz == 0 {
        return us;
    }
    let c = (u64::from(cycles) * 1_000_000).div_ceil(u64::from(core_clock_hz));
    us.max(c.min(u64::from(u32::MAX)) as u32)
}

/// The DDK's `RGXAckIrqDedicated` (rgxinit_common.c `RGXAckHwIrq` with the
/// OS0 event registers): if the event is pending, clear it and read the clear
/// register back as a barrier.
pub fn ack_irq(m: &impl Mmio, regs: usize) -> bool {
    let status = m.read32(regs + RGX_CR_IRQ_OS0_EVENT_STATUS as usize);
    if status & RGX_CR_IRQ_OS0_EVENT_STATUS_SOURCE_EN == 0 {
        return false;
    }
    let clear = RGX_CR_IRQ_OS0_EVENT_CLEAR_SOURCE_EN & status;
    m.write32(regs + RGX_CR_IRQ_OS0_EVENT_CLEAR as usize, clear);
    let _ = m.read32(regs + RGX_CR_IRQ_OS0_EVENT_CLEAR as usize);
    true
}

/// The feature-value table's answer (`RGXDeviceGetFeatureValue`).
pub fn feature_value(idx: u64) -> i32 {
    match usize::try_from(idx) {
        Ok(i) if i < FEATURE_VALUES.len() => FEATURE_VALUES[i],
        _ => -1,
    }
}

fn feature_u32(idx: usize) -> u32 {
    // Absent (-1) reads as 0, as the DDK's RGX_IS_FEATURE_VALUE_SUPPORTED
    // guard makes it.
    u32::try_from(FEATURE_VALUES[idx]).unwrap_or(0)
}

// ---- the context -----------------------------------------------------------------

/// # Safety
/// `h` is a valid `struct yd_pvr_layer` whose `env` is valid, for the call.
unsafe fn layer<'a>(h: *const c_void) -> &'a YdPvrLayer {
    // SAFETY: the caller's contract.
    unsafe { &*(h as *const YdPvrLayer) }
}

impl YdPvrLayer {
    fn bus(&self) -> Bus<'_> {
        // SAFETY: null or the C side's hooks (the type's contract).
        Bus(unsafe { self.bus.as_ref() })
    }

    fn env(&self) -> &YdEnv {
        // SAFETY: required non-null (the type's contract).
        unsafe { &*self.env }
    }

    fn reg(&self, off: u32) -> usize {
        self.regs + off as usize
    }

    fn poll_bounds(&self) -> (u32, u32) {
        let t = if self.poll_timeout_us == 0 { POLL_TIMEOUT_US_DEFAULT } else { self.poll_timeout_us };
        let s = if self.poll_step_us == 0 { POLL_STEP_US_DEFAULT } else { self.poll_step_us };
        (t, s)
    }
}

fn not_supported(l: &YdPvrLayer, what: &str) {
    ylog!(l.env(), Err, "rgxlayer: {} is not supported (RISC-V firmware)", what);
}

// ---- the rgxlayer.h functions ------------------------------------------------
//
// Every one: `hPrivate` is a valid `struct yd_pvr_layer` (see [`layer`]),
// except where a function says it does not read it.

/// `RGXMemCopy`. The DDK's is `OSDeviceMemCopy`; the destinations here are
/// the firmware allocations in the carveout, mapped Normal non-cacheable, where
/// a plain copy is allowed (unaligned accesses included).
///
/// # Safety
/// `pvDst` and `pvSrc` valid for `uiSize` bytes and not overlapping.
#[no_mangle]
pub unsafe extern "C" fn RGXMemCopy(_hPrivate: *const c_void, pvDst: *mut c_void, pvSrc: *mut c_void, uiSize: usize) {
    // SAFETY: the caller's contract.
    unsafe { core::ptr::copy_nonoverlapping(pvSrc as *const u8, pvDst as *mut u8, uiSize) }
}

/// `RGXMemSet` (the DDK's `OSDeviceMemSet`).
///
/// # Safety
/// `pvDst` valid for `uiSize` bytes.
#[no_mangle]
pub unsafe extern "C" fn RGXMemSet(_hPrivate: *const c_void, pvDst: *mut c_void, ui8Value: u8, uiSize: usize) {
    // SAFETY: the caller's contract.
    unsafe { core::ptr::write_bytes(pvDst as *mut u8, ui8Value, uiSize) }
}

/// `RGXGetOSPageSize` (MIPS firmware only: the FW page size check of
/// `RGXValidateFWHeaderVersion1`). 4 KiB, Zephyr's page on this SoC. Does not
/// read `hPrivate`.
#[no_mangle]
pub extern "C" fn RGXGetOSPageSize(_hPrivate: *const c_void) -> u32 {
    4096
}

/// `RGXDeviceGetFeatureValue`: [`FEATURE_VALUES`]. Does not read `hPrivate`.
#[no_mangle]
pub extern "C" fn RGXDeviceGetFeatureValue(_hPrivate: *const c_void, ui64Feature: u64) -> i32 {
    feature_value(ui64Feature)
}

/// `RGXDeviceHasFeature`: [`FEATURES`] (one word: every feature of this DDK
/// has array index 0). Does not read `hPrivate`.
#[no_mangle]
pub extern "C" fn RGXDeviceHasFeature(_hPrivate: *const c_void, ui16FeatureIndex: u16, ui64Feature: u64) -> ImgBool {
    ui16FeatureIndex == 0 && FEATURES & ui64Feature != 0
}

/// `RGXDeviceHasErnBrn`: [`ERNS_BRNS`]. Does not read `hPrivate`.
#[no_mangle]
pub extern "C" fn RGXDeviceHasErnBrn(_hPrivate: *const c_void, ui64ErnsBrns: u64) -> ImgBool {
    ERNS_BRNS & ui64ErnsBrns != 0
}

/// `RGXGetFWCorememSize`: META_COREMEM_SIZE, which this core does not have:
/// 0, as the DDK's. Does not read `hPrivate`.
#[no_mangle]
pub extern "C" fn RGXGetFWCorememSize(_hPrivate: *const c_void) -> u32 {
    feature_u32(IDX_META_COREMEM_SIZE)
}

/// `RGXWriteReg32`.
///
/// # Safety
/// See the section comment.
#[no_mangle]
pub unsafe extern "C" fn RGXWriteReg32(hPrivate: *const c_void, ui32RegAddr: u32, ui32RegValue: u32) {
    // SAFETY: the caller's contract.
    let l = unsafe { layer(hPrivate) };
    l.bus().write32(l.reg(ui32RegAddr), ui32RegValue);
}

/// `RGXWriteReg64`: [`write64`].
///
/// # Safety
/// See the section comment.
#[no_mangle]
pub unsafe extern "C" fn RGXWriteReg64(hPrivate: *const c_void, ui32RegAddr: u32, ui64RegValue: u64) {
    // SAFETY: the caller's contract.
    let l = unsafe { layer(hPrivate) };
    write64(&l.bus(), l.reg(ui32RegAddr), ui64RegValue);
}

/// `RGXReadReg32`.
///
/// # Safety
/// See the section comment.
#[no_mangle]
pub unsafe extern "C" fn RGXReadReg32(hPrivate: *const c_void, ui32RegAddr: u32) -> u32 {
    // SAFETY: the caller's contract.
    let l = unsafe { layer(hPrivate) };
    l.bus().read32(l.reg(ui32RegAddr))
}

/// `RGXReadReg64`: [`read64`].
///
/// # Safety
/// See the section comment.
#[no_mangle]
pub unsafe extern "C" fn RGXReadReg64(hPrivate: *const c_void, ui32RegAddr: u32) -> u64 {
    // SAFETY: the caller's contract.
    let l = unsafe { layer(hPrivate) };
    read64(&l.bus(), l.reg(ui32RegAddr))
}

/// `RGXReadModifyWriteReg64`: [`read_modify_write64`]; returns `PVRSRV_OK`
/// (declared `IMG_UINT32`, as the DDK's).
///
/// # Safety
/// See the section comment.
#[no_mangle]
pub unsafe extern "C" fn RGXReadModifyWriteReg64(
    hPrivate: *const c_void,
    ui32RegAddr: u32,
    ui64RegValue: u64,
    ui64RegKeepMask: u64,
) -> u32 {
    // SAFETY: the caller's contract.
    let l = unsafe { layer(hPrivate) };
    read_modify_write64(&l.bus(), l.reg(ui32RegAddr), ui64RegValue, ui64RegKeepMask);
    PVRSRV_OK
}

/// `RGXPollReg32`: [`poll32`] within the context's bound; on timeout logs the
/// DDK's line and returns `PVRSRV_ERROR_TIMEOUT`.
///
/// # Safety
/// See the section comment.
#[no_mangle]
pub unsafe extern "C" fn RGXPollReg32(
    hPrivate: *const c_void,
    ui32RegAddr: u32,
    ui32RegValue: u32,
    ui32RegMask: u32,
) -> PvrsrvError {
    // SAFETY: the caller's contract.
    let l = unsafe { layer(hPrivate) };
    let (t, s) = l.poll_bounds();
    if poll32(&l.bus(), l.env(), l.reg(ui32RegAddr), ui32RegValue, ui32RegMask, t, s) {
        PVRSRV_OK
    } else {
        ylog!(l.env(), Err, "RGXPollReg32: Poll for Reg (0x{:x}) failed", ui32RegAddr);
        PVRSRV_ERROR_TIMEOUT
    }
}

/// `RGXPollReg64`: as the DDK's, the upper word is polled first, then the
/// lower, each with its own bound.
///
/// # Safety
/// See the section comment.
#[no_mangle]
pub unsafe extern "C" fn RGXPollReg64(
    hPrivate: *const c_void,
    ui32RegAddr: u32,
    ui64RegValue: u64,
    ui64RegMask: u64,
) -> PvrsrvError {
    // SAFETY: the caller's contract.
    let l = unsafe { layer(hPrivate) };
    let (t, s) = l.poll_bounds();
    let bus = l.bus();
    let a = l.reg(ui32RegAddr);
    if !poll32(&bus, l.env(), a + 4, (ui64RegValue >> 32) as u32, (ui64RegMask >> 32) as u32, t, s) {
        ylog!(l.env(), Err, "RGXPollReg64: Poll for upper part of Reg (0x{:x}) failed", ui32RegAddr);
        return PVRSRV_ERROR_TIMEOUT;
    }
    if !poll32(&bus, l.env(), a, ui64RegValue as u32, ui64RegMask as u32, t, s) {
        ylog!(l.env(), Err, "RGXPollReg64: Poll for lower part of Reg (0x{:x}) failed", ui32RegAddr);
        return PVRSRV_ERROR_TIMEOUT;
    }
    PVRSRV_OK
}

/// `RGXWriteRegMemoryBarrier32`: see [`Bus::wmb`].
///
/// # Safety
/// See the section comment.
#[no_mangle]
pub unsafe extern "C" fn RGXWriteRegMemoryBarrier32(hPrivate: *const c_void, ui32RegAddr: u32) {
    // SAFETY: the caller's contract.
    let l = unsafe { layer(hPrivate) };
    l.bus().wmb(l.reg(ui32RegAddr));
}

/// `RGXWriteRegMemoryBarrier64`: see [`Bus::wmb`].
///
/// # Safety
/// See the section comment.
#[no_mangle]
pub unsafe extern "C" fn RGXWriteRegMemoryBarrier64(hPrivate: *const c_void, ui32RegAddr: u32) {
    // SAFETY: the caller's contract.
    let l = unsafe { layer(hPrivate) };
    l.bus().wmb(l.reg(ui32RegAddr));
}

/// `RGXSetPoweredState`: recorded in the context (the DDK's sets
/// `bRGXPowered`, which gates its interrupt handler; ours reads `powered`).
///
/// # Safety
/// See the section comment; the context must be writable.
#[no_mangle]
pub unsafe extern "C" fn RGXSetPoweredState(hPrivate: *const c_void, bPowered: ImgBool) {
    let p = hPrivate as *mut YdPvrLayer;
    // SAFETY: the caller's contract; the C object is not const.
    unsafe { core::ptr::addr_of_mut!((*p).powered).write_volatile(u32::from(bPowered)) }
}

/// `RGXWaitCycles`: busy-wait [`wait_cycles_us`].
///
/// # Safety
/// See the section comment.
#[no_mangle]
pub unsafe extern "C" fn RGXWaitCycles(hPrivate: *const c_void, ui32Cycles: u32, ui32WaitUs: u32) {
    // SAFETY: the caller's contract.
    let l = unsafe { layer(hPrivate) };
    l.env().busy_wait_us(wait_cycles_us(l.core_clock_hz, ui32Cycles, ui32WaitUs));
}

/// `RGXAcquireKernelMMUPC`: `kernel_pc`.
///
/// # Safety
/// See the section comment; `psPCAddr` writable.
#[no_mangle]
pub unsafe extern "C" fn RGXAcquireKernelMMUPC(hPrivate: *const c_void, psPCAddr: *mut ImgDevPhyAddr) {
    // SAFETY: the caller's contract.
    unsafe { *psPCAddr = ImgDevPhyAddr { ui_addr: layer(hPrivate).kernel_pc } }
}

/// Write 0 to an address out-parameter of a MIPS entry point and log.
///
/// # Safety
/// As the entry points.
unsafe fn mips_addr(hPrivate: *const c_void, out: *mut ImgDevPhyAddr, what: &str) {
    // SAFETY: the caller's contract.
    unsafe {
        *out = ImgDevPhyAddr { ui_addr: 0 };
        not_supported(layer(hPrivate), what);
    }
}

/// `RGXAcquireGPURegsAddr` (MIPS): not supported; 0.
///
/// # Safety
/// See the section comment; the out-parameter writable.
#[no_mangle]
pub unsafe extern "C" fn RGXAcquireGPURegsAddr(hPrivate: *const c_void, psGPURegsAddr: *mut ImgDevPhyAddr) {
    // SAFETY: the caller's contract.
    unsafe { mips_addr(hPrivate, psGPURegsAddr, "RGXAcquireGPURegsAddr") }
}

/// `RGXAcquireBootRemapAddr` (MIPS): not supported; 0.
///
/// # Safety
/// See the section comment; the out-parameter writable.
#[no_mangle]
pub unsafe extern "C" fn RGXAcquireBootRemapAddr(hPrivate: *const c_void, psBootRemapAddr: *mut ImgDevPhyAddr) {
    // SAFETY: the caller's contract.
    unsafe { mips_addr(hPrivate, psBootRemapAddr, "RGXAcquireBootRemapAddr") }
}

/// `RGXAcquireCodeRemapAddr` (MIPS): not supported; 0.
///
/// # Safety
/// See the section comment; the out-parameter writable.
#[no_mangle]
pub unsafe extern "C" fn RGXAcquireCodeRemapAddr(hPrivate: *const c_void, psCodeRemapAddr: *mut ImgDevPhyAddr) {
    // SAFETY: the caller's contract.
    unsafe { mips_addr(hPrivate, psCodeRemapAddr, "RGXAcquireCodeRemapAddr") }
}

/// `RGXAcquireDataRemapAddr` (MIPS): not supported; 0.
///
/// # Safety
/// See the section comment; the out-parameter writable.
#[no_mangle]
pub unsafe extern "C" fn RGXAcquireDataRemapAddr(hPrivate: *const c_void, psDataRemapAddr: *mut ImgDevPhyAddr) {
    // SAFETY: the caller's contract.
    unsafe { mips_addr(hPrivate, psDataRemapAddr, "RGXAcquireDataRemapAddr") }
}

/// `RGXAcquireTrampolineRemapAddr` (MIPS): not supported; 0.
///
/// # Safety
/// See the section comment; the out-parameter writable.
#[no_mangle]
pub unsafe extern "C" fn RGXAcquireTrampolineRemapAddr(
    hPrivate: *const c_void,
    psTrampolineRemapAddr: *mut ImgDevPhyAddr,
) {
    // SAFETY: the caller's contract.
    unsafe { mips_addr(hPrivate, psTrampolineRemapAddr, "RGXAcquireTrampolineRemapAddr") }
}

/// `RGXDoFWSlaveBoot` (META): no slave boot. The DDK's answers whether the
/// system snoops the CPU cache, for META only.
///
/// # Safety
/// See the section comment.
#[no_mangle]
pub unsafe extern "C" fn RGXDoFWSlaveBoot(hPrivate: *const c_void) -> ImgBool {
    // SAFETY: the caller's contract.
    not_supported(unsafe { layer(hPrivate) }, "RGXDoFWSlaveBoot");
    false
}

/// `RGXFabricCoherencyTest` (META slave boot only): not supported.
///
/// # Safety
/// See the section comment.
#[no_mangle]
pub unsafe extern "C" fn RGXFabricCoherencyTest(hPrivate: *const c_void) -> PvrsrvError {
    // SAFETY: the caller's contract.
    not_supported(unsafe { layer(hPrivate) }, "RGXFabricCoherencyTest");
    PVRSRV_ERROR_NOT_SUPPORTED
}

/// `RGXGetDeviceSLCBanks`: SLC_BANKS (1). Does not read `hPrivate`.
#[no_mangle]
pub extern "C" fn RGXGetDeviceSLCBanks(_hPrivate: *const c_void) -> u32 {
    feature_u32(IDX_SLC_BANKS)
}

/// `RGXGetDeviceCacheLineSize`: SLC_CACHE_LINE_SIZE_BITS (512). Does not
/// read `hPrivate`.
#[no_mangle]
pub extern "C" fn RGXGetDeviceCacheLineSize(_hPrivate: *const c_void) -> u32 {
    feature_u32(IDX_SLC_CACHE_LINE_SIZE_BITS)
}

/// `RGXGetDevicePhysBusWidth` (MIPS paths): PHYS_BUS_WIDTH (36). Does not
/// read `hPrivate`.
#[no_mangle]
pub extern "C" fn RGXGetDevicePhysBusWidth(_hPrivate: *const c_void) -> u32 {
    feature_u32(IDX_PHYS_BUS_WIDTH)
}

/// `RGXDevicePA0IsValid` (MIPS paths): no. The AM62P has no memory at
/// physical address 0 (DDR starts at 0x8000_0000). Does not read `hPrivate`.
#[no_mangle]
pub extern "C" fn RGXDevicePA0IsValid(_hPrivate: *const c_void) -> ImgBool {
    false
}

/// `RGXAcquireBootCodeAddr`: `fw_code_devva`.
///
/// # Safety
/// See the section comment; `psBootCodeAddr` writable.
#[no_mangle]
pub unsafe extern "C" fn RGXAcquireBootCodeAddr(hPrivate: *const c_void, psBootCodeAddr: *mut ImgDevVirtAddr) {
    // SAFETY: the caller's contract.
    unsafe { *psBootCodeAddr = ImgDevVirtAddr { ui_addr: layer(hPrivate).fw_code_devva } }
}

/// `RGXAcquireBootDataAddr`: `fw_data_devva`.
///
/// # Safety
/// See the section comment; `psBootDataAddr` writable.
#[no_mangle]
pub unsafe extern "C" fn RGXAcquireBootDataAddr(hPrivate: *const c_void, psBootDataAddr: *mut ImgDevVirtAddr) {
    // SAFETY: the caller's contract.
    unsafe { *psBootDataAddr = ImgDevVirtAddr { ui_addr: layer(hPrivate).fw_data_devva } }
}

/// `RGXDeviceAckIrq`: [`ack_irq`].
///
/// # Safety
/// See the section comment.
#[no_mangle]
pub unsafe extern "C" fn RGXDeviceAckIrq(hPrivate: *const c_void) -> ImgBool {
    // SAFETY: the caller's contract.
    let l = unsafe { layer(hPrivate) };
    ack_irq(&l.bus(), l.regs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Access, Model, GPU};
    use std::cell::RefCell;

    struct Rec {
        waits: RefCell<Vec<u32>>,
        lines: RefCell<Vec<String>>,
    }

    unsafe extern "C" fn wait(ctx: *mut c_void, us: u32) {
        // SAFETY: ctx is the test's Rec
        unsafe { &*(ctx as *const Rec) }.waits.borrow_mut().push(us);
    }
    unsafe extern "C" fn sleep(_: *mut c_void, _: u32) {}
    unsafe extern "C" fn log(ctx: *mut c_void, _: u32, b: *const u8, n: usize) {
        // SAFETY: ctx is the test's Rec; b valid for n
        let r = unsafe { &*(ctx as *const Rec) };
        r.lines.borrow_mut().push(String::from_utf8_lossy(unsafe { core::slice::from_raw_parts(b, n) }).into_owned());
    }
    unsafe extern "C" fn i2c_w(_: *mut c_void, _: u32, _: *const u8, _: usize) -> i32 {
        -crate::errno::ENOTSUP
    }
    unsafe extern "C" fn i2c_wr(_: *mut c_void, _: u32, _: *const u8, _: usize, _: *mut u8, _: usize) -> i32 {
        -crate::errno::ENOTSUP
    }
    unsafe extern "C" fn gpio(_: *mut c_void, _: u32, _: u32) -> i32 {
        -crate::errno::ENOTSUP
    }

    // The model as a C bus, so the exported functions themselves are tested.
    unsafe extern "C" fn bus_r(ctx: *mut c_void, a: usize) -> u32 {
        // SAFETY: ctx is the test's Model
        unsafe { &*(ctx as *const Model) }.read32(a)
    }
    unsafe extern "C" fn bus_w(ctx: *mut c_void, a: usize, v: u32) {
        // SAFETY: as above
        unsafe { &*(ctx as *const Model) }.write32(a, v)
    }
    unsafe extern "C" fn bus_b(_: *mut c_void, _: usize) {}

    struct Rig {
        m: Box<Model>,
        rec: Box<Rec>,
        env: Box<YdEnv>,
        bus: Box<YdPvrBus>,
        l: Box<YdPvrLayer>,
    }

    fn rig() -> Rig {
        let m = Box::new(Model::new());
        let rec = Box::new(Rec { waits: RefCell::new(vec![]), lines: RefCell::new(vec![]) });
        let env = Box::new(YdEnv {
            ctx: &*rec as *const Rec as *mut c_void,
            busy_wait_us: wait,
            sleep_ms: sleep,
            log,
            i2c_write: i2c_w,
            i2c_write_read: i2c_wr,
            gpio_set: gpio,
        });
        let bus = Box::new(YdPvrBus {
            ctx: &*m as *const Model as *mut c_void,
            read32: bus_r,
            write32: bus_w,
            barrier: bus_b,
        });
        let l = Box::new(YdPvrLayer {
            regs: m.base(GPU),
            env: &*env,
            bus: &*bus,
            kernel_pc: 0xe030_0000,
            fw_code_devva: 0xe1_c000_0000,
            fw_data_devva: 0xe1_c008_0000,
            core_clock_hz: 800_000_000,
            poll_timeout_us: 200,
            poll_step_us: 50,
            powered: 0,
        });
        Rig { m, rec, env, bus, l }
    }

    impl Rig {
        fn h(&self) -> *const c_void {
            &*self.l as *const YdPvrLayer as *const c_void
        }
    }

    #[test]
    fn a_64_bit_write_is_low_word_then_high_word() {
        let r = rig();
        unsafe { RGXWriteReg64(r.h(), 0x100, 0x1122_3344_5566_7788) };
        assert_eq!(r.m.writes(), vec![(GPU, 0x100, 0x5566_7788), (GPU, 0x104, 0x1122_3344)]);
        assert_eq!(unsafe { RGXReadReg64(r.h(), 0x100) }, 0x1122_3344_5566_7788);
        let _ = (&r.env, &r.bus);
    }

    #[test]
    fn read_modify_write_keeps_the_keep_mask() {
        let r = rig();
        r.m.poke(GPU, 0x3828, 0xf0f0_f0f0);
        r.m.poke(GPU, 0x382c, 0x0000_00ff);
        let new = 0x3u64;
        assert_eq!(unsafe { RGXReadModifyWriteReg64(r.h(), 0x3828, new, !new) }, PVRSRV_OK);
        assert_eq!(r.m.peek(GPU, 0x3828), 0xf0f0_f0f3);
        assert_eq!(r.m.peek(GPU, 0x382c), 0x0000_00ff);
    }

    #[test]
    fn a_poll_that_holds_reads_once_and_waits_not() {
        let r = rig();
        r.m.poke(GPU, 0x8f8, 0x3);
        assert_eq!(unsafe { RGXPollReg32(r.h(), 0x8f8, 0x3, 0x3) }, PVRSRV_OK);
        assert_eq!(r.m.accesses(), vec![Access::R(GPU, 0x8f8, 3)]);
        assert!(r.rec.waits.borrow().is_empty());
    }

    #[test]
    fn a_poll_that_never_holds_times_out_after_the_bound() {
        let r = rig();
        assert_eq!(unsafe { RGXPollReg32(r.h(), 0x8f8, 0x3, 0x3) }, PVRSRV_ERROR_TIMEOUT);
        // 200 us at 50 us a step: reads at 0, 50, 100, 150, 200.
        assert_eq!(r.m.accesses().len(), 5);
        assert_eq!(*r.rec.waits.borrow(), vec![50; 4]);
        assert!(r.rec.lines.borrow()[0].contains("Poll for Reg (0x8f8) failed"));
    }

    #[test]
    fn a_64_bit_poll_reads_the_upper_word_first() {
        let r = rig();
        assert_eq!(unsafe { RGXPollReg64(r.h(), 0x3870, 0, u64::MAX) }, PVRSRV_OK);
        assert_eq!(r.m.accesses(), vec![Access::R(GPU, 0x3874, 0), Access::R(GPU, 0x3870, 0)]);
    }

    #[test]
    fn wait_cycles_takes_the_longer_of_cycles_and_microseconds() {
        assert_eq!(wait_cycles_us(800_000_000, 32, 3), 3);
        assert_eq!(wait_cycles_us(1_000_000, 32, 3), 32);
        assert_eq!(wait_cycles_us(0, 32, 3), 3);
        let r = rig();
        unsafe { RGXWaitCycles(r.h(), 32, 3) };
        assert_eq!(*r.rec.waits.borrow(), vec![3]);
    }

    #[test]
    fn the_irq_ack_clears_only_a_pending_event() {
        let r = rig();
        assert!(!unsafe { RGXDeviceAckIrq(r.h()) });
        assert!(r.m.writes().is_empty());
        r.m.poke(GPU, 0xbd8, 1);
        assert!(unsafe { RGXDeviceAckIrq(r.h()) });
        assert_eq!(r.m.writes(), vec![(GPU, 0xbe8, 1)]);
    }

    #[test]
    fn the_powered_state_and_the_addresses_come_from_the_context() {
        let r = rig();
        unsafe { RGXSetPoweredState(r.h(), true) };
        assert_eq!(r.l.powered, 1);
        let mut pc = ImgDevPhyAddr { ui_addr: 1 };
        let mut code = ImgDevVirtAddr { ui_addr: 1 };
        let mut data = ImgDevVirtAddr { ui_addr: 1 };
        unsafe {
            RGXAcquireKernelMMUPC(r.h(), &mut pc);
            RGXAcquireBootCodeAddr(r.h(), &mut code);
            RGXAcquireBootDataAddr(r.h(), &mut data);
        }
        assert_eq!((pc.ui_addr, code.ui_addr, data.ui_addr), (0xe030_0000, 0xe1_c000_0000, 0xe1_c008_0000));
    }

    #[test]
    fn meta_and_mips_entry_points_refuse_and_log() {
        let r = rig();
        let mut a = ImgDevPhyAddr { ui_addr: 7 };
        unsafe { RGXAcquireBootRemapAddr(r.h(), &mut a) };
        assert_eq!(a.ui_addr, 0);
        assert!(!unsafe { RGXDoFWSlaveBoot(r.h()) });
        assert_eq!(unsafe { RGXFabricCoherencyTest(r.h()) }, PVRSRV_ERROR_NOT_SUPPORTED);
        assert_eq!(r.rec.lines.borrow().len(), 3);
        assert!(r.m.accesses().is_empty());
    }

    #[test]
    fn the_core_answers() {
        let n = core::ptr::null();
        // RISCV_FW_PROCESSOR, SYS_BUS_SECURE_RESET, PBE2_IN_XE, AXI_ACELITE yes;
        // MIPS (0x1000), XT_TOP_INFRASTRUCTURE (1 << 42) no.
        assert!(RGXDeviceHasFeature(n, 0, 0x80000));
        assert!(RGXDeviceHasFeature(n, 0, 0x1000_0000));
        assert!(RGXDeviceHasFeature(n, 0, 0x2000));
        assert!(RGXDeviceHasFeature(n, 0, 0x1));
        assert!(!RGXDeviceHasFeature(n, 0, 0x1000));
        assert!(!RGXDeviceHasFeature(n, 0, 1 << 42));
        assert!(!RGXDeviceHasFeature(n, 1, 0x1));
        assert!(RGXDeviceHasErnBrn(n, 0x80000)); // FIX_HW_BRN_71317
        assert!(!RGXDeviceHasErnBrn(n, 0x40000)); // FIX_HW_BRN_71242
        assert_eq!(RGXDeviceGetFeatureValue(n, 1), 2); // ECC_RAMS
        assert_eq!(RGXDeviceGetFeatureValue(n, 8), 1); // LAYOUT_MARS
        assert_eq!(RGXDeviceGetFeatureValue(n, 9), -1); // META
        assert_eq!(RGXDeviceGetFeatureValue(n, 30), -1);
        assert_eq!(RGXDeviceGetFeatureValue(n, u64::MAX), -1);
        assert_eq!((RGXGetDeviceSLCBanks(n), RGXGetDeviceCacheLineSize(n)), (1, 512));
        assert_eq!((RGXGetDevicePhysBusWidth(n), RGXGetFWCorememSize(n)), (36, 0));
    }

    /// The two words are written twice, here and in yakogl_drivers.h (where
    /// the board's build asserts them against the DDK headers): equal.
    #[test]
    fn the_words_match_the_c_header() {
        let h = include_str!("../../../include/yakogl_drivers.h");
        let word = |name: &str| -> u64 {
            let line = h.lines().find(|l| l.starts_with(&format!("#define {name} "))).expect(name);
            let v = line.split_whitespace().nth(2).unwrap().trim_end_matches("ULL");
            u64::from_str_radix(v.trim_start_matches("0x"), 16).unwrap()
        };
        assert_eq!(word("YD_PVR_RGX_FEATURES"), FEATURES);
        assert_eq!(word("YD_PVR_RGX_ERNS_BRNS"), ERNS_BRNS);
    }

    #[test]
    fn copy_and_set() {
        let mut d = [0u8; 8];
        let mut s = *b"abcdefgh";
        unsafe {
            RGXMemCopy(core::ptr::null(), d.as_mut_ptr().cast(), s.as_mut_ptr().cast(), 8);
            RGXMemSet(core::ptr::null(), d.as_mut_ptr().add(4).cast(), 0, 4);
        }
        assert_eq!(&d, b"abcd\0\0\0\0");
    }
}
