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
    /// With [`FWINIT_HWPERF`]: the HWPerf buffer's bytes (0:
    /// [`HWPERF_BUF_DEFAULT`]). Ignored without the flag.
    pub hwperf_buf_size: u32,
}

/// Halt (do not reset) on a hardware recovery, so the fault registers persist
/// (`RGXFWIF_INICFG_ASSERT_ON_HWR_TRIGGER`; a bring-up aid).
pub const FWINIT_ASSERT_ON_HWR: u32 = 1 << 0;
/// No context switching (`ui32FwOsConfigFlags` 0). Measured: the geometry
/// data master then processes nothing; a bring-up aid.
pub const FWINIT_CTXSWITCH_OFF: u32 = 1 << 1;

/// The partial-render knob is on (`render cfg partial_render != 0`): the
/// trace also logs the firmware's debug group. Set by the C caller; clear,
/// the init words are what they were without the knob.
pub const FWINIT_PARTIAL_RENDER: u32 = 1 << 2;

/// The firmware's HWPerf event stream (`CONFIG_YAKOGL_DRV_GPU_PVR_HWPERF`, an
/// instrument; docs/notes/pvr-hwperf.md (a), (d)): a buffer of
/// `hwperf_buf_size` bytes in the firmware heap, its address in the runtime
/// HWPerf configuration, its size and [`super::hwperf::PROBE_FILTER`] in the
/// HWPerf data, `RGXFWIF_INICFG_HWPERF_EN` in the system data's flags.
/// Clear, the init words are what they were without it. Read back with
/// [`super::hwperf::drain`].
pub const FWINIT_HWPERF: u32 = 1 << 3;

/// The HWPerf buffer's default size: 1 MiB. Case 216's 20 frames need about
/// 0.6 MB (docs/notes/pvr-hwperf.md (a)); the DDK's 2 MiB would leave about
/// 1 MiB of the 4 MiB backed main region (`PVR_FW_HEAP_MAIN_PHYS`) after the
/// firmware image and today's 0xd8d28 bytes of init objects.
pub const HWPERF_BUF_DEFAULT: u32 = 1 << 20;

/// `RGXFWIF_TRACEBUF.ui32LogType` for `flags`. Today's mask is every group
/// but DEBUG, MTS and POW. With the partial-render knob the DEBUG group
/// (0x80000000) is added; the SPM (0x100) and MAIN (0x2) groups that carry
/// the partial-render lines (rgx_fwif_sf.h: SPM "Force Z-Load/Z-Store for
/// partial render" at 1500/1502, "ZStore/SStore/ZLoad/SLoad address" at
/// 1534-1540; MAIN "No ZS Buffer used for partial render" at 210) are
/// already in today's mask. MTS and POW stay masked (idle chatter).
fn trace_log_type(flags: u32) -> u32 {
    let mut m = LOG_TYPE_MASK & !LOG_TYPE_GROUP_DEBUG & !(LOG_TYPE_GROUP_MTS | LOG_TYPE_GROUP_POW);
    if flags & FWINIT_PARTIAL_RENDER != 0 {
        m |= LOG_TYPE_GROUP_DEBUG;
    }
    m
}

const _: () = {
    assert!(core::mem::size_of::<YdPvrFwinit>() == 28);
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
        wr32(tbuf_p, TRACEBUF_LOG_TYPE, trace_log_type(cfg.flags));
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
    // The HWPerf buffer is the last object (below), so the knob only appends
    // to the heap; decided here, since the enable bit goes in now.
    let hwperf_size = if cfg.flags & FWINIT_HWPERF != 0 { hwperf_buf_size(k, cfg) } else { 0 };
    if hwperf_size != 0 {
        sys_flags |= INICFG_HWPERF_EN;
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
    // unset L1 buffer). With FWINIT_HWPERF the buffer, its size and the
    // filter are set below, all before the firmware starts.
    let (_, hp_p, hp_fa) = obj(k, HWPERFDATA_SIZE as u32, 8);
    // SAFETY: as above.
    unsafe {
        wr32(osi, OSINIT_HWPERF_DATA, hp_fa);
        // The pointers the DDK's RGXSetupFwOsInit sets and the firmware
        // dereferences after init (a null one faults).
        wr32(osi, OSINIT_GPU_UTIL_FW_CTL, gpu_util_fa);
    }
    // The runtime HWPerf configuration: zeroed, 512 bytes (more than its
    // 24, as the C driver had it).
    let (_, rc_p, rc_fa) = obj(k, 512, 8);
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

    if hwperf_size != 0 {
        // The HWPerf buffer, page aligned (docs/notes/pvr-hwperf.md (d)).
        // The firmware faults on a filter without a buffer: the address and
        // size go in with it, and RIdx, WIdx and the wrap mark stay 0.
        let (_, _, buf_fa) = obj(k, hwperf_size, 0x1000);
        // SAFETY: the HWPerf data and runtime configuration allocated above.
        unsafe {
            wr32(rc_p, RUNTIME_HWPERFCFG_BUF, buf_fa);
            wr32(hp_p, HWPERFDATA_SIZE_FIELD, hwperf_size);
            wr64(hp_p, HWPERFDATA_FILTER, super::hwperf::PROBE_FILTER);
        }
        k.flush(rc_p, RUNTIME_HWPERFCFG_BUF_SIZE);
        k.flush(hp_p, HWPERFDATA_SIZE);
        crate::km_log!(
            k,
            Inf,
            "HWPerf on: buffer {} KiB at fw 0x{:x}, filter 0x{:016x}",
            hwperf_size / 1024,
            buf_fa,
            super::hwperf::PROBE_FILTER
        );
    }

    crate::km_log!(k, Inf, "FWIF structs built (KCCB/FWCCB/sysinit/osinit); heap used to 0x{:x}", k.heap_next);
}

/// The HWPerf buffer's size for `cfg` (8-byte rounded), or 0 when it would
/// not fit in the backed main region with a page of alignment: HWPerf then
/// stays off, said in the log, rather than the heap's overflow panic.
fn hwperf_buf_size(k: &YdPvrKm, cfg: &YdPvrFwinit) -> u32 {
    let want = if cfg.hwperf_buf_size != 0 { cfg.hwperf_buf_size } else { HWPERF_BUF_DEFAULT };
    let want = want.div_ceil(8) * 8;
    // Room for every object still to come before the buffer: well under the
    // 64 KiB margin (the config slots are not in the main region).
    let need = u64::from(k.heap_next) + 0x1_0000 + 0x1000 + u64::from(want);
    if need > u64::from(k.fw_main_size) {
        crate::km_log!(
            k,
            Err,
            "HWPerf: a {} KiB buffer does not fit (heap at 0x{:x} of 0x{:x}); HWPerf stays off",
            want / 1024,
            k.heap_next,
            k.fw_main_size
        );
        return 0;
    }
    want
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn knob_off_keeps_todays_mask() {
        // 0x8007FFFF without DEBUG, MTS (0x4) and POW (0x200).
        assert_eq!(trace_log_type(0), 0x0007_FDFB);
        assert_eq!(trace_log_type(FWINIT_ASSERT_ON_HWR), 0x0007_FDFB);
    }

    #[test]
    fn knob_on_enables_debug_and_keeps_spm_main() {
        let m = trace_log_type(FWINIT_PARTIAL_RENDER);
        assert_eq!(m, 0x8007_FDFB);
        assert_ne!(m & 0x0000_0100, 0, "SPM");
        assert_ne!(m & 0x0000_0002, 0, "MAIN");
        assert_ne!(m & LOG_TYPE_GROUP_DEBUG, 0);
        assert_eq!(m & (LOG_TYPE_GROUP_MTS | LOG_TYPE_GROUP_POW), 0);
    }

    // ---- the init words on a host heap ---------------------------------------

    use super::super::km::YdPvrKmOps;
    use core::cell::RefCell;
    use core::ffi::c_void;
    use std::boxed::Box;
    use std::vec::Vec;

    unsafe extern "C" fn nop_range(_: *mut c_void, _: *const c_void, _: usize) {}
    unsafe extern "C" fn nop(_: *mut c_void) {}
    unsafe extern "C" fn nop_wait(_: *mut c_void, _: u32) {}
    unsafe extern "C" fn log(ctx: *mut c_void, _: u32, b: *const u8, n: usize) {
        // SAFETY: ctx is the fixture's line list; b is valid for n bytes.
        let lines = unsafe { &*(ctx as *const RefCell<Vec<std::string::String>>) };
        lines
            .borrow_mut()
            .push(std::string::String::from_utf8_lossy(unsafe { core::slice::from_raw_parts(b, n) }).into_owned());
    }

    /// The board's layout (driver/src/pvr_internal.h): a 4 MiB backed main
    /// region, the config region after it; the host's objects start where the
    /// AM62P firmware image ends.
    const MAIN: u32 = 4 << 20;
    const OBJS_OFF: u32 = 0x9_0000;

    /// A zeroed carveout and a `YdPvrKm` over it, as the C driver fills it.
    pub(in crate::gpu::pvr) struct HostKm {
        pub mem: Vec<u64>,
        pub k: Box<YdPvrKm>,
        pub log: Box<RefCell<Vec<std::string::String>>>,
        _ops: Box<YdPvrKmOps>,
    }

    pub(in crate::gpu::pvr) fn host_km() -> HostKm {
        let total = MAIN as usize + FW_CONFIG_HEAP_SIZE as usize;
        let mut mem = std::vec![0u64; total / 8];
        let lines: Box<RefCell<Vec<std::string::String>>> = Box::default();
        let ops = Box::new(YdPvrKmOps {
            ctx: &*lines as *const RefCell<Vec<std::string::String>> as *mut c_void,
            flush: nop_range,
            invd: nop_range,
            fence: nop,
            busy_wait_us: nop_wait,
            log,
        });
        // SAFETY: all-zero is the documented state before the first call.
        let mut k: Box<YdPvrKm> = Box::new(unsafe { core::mem::zeroed() });
        k.cpu = mem.as_mut_ptr() as *mut u8;
        k.phys = 0xA000_0000;
        k.size = total as u64;
        k.ops = &*ops;
        k.fw_main_size = MAIN;
        k.fw_config_off = FW_HEAP_SIZE - FW_CONFIG_HEAP_SIZE;
        k.heap_next = OBJS_OFF;
        k.fw_uncached = 1;
        HostKm { mem, k, log: lines, _ops: ops }
    }

    impl HostKm {
        /// The log lines so far.
        fn lines(&self) -> Vec<std::string::String> {
            self.log.borrow().clone()
        }
    }

    fn cfg(flags: u32, hwperf_buf_size: u32) -> YdPvrFwinit {
        YdPvrFwinit {
            pm_latency_ms: 5,
            core_clock_hz: 800_000_000,
            flags,
            ccb_ctl_size: 0,
            tracebuf_size: 0,
            gpu_util_size: 0,
            hwperf_buf_size,
        }
    }

    /// FNV-1a over the whole carveout and the offsets the init leaves in the
    /// context.
    fn fnv(h: &HostKm) -> u64 {
        let mut x: u64 = 0xCBF2_9CE4_8422_2325;
        let k = &h.k;
        let tail = [k.heap_next, k.kccb_ctl_off, k.kccb_cmds_off, k.kccb_rtn_off, k.fwccb_ctl_off, k.fwccb_cmds_off];
        let tail2 = [k.sysinit_off, k.tracebuf_off, k.trace_buf_off, k.hwrinfobuf_off];
        for w in h.mem.iter().copied().chain(tail.iter().chain(tail2.iter()).map(|&v| u64::from(v))) {
            for b in w.to_le_bytes() {
                x = (x ^ u64::from(b)).wrapping_mul(0x100_0000_01B3);
            }
        }
        x
    }

    fn built(flags: u32, hwperf_buf_size: u32) -> HostKm {
        let mut h = host_km();
        build(&mut h.k, &cfg(flags, hwperf_buf_size));
        h
    }

    /// Byte `off` of the carveout as a word.
    fn word(h: &HostKm, carveout_off: usize) -> u32 {
        // SAFETY: inside the carveout.
        unsafe { rd32(h.mem.as_ptr() as *const u8, carveout_off) }
    }

    /// Carveout offset of a firmware address.
    fn at(h: &HostKm, fwaddr: u32) -> usize {
        h.k.heap_carveout_off(fwaddr & (FW_HEAP_SIZE - 1)) as usize
    }

    /// The init without FWINIT_HWPERF is byte for byte what it was before
    /// the flag existed. The hashes were taken from the build function of
    /// cafd14a5 (before this change) on the same fixture, whatever the
    /// buffer size field says.
    #[test]
    fn hwperf_off_keeps_todays_init_bytes() {
        const TODAY: u64 = 0xfa01_8ab6_8785_b0a2;
        const TODAY_PR: u64 = 0x4997_d041_1c10_f522;
        assert_eq!(fnv(&built(0, 0)), TODAY);
        assert_eq!(fnv(&built(0, 64 << 10)), TODAY);
        assert_eq!(fnv(&built(FWINIT_PARTIAL_RENDER, 0)), TODAY_PR);
        let h = built(0, 0);
        assert!(!h.lines().iter().any(|l| l.contains("HWPerf")), "{:?}", h.lines());
    }

    /// With FWINIT_HWPERF exactly these words change: the system data's
    /// HWPERF_EN, the HWPerf data's size and filter, the runtime HWPerf
    /// configuration's buffer address; the buffer is the last object, page
    /// aligned, zeroed, and the heap ends after it.
    #[test]
    fn hwperf_on_sets_flag_size_filter_and_buffer() {
        let off = built(0, 0);
        let on = built(FWINIT_HWPERF, 0);
        let osi = off.k.heap_carveout_off(off.k.fw_config_off + FW_CONFIG_HEAP_ALLOC_GRANULARITY) as usize;
        let sys = off.k.heap_carveout_off(off.k.sysinit_off) as usize;
        let sysdata = at(&on, word(&on, sys + SYSINIT_FW_SYS_DATA));
        let hp = at(&on, word(&on, osi + OSINIT_HWPERF_DATA));
        let rc = at(&on, word(&on, osi + OSINIT_HWPERF_RUNTIME_CFG));
        let buf_fa = word(&on, rc + RUNTIME_HWPERFCFG_BUF);
        let buf = at(&on, buf_fa);

        assert_eq!(word(&on, sysdata + SYSDATA_CONFIG_FLAGS), INICFG_CTXSWITCH_PROFILE_MEDIUM | INICFG_HWPERF_EN);
        assert_eq!(word(&off, sysdata + SYSDATA_CONFIG_FLAGS), INICFG_CTXSWITCH_PROFILE_MEDIUM);
        assert_eq!(word(&on, hp + HWPERFDATA_SIZE_FIELD), HWPERF_BUF_DEFAULT);
        assert_eq!(word(&on, hp + HWPERFDATA_FILTER), super::super::hwperf::PROBE_FILTER as u32);
        assert_eq!(word(&on, hp + HWPERFDATA_FILTER + 4), (super::super::hwperf::PROBE_FILTER >> 32) as u32);
        assert_eq!(super::super::hwperf::PROBE_FILTER, 0x0303_0708_0FFB_FFFE);
        for o in [HWPERFDATA_RIDX, HWPERFDATA_WIDX, HWPERFDATA_WRAP_COUNT, HWPERFDATA_DROP_COUNT] {
            assert_eq!(word(&on, hp + o), 0);
        }
        // Firmware address in the uncached window; the buffer page aligned,
        // after every other object, the heap's end right after it.
        assert_eq!(buf_fa & 0xC000_0000, RISCV_SHARED_UNCACHED_DATA0_BASE);
        assert_eq!(buf_fa & 0xFFF, 0);
        assert!(buf_fa & (FW_HEAP_SIZE - 1) >= off.k.heap_next);
        assert_eq!(on.k.heap_next, (buf_fa & (FW_HEAP_SIZE - 1)) + HWPERF_BUF_DEFAULT);
        assert!((0..HWPERF_BUF_DEFAULT as usize).step_by(4).all(|o| word(&on, buf + o) == 0));

        // Every other word of the carveout is as without the flag.
        let changed: Vec<usize> =
            (0..off.mem.len() * 2).map(|i| i * 4).filter(|&o| word(&off, o) != word(&on, o)).collect();
        let mut want =
            std::vec![sysdata, hp + HWPERFDATA_SIZE_FIELD, hp + HWPERFDATA_FILTER, hp + HWPERFDATA_FILTER + 4, rc];
        want.sort();
        assert_eq!(changed, want);
        assert!(on.lines().iter().any(|l| l.starts_with("HWPerf on: buffer 1024 KiB")), "{:?}", on.lines());
        // The other offsets the context keeps are unchanged.
        assert_eq!(
            (on.k.kccb_ctl_off, on.k.sysinit_off, on.k.tracebuf_off, on.k.hwrinfobuf_off),
            (off.k.kccb_ctl_off, off.k.sysinit_off, off.k.tracebuf_off, off.k.hwrinfobuf_off)
        );
    }

    #[test]
    fn hwperf_buffer_size_from_cfg_and_too_big_stays_off() {
        let h = built(FWINIT_HWPERF, 64 << 10);
        let osi = h.k.heap_carveout_off(h.k.fw_config_off + FW_CONFIG_HEAP_ALLOC_GRANULARITY) as usize;
        let hp = at(&h, word(&h, osi + OSINIT_HWPERF_DATA));
        assert_eq!(word(&h, hp + HWPERFDATA_SIZE_FIELD), 64 << 10);

        // A buffer the 4 MiB main region cannot hold: logged, and the init
        // is the knob-off one.
        let big = built(FWINIT_HWPERF, 4 << 20);
        assert_eq!(fnv(&big), fnv(&built(0, 0)));
        assert!(big.lines().iter().any(|l| l.contains("HWPerf stays off")), "{:?}", big.lines());
    }

    // ---- the drain over a buffer the test fills as the firmware would ---------

    /// One packet: header and payload words (little-endian, 8-byte padded).
    fn pkt(out: &mut Vec<u8>, ty: u32, ord: u32, ts: u64, payload: &[u32]) {
        let mut p: Vec<u8> = payload.iter().flat_map(|w| w.to_le_bytes()).collect();
        while !p.len().is_multiple_of(8) {
            p.push(0);
        }
        let size = (super::super::hwperf::HDR_SIZE + p.len()) as u32;
        for w in [super::super::hwperf::SIG_V2B, size, ty, ord] {
            out.extend_from_slice(&w.to_le_bytes());
        }
        out.extend_from_slice(&ts.to_le_bytes());
        out.extend_from_slice(&p);
    }

    /// A kick: the firmware's take-up at `t0`, TA 30..80, 3D 90..290.
    fn kick(out: &mut Vec<u8>, ord: &mut u32, t0: u64) {
        use super::super::hwperf::ev;
        let hw = [0u32; super::super::hwperf::HW_DATA_SIZE / 4];
        let fw = [0u32; super::super::hwperf::FW_DATA_SIZE / 4];
        for (ty, dt, p) in [
            (ev::FW_BGSTART, 0, &fw[..]),
            (ev::HW_TAKICK, 30, &hw[..]),
            (ev::HW_TAFINISHED, 80, &hw[..]),
            (ev::HW_3DKICK, 90, &hw[..]),
            (ev::HW_3DFINISHED, 290, &hw[..]),
        ] {
            pkt(out, ty, *ord, t0 + dt, p);
            *ord += 1;
        }
    }

    /// Copies `bytes` to the buffer at `at_off` and sets WIdx (and the wrap
    /// mark) as the firmware would.
    fn fw_writes(h: &mut HostKm, at_off: usize, bytes: &[u8], widx: u32, wrap: u32) {
        let osi = h.k.heap_carveout_off(h.k.fw_config_off + FW_CONFIG_HEAP_ALLOC_GRANULARITY) as usize;
        let hp = at(h, word(h, osi + OSINIT_HWPERF_DATA));
        let rc = at(h, word(h, osi + OSINIT_HWPERF_RUNTIME_CFG));
        let buf = at(h, word(h, rc + RUNTIME_HWPERFCFG_BUF));
        let m = h.mem.as_mut_ptr() as *mut u8;
        // SAFETY: inside the carveout (the buffer and the HWPerf data).
        unsafe {
            core::ptr::copy_nonoverlapping(bytes.as_ptr(), m.add(buf + at_off), bytes.len());
            wr32(m.add(hp), HWPERFDATA_WIDX, widx);
            wr32(m.add(hp), HWPERFDATA_WRAP_COUNT, wrap);
        }
    }

    #[test]
    fn drain_off_reads_nothing() {
        let h = built(0, 0);
        assert!(super::super::hwperf::drain(&h.k, 0, &mut []).is_none());
        assert_eq!(super::super::hwperf::drain_log(&h.k, 0, 800_000_000), -errno::ENODEV);
        assert!(h.lines().last().unwrap().starts_with("pvr-hwperf: off"));
    }

    #[test]
    fn drain_summarises_each_stretch_and_logs_per_lane() {
        use super::super::hwperf::{drain, drain_log, Lane};
        let mut h = built(FWINIT_HWPERF, 64 << 10);
        let (mut a, mut ord) = (Vec::new(), 1u32);
        kick(&mut a, &mut ord, 1000);
        kick(&mut a, &mut ord, 5000);
        let n1 = a.len() as u32;
        fw_writes(&mut h, 0, &a, n1, 0);

        let d = drain(&h.k, 0, &mut []).unwrap();
        assert_eq!((d.from, d.widx, d.size, d.fw_drops), (0, n1, 64 << 10, 0));
        let s = d.summary;
        assert_eq!((s.packets, s.dropped, s.tasks, s.tasks_lost, s.error), (10, 0, 4, 4, None));
        assert_eq!(s.gpu_ticks_of(Lane::Ta), 100);
        assert_eq!(s.gpu_ticks_of(Lane::ThreeD), 400);
        assert_eq!(s.lane_kick_to_start_ticks, [60, 180, 0, 0, 0, 0, 0]);
        assert_eq!(s.lane_kick_to_start_n, [2, 2, 0, 0, 0, 0, 0]);

        // The record lines: the next `from` is the write index.
        h.log.borrow_mut().clear();
        assert_eq!(drain_log(&h.k, 0, 800_000_000), n1 as i32);
        let lines = h.lines();
        assert_eq!(lines.len(), 3, "{lines:?}");
        assert!(lines[0].starts_with(&std::format!(
            "pvr-hwperf: from=0 widx={n1} wrap=0 size=65536 fw_drops=0 packets=10 dropped=0 other=0 tasks=4 "
        )));
        assert!(lines[0].ends_with("err=None"), "{}", lines[0]);
        // 100 ticks at 800 MHz: 100 * 256 / 0.8 = 32000 ns.
        assert_eq!(lines[1], "pvr-hwperf lane=ta tasks=2 gpu_ticks=100 gpu_ns=32000 k2s_n=2 k2s_ticks=60 k2s_ns=19200");
        assert_eq!(
            lines[2],
            "pvr-hwperf lane=3d tasks=2 gpu_ticks=400 gpu_ns=128000 k2s_n=2 k2s_ticks=180 k2s_ns=57600"
        );

        // A second stretch: only the new packets.
        let mut b = Vec::new();
        kick(&mut b, &mut ord, 9000);
        fw_writes(&mut h, n1 as usize, &b, n1 + b.len() as u32, 0);
        let d = drain(&h.k, n1, &mut []).unwrap();
        assert_eq!((d.summary.packets, d.summary.tasks), (5, 2));

        // The read index is handed back: the write index reached.
        let osi = h.k.heap_carveout_off(h.k.fw_config_off + FW_CONFIG_HEAP_ALLOC_GRANULARITY) as usize;
        let hp = at(&h, word(&h, osi + OSINIT_HWPERF_DATA));
        assert_eq!(word(&h, hp + HWPERFDATA_RIDX), n1 + b.len() as u32);
    }

    #[test]
    fn drain_past_a_wrap_reads_both_ranges() {
        use super::super::hwperf::drain;
        let mut h = built(FWINIT_HWPERF, 64 << 10);
        let (mut a, mut ord) = (Vec::new(), 1u32);
        kick(&mut a, &mut ord, 1000);
        let mut b = Vec::new();
        kick(&mut b, &mut ord, 5000);
        // The first kick ends the buffer's data at `wrap`; the second is at 0.
        let start = (64 << 10) - a.len() - 8;
        let wrap = (start + a.len()) as u32;
        fw_writes(&mut h, start, &a, 0, wrap);
        fw_writes(&mut h, 0, &b, b.len() as u32, wrap);
        let d = drain(&h.k, start as u32, &mut []).unwrap();
        assert_eq!((d.summary.packets, d.summary.tasks, d.summary.error), (10, 4, None));
    }
}
