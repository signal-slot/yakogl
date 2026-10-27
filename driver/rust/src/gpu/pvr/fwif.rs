// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The numbers of the TI/Img Rogue DDK 25.2's firmware interface and
//! hardware definitions the kernel side uses: page-table formats
//! (`hwdefs/rogue/km/rgxmmudefs_km.h`), heap bases (`rgxheapconfig.h`,
//! `rgx_heap_firmware.h`, `rgx_riscv.h`), structure layouts and command
//! numbers (`rgx_fwif_km.h`, `rgx_fwif_shared.h`), registers
//! (`rgx_cr_defs_km.h`), all vendored unmodified in
//! `third_party/ti-img-rogue-ddk-25.2` and compiled with the AM62P
//! configuration (`config/am62p_linux_release/config_kernel.h`).
//!
//! Every constant here is pinned: [`PINS`] pairs it with the C expression
//! that names it in those headers, `km_check pins` prints the pairs as
//! `PIN(value, expression)` lines (the format of
//! `driver/tools/offsetof/ddk_abi_pins.h`) and `tools/check.sh` compiles them
//! into `_Static_assert`s against the headers. A constant the headers move
//! stops the gate.

macro_rules! pins {
    ($( $(#[$m:meta])* $name:ident : $ty:ty = $val:expr => $c:literal; )*) => {
        $( $(#[$m])* pub const $name: $ty = $val; )*
        /// Every constant above with the C expression it is checked against.
        pub const PINS: &[(&str, u64)] = &[ $( ($c, $name as u64), )* ];
    };
}

pins! {
    // ---- rgxmmudefs_km.h: the three-level page tables (4 KiB pages) ----
    /// Virtual address bits that index the page catalogue (39..30).
    MMU_VADDR_PC_INDEX_SHIFT: u32 = 30 => "RGX_MMUCTRL_VADDR_PC_INDEX_SHIFT";
    /// ... the page directory (29..21).
    MMU_VADDR_PD_INDEX_SHIFT: u32 = 21 => "RGX_MMUCTRL_VADDR_PD_INDEX_SHIFT";
    /// ... the page table (20..12).
    MMU_VADDR_PT_INDEX_SHIFT: u32 = 12 => "RGX_MMUCTRL_VADDR_PT_INDEX_SHIFT";
    MMU_ENTRIES_PC: u32 = 1024 => "RGX_MMUCTRL_ENTRIES_PC_VALUE";
    MMU_ENTRIES_PD: u32 = 512 => "RGX_MMUCTRL_ENTRIES_PD_VALUE";
    MMU_ENTRIES_PT: u32 = 512 => "RGX_MMUCTRL_ENTRIES_PT_VALUE";
    /// A catalogue entry is 32 bits: the page directory's 4 KiB page number
    /// from bit 4, valid in bit 0.
    MMU_PC_DATA_PD_BASE_SHIFT: u32 = 4 => "RGX_MMUCTRL_PC_DATA_PD_BASE_SHIFT";
    MMU_PC_DATA_PD_BASE_ALIGNSHIFT: u32 = 12 => "RGX_MMUCTRL_PC_DATA_PD_BASE_ALIGNSHIFT";
    MMU_PC_DATA_VALID_EN: u32 = 0x1 => "RGX_MMUCTRL_PC_DATA_VALID_EN";
    /// A directory entry is 64 bits: the page table's address in bits 39..5
    /// (in place), the page size in 3..1 (0: 4 KiB), valid in bit 0.
    MMU_PD_DATA_PT_BASE_CLRMSK: u64 = 0xFFFF_FF00_0000_001F => "RGX_MMUCTRL_PD_DATA_PT_BASE_CLRMSK";
    MMU_PD_DATA_PAGE_SIZE_4KB: u64 = 0 => "RGX_MMUCTRL_PD_DATA_PAGE_SIZE_4KB";
    MMU_PD_DATA_VALID_EN: u64 = 0x1 => "RGX_MMUCTRL_PD_DATA_VALID_EN";
    /// A table entry is 64 bits: the page's address in bits 39..12 (in
    /// place), the protection bits, valid in bit 0.
    MMU_PT_DATA_PAGE_CLRMSK: u64 = 0xFFFF_FF00_0000_0FFF => "RGX_MMUCTRL_PT_DATA_PAGE_CLRMSK";
    MMU_PT_DATA_VALID_EN: u64 = 0x1 => "RGX_MMUCTRL_PT_DATA_VALID_EN";
    MMU_PT_DATA_SLC_BYPASS_CTRL_EN: u64 = 0x8 => "RGX_MMUCTRL_PT_DATA_SLC_BYPASS_CTRL_EN";
    MMU_PT_DATA_PM_META_PROTECT_EN: u64 = 0x4000_0000_0000_0000 => "RGX_MMUCTRL_PT_DATA_PM_META_PROTECT_EN";

    // ---- heaps (rgxheapconfig.h, rgx_heap_firmware.h, rgx_riscv.h) ----
    /// The firmware heap: a 32 MiB window, its last 192 KiB the config
    /// region (connection control, OS init, system init at fixed offsets).
    FW_HEAP_BASE: u64 = 0xE1_C000_0000 => "RGX_FIRMWARE_RAW_HEAP_BASE";
    FW_HEAP_SIZE: u32 = 0x0200_0000 => "RGX_FIRMWARE_RAW_HEAP_SIZE";
    FW_CONFIG_HEAP_SIZE: u32 = 0x3_0000 => "RGX_FIRMWARE_CONFIG_HEAP_SIZE";
    FW_CONFIG_HEAP_ALLOC_GRANULARITY: u32 = 0x1_0000 => "RGX_FIRMWARE_CONFIG_HEAP_ALLOC_GRANULARITY";
    /// The RISC-V firmware's cached shared-data region: a firmware address
    /// is this ORed with the heap offset (the firmware remaps it to the heap
    /// base itself).
    RISCV_SHARED_CACHED_DATA0_BASE: u32 = 0xC000_0000 => "RGXRISCVFW_SHARED_CACHED_DATA0_BASE";
    /// The firmware's uncached window onto the same heap (rgx_riscv.h:103,
    /// :107-108), the one `YdPvrKm::heap_fwaddr` hands out with
    /// `CONFIG_YAKOGL_DRV_GPU_PVR_FW_UNCACHED`.
    RISCV_SHARED_UNCACHED_DATA0_BASE: u32 = 0x4000_0000 => "RGXRISCVFW_SHARED_UNCACHED_DATA0_BASE";
    GENERAL_HEAP_BASE: u64 = 0x80_0000_0000 => "RGX_GENERAL_HEAP_BASE";
    PDSCODEDATA_HEAP_BASE: u64 = 0xDA_0000_0000 => "RGX_PDSCODEDATA_HEAP_BASE";
    USCCODE_HEAP_BASE: u64 = 0xE0_0000_0000 => "RGX_USCCODE_HEAP_BASE";
    PMMETA_PROTECT_HEAP_BASE: u64 = 0xFA_0000_0000 => "RGX_PMMETA_PROTECT_HEAP_BASE";

    // ---- the kernel CCB and the firmware CCB (rgx_fwif_km.h) ----
    /// `RGXFWIF_CCB_CTL`: the write offset (the producer's), the read offset
    /// (the consumer's), the wrap mask (slots - 1). Offsets count commands.
    CCB_CTL_SIZE: usize = 16 => "sizeof(RGXFWIF_CCB_CTL)";
    CCB_CTL_WRITE_OFFSET: usize = 0 => "offsetof(RGXFWIF_CCB_CTL, ui32WriteOffset)";
    CCB_CTL_READ_OFFSET: usize = 4 => "offsetof(RGXFWIF_CCB_CTL, ui32ReadOffset)";
    CCB_CTL_WRAP_MASK: usize = 8 => "offsetof(RGXFWIF_CCB_CTL, ui32WrapMask)";
    /// `RGXFWIF_KCCB_CMD`: the firmware indexes the ring by its size (the
    /// control block has no command size).
    KCCB_CMD_SIZE: usize = 160 => "sizeof(RGXFWIF_KCCB_CMD)";
    KCCB_CMD_TYPE: usize = 0 => "offsetof(RGXFWIF_KCCB_CMD, eCmdType)";
    KCCB_CMD_FLAGS: usize = 4 => "offsetof(RGXFWIF_KCCB_CMD, ui32KCCBFlags)";
    KCCB_CMD_DATA: usize = 8 => "offsetof(RGXFWIF_KCCB_CMD, uCmdData)";
    /// `RGXFWIF_FREELIST_GS_DATA` in a KCCB command.
    KCCB_FL_GS_FWADDR: usize = 8 => "offsetof(RGXFWIF_KCCB_CMD, uCmdData.sFreeListGSData.sFreeListFWDevVAddr)";
    KCCB_FL_GS_DELTA: usize = 12 => "offsetof(RGXFWIF_KCCB_CMD, uCmdData.sFreeListGSData.ui32DeltaPages)";
    KCCB_FL_GS_NEW: usize = 16 => "offsetof(RGXFWIF_KCCB_CMD, uCmdData.sFreeListGSData.ui32NewPages)";
    KCCB_FL_GS_READY: usize = 20 => "offsetof(RGXFWIF_KCCB_CMD, uCmdData.sFreeListGSData.ui32ReadyPages)";
    /// `RGXFWIF_FREELISTS_RECONSTRUCTION_DATA` in a KCCB command (IDs are
    /// 64-bit).
    KCCB_FL_RECON_COUNT: usize = 8 =>
        "offsetof(RGXFWIF_KCCB_CMD, uCmdData.sFreeListsReconstructionData.ui32FreelistsCount)";
    KCCB_FL_RECON_IDS: usize = 16 =>
        "offsetof(RGXFWIF_KCCB_CMD, uCmdData.sFreeListsReconstructionData.aui64FreelistIDs)";
    /// The MMUCACHE command's sync value (the slot probe prints it).
    KCCB_MMUCACHE_SYNC_VALUE: usize = 16 =>
        "offsetof(RGXFWIF_KCCB_CMD, uCmdData.sMMUCacheData.ui32MMUCacheSyncUpdateValue)";
    KCCB_CMD_FREELIST_GROW_UPDATE: u32 = 0x2ABC_006C => "RGXFWIF_KCCB_CMD_FREELIST_GROW_UPDATE";
    KCCB_CMD_FREELISTS_RECONSTRUCTION_UPDATE: u32 = 0x2ABC_006D =>
        "RGXFWIF_KCCB_CMD_FREELISTS_RECONSTRUCTION_UPDATE";
    /// The host clears a return slot to this before its command lands.
    KCCB_RTN_SLOT_NO_RESPONSE: u32 = 0 => "RGXFWIF_KCCB_RTN_SLOT_NO_RESPONSE";
    /// `RGXFWIF_FWCCB_CMD`.
    FWCCB_CMD_SIZE: usize = 160 => "sizeof(RGXFWIF_FWCCB_CMD)";
    FWCCB_CMD_TYPE: usize = 0 => "offsetof(RGXFWIF_FWCCB_CMD, eCmdType)";
    FWCCB_CMD_DATA: usize = 8 => "offsetof(RGXFWIF_FWCCB_CMD, uCmdData)";
    FWCCB_FL_GS_ID: usize = 8 => "offsetof(RGXFWIF_FWCCB_CMD, uCmdData.sCmdFreeListGS.ui64FreelistID)";
    FWCCB_FL_RECON_COUNT: usize = 8 =>
        "offsetof(RGXFWIF_FWCCB_CMD, uCmdData.sCmdFreeListsReconstruction.ui32FreelistsCount)";
    FWCCB_FL_RECON_IDS: usize = 16 =>
        "offsetof(RGXFWIF_FWCCB_CMD, uCmdData.sCmdFreeListsReconstruction.aui64FreelistIDs)";
    FWCCB_CMD_FREELIST_GROW: u32 = 0x2ABC_0067 => "RGXFWIF_FWCCB_CMD_FREELIST_GROW";
    FWCCB_CMD_FREELISTS_RECONSTRUCTION: u32 = 0x2ABC_0068 => "RGXFWIF_FWCCB_CMD_FREELISTS_RECONSTRUCTION";
    FWCCB_CMD_CONTEXT_RESET_NOTIFICATION: u32 = 0x2ABC_0069 => "RGXFWIF_FWCCB_CMD_CONTEXT_RESET_NOTIFICATION";
    FWCCB_CMD_UPDATE_STATS: u32 = 0x2ABC_006B => "RGXFWIF_FWCCB_CMD_UPDATE_STATS";
    /// Every CCB command type carries this in its high half
    /// (rgx_fwif_km.h:495-498).
    CMD_MAGIC_DWORD_SHIFTED: u32 = 0x2ABC_0000 => "RGX_CMD_MAGIC_DWORD_SHIFTED";
    CMD_MAGIC_DWORD_MASK: u32 = 0xFFFF_0000 => "RGX_CMD_MAGIC_DWORD_MASK";
    /// `RGXFWIF_FWCCB_CMD_UPDATE_STATS_DATA`: which statistic, and by how much.
    FWCCB_STATS_ELEMENT: usize = 8 =>
        "offsetof(RGXFWIF_FWCCB_CMD, uCmdData.sCmdUpdateStatsData.eElementToUpdate)";
    FWCCB_STATS_ADJUST: usize = 16 =>
        "offsetof(RGXFWIF_FWCCB_CMD, uCmdData.sCmdUpdateStatsData.i32AdjustmentValue)";
    /// The first and last `RGXFWIF_FWCCB_CMD_UPDATE_STATS_TYPE`; the names in
    /// between are [`super::ccb`]'s.
    FWCCB_STATS_PARTIAL_RENDERS: u32 = 1 => "RGXFWIF_FWCCB_CMD_UPDATE_NUM_PARTIAL_RENDERS";
    FWCCB_STATS_OUT_OF_MEMORY: u32 = 2 => "RGXFWIF_FWCCB_CMD_UPDATE_NUM_OUT_OF_MEMORY";
    FWCCB_STATS_TDM_STORES: u32 = 6 => "RGXFWIF_FWCCB_CMD_UPDATE_NUM_TDM_STORES";
    MAX_FREELISTS_TO_RECONSTRUCT: u32 = 18 => "RGXFWIF_MAX_FREELISTS_TO_RECONSTRUCT";
    /// The doorbell: an MTS schedule of the general-purpose data master (the
    /// kernel CCB's), `RGXFWIF_DM_GP & 0xF`.
    CR_MTS_SCHEDULE: u32 = 0x0B00 => "RGX_CR_MTS_SCHEDULE";
    DM_GP: u32 = 0 => "RGXFWIF_DM_GP";

    // ---- RGXFWIF_FREELIST (rgx_fwif_km.h) ----
    FREELIST_SIZE: usize = 64 => "sizeof(RGXFWIF_FREELIST)";
    FREELIST_DEVVADDR: usize = 0 => "offsetof(RGXFWIF_FREELIST, psFreeListDevVAddr)";
    FREELIST_CURRENT_DEVVADDR: usize = 8 => "offsetof(RGXFWIF_FREELIST, ui64CurrentDevVAddr)";
    FREELIST_CURRENT_STACK_TOP: usize = 16 => "offsetof(RGXFWIF_FREELIST, ui32CurrentStackTop)";
    FREELIST_MAX_PAGES: usize = 20 => "offsetof(RGXFWIF_FREELIST, ui32MaxPages)";
    FREELIST_GROW_PAGES: usize = 24 => "offsetof(RGXFWIF_FREELIST, ui32GrowPages)";
    FREELIST_CURRENT_PAGES: usize = 28 => "offsetof(RGXFWIF_FREELIST, ui32CurrentPages)";
    FREELIST_ALLOCATED_PAGE_COUNT: usize = 32 => "offsetof(RGXFWIF_FREELIST, ui32AllocatedPageCount)";
    FREELIST_ALLOCATED_MMU_PAGE_COUNT: usize = 36 => "offsetof(RGXFWIF_FREELIST, ui32AllocatedMMUPageCount)";
    FREELIST_ID: usize = 40 => "offsetof(RGXFWIF_FREELIST, ui64FreeListID)";
    /// An `IMG_BOOL`, one byte.
    FREELIST_GROW_PENDING: usize = 48 => "offsetof(RGXFWIF_FREELIST, bGrowPending)";
    FREELIST_GROW_PENDING_SIZE: usize = 1 => "sizeof(((RGXFWIF_FREELIST *)0)->bGrowPending)";
    FREELIST_READY_PAGES: usize = 52 => "offsetof(RGXFWIF_FREELIST, ui32ReadyPages)";
    FREELIST_FLAGS: usize = 56 => "offsetof(RGXFWIF_FREELIST, ui32FreelistFlags)";
    FREELIST_PM_GLOBAL_PB: usize = 60 => "offsetof(RGXFWIF_FREELIST, ui32PmGlobalPb)";

    // ---- the firmware heap's sub-heaps (rgx_heap_firmware.h, with the
    // configuration's RGX_FW_*_SECTION_SIZE): where the firmware expects its
    // RW data and RO data relative to its code, and where the host's own
    // objects start (the DDK's host main heap)
    FW_PRIV_DATA_HEAP_OFF: u32 = 0x8_0000 => "RGX_FIRMWARE_PRIV_DATA_HEAP_BASE - RGX_FIRMWARE_RAW_HEAP_BASE";
    FW_PRIV_RODATA_HEAP_OFF: u32 = 0xA_0000 => "RGX_FIRMWARE_PRIV_RODATA_HEAP_BASE - RGX_FIRMWARE_RAW_HEAP_BASE";
    FW_HOST_MAIN_HEAP_OFF: u32 = 0xC_0000 => "RGX_FIRMWARE_HOST_MAIN_HEAP_BASE - RGX_FIRMWARE_RAW_HEAP_BASE";

    // ---- the FWIF structures the host builds (rgx_fwif_km.h, rgx_fwif_shared.h)
    SYSINIT_SIZE: usize = 880 => "sizeof(RGXFWIF_SYSINIT)";
    OSINIT_SIZE: usize = 160 => "sizeof(RGXFWIF_OSINIT)";
    SYSDATA_SIZE: usize = 3896 => "sizeof(RGXFWIF_SYSDATA)";
    OSDATA_SIZE: usize = 1088 => "sizeof(RGXFWIF_OSDATA)";
    RUNTIME_CFG_SIZE: usize = 160 => "sizeof(RGXFWIF_RUNTIME_CFG)";
    TRACEBUF_SIZE: usize = 440 => "sizeof(RGXFWIF_TRACEBUF)";
    HWRINFOBUF_SIZE: usize = 2336 => "sizeof(RGXFWIF_HWRINFOBUF)";
    HWPERFDATA_SIZE: usize = 40 => "sizeof(RGXFWIF_HWPERFDATA)";
    GPU_UTIL_FW_SIZE: usize = 10256 => "sizeof(RGXFWIF_GPU_UTIL_FW)";
    CONNECTION_CTL_SIZE: usize = 16 => "sizeof(RGXFWIF_CONNECTION_CTL)";
    /// The trace: one thread, the default ring size.
    TRACE_BUF_DEFAULT_SIZE_IN_DWORDS: u32 = 12000 => "RGXFW_TRACE_BUF_DEFAULT_SIZE_IN_DWORDS";
    LOG_TYPE_MASK: u32 = 0x8007_FFFF => "RGXFWIF_LOG_TYPE_MASK";
    LOG_TYPE_GROUP_DEBUG: u32 = 0x8000_0000 => "RGXFWIF_LOG_TYPE_GROUP_DEBUG";
    LOG_TYPE_GROUP_MTS: u32 = 0x4 => "RGXFWIF_LOG_TYPE_GROUP_MTS";
    LOG_TYPE_GROUP_POW: u32 = 0x200 => "RGXFWIF_LOG_TYPE_GROUP_POW";
    TRACEBUF_LOG_TYPE: usize = 0 => "offsetof(RGXFWIF_TRACEBUF, ui32LogType)";
    TRACEBUF_TRACE_POINTER: usize = 8 => "offsetof(RGXFWIF_TRACEBUF, sTraceBuf[0].ui32TracePointer)";
    TRACEBUF_WRAP_COUNT: usize = 12 => "offsetof(RGXFWIF_TRACEBUF, sTraceBuf[0].ui32WrapCount)";
    TRACEBUF_BUFFER: usize = 16 => "offsetof(RGXFWIF_TRACEBUF, sTraceBuf[0].pui32RGXFWIfTraceBuffer)";
    TRACEBUF_ASSERT_BUF: usize = 24 => "offsetof(RGXFWIF_TRACEBUF, sTraceBuf[0].sAssertBuf)";
    TRACEBUF_SIZE_IN_DWORDS: usize = 432 => "offsetof(RGXFWIF_TRACEBUF, ui32TraceBufSizeInDWords)";
    TRACEBUF_FLAGS: usize = 436 => "offsetof(RGXFWIF_TRACEBUF, ui32TracebufFlags)";
    FILE_INFO_BUF_PATH: usize = 0 => "offsetof(RGXFWIF_FILE_INFO_BUF, szPath)";
    FILE_INFO_BUF_INFO: usize = 200 => "offsetof(RGXFWIF_FILE_INFO_BUF, szInfo)";
    FILE_INFO_BUF_LINE_NUM: usize = 400 => "offsetof(RGXFWIF_FILE_INFO_BUF, ui32LineNum)";
    INICFG_CTXSWITCH_PROFILE_MEDIUM: u32 = 0x2_0000 => "RGXFWIF_INICFG_CTXSWITCH_PROFILE_MEDIUM";
    INICFG_ASSERT_ON_HWR_TRIGGER: u32 = 0x10_0000 => "RGXFWIF_INICFG_ASSERT_ON_HWR_TRIGGER";
    INICFG_OS_CTXSWITCH_TDM_EN: u32 = 0x1 => "RGXFWIF_INICFG_OS_CTXSWITCH_TDM_EN";
    INICFG_OS_CTXSWITCH_GEOM_EN: u32 = 0x2 => "RGXFWIF_INICFG_OS_CTXSWITCH_GEOM_EN";
    INICFG_OS_CTXSWITCH_3D_EN: u32 = 0x4 => "RGXFWIF_INICFG_OS_CTXSWITCH_3D_EN";
    INICFG_OS_CTXSWITCH_CDM_EN: u32 = 0x8 => "RGXFWIF_INICFG_OS_CTXSWITCH_CDM_EN";
    SYSDATA_CONFIG_FLAGS: usize = 0 => "offsetof(RGXFWIF_SYSDATA, ui32ConfigFlags)";
    OSDATA_FW_OS_CONFIG_FLAGS: usize = 0 => "offsetof(RGXFWIF_OSDATA, ui32FwOsConfigFlags)";
    OSDATA_POWER_SYNC: usize = 568 => "offsetof(RGXFWIF_OSDATA, sPowerSync)";
    RUNTIME_ACTIVE_PM_LATENCY_MS: usize = 0 => "offsetof(RGXFWIF_RUNTIME_CFG, ui32ActivePMLatencyms)";
    RUNTIME_ACTIVE_PM_LATENCY_PERSISTANT: usize = 8 => "offsetof(RGXFWIF_RUNTIME_CFG, bActivePMLatencyPersistant)";
    RUNTIME_CORE_CLOCK_SPEED: usize = 12 => "offsetof(RGXFWIF_RUNTIME_CFG, ui32CoreClockSpeed)";
    RUNTIME_SOC_CLOCK_SPEED: usize = 16 => "offsetof(RGXFWIF_RUNTIME_CFG, ui32SOCClockSpeed)";
    RUNTIME_POW_UNITS_STATE: usize = 20 => "offsetof(RGXFWIF_RUNTIME_CFG, ui32PowUnitsState)";
    RUNTIME_PHR_MODE: usize = 24 => "offsetof(RGXFWIF_RUNTIME_CFG, ui32PHRMode)";
    RUNTIME_HCS_DEADLINE_MS: usize = 28 => "offsetof(RGXFWIF_RUNTIME_CFG, ui32HCSDeadlineMS)";
    RUNTIME_WDG_PERIOD_US: usize = 32 => "offsetof(RGXFWIF_RUNTIME_CFG, ui32WdgPeriodUs)";
    HCS_DEFAULT_DEADLINE_MS: u32 = 0xFFFF_FFFF => "RGX_HCS_DEFAULT_DEADLINE_MS";
    SAFETY_WATCHDOG_PERIOD_IN_US: u32 = 2_000_000 => "RGXFW_SAFETY_WATCHDOG_PERIOD_IN_US";
    OSINIT_KCCB_CTL: usize = 0 => "offsetof(RGXFWIF_OSINIT, psKernelCCBCtl)";
    OSINIT_KCCB: usize = 4 => "offsetof(RGXFWIF_OSINIT, psKernelCCB)";
    OSINIT_KCCB_RTN_SLOTS: usize = 8 => "offsetof(RGXFWIF_OSINIT, psKernelCCBRtnSlots)";
    OSINIT_FWCCB_CTL: usize = 12 => "offsetof(RGXFWIF_OSINIT, psFirmwareCCBCtl)";
    OSINIT_FWCCB: usize = 16 => "offsetof(RGXFWIF_OSINIT, psFirmwareCCB)";
    OSINIT_HWRINFOBUF_CTL: usize = 28 => "offsetof(RGXFWIF_OSINIT, sRGXFWIfHWRInfoBufCtl)";
    OSINIT_FW_OS_DATA: usize = 36 => "offsetof(RGXFWIF_OSINIT, sFwOsData)";
    OSINIT_HWPERF_DATA: usize = 144 => "offsetof(RGXFWIF_OSINIT, sHwPerfData)";
    OSINIT_HWPERF_RUNTIME_CFG: usize = 148 => "offsetof(RGXFWIF_OSINIT, sHwPerfRuntimeCfg)";
    OSINIT_GPU_UTIL_FW_CTL: usize = 152 => "offsetof(RGXFWIF_OSINIT, sGpuUtilFWCtl)";
    SYSINIT_FAULT_PHYS_ADDR: usize = 560 => "offsetof(RGXFWIF_SYSINIT, sFaultPhysAddr)";
    SYSINIT_PDS_EXEC_BASE: usize = 568 => "offsetof(RGXFWIF_SYSINIT, sPDSExecBase)";
    SYSINIT_USC_EXEC_BASE: usize = 576 => "offsetof(RGXFWIF_SYSINIT, sUSCExecBase)";
    SYSINIT_RUNTIME_CFG: usize = 628 => "offsetof(RGXFWIF_SYSINIT, sRuntimeCfg)";
    SYSINIT_TRACEBUF_CTL: usize = 632 => "offsetof(RGXFWIF_SYSINIT, sTraceBufCtl)";
    SYSINIT_FW_SYS_DATA: usize = 636 => "offsetof(RGXFWIF_SYSINIT, sFwSysData)";
    SYSINIT_HWPERF_CTL: usize = 644 => "offsetof(RGXFWIF_SYSINIT, sHWPerfCtl)";
    SYSINIT_ALIGN_CHECKS: usize = 648 => "offsetof(RGXFWIF_SYSINIT, sAlignChecks)";
    SYSINIT_INITIAL_CORE_CLOCK_SPEED: usize = 652 => "offsetof(RGXFWIF_SYSINIT, ui32InitialCoreClockSpeed)";
    /// An `IMG_BOOL`, one byte; the firmware sets it once it has booted.
    SYSINIT_FIRMWARE_STARTED: usize = 664 => "offsetof(RGXFWIF_SYSINIT, bFirmwareStarted)";
    SYSINIT_FIRMWARE_STARTED_SIZE: usize = 1 => "sizeof(((RGXFWIF_SYSINIT *)0)->bFirmwareStarted)";
    SYSINIT_MARKER_VAL: usize = 668 => "offsetof(RGXFWIF_SYSINIT, ui32MarkerVal)";
    ALIGN_CHECKS_UM_MAX: u32 = 128 => "RGXFW_ALIGN_CHECKS_UM_MAX";

    // ---- the render context (rgx_fwif_km.h, rgx_fwif_shared.h)
    FWMEMCONTEXT_SIZE: usize = 32 => "sizeof(RGXFWIF_FWMEMCONTEXT)";
    FWMEMCONTEXT_PC_DEV_PADDR: usize = 0 => "offsetof(RGXFWIF_FWMEMCONTEXT, sPCDevPAddr)";
    FWMEMCONTEXT_PAGE_CAT_BASE_REG_SET: usize = 8 => "offsetof(RGXFWIF_FWMEMCONTEXT, uiPageCatBaseRegSet)";
    FWRENDERCONTEXT_SIZE: usize = 912 => "sizeof(RGXFWIF_FWRENDERCONTEXT)";
    FWRENDERCONTEXT_TA_CONTEXT: usize = 0 => "offsetof(RGXFWIF_FWRENDERCONTEXT, sTAContext)";
    FWRENDERCONTEXT_3D_CONTEXT: usize = 456 => "offsetof(RGXFWIF_FWRENDERCONTEXT, s3DContext)";
    FWCOMMONCONTEXT_CCB_CTL: usize = 0 => "offsetof(RGXFWIF_FWCOMMONCONTEXT, psCCBCtl)";
    FWCOMMONCONTEXT_CCB: usize = 4 => "offsetof(RGXFWIF_FWCOMMONCONTEXT, psCCB)";
    FWCOMMONCONTEXT_CONTEXT_STATE: usize = 8 => "offsetof(RGXFWIF_FWCOMMONCONTEXT, psContextState)";
    FWCOMMONCONTEXT_PRIORITY: usize = 16 => "offsetof(RGXFWIF_FWCOMMONCONTEXT, i32Priority)";
    FWCOMMONCONTEXT_COMPAT_FLAGS: usize = 28 => "offsetof(RGXFWIF_FWCOMMONCONTEXT, ui32CompatFlags)";
    FWCOMMONCONTEXT_MAX_DEADLINE_MS: usize = 64 => "offsetof(RGXFWIF_FWCOMMONCONTEXT, ui32MaxDeadlineMS)";
    FWCOMMONCONTEXT_FW_MEM_CONTEXT: usize = 96 => "offsetof(RGXFWIF_FWCOMMONCONTEXT, psFWMemContext)";
    FWCOMMONCONTEXT_SERVER_COMMON_CONTEXT_ID: usize = 100 =>
        "offsetof(RGXFWIF_FWCOMMONCONTEXT, ui32ServerCommonContextID)";
    FWCOMMONCONTEXT_PID: usize = 104 => "offsetof(RGXFWIF_FWCOMMONCONTEXT, ui32PID)";
    FWCOMMONCONTEXT_RANGE_CHECK_DM: usize = 204 => "offsetof(RGXFWIF_FWCOMMONCONTEXT, sRangeCheckBeforeUse.eDM)";
    CONTEXT_COMPAT_FLAGS_HAS_DEFER_COUNT: u32 = 0x2 => "RGXFWIF_CONTEXT_COMPAT_FLAGS_HAS_DEFER_COUNT";
    DM_GEOM: u32 = 2 => "RGXFWIF_DM_GEOM";
    DM_3D: u32 = 3 => "RGXFWIF_DM_3D";
    CCCB_CTL_SIZE: usize = 32 => "sizeof(RGXFWIF_CCCB_CTL)";
    CCCB_CTL_WRITE_OFFSET: usize = 0 => "offsetof(RGXFWIF_CCCB_CTL, ui32WriteOffset)";
    CCCB_CTL_READ_OFFSET: usize = 4 => "offsetof(RGXFWIF_CCCB_CTL, ui32ReadOffset)";
    CCCB_CTL_WRAP_MASK: usize = 12 => "offsetof(RGXFWIF_CCCB_CTL, ui32WrapMask)";
    TACTX_STATE_VDM_CALL_STACK_POINTER_INIT: usize = 8 =>
        "offsetof(RGXFWIF_TACTX_STATE, asGeomCore[0].uTAReg_VDM_CALL_STACK_POINTER_Init)";

    // ---- the HWRT data (rgx_fwif_km.h)
    HWRTDATA_COMMON_SIZE: usize = 88 => "sizeof(RGXFWIF_HWRTDATA_COMMON)";
    HWRTDATA_COMMON_TA_CACHES_NEED_ZEROING: usize = 0 => "offsetof(RGXFWIF_HWRTDATA_COMMON, bTACachesNeedZeroing)";
    HWRTDATA_COMMON_TA_CACHES_NEED_ZEROING_SIZE: usize = 1 =>
        "sizeof(((RGXFWIF_HWRTDATA_COMMON *)0)->bTACachesNeedZeroing)";
    HWRTDATA_COMMON_SCREEN_PIXEL_MAX: usize = 4 => "offsetof(RGXFWIF_HWRTDATA_COMMON, ui32ScreenPixelMax)";
    HWRTDATA_COMMON_MULTI_SAMPLE_CTL: usize = 8 => "offsetof(RGXFWIF_HWRTDATA_COMMON, ui64MultiSampleCtl)";
    HWRTDATA_COMMON_FLIPPED_MULTI_SAMPLE_CTL: usize = 16 =>
        "offsetof(RGXFWIF_HWRTDATA_COMMON, ui64FlippedMultiSampleCtl)";
    HWRTDATA_COMMON_TPC_STRIDE: usize = 24 => "offsetof(RGXFWIF_HWRTDATA_COMMON, ui32TPCStride)";
    HWRTDATA_COMMON_TPC_SIZE: usize = 28 => "offsetof(RGXFWIF_HWRTDATA_COMMON, ui32TPCSize)";
    HWRTDATA_COMMON_TE_SCREEN: usize = 32 => "offsetof(RGXFWIF_HWRTDATA_COMMON, ui32TEScreen)";
    HWRTDATA_COMMON_MTILE_STRIDE: usize = 36 => "offsetof(RGXFWIF_HWRTDATA_COMMON, ui32MTileStride)";
    HWRTDATA_COMMON_TE_AA: usize = 40 => "offsetof(RGXFWIF_HWRTDATA_COMMON, ui32TEAA)";
    HWRTDATA_COMMON_TE_MTILE1: usize = 44 => "offsetof(RGXFWIF_HWRTDATA_COMMON, ui32TEMTILE1)";
    HWRTDATA_COMMON_TE_MTILE2: usize = 48 => "offsetof(RGXFWIF_HWRTDATA_COMMON, ui32TEMTILE2)";
    HWRTDATA_COMMON_RGN_HEADER_SIZE: usize = 76 => "offsetof(RGXFWIF_HWRTDATA_COMMON, uiRgnHeaderSize)";
    HWRTDATA_COMMON_RGN_HEADER_SIZE_SIZE: usize = 4 => "sizeof(((RGXFWIF_HWRTDATA_COMMON *)0)->uiRgnHeaderSize)";
    HWRTDATA_COMMON_ISP_MTILE_SIZE: usize = 80 => "offsetof(RGXFWIF_HWRTDATA_COMMON, ui32ISPMtileSize)";
    HWRTDATA_SIZE: usize = 256 => "sizeof(RGXFWIF_HWRTDATA)";
    HWRTDATA_PM_MLIST_DEVVADDR: usize = 0 => "offsetof(RGXFWIF_HWRTDATA, psPMMListDevVAddr)";
    /// The PM's ALIST and MList stack pointers the firmware keeps in the
    /// HWRT data (the runaway batch's dump, `render::runaway_dump`).
    HWRTDATA_PM_ALIST_STACK_POINTER: usize = 56 => "offsetof(RGXFWIF_HWRTDATA, ui64PMAListStackPointer)";
    HWRTDATA_PM_MLIST_STACK_POINTER: usize = 64 => "offsetof(RGXFWIF_HWRTDATA, ui32PMMListStackPointer)";
    HWRTDATA_COMMON_FWADDR: usize = 68 => "offsetof(RGXFWIF_HWRTDATA, sHWRTDataCommonFwAddr)";
    HWRTDATA_STATE: usize = 76 => "offsetof(RGXFWIF_HWRTDATA, eState)";
    HWRTDATA_STATE_SIZE: usize = 4 => "sizeof(((RGXFWIF_HWRTDATA *)0)->eState)";
    HWRTDATA_FREELISTS: usize = 80 => "offsetof(RGXFWIF_HWRTDATA, apsFreeLists)";
    HWRTDATA_FREELIST_PTR_SIZE: usize = 4 => "sizeof(((RGXFWIF_HWRTDATA *)0)->apsFreeLists[0])";
    HWRTDATA_VHEAP_TABLE_DEVVADDR: usize = 104 => "offsetof(RGXFWIF_HWRTDATA, psVHeapTableDevVAddr)";
    HWRTDATA_TAIL_PTRS_DEVVADDR: usize = 144 => "offsetof(RGXFWIF_HWRTDATA, sTailPtrsDevVAddr)";
    HWRTDATA_MACROTILE_ARRAY_DEVVADDR: usize = 152 => "offsetof(RGXFWIF_HWRTDATA, sMacrotileArrayDevVAddr)";
    HWRTDATA_RGN_HEADER_DEVVADDR: usize = 160 => "offsetof(RGXFWIF_HWRTDATA, sRgnHeaderDevVAddr)";
    HWRTDATA_RTC_DEVVADDR: usize = 168 => "offsetof(RGXFWIF_HWRTDATA, sRTCDevVAddr)";
    HWRTDATA_CLEANUP_STATE: usize = 192 => "offsetof(RGXFWIF_HWRTDATA, sCleanupState)";
    HWRTDATA_CLEANUP_SUBMITTED: usize = 192 => "offsetof(RGXFWIF_HWRTDATA, sCleanupState.ui32SubmittedCommands)";
    HWRTDATA_CLEANUP_EXECUTED: usize = 196 => "offsetof(RGXFWIF_HWRTDATA, sCleanupState.ui32ExecutedCommands)";
    RTDATA_STATE_NONE: u32 = 0 => "RGXFWIF_RTDATA_STATE_NONE";
    /// The state a freelist reconstruction leaves an HWRT data in (Linux
    /// drm/imagination `pvr_free_list_reconstruct`).
    RTDATA_STATE_HWR: u32 = 9 => "RGXFWIF_RTDATA_STATE_HWR";
    HWRTDATA_FLAGS: usize = 72 => "offsetof(RGXFWIF_HWRTDATA, ui32HWRTDataFlags)";
    HWRTDATA_FLAGS_SIZE: usize = 4 => "sizeof(((RGXFWIF_HWRTDATA *)0)->ui32HWRTDataFlags)";
    /// The flag a reconstruction clears (drm/imagination calls it
    /// HWRTDATA_HAS_LAST_GEOM).
    HWRTDATA_HAS_LAST_TA: u32 = 0x4 => "HWRTDATA_HAS_LAST_TA";
    LOCAL_FREELIST: usize = 0 => "RGXFW_LOCAL_FREELIST";
    GLOBAL_FREELIST: usize = 1 => "RGXFW_GLOBAL_FREELIST";
    GLOBAL2_FREELIST: usize = 2 => "RGXFW_GLOBAL2_FREELIST";

    // ---- the client CCB commands (rgx_fwif_shared.h)
    CCB_CMD_HEADER_SIZE: usize = 16 => "sizeof(RGXFWIF_CCB_CMD_HEADER)";
    CCB_CMD_HEADER_TYPE: usize = 0 => "offsetof(RGXFWIF_CCB_CMD_HEADER, eCmdType)";
    CCB_CMD_HEADER_SIZE_FIELD: usize = 4 => "offsetof(RGXFWIF_CCB_CMD_HEADER, ui32CmdSize)";
    CCB_CMD_TYPE_GEOM: u32 = 0x2ABC_80C9 => "RGXFWIF_CCB_CMD_TYPE_GEOM";
    CCB_CMD_TYPE_3D: u32 = 0x2ABC_80CB => "RGXFWIF_CCB_CMD_TYPE_3D";
    CCB_CMD_TYPE_FENCE: u32 = 0x2ABC_00D4 => "RGXFWIF_CCB_CMD_TYPE_FENCE";
    CCB_CMD_TYPE_UPDATE: u32 = 0x2ABC_00D5 => "RGXFWIF_CCB_CMD_TYPE_UPDATE";
    CCB_CMD_TYPE_FENCE_PR: u32 = 0x2ABC_00D7 => "RGXFWIF_CCB_CMD_TYPE_FENCE_PR";
    CCB_CMD_TYPE_PADDING: u32 = 0x2ABC_00DD => "RGXFWIF_CCB_CMD_TYPE_PADDING";
    UFO_SIZE: usize = 8 => "sizeof(RGXFWIF_UFO)";
    UFO_ADDR: usize = 0 => "offsetof(RGXFWIF_UFO, puiAddrUFO)";
    UFO_VALUE: usize = 4 => "offsetof(RGXFWIF_UFO, ui32Value)";
    CMDTA3D_SHARED_HWRT_DATA: usize = 4 => "offsetof(CMDTA3D_SHARED, sHWRTData)";

    // ---- the kick, MMU-cache and cleanup commands (rgx_fwif_km.h)
    KCCB_CMD_KICK: u32 = 0x2ABC_0065 => "RGXFWIF_KCCB_CMD_KICK";
    KCCB_CMD_MMUCACHE: u32 = 0x2ABC_0066 => "RGXFWIF_KCCB_CMD_MMUCACHE";
    KCCB_CMD_CLEANUP: u32 = 0x2ABC_0069 => "RGXFWIF_KCCB_CMD_CLEANUP";
    KCCB_CMD_COMBINED_TA_3D_KICK: u32 = 0x2ABC_0071 => "RGXFWIF_KCCB_CMD_COMBINED_TA_3D_KICK";
    KCCB_KICK_CONTEXT: usize = 8 => "offsetof(RGXFWIF_KCCB_CMD, uCmdData.sCmdKickData.psContext)";
    KCCB_KICK_CWOFF: usize = 12 => "offsetof(RGXFWIF_KCCB_CMD, uCmdData.sCmdKickData.ui32CWoffUpdate)";
    KCCB_KICK_CWRAPMASK: usize = 16 => "offsetof(RGXFWIF_KCCB_CMD, uCmdData.sCmdKickData.ui32CWrapMaskUpdate)";
    KCCB_KICK_NUM_CLEANUP: usize = 20 => "offsetof(RGXFWIF_KCCB_CMD, uCmdData.sCmdKickData.ui32NumCleanupCtl)";
    KCCB_KICK_CLEANUP_CTL: usize = 24 => "offsetof(RGXFWIF_KCCB_CMD, uCmdData.sCmdKickData.apsCleanupCtl)";
    KCCB_KICK_DATA_SIZE: usize = 28 => "sizeof(RGXFWIF_KCCB_CMD_KICK_DATA)";
    KCCB_COMBINED_TA: usize = 8 => "offsetof(RGXFWIF_KCCB_CMD, uCmdData.sCombinedTA3DCmdKickData.sTACmdKickData)";
    KCCB_COMBINED_3D: usize = 36 => "offsetof(RGXFWIF_KCCB_CMD, uCmdData.sCombinedTA3DCmdKickData.s3DCmdKickData)";
    KCCB_MMUCACHE_FLAGS: usize = 8 => "offsetof(RGXFWIF_KCCB_CMD, uCmdData.sMMUCacheData.ui32CacheFlags)";
    KCCB_MMUCACHE_SYNC: usize = 12 => "offsetof(RGXFWIF_KCCB_CMD, uCmdData.sMMUCacheData.sMMUCacheSync)";
    MMUCACHEDATA_FLAGS_PT: u32 = 0x1 => "RGXFWIF_MMUCACHEDATA_FLAGS_PT";
    MMUCACHEDATA_FLAGS_PD: u32 = 0x2 => "RGXFWIF_MMUCACHEDATA_FLAGS_PD";
    MMUCACHEDATA_FLAGS_PC: u32 = 0x4 => "RGXFWIF_MMUCACHEDATA_FLAGS_PC";
    MMUCACHEDATA_FLAGS_TLB: u32 = 0x18 => "RGXFWIF_MMUCACHEDATA_FLAGS_TLB";
    KCCB_CLEANUP_TYPE: usize = 8 => "offsetof(RGXFWIF_KCCB_CMD, uCmdData.sCleanupData.eCleanupType)";
    KCCB_CLEANUP_DATA: usize = 12 => "offsetof(RGXFWIF_KCCB_CMD, uCmdData.sCleanupData.uCleanupData)";
    CLEANUP_HWRTDATA: u32 = 1 => "RGXFWIF_CLEANUP_HWRTDATA";
    KCCB_RTN_SLOT_CMD_EXECUTED: u32 = 0x1 => "RGXFWIF_KCCB_RTN_SLOT_CMD_EXECUTED";
    KCCB_RTN_SLOT_CLEANUP_BUSY: u32 = 0x2 => "RGXFWIF_KCCB_RTN_SLOT_CLEANUP_BUSY";

    // ---- the HWR information (rgx_fwif_km.h)
    HWRINFOBUF_HWR_INFO: usize = 0 => "offsetof(RGXFWIF_HWRINFOBUF, sHWRInfo)";
    HWRINFOBUF_HWR_COUNTER: usize = 2176 => "offsetof(RGXFWIF_HWRINFOBUF, ui32HwrCounter)";
    HWRINFO_SIZE: usize = 136 => "sizeof(RGX_HWRINFO)";
    HWINFO_MAX: u32 = 16 => "RGXFWIF_HWINFO_MAX";
    HWRINFO_BIF_REQ_STATUS: usize = 0 => "offsetof(RGX_HWRINFO, uHWRData.sBIFInfo.ui64BIFReqStatus)";
    HWRINFO_BIF_MMU_STATUS: usize = 8 => "offsetof(RGX_HWRINFO, uHWRData.sBIFInfo.ui64BIFMMUStatus)";
    HWRINFO_BIF_PC_ADDRESS: usize = 16 => "offsetof(RGX_HWRINFO, uHWRData.sBIFInfo.ui64PCAddress)";
    HWRINFO_HWR_NUMBER: usize = 60 => "offsetof(RGX_HWRINFO, ui32HWRNumber)";
    HWRINFO_EVENT_STATUS: usize = 64 => "offsetof(RGX_HWRINFO, ui32EventStatus)";
    HWRINFO_RECOVERY_FLAGS: usize = 68 => "offsetof(RGX_HWRINFO, ui32HWRRecoveryFlags)";
    HWRINFO_HWR_TYPE: usize = 72 => "offsetof(RGX_HWRINFO, eHWRType)";
    HWRINFO_DM: usize = 76 => "offsetof(RGX_HWRINFO, eDM)";
}

/// `RGXFW_ALIGN_CHECKS_INIT_KM` (rgx_fwif_alignchecks.h): the host's
/// structure sizes and offsets the firmware compares with its own at boot,
/// refusing to boot on a difference. `km_check pins` prints it as `ALIGN`
/// lines and the host check compares them with the macro element by element.
pub const ALIGN_CHECKS_INIT_KM: [u32; 25] =
    [880, 560, 568, 576, 584, 632, 636, 160, 0, 4, 12, 16, 36, 40, 912, 0, 456, 456, 96, 80, 4, 12, 0, 4, 8];

/// The alignment-check table for the host check: entry `i`, false past the end.
///
/// # Safety
/// `val` valid for writing.
#[no_mangle]
pub unsafe extern "C" fn yd_pvr_fwif_align_check(i: u32, val: *mut u32) -> bool {
    match ALIGN_CHECKS_INIT_KM.get(i as usize) {
        Some(&v) => {
            // SAFETY: the caller's contract.
            unsafe { *val = v };
            true
        }
        None => false,
    }
}

// ---- shared-memory access -------------------------------------------------------
//
// The structures the firmware reads are written field by field at their DDK
// offsets (above), volatile, as the C driver's PVR_FW_WR32/64 did.

/// # Safety
/// `base + off` is a valid, 4-byte aligned word of shared memory.
pub unsafe fn wr32(base: *mut u8, off: usize, v: u32) {
    // SAFETY: the caller's contract.
    unsafe { (base.add(off) as *mut u32).write_volatile(v) }
}

/// # Safety
/// As [`wr32`], 8-byte aligned.
pub unsafe fn wr64(base: *mut u8, off: usize, v: u64) {
    // SAFETY: the caller's contract.
    unsafe { (base.add(off) as *mut u64).write_volatile(v) }
}

/// # Safety
/// As [`wr32`].
pub unsafe fn rd32(base: *const u8, off: usize) -> u32 {
    // SAFETY: the caller's contract.
    unsafe { (base.add(off) as *const u32).read_volatile() }
}

/// # Safety
/// As [`wr64`].
pub unsafe fn rd64(base: *const u8, off: usize) -> u64 {
    // SAFETY: the caller's contract.
    unsafe { (base.add(off) as *const u64).read_volatile() }
}

/// The pin table for the host check: entry `i`'s C expression (`expr`,
/// `len` bytes, not NUL-terminated) and value. False past the end.
///
/// # Safety
/// The three pointers valid for writing.
#[no_mangle]
pub unsafe extern "C" fn yd_pvr_fwif_pin(i: u32, expr: *mut *const u8, len: *mut usize, val: *mut u64) -> bool {
    match PINS.get(i as usize) {
        Some(&(e, v)) => {
            // SAFETY: the caller's contract.
            unsafe {
                *expr = e.as_ptr();
                *len = e.len();
                *val = v;
            }
            true
        }
        None => false,
    }
}
