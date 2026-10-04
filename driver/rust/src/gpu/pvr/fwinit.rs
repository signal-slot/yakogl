// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The firmware-interface structures the host builds before the firmware
//! starts, in the layouts of the DDK's `rgx_fwif_km.h` / `rgx_fwif_shared.h`
//! (offsets in [`super::fwif`], pinned), and the wait for the firmware's
//! "started" flag.
//!
//! Three structures sit at fixed offsets of the firmware heap's config
//! region, where the firmware is linked to find them (`rgx_heap_firmware.h`:
//! one `RGX_FIRMWARE_CONFIG_HEAP_ALLOC_GRANULARITY` slot each): the
//! connection control, the OS init (the CCBs, the OS data, the HWR info
//! buffer, the HWPerf and GPU utilisation controls) and the system init (the
//! fault page, the PDS/USC execution bases, the runtime configuration, the
//! trace buffer, the system data, the alignment checks). Everything they
//! point at is allocated from the heap's main region in the order the DDK's
//! `RGXSetupFirmware` / `RGXSetupFwSysInit` / `RGXSetupFwOsInit`
//! (rgxfwutils.c) would need it, each at its structure's size.
//!
//! Before docs/source-form.md step 7 this was C in `pvr_fw_boot.c` that began
//! in a port of Linux drm/imagination (`pvr_fw.c`); the objects were then
//! allocated at three sizes kept from the mainline structures
//! (`PVR_LEGACY_SIZE_*`: the CCB controls at 24 bytes, the trace buffer at
//! 864, the GPU utilisation at 10280). They are the DDK's sizes now
//! ([`YdPvrFwinit`]'s size fields 0); the host check builds both and compares
//! the former with the C, byte for byte (`tools/pvr/km`).

use super::ccb::{FWCCB_NUM_CMDS, KCCB_NUM_CMDS};
use super::fwif::*;
use super::km::{km, YdPvrKm};
use super::mem::heap_alloc;
use crate::errno;

/// `struct yd_pvr_fwinit`: what the driver chooses.
#[repr(C)]
pub struct YdPvrFwinit {
    /// `RGXFWIF_RUNTIME_CFG.ui32ActivePMLatencyms`.
    pub pm_latency_ms: u32,
    /// The core clock the firmware is told (runtime configuration: core and
    /// SOC clock; system init: initial core clock).
    pub core_clock_hz: u32,
    /// `FWINIT_*`.
    pub flags: u32,
    /// 0: the DDK structure's size. Otherwise the size to allocate (the host
    /// check's former sizes).
    pub ccb_ctl_size: u32,
    pub tracebuf_size: u32,
    pub gpu_util_size: u32,
}

/// Halt (do not reset) on a hardware recovery, so the fault registers persist
/// (`RGXFWIF_INICFG_ASSERT_ON_HWR_TRIGGER`; a bring-up aid).
pub const FWINIT_ASSERT_ON_HWR: u32 = 1 << 0;
/// No context switching (`ui32FwOsConfigFlags` 0). Measured: the geometry
/// data master then processes nothing; a bring-up aid.
pub const FWINIT_CTXSWITCH_OFF: u32 = 1 << 1;

const _: () = {
    assert!(core::mem::size_of::<YdPvrFwinit>() == 24);
};

fn size_or(v: u32, ddk: usize) -> u32 {
    if v != 0 {
        v
    } else {
        ddk as u32
    }
}

/// A zeroed object of `size` bytes in the heap's main region; its heap
/// offset, CPU address and firmware address.
fn obj(k: &mut YdPvrKm, size: u32, align: u32) -> (u32, *mut u8, u32) {
    let off = heap_alloc(k, size, align);
    (off, k.heap_cpu(off), k.heap_fwaddr(off))
}

/// The config region's slot `n` (0 connection control, 1 OS init, 2 system
/// init), as a heap offset.
fn config_slot(k: &YdPvrKm, n: u32) -> u32 {
    k.fw_config_off + n * FW_CONFIG_HEAP_ALLOC_GRANULARITY
}

/// # Safety
/// `ctl` a CCB control block.
unsafe fn ccb_ctl_init(ctl: *mut u8, num: u32) {
    // SAFETY: the caller's contract.
    unsafe {
        wr32(ctl, CCB_CTL_WRITE_OFFSET, 0);
        wr32(ctl, CCB_CTL_READ_OFFSET, 0);
        wr32(ctl, CCB_CTL_WRAP_MASK, num - 1);
    }
}

/// Build every structure the firmware reads at boot. The firmware image is
/// loaded and `k.heap_next` is where the host's objects start. The
/// structures are written, not cleaned: the driver cleans the whole carveout
/// before it starts the firmware.
pub fn build(k: &mut YdPvrKm, cfg: &YdPvrFwinit) {
    let ccb_ctl_size = size_or(cfg.ccb_ctl_size, CCB_CTL_SIZE);

    // The KCCB: control, the command ring (the firmware indexes it by the
    // command size), one return word per slot.
    let (kccb_ctl, kccb_ctl_p, kccb_ctl_fa) = obj(k, ccb_ctl_size, 8);
    let (kccb_cmds, _, kccb_cmds_fa) = obj(k, KCCB_NUM_CMDS * KCCB_CMD_SIZE as u32, 8);
    let (kccb_rtn, _, kccb_rtn_fa) = obj(k, KCCB_NUM_CMDS * 4, 8);
    // SAFETY: the control block just allocated.
    unsafe { ccb_ctl_init(kccb_ctl_p, KCCB_NUM_CMDS) };
    // The FWCCB: control and the command ring.
    let (fwccb_ctl, fwccb_ctl_p, fwccb_ctl_fa) = obj(k, ccb_ctl_size, 8);
    let (fwccb_cmds, _, fwccb_cmds_fa) = obj(k, FWCCB_NUM_CMDS * FWCCB_CMD_SIZE as u32, 8);
    // SAFETY: as above.
    unsafe { ccb_ctl_init(fwccb_ctl_p, FWCCB_NUM_CMDS) };
    k.kccb_ctl_off = kccb_ctl;
    k.kccb_cmds_off = kccb_cmds;
    k.kccb_rtn_off = kccb_rtn;
    k.fwccb_ctl_off = fwccb_ctl;
    k.fwccb_cmds_off = fwccb_cmds;

    // The supporting objects.
    let (_, _, power_sync_fa) = obj(k, 4, 8);
    let (hwrinfobuf, _, hwrinfobuf_fa) = obj(k, HWRINFOBUF_SIZE as u32, 8);
    k.hwrinfobuf_off = hwrinfobuf;
    let (_, sysdata_p, sysdata_fa) = obj(k, SYSDATA_SIZE as u32, 8);
    let (_, osdata_p, osdata_fa) = obj(k, OSDATA_SIZE as u32, 8);
    let (_, runtime_p, runtime_cfg_fa) = obj(k, RUNTIME_CFG_SIZE as u32, 8);
    let (_, _, gpu_util_fa) = obj(k, size_or(cfg.gpu_util_size, GPU_UTIL_FW_SIZE), 8);
    let (tracebuf, tbuf_p, tracebuf_fa) = obj(k, size_or(cfg.tracebuf_size, TRACEBUF_SIZE), 8);
    k.tracebuf_off = tracebuf;

    // The trace: one thread (RGXFW_THREAD_NUM 1) at the default size, every
    // group logged but MTS and POW. Those two are the scheduler's and the
    // power manager's, written several times a millisecond while idle: the
    // ring is a wrap buffer, and at a stall it would hold a millisecond of
    // idle chatter instead of the kicks and faults that led up to it
    // (cts-runner's captures came back 1050 MTS and 400 POW entries against
    // 18 MAIN, twice).
    let dwords = TRACE_BUF_DEFAULT_SIZE_IN_DWORDS;
    let (ring, _, ring_fa) = obj(k, dwords * 4, 8);
    k.trace_buf_off = ring;
    // SAFETY: the trace buffer structure just allocated.
    unsafe {
        wr32(
            tbuf_p,
            TRACEBUF_LOG_TYPE,
            (LOG_TYPE_MASK & !LOG_TYPE_GROUP_DEBUG) & !(LOG_TYPE_GROUP_MTS | LOG_TYPE_GROUP_POW),
        );
        wr32(tbuf_p, TRACEBUF_TRACE_POINTER, 0);
        wr32(tbuf_p, TRACEBUF_WRAP_COUNT, 0);
        wr32(tbuf_p, TRACEBUF_BUFFER, ring_fa);
        wr32(tbuf_p, TRACEBUF_SIZE_IN_DWORDS, dwords);
        wr32(tbuf_p, TRACEBUF_FLAGS, 0);
    }

    // The fault page: where the MMU sends an access it cannot translate.
    let (fault, fault_p, _) = obj(k, 4096, 0x1000);
    for i in 0..1024 {
        // SAFETY: the page just allocated.
        unsafe { wr32(fault_p, i * 4, 0xDEAD_BEE0) };
    }
    let fault_phys = k.heap_phys(fault);

    // System data: the DDK's default context-switch profile (GetFWConfigFlags,
    // FWCONTEXTSWITCHPROFILE_MEDIUM; captured from a running DDK as
    // 0x00020000).
    let mut sys_flags = INICFG_CTXSWITCH_PROFILE_MEDIUM;
    if cfg.flags & FWINIT_ASSERT_ON_HWR != 0 {
        sys_flags |= INICFG_ASSERT_ON_HWR_TRIGGER;
    }
    // OS data: context switching on for every data master (the DDK runs
    // "FW OS config flags 0x0F"; measured required: without it the firmware
    // does not apply the render context's static context-switch registers
    // and a geometry kick processes nothing), and the power sync word.
    let os_flags = if cfg.flags & FWINIT_CTXSWITCH_OFF != 0 {
        0
    } else {
        INICFG_OS_CTXSWITCH_TDM_EN
            | INICFG_OS_CTXSWITCH_GEOM_EN
            | INICFG_OS_CTXSWITCH_3D_EN
            | INICFG_OS_CTXSWITCH_CDM_EN
    };
    // SAFETY: the objects just allocated.
    unsafe {
        wr32(sysdata_p, SYSDATA_CONFIG_FLAGS, sys_flags);
        wr32(osdata_p, OSDATA_FW_OS_CONFIG_FLAGS, os_flags);
        wr32(osdata_p, OSDATA_POWER_SYNC, power_sync_fa);
    }

    // The runtime configuration. The core clock once went to the wrong
    // offset, leaving the core and SOC clocks 0: the firmware's deadline
    // scheduling (SOC timer) then never scheduled the data masters.
    crate::km_log!(k, Inf, "runtime cfg: ActivePMLatency {} ms", cfg.pm_latency_ms);
    // SAFETY: as above.
    unsafe {
        wr32(runtime_p, RUNTIME_ACTIVE_PM_LATENCY_MS, cfg.pm_latency_ms);
        // bActivePMLatencyPersistant is an IMG_BOOL (one byte); the word
        // store also zeroes the three padding bytes after it.
        wr32(runtime_p, RUNTIME_ACTIVE_PM_LATENCY_PERSISTANT, 1);
        wr32(runtime_p, RUNTIME_CORE_CLOCK_SPEED, cfg.core_clock_hz);
        wr32(runtime_p, RUNTIME_SOC_CLOCK_SPEED, cfg.core_clock_hz);
        // MAXPowUnitCount (BXS-4-64: 1)
        wr32(runtime_p, RUNTIME_POW_UNITS_STATE, 1);
        // RGXFWIF_PHR_MODE_RD_RESET
        wr32(runtime_p, RUNTIME_PHR_MODE, 1);
        wr32(runtime_p, RUNTIME_HCS_DEADLINE_MS, HCS_DEFAULT_DEADLINE_MS);
        wr32(runtime_p, RUNTIME_WDG_PERIOD_US, SAFETY_WATCHDOG_PERIOD_IN_US);
    }
    // The GPU utilisation structure stays zeroed, as the DDK's
    // RGXSetupFwGpuUtil leaves it apart from its flags.

    // The connection control (config slot 0): zero. The config region is not
    // an allocation, so it is cleared here.
    let conn = k.heap_cpu(config_slot(k, 0));
    // SAFETY: the config region is backed (pvr_internal.h).
    unsafe { core::ptr::write_bytes(conn, 0, CONNECTION_CTL_SIZE) };

    // The OS init (config slot 1). No work-estimation CCB (the firmware is
    // not built with SUPPORT_WORKLOAD_ESTIMATION); sRGXCompChecks is the
    // firmware's to fill.
    let osi = k.heap_cpu(config_slot(k, 1));
    // SAFETY: as above.
    unsafe {
        core::ptr::write_bytes(osi, 0, OSINIT_SIZE);
        wr32(osi, OSINIT_KCCB_CTL, kccb_ctl_fa);
        wr32(osi, OSINIT_KCCB, kccb_cmds_fa);
        wr32(osi, OSINIT_KCCB_RTN_SLOTS, kccb_rtn_fa);
        wr32(osi, OSINIT_FWCCB_CTL, fwccb_ctl_fa);
        wr32(osi, OSINIT_FWCCB, fwccb_cmds_fa);
        wr32(osi, OSINIT_FW_OS_DATA, osdata_fa);
        wr32(osi, OSINIT_HWRINFOBUF_CTL, hwrinfobuf_fa);
    }
    // A zeroed RGXFWIF_HWPERFDATA: a HWPerf filter of 0 disables HWPerf, so
    // the firmware does not emit the boot packet (which would fault on the
    // unset L1 buffer).
    let (_, _, hp_fa) = obj(k, HWPERFDATA_SIZE as u32, 8);
    // SAFETY: as above.
    unsafe {
        wr32(osi, OSINIT_HWPERF_DATA, hp_fa);
        // The pointers the DDK's RGXSetupFwOsInit sets and the firmware
        // dereferences after init (a null one faults).
        wr32(osi, OSINIT_GPU_UTIL_FW_CTL, gpu_util_fa);
    }
    // The runtime HWPerf configuration: zeroed, 512 bytes (more than its
    // 24, as the C driver had it).
    let (_, _, rc_fa) = obj(k, 512, 8);
    // SAFETY: as above.
    unsafe { wr32(osi, OSINIT_HWPERF_RUNTIME_CFG, rc_fa) };

    // The system init (config slot 2).
    let sysinit = config_slot(k, 2);
    k.sysinit_off = sysinit;
    let sys = k.heap_cpu(sysinit);
    // SAFETY: as above.
    unsafe { core::ptr::write_bytes(sys, 0, SYSINIT_SIZE) };
    // The alignment checks (the DDK's RGXFWSetupAlignChecks): [KM count]
    // [the KM table][UM count = 0][the firmware fills the UM part]. The
    // firmware compares the KM table with its own and asserts on a
    // difference, so it must be filled.
    let n = ALIGN_CHECKS_INIT_KM.len() as u32;
    let (_, ac_p, ac_fa) = obj(k, (ALIGN_CHECKS_UM_MAX + 64) * 4, 8);
    // SAFETY: the table just allocated, (128 + 64) words.
    unsafe {
        wr32(ac_p, 0, n);
        for (i, &v) in ALIGN_CHECKS_INIT_KM.iter().enumerate() {
            wr32(ac_p, 4 * (1 + i), v);
        }
        wr32(ac_p, 4 * (1 + n as usize), 0);
        wr64(sys, SYSINIT_FAULT_PHYS_ADDR, fault_phys);
        wr64(sys, SYSINIT_PDS_EXEC_BASE, PDSCODEDATA_HEAP_BASE);
        wr64(sys, SYSINIT_USC_EXEC_BASE, USCCODE_HEAP_BASE);
        wr32(sys, SYSINIT_RUNTIME_CFG, runtime_cfg_fa);
        wr32(sys, SYSINIT_TRACEBUF_CTL, tracebuf_fa);
        wr32(sys, SYSINIT_FW_SYS_DATA, sysdata_fa);
        wr32(sys, SYSINIT_ALIGN_CHECKS, ac_fa);
        wr32(sys, SYSINIT_INITIAL_CORE_CLOCK_SPEED, cfg.core_clock_hz);
        wr32(sys, SYSINIT_MARKER_VAL, 1);
    }
    // The HWPerf counters control the DDK's RGXSetupFwSysInit sets: zeroed,
    // so the firmware has something to read.
    let (_, _, hc_fa) = obj(k, 2048, 8);
    // SAFETY: as above. bFirmwareStarted stays 0: the firmware sets it.
    unsafe { wr32(sys, SYSINIT_HWPERF_CTL, hc_fa) };

    crate::km_log!(k, Inf, "FWIF structs built (KCCB/FWCCB/sysinit/osinit); heap used to 0x{:x}", k.heap_next);
}

/// A NUL-terminated string at `p`, at most `max` bytes, as text.
///
/// # Safety
/// `p` valid for `max` bytes.
unsafe fn cstr<'a>(p: *const u8, max: usize) -> &'a str {
    // SAFETY: the caller's contract.
    let b = unsafe { core::slice::from_raw_parts(p, max) };
    let n = b.iter().position(|&c| c == 0).unwrap_or(max);
    core::str::from_utf8(&b[..n]).unwrap_or("?")
}

/// Wait for the firmware's `bFirmwareStarted` (up to 5 s at 1 ms). On a
/// timeout, dump the start of its trace ring and its assert record (the
/// firmware asserts with an `ebreak`, mcause 3). Returns 0 or -ETIMEDOUT.
pub fn wait_started(k: &mut YdPvrKm) -> i32 {
    // bFirmwareStarted is an IMG_BOOL (one byte); the word read also covers
    // the zeroed padding after it.
    let started = k.heap_cpu(k.sysinit_off).wrapping_add(SYSINIT_FIRMWARE_STARTED);
    for i in 0..5000u32 {
        // The firmware writes it in memory: invalidate our view.
        k.invd(started, 8);
        // SAFETY: the system init structure.
        let v = unsafe { rd32(started, 0) };
        if v != 0 {
            crate::km_log!(k, Inf, "FW boot: firmware_started=0x{:x} after ~{} ms", v, i);
            return 0;
        }
        k.busy_wait_us(1000);
    }
    crate::km_log!(k, Err, "FW boot TIMEOUT (firmware_started still 0)");
    let tb = k.heap_cpu(k.trace_buf_off);
    let tbuf = k.heap_cpu(k.tracebuf_off);
    k.invd(tbuf, 32);
    k.invd(tb, 256 * 4);
    // SAFETY: the trace buffer structure and its ring (12000 words).
    unsafe {
        let tptr = rd32(tbuf, TRACEBUF_TRACE_POINTER);
        crate::km_log!(k, Err, "DDK FW trace: TracePointer={}; first dwords:", tptr);
        for i in (0..48usize).step_by(6) {
            crate::km_log!(
                k,
                Err,
                " t[{}]: {:08x} {:08x} {:08x} {:08x} {:08x} {:08x}",
                i,
                rd32(tb, 4 * i),
                rd32(tb, 4 * (i + 1)),
                rd32(tb, 4 * (i + 2)),
                rd32(tb, 4 * (i + 3)),
                rd32(tb, 4 * (i + 4)),
                rd32(tb, 4 * (i + 5))
            );
        }
        // RGXFWIF_FILE_INFO_BUF: the path, the info text, the line.
        let ab = tbuf.wrapping_add(TRACEBUF_ASSERT_BUF);
        let line = rd32(ab, FILE_INFO_BUF_LINE_NUM);
        ab.add(FILE_INFO_BUF_INFO - 1).write_volatile(0);
        ab.add(FILE_INFO_BUF_LINE_NUM - 1).write_volatile(0);
        crate::km_log!(k, Err, "DDK FW ASSERT: {}:{}", cstr(ab.add(FILE_INFO_BUF_PATH), FILE_INFO_BUF_INFO), line);
        crate::km_log!(
            k,
            Err,
            "DDK FW ASSERT info: {}",
            cstr(ab.add(FILE_INFO_BUF_INFO), FILE_INFO_BUF_LINE_NUM - FILE_INFO_BUF_INFO)
        );
    }
    -errno::ETIMEDOUT
}

// ---- C entry points ------------------------------------------------------------

/// # Safety
/// `k` is the C driver's `struct yd_pvr_km` (see [`km`]); `cfg` valid.
#[no_mangle]
pub unsafe extern "C" fn yd_pvr_fwinit_build(k: *mut YdPvrKm, cfg: *const YdPvrFwinit) {
    // SAFETY: the caller's contract.
    unsafe { build(km(k), &*cfg) }
}

/// # Safety
/// `k` as above, the structures built.
#[no_mangle]
pub unsafe extern "C" fn yd_pvr_fw_wait_started(k: *mut YdPvrKm) -> i32 {
    // SAFETY: the caller's contract.
    wait_started(unsafe { km(k) })
}
