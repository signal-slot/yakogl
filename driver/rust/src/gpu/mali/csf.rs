// SPDX-License-Identifier: Apache-2.0 AND MIT
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// SPDX-FileCopyrightText: 2023 Collabora ltd.
//! The CSF firmware's host side (driver/docs/nanopc-t6-mali.md, "The
//! firmware (Z3)"): load the sections of a parsed image ([`super::fw`]),
//! start the MCU, find the shared interface (the global block, the command
//! stream groups and their streams), write the global configuration, and
//! ping the firmware. Z3 on the board is this sequence answering with GLB
//! version 1.5.0 and an acknowledged `PING`.
//!
//! Derived from Linux v6.18's drivers/gpu/drm/panthor (Copyright 2023
//! Collabora ltd.), used under the MIT option of its `GPL-2.0 or MIT`
//! licence (the design page's decision 1): the interface structures and the
//! req/ack helpers of panthor_fw.h:15-269 and :403-458 (and `CSF_DOORBELL`,
//! `JOB_INT_*`, `MCU_CONTROL` of panthor_regs.h:121-138, 205-206); in
//! panthor_fw.c the section copy (`panthor_fw_init_section_mem`, :398-420),
//! the timer conversion (:330-360), the interface discovery
//! (`iface_fw_to_cpu_addr`, `panthor_init_cs_iface`,
//! `panthor_init_csg_iface`, `panthor_fw_init_ifaces`, :810-976), the
//! global init (:978-1009), the boot (`panthor_fw_start`, :1028-1059), the
//! acknowledge wait (:1203-1257), the group doorbells (:1305-1311), the
//! ping (:1313-1335) and the order of `panthor_fw_init` (:1343-1403). Where
//! this differs from panthor it says so.
//!
//! Everything goes through [`Mmio`]: the GPU's registers at their base, and
//! the memory behind the MCU's sections at the CPU addresses a
//! [`McuMemory`] gives. AS0, which maps those sections for the MCU, is Z2's
//! (the page tables and the cache flush); this module only asks it where a
//! section is and to clean what was written. So on the host the whole
//! sequence runs against a model of the MCU (the tests below).

use super::fw::{Image, Section, MCU_SHARED_REGION_START};
use super::MCU_STATUS;
use crate::mmio::Mmio;
use core::mem::offset_of;

/// `MCU_CONTROL` and its values (panthor_regs.h:121-124).
pub const MCU_CONTROL: usize = 0x700;
pub const MCU_CONTROL_DISABLE: u32 = 0;
pub const MCU_CONTROL_ENABLE: u32 = 1;
pub const MCU_CONTROL_AUTO: u32 = 2;
/// `MCU_STATUS` values (panthor_regs.h:126-130).
pub const MCU_STATUS_DISABLED: u32 = 0;
pub const MCU_STATUS_ENABLED: u32 = 1;
pub const MCU_STATUS_HALT: u32 = 2;
pub const MCU_STATUS_FATAL: u32 = 3;

/// The job interrupt block (panthor_regs.h:133-138): bit 31 is the global
/// interface, bit n the command stream group n.
pub const JOB_INT_RAWSTAT: usize = 0x1000;
pub const JOB_INT_CLEAR: usize = 0x1004;
pub const JOB_INT_MASK: usize = 0x1008;
pub const JOB_INT_STAT: usize = 0x100c;
pub const JOB_INT_GLOBAL_IF: u32 = 1 << 31;
pub const fn job_int_csg_if(i: u32) -> u32 {
    1 << i
}

/// `CSF_DOORBELL(i)` (panthor_regs.h:205): one page of doorbell per id; id
/// 0 is the global interface's (`CSF_GLB_DOORBELL_ID`), a group's streams
/// ring their CSG slot + 1.
pub const fn doorbell(id: u32) -> usize {
    0x8_0000 + id as usize * 0x1_0000
}
pub const GLB_DOORBELL_ID: u32 = 0;

/// The interface's place in the shared section (panthor_fw.c:211-213): the
/// GLB control block at its start, group n's control block at
/// `CSF_GROUP_CONTROL_OFFSET` + n · group_stride, stream m's at the group's
/// plus `CSF_STREAM_CONTROL_OFFSET` + m · stream_stride. The board's file
/// has 8 groups of stride 0xa0, 8 streams of stride 0xc each, before boot.
pub const GROUP_CONTROL_OFFSET: u32 = 0x1000;
pub const STREAM_CONTROL_OFFSET: u32 = 0x40;
/// `CSF_UNPRESERVED_REG_COUNT`: work registers the firmware does not keep
/// across a suspend; the stream's user registers are the rest.
pub const UNPRESERVED_REG_COUNT: u32 = 4;
/// panthor's bounds on the counts the firmware reports (`MAX_CSGS`,
/// panthor_fw.h:12-13; `MIN_CSGS`, `MIN_CS_PER_CSG`, panthor_fw.c:202-203).
pub const MAX_CSGS: u32 = 31;
pub const MIN_CSGS: u32 = 3;
pub const MAX_CS_PER_CSG: u32 = 32;
pub const MIN_CS_PER_CSG: u32 = 8;

/// The waits: the MCU's boot 1 s (`panthor_fw_start`), a global request's
/// acknowledge 100 ms (the ping's, `panthor_fw_ping_work`).
pub const BOOT_WAIT_US: u32 = 1_000_000;
pub const GLB_ACK_WAIT_US: u32 = 100_000;
/// `PING_INTERVAL_MS` (panthor_fw.c:30): panthor pings the firmware this
/// often; the kernel side must do the same once the firmware runs.
pub const PING_INTERVAL_MS: u32 = 12_000;

/// The global timers (panthor_fw.c:31-34): the progress timeout in GPU
/// cycles, scaled down by 2^10; the idle and power-off hysteresis in
/// microseconds, converted to timer ticks by [`conv_timeout`].
pub const PROGRESS_TIMEOUT_CYCLES: u64 = 5 * 500 * 1024 * 1024;
pub const PROGRESS_TIMEOUT_SCALE_SHIFT: u32 = 10;
pub const IDLE_HYSTERESIS_US: u32 = 800;
pub const PWROFF_HYSTERESIS_US: u32 = 10_000;
/// The arch timer's rate on the RK3588 (CNTFRQ_EL0 under Armbian; the
/// design page's "timers in 24 MHz ticks").
pub const RK3588_TIMER_HZ: u32 = 24_000_000;

/// `CSF_IFACE_VERSION(major, minor, patch)` and its fields
/// (panthor_fw.c:205-209).
pub const fn iface_version(major: u32, minor: u32, patch: u32) -> u32 {
    major << 24 | minor << 16 | patch
}
pub const fn version_major(v: u32) -> u32 {
    v >> 24
}
pub const fn version_minor(v: u32) -> u32 {
    (v >> 16) & 0xff
}
pub const fn version_patch(v: u32) -> u32 {
    v & 0xffff
}

// The interface, as panthor_fw.h lays it out. These structures are layouts
// only: the fields are read and written one at a time through Mmio at
// `offset_of!` of them, never through a reference into shared memory.

/// `struct panthor_fw_global_control_iface` (panthor_fw.h:193-202).
#[repr(C)]
pub struct GlbControl {
    pub version: u32,
    pub features: u32,
    pub input_va: u32,
    pub output_va: u32,
    pub group_num: u32,
    pub group_stride: u32,
    pub perfcnt_size: u32,
    pub instr_features: u32,
}

/// `struct panthor_fw_global_input_iface` (panthor_fw.h:204-253).
#[repr(C)]
pub struct GlbInput {
    pub req: u32,
    pub ack_irq_mask: u32,
    pub doorbell_req: u32,
    pub reserved1: u32,
    pub progress_timer: u32,
    pub poweroff_timer: u32,
    pub core_en_mask: u64,
    pub reserved2: u32,
    pub perfcnt_as: u32,
    pub perfcnt_base: u64,
    pub perfcnt_extract: u32,
    pub reserved3: [u32; 3],
    pub perfcnt_config: u32,
    pub perfcnt_csg_select: u32,
    pub perfcnt_fw_enable: u32,
    pub perfcnt_csg_enable: u32,
    pub perfcnt_csf_enable: u32,
    pub perfcnt_shader_enable: u32,
    pub perfcnt_tiler_enable: u32,
    pub perfcnt_mmu_l2_enable: u32,
    pub reserved4: [u32; 8],
    pub idle_timer: u32,
}

/// `struct panthor_fw_global_output_iface` (panthor_fw.h:261-269).
#[repr(C)]
pub struct GlbOutput {
    pub ack: u32,
    pub reserved1: u32,
    pub doorbell_ack: u32,
    pub reserved2: u32,
    pub halt_status: u32,
    pub perfcnt_status: u32,
    pub perfcnt_insert: u32,
}

/// GLB request bits (panthor_fw.h:205-227).
pub const GLB_HALT: u32 = 1 << 0;
pub const GLB_CFG_PROGRESS_TIMER: u32 = 1 << 1;
pub const GLB_CFG_ALLOC_EN: u32 = 1 << 2;
pub const GLB_CFG_POWEROFF_TIMER: u32 = 1 << 3;
pub const GLB_PROTM_ENTER: u32 = 1 << 4;
pub const GLB_PERFCNT_EN: u32 = 1 << 5;
pub const GLB_PERFCNT_SAMPLE: u32 = 1 << 6;
pub const GLB_COUNTER_EN: u32 = 1 << 7;
pub const GLB_PING: u32 = 1 << 8;
pub const GLB_FWCFG_UPDATE: u32 = 1 << 9;
pub const GLB_IDLE_EN: u32 = 1 << 10;
pub const GLB_SLEEP: u32 = 1 << 12;
pub const GLB_INACTIVE_COMPUTE: u32 = 1 << 20;
pub const GLB_INACTIVE_FRAGMENT: u32 = 1 << 21;
pub const GLB_INACTIVE_TILER: u32 = 1 << 22;
pub const GLB_PROTM_EXIT: u32 = 1 << 23;
pub const GLB_PERFCNT_THRESHOLD: u32 = 1 << 24;
pub const GLB_PERFCNT_OVERFLOW: u32 = 1 << 25;
pub const GLB_IDLE: u32 = 1 << 26;
pub const GLB_DBG_CSF: u32 = 1 << 30;
pub const GLB_DBG_HOST: u32 = 1 << 31;
/// `GLB_REQ_MASK` (bits 10:0, the toggles the host asks with) and
/// `GLB_EVT_MASK` (26:20, the firmware's events).
pub const GLB_REQ_MASK: u32 = 0x7ff;
pub const GLB_EVT_MASK: u32 = 0x07f0_0000;
/// `GLB_TIMER_VAL(x)`, `GLB_TIMER_SOURCE_GPU_COUNTER`.
pub const GLB_TIMER_VAL_MASK: u32 = 0x7fff_ffff;
pub const GLB_TIMER_SOURCE_GPU_COUNTER: u32 = 1 << 31;
/// `enum panthor_fw_halt_status`, the GLB output's `halt_status`.
pub const HALT_OK: u32 = 0;
pub const HALT_ON_PANIC: u32 = 0x4e;
pub const HALT_ON_WATCHDOG_EXPIRATION: u32 = 0x4f;

/// `struct panthor_fw_csg_control_iface` (panthor_fw.h:127-135).
#[repr(C)]
pub struct CsgControl {
    pub features: u32,
    pub input_va: u32,
    pub output_va: u32,
    pub suspend_size: u32,
    pub protm_suspend_size: u32,
    pub stream_num: u32,
    pub stream_stride: u32,
}

/// `struct panthor_fw_csg_input_iface` (panthor_fw.h:137-178).
#[repr(C)]
pub struct CsgInput {
    pub req: u32,
    pub ack_irq_mask: u32,
    pub doorbell_req: u32,
    pub cs_irq_ack: u32,
    pub reserved1: [u32; 4],
    pub allow_compute: u64,
    pub allow_fragment: u64,
    pub allow_other: u32,
    pub endpoint_req: u32,
    pub reserved2: [u32; 2],
    pub suspend_buf: u64,
    pub protm_suspend_buf: u64,
    pub config: u32,
    pub iter_trace_config: u32,
}

/// `struct panthor_fw_csg_output_iface` (panthor_fw.h:180-191).
#[repr(C)]
pub struct CsgOutput {
    pub ack: u32,
    pub reserved1: u32,
    pub doorbell_ack: u32,
    pub cs_irq_req: u32,
    pub status_endpoint_current: u32,
    pub status_endpoint_req: u32,
    pub status_state: u32,
    pub resource_dep: u32,
}

/// CSG request bits (panthor_fw.h:138-154) and `CSG_STATUS_STATE_IS_IDLE`.
pub const CSG_STATE_MASK: u32 = 0x7;
pub const CSG_STATE_TERMINATE: u32 = 0;
pub const CSG_STATE_START: u32 = 1;
pub const CSG_STATE_SUSPEND: u32 = 2;
pub const CSG_STATE_RESUME: u32 = 3;
pub const CSG_ENDPOINT_CONFIG: u32 = 1 << 4;
pub const CSG_STATUS_UPDATE: u32 = 1 << 5;
pub const CSG_SYNC_UPDATE: u32 = 1 << 28;
pub const CSG_IDLE: u32 = 1 << 29;
pub const CSG_DOORBELL: u32 = 1 << 30;
pub const CSG_PROGRESS_TIMER_EVENT: u32 = 1 << 31;
pub const CSG_REQ_MASK: u32 = CSG_STATE_MASK | CSG_ENDPOINT_CONFIG | CSG_STATUS_UPDATE;
pub const CSG_EVT_MASK: u32 = CSG_SYNC_UPDATE | CSG_IDLE | CSG_PROGRESS_TIMER_EVENT;
pub const CSG_STATUS_STATE_IS_IDLE: u32 = 1 << 0;
/// `CSG_EP_REQ_*` (panthor_fw.h:165-171): the endpoint request word.
pub const fn csg_ep_req(compute: u32, fragment: u32, tiler: u32, priority: u32) -> u32 {
    (compute & 0xff) | ((fragment << 8) & 0xff00) | ((tiler << 16) & 0xf_0000) | ((priority << 28) & 0xf000_0000)
}
pub const CSG_EP_REQ_EXCL_COMPUTE: u32 = 1 << 20;
pub const CSG_EP_REQ_EXCL_FRAGMENT: u32 = 1 << 21;

/// `struct panthor_fw_cs_control_iface` (panthor_fw.h:25-34).
#[repr(C)]
pub struct CsControl {
    pub features: u32,
    pub input_va: u32,
    pub output_va: u32,
}

/// `CS_FEATURES_WORK_REGS(x)`, `CS_FEATURES_SCOREBOARDS(x)` and the
/// stream kinds (panthor_fw.h:26-30).
pub const fn cs_features_work_regs(f: u32) -> u32 {
    (f & 0xff) + 1
}
pub const fn cs_features_scoreboards(f: u32) -> u32 {
    (f >> 8) & 0xff
}
pub const CS_FEATURES_COMPUTE: u32 = 1 << 16;
pub const CS_FEATURES_FRAGMENT: u32 = 1 << 17;
pub const CS_FEATURES_TILER: u32 = 1 << 18;

/// `struct panthor_fw_cs_input_iface` (panthor_fw.h:36-77).
#[repr(C)]
pub struct CsInput {
    pub req: u32,
    pub config: u32,
    pub reserved1: u32,
    pub ack_irq_mask: u32,
    pub ringbuf_base: u64,
    pub ringbuf_size: u32,
    pub reserved2: u32,
    pub heap_start: u64,
    pub heap_end: u64,
    pub ringbuf_input: u64,
    pub ringbuf_output: u64,
    pub instr_config: u32,
    pub instrbuf_size: u32,
    pub instrbuf_base: u64,
    pub instrbuf_offset_ptr: u64,
}

/// `struct panthor_fw_cs_output_iface` (panthor_fw.h:79-125).
#[repr(C)]
pub struct CsOutput {
    pub ack: u32,
    pub reserved1: [u32; 15],
    pub status_cmd_ptr: u64,
    pub status_wait: u32,
    pub status_req_resource: u32,
    pub status_wait_sync_ptr: u64,
    pub status_wait_sync_value: u32,
    pub status_scoreboards: u32,
    pub status_blocked_reason: u32,
    pub status_wait_sync_value_hi: u32,
    pub reserved2: [u32; 6],
    pub fault: u32,
    pub fatal: u32,
    pub fault_info: u64,
    pub fatal_info: u64,
    pub reserved3: [u32; 10],
    pub heap_vt_start: u32,
    pub heap_vt_end: u32,
    pub reserved4: u32,
    pub heap_frag_end: u32,
    pub heap_address: u64,
}

/// CS request bits (panthor_fw.h:37-58).
pub const CS_STATE_MASK: u32 = 0x7;
pub const CS_STATE_STOP: u32 = 0;
pub const CS_STATE_START: u32 = 1;
pub const CS_EXTRACT_EVENT: u32 = 1 << 4;
pub const CS_IDLE_SYNC_WAIT: u32 = 1 << 8;
pub const CS_IDLE_PROTM_PENDING: u32 = 1 << 9;
pub const CS_IDLE_EMPTY: u32 = 1 << 10;
pub const CS_IDLE_RESOURCE_REQ: u32 = 1 << 11;
pub const CS_TILER_OOM: u32 = 1 << 26;
pub const CS_PROTM_PENDING: u32 = 1 << 27;
pub const CS_FATAL: u32 = 1 << 30;
pub const CS_FAULT: u32 = 1 << 31;
pub const CS_REQ_MASK: u32 =
    CS_STATE_MASK | CS_EXTRACT_EVENT | CS_IDLE_SYNC_WAIT | CS_IDLE_PROTM_PENDING | CS_IDLE_EMPTY | CS_IDLE_RESOURCE_REQ;
pub const CS_EVT_MASK: u32 = CS_TILER_OOM | CS_PROTM_PENDING | CS_FATAL | CS_FAULT;
/// `CS_CONFIG_PRIORITY(x)`, `CS_CONFIG_DOORBELL(x)` (panthor_fw.h:61-62).
pub const fn cs_config(priority: u32, doorbell: u32) -> u32 {
    (priority & 0xf) | ((doorbell << 8) & 0xff00)
}
/// `CS_EXCEPTION_TYPE(x)`, `CS_EXCEPTION_DATA(x)` (panthor_fw.h:113-114):
/// the `fault` and `fatal` words.
pub const fn cs_exception_type(x: u32) -> u32 {
    x & 0xff
}
pub const fn cs_exception_data(x: u32) -> u32 {
    (x >> 8) & 0xff_ffff
}

/// Where the MCU's sections are for the CPU: AS0's side of things (Z2).
///
/// The shared section must be mapped uncached for the CPU, as panthor's
/// kernel buffer is on a non-coherent GPU (panthor_fw.h:282-285: "FW
/// interface sections are mapped uncached/write-combined"): the req/ack
/// words are read and written as they are, with no cache maintenance per
/// access.
pub trait McuMemory {
    /// The CPU address of the `len` bytes at MCU address `va` in AS0, or
    /// `None` when AS0 does not map all of them at one CPU range.
    fn cpu(&self, va: u32, len: u32) -> Option<usize>;
    /// Make what the CPU wrote to `[cpu, cpu + len)` visible to the GPU
    /// (panthor's `dma_sync_sgtable_for_device` after the copy,
    /// panthor_fw.c:622: a clean to the point of coherency; nothing for an
    /// uncached mapping).
    fn clean(&self, cpu: usize, len: usize);
}

/// Why a step stopped. The value crosses to C, one number per cause, with
/// the value that decided it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum CsfRefusal {
    /// AS0 does not map a section (the value: its MCU address): Z2's map
    /// is missing or short.
    SectionUnmapped = 1,
    /// `MCU_STATUS` was not `DISABLED` before the start (the value).
    McuNotDisabled = 2,
    /// `JOB_INT_RAWSTAT`'s global bit still reads set after its clear: a
    /// boot could not be told from an old event (the value: the raw word).
    BootIrqStuck = 3,
    /// `JOB_INT` bit 31 did not rise within [`BOOT_WAIT_US`] (the value:
    /// `MCU_STATUS`; panthor: "Failed to boot MCU").
    BootTimeout = 4,
    /// The GLB control block's version reads 0 (panthor: "Firmware
    /// version is 0. Firmware may have failed to boot").
    VersionZero = 5,
    /// An interface page or control block outside the shared section (the
    /// value: its MCU address; panthor: "Invalid ... input/output VA", and
    /// the offset bound of `panthor_init_csg_iface`).
    IfaceOutsideShared = 6,
    /// `group_num` outside [`MIN_CSGS`]..=[`MAX_CSGS`] (the value; panthor:
    /// "Invalid number of control groups").
    GroupCount = 7,
    /// A group's `stream_num` outside
    /// [`MIN_CS_PER_CSG`]..=[`MAX_CS_PER_CSG`] (the value).
    StreamCount = 8,
    /// The global configuration's toggles were not all acknowledged within
    /// [`GLB_ACK_WAIT_US`] (the value: the bits that were).
    GlbConfigAckTimeout = 9,
    /// The `PING` was not acknowledged within [`GLB_ACK_WAIT_US`] (panthor:
    /// "FW ping timeout, scheduling a reset").
    PingTimeout = 10,
    /// A group's control block differs from group 0's in features, suspend
    /// sizes or stream count (the value: the group; panthor: "Expecting
    /// identical CSG slots").
    CsgSlotsDiffer = 11,
    /// A stream's features differ from stream 0 of group 0's (the value:
    /// the group's index · 256 + the stream's; panthor: "Expecting
    /// identical CS slots").
    CsSlotsDiffer = 12,
}

type Refused = (CsfRefusal, u32);

fn write64(m: &impl Mmio, addr: usize, v: u64) {
    m.write32(addr, v as u32);
    m.write32(addr + 4, (v >> 32) as u32);
}

/// Copy one section to the CPU range AS0 maps it at and zero the rest when
/// the section says so (`panthor_fw_init_section_mem`); then clean it.
/// A section with no data and no zero flag is left as it is, as panthor
/// leaves it.
fn load_section(m: &impl Mmio, mem: &impl McuMemory, bytes: &[u8], s: &Section) -> Result<(), Refused> {
    if s.va_size() == 0 {
        return Ok(());
    }
    let cpu = mem.cpu(s.va_start, s.va_size()).ok_or((CsfRefusal::SectionUnmapped, s.va_start))?;
    let data = s.data(bytes);
    if data.is_empty() && !s.zeroed() {
        return Ok(());
    }
    let (words, rest) = data.as_chunks::<4>();
    let mut at = cpu;
    for w in words {
        m.write32(at, u32::from_le_bytes(*w));
        at += 4;
    }
    for &b in rest {
        m.write8(at, b);
        at += 1;
    }
    if s.zeroed() {
        let end = cpu + s.va_size() as usize;
        while !at.is_multiple_of(4) {
            m.write8(at, 0);
            at += 1;
        }
        while at < end {
            m.write32(at, 0);
            at += 4;
        }
    }
    mem.clean(cpu, s.va_size() as usize);
    Ok(())
}

/// Load every section of `img` (`panthor_fw_load`'s second half). The MCU
/// must not be running.
pub fn load_sections(m: &impl Mmio, mem: &impl McuMemory, img: &Image) -> Result<(), Refused> {
    for s in img.sections() {
        load_section(m, mem, img.bytes, s)?;
    }
    Ok(())
}

/// What [`start_mcu`] saw.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Start {
    pub mcu_status_before: u32,
    pub job_raw_before: u32,
    /// Polls (each followed by a 1 µs wait) before the global bit rose.
    pub waited_us: u32,
    /// `JOB_INT_RAWSTAT` when it rose, and `MCU_STATUS` then.
    pub job_raw: u32,
    pub mcu_status: u32,
}

/// `panthor_fw_start`: `MCU_CONTROL` = `AUTO`, then wait up to 1 s for the
/// global interface's interrupt, `JOB_INT` bit 31. panthor waits for the
/// interrupt with the job IRQ unmasked (`panthor_job_irq_resume(~0)`) and
/// its handler clears the status it saw (:1013); this polls
/// `JOB_INT_RAWSTAT` with the mask as found, and clears the bit before the
/// start (refusing when it does not clear, so the bit that rises is this
/// boot's) and after it. The status check before the start is ours.
pub fn start_mcu(m: &impl Mmio, gpu: usize, delay_us: &mut impl FnMut(u32)) -> Result<Start, Refused> {
    let mut out = Start { mcu_status_before: m.read32(gpu + MCU_STATUS), ..Start::default() };
    if out.mcu_status_before != MCU_STATUS_DISABLED {
        return Err((CsfRefusal::McuNotDisabled, out.mcu_status_before));
    }
    out.job_raw_before = m.read32(gpu + JOB_INT_RAWSTAT);
    m.write32(gpu + JOB_INT_CLEAR, JOB_INT_GLOBAL_IF);
    let cleared = m.read32(gpu + JOB_INT_RAWSTAT);
    if cleared & JOB_INT_GLOBAL_IF != 0 {
        return Err((CsfRefusal::BootIrqStuck, cleared));
    }
    m.write32(gpu + MCU_CONTROL, MCU_CONTROL_AUTO);
    for t in 0..=BOOT_WAIT_US {
        let raw = m.read32(gpu + JOB_INT_RAWSTAT);
        if raw & JOB_INT_GLOBAL_IF != 0 {
            m.write32(gpu + JOB_INT_CLEAR, JOB_INT_GLOBAL_IF);
            out.waited_us = t;
            out.job_raw = raw;
            out.mcu_status = m.read32(gpu + MCU_STATUS);
            return Ok(out);
        }
        delay_us(1);
    }
    Err((CsfRefusal::BootTimeout, m.read32(gpu + MCU_STATUS)))
}

/// The shared section as the CPU sees it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Shared {
    pub cpu: usize,
    pub va: u32,
    pub size: u32,
}

impl Shared {
    /// `iface_fw_to_cpu_addr`: the CPU address of an MCU address in the
    /// shared section, `None` outside it.
    pub fn at(&self, mcu_va: u32) -> Option<usize> {
        let off = mcu_va.checked_sub(self.va)?;
        (off < self.size).then_some(self.cpu + off as usize)
    }
    /// A control block of `len` bytes at offset `off`, whole inside the
    /// section (the offset bound of `panthor_init_csg_iface` and
    /// `panthor_init_cs_iface`).
    fn block(&self, off: u32, len: usize) -> Result<usize, Refused> {
        let va = self.va.wrapping_add(off);
        match off.checked_add(len as u32) {
            Some(end) if end <= self.size => Ok(self.cpu + off as usize),
            _ => Err((CsfRefusal::IfaceOutsideShared, va)),
        }
    }
}

/// The interface, found after boot: the GLB pages, and the place and
/// stride of the group and stream blocks (each checked by [`discover`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Iface {
    pub shared: Shared,
    pub version: u32,
    pub features: u32,
    /// `instr_features`, 0 for an interface before 1.1.0
    /// (`panthor_get_instr_features`).
    pub instr_features: u32,
    pub group_num: u32,
    pub group_stride: u32,
    /// CPU addresses of the GLB control, input and output blocks.
    pub glb_control: usize,
    pub glb_input: usize,
    pub glb_output: usize,
}

/// One command stream group's blocks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Csg {
    pub control: usize,
    pub input: usize,
    pub output: usize,
    pub features: u32,
    pub suspend_size: u32,
    pub protm_suspend_size: u32,
    pub stream_num: u32,
    pub stream_stride: u32,
}

impl Csg {
    /// `compare_csg`: what panthor requires equal across the groups.
    fn same_slot(&self, o: &Csg) -> bool {
        (self.features, self.suspend_size, self.protm_suspend_size, self.stream_num)
            == (o.features, o.suspend_size, o.protm_suspend_size, o.stream_num)
    }
}

/// One command stream's blocks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Cs {
    pub control: usize,
    pub input: usize,
    pub output: usize,
    pub features: u32,
}

impl Iface {
    /// Group `i` (`panthor_init_csg_iface`, its own checks: the block
    /// inside the section, the stream count, the pages).
    pub fn csg(&self, m: &impl Mmio, i: u32) -> Result<Csg, Refused> {
        let off = GROUP_CONTROL_OFFSET + i * self.group_stride;
        let control = self.shared.block(off, size_of::<CsgControl>())?;
        let rd = |f: usize| m.read32(control + f);
        let (input_va, output_va) = (rd(offset_of!(CsgControl, input_va)), rd(offset_of!(CsgControl, output_va)));
        let stream_num = rd(offset_of!(CsgControl, stream_num));
        if !(MIN_CS_PER_CSG..=MAX_CS_PER_CSG).contains(&stream_num) {
            return Err((CsfRefusal::StreamCount, stream_num));
        }
        Ok(Csg {
            control,
            input: self.shared.at(input_va).ok_or((CsfRefusal::IfaceOutsideShared, input_va))?,
            output: self.shared.at(output_va).ok_or((CsfRefusal::IfaceOutsideShared, output_va))?,
            features: rd(offset_of!(CsgControl, features)),
            suspend_size: rd(offset_of!(CsgControl, suspend_size)),
            protm_suspend_size: rd(offset_of!(CsgControl, protm_suspend_size)),
            stream_num,
            stream_stride: rd(offset_of!(CsgControl, stream_stride)),
        })
    }

    /// Stream `j` of group `csg` (`panthor_init_cs_iface`).
    pub fn cs(&self, m: &impl Mmio, csg: &Csg, j: u32) -> Result<Cs, Refused> {
        let off = (csg.control - self.shared.cpu) as u32 + STREAM_CONTROL_OFFSET + j * csg.stream_stride;
        let control = self.shared.block(off, size_of::<CsControl>())?;
        let rd = |f: usize| m.read32(control + f);
        let (input_va, output_va) = (rd(offset_of!(CsControl, input_va)), rd(offset_of!(CsControl, output_va)));
        Ok(Cs {
            control,
            input: self.shared.at(input_va).ok_or((CsfRefusal::IfaceOutsideShared, input_va))?,
            output: self.shared.at(output_va).ok_or((CsfRefusal::IfaceOutsideShared, output_va))?,
            features: rd(offset_of!(CsControl, features)),
        })
    }
}

/// `panthor_fw_init_ifaces`: read the GLB control block; a version of 0 is
/// a failed boot. Then every group and every stream of it, each block and
/// page required inside the shared section, every group equal to group 0
/// and every stream's features equal to stream 0's, as panthor requires.
pub fn discover(m: &impl Mmio, mem: &impl McuMemory, img: &Image) -> Result<Iface, Refused> {
    let s = img.shared_section();
    let cpu = mem.cpu(s.va_start, s.va_size()).ok_or((CsfRefusal::SectionUnmapped, s.va_start))?;
    let shared = Shared { cpu, va: s.va_start, size: s.va_size() };
    debug_assert_eq!(shared.va, MCU_SHARED_REGION_START);
    let rd = |f: usize| m.read32(cpu + f);
    let version = rd(offset_of!(GlbControl, version));
    if version == 0 {
        return Err((CsfRefusal::VersionZero, 0));
    }
    let (input_va, output_va) = (rd(offset_of!(GlbControl, input_va)), rd(offset_of!(GlbControl, output_va)));
    let iface = Iface {
        shared,
        version,
        features: rd(offset_of!(GlbControl, features)),
        instr_features: if version < iface_version(1, 1, 0) { 0 } else { rd(offset_of!(GlbControl, instr_features)) },
        group_num: rd(offset_of!(GlbControl, group_num)),
        group_stride: rd(offset_of!(GlbControl, group_stride)),
        glb_control: cpu,
        glb_input: shared.at(input_va).ok_or((CsfRefusal::IfaceOutsideShared, input_va))?,
        glb_output: shared.at(output_va).ok_or((CsfRefusal::IfaceOutsideShared, output_va))?,
    };
    if !(MIN_CSGS..=MAX_CSGS).contains(&iface.group_num) {
        return Err((CsfRefusal::GroupCount, iface.group_num));
    }
    let mut first_csg = None;
    let mut first_cs = None;
    for i in 0..iface.group_num {
        let csg = iface.csg(m, i)?;
        let first = *first_csg.get_or_insert(csg);
        if !csg.same_slot(&first) {
            return Err((CsfRefusal::CsgSlotsDiffer, i));
        }
        for j in 0..csg.stream_num {
            let cs = iface.cs(m, &csg, j)?;
            if cs.features != *first_cs.get_or_insert(cs.features) {
                return Err((CsfRefusal::CsSlotsDiffer, i << 8 | j));
            }
        }
    }
    Ok(iface)
}

/// `panthor_fw_toggle_reqs`: make the `mask` bits of the request word
/// differ from the acknowledge word, which asks the firmware for each of
/// them; the other bits keep their value.
pub fn toggle_reqs(m: &impl Mmio, req: usize, ack: usize, mask: u32) {
    let cur = m.read32(req);
    let ackv = m.read32(ack);
    m.write32(req, ((ackv ^ mask) & mask) | (cur & !mask));
}

/// `panthor_fw_update_reqs`: set the `mask` bits of the request word to
/// `val` (the level-type bits, like `GLB_IDLE_EN`).
pub fn update_reqs(m: &impl Mmio, req: usize, val: u32, mask: u32) {
    let cur = m.read32(req);
    m.write32(req, (cur & !mask) | (val & mask));
}

/// The `mask` bits the firmware has acknowledged: those where the request
/// and the acknowledge words agree (`panthor_fw_wait_acks`'s `*acked`).
pub fn acked(m: &impl Mmio, req: usize, ack: usize, mask: u32) -> u32 {
    !(m.read32(req) ^ m.read32(ack)) & mask
}

/// `panthor_fw_wait_acks`, polled: wait up to `budget_us` for every `mask`
/// bit to be acknowledged. The microseconds it took, or the bits that were.
/// panthor busy-waits 10 µs, then sleeps on the job interrupt; this polls
/// the words throughout, with no interrupt.
pub fn wait_acks(
    m: &impl Mmio,
    req: usize,
    ack: usize,
    mask: u32,
    budget_us: u32,
    delay_us: &mut impl FnMut(u32),
) -> Result<u32, u32> {
    for t in 0..=budget_us {
        if acked(m, req, ack, mask) == mask {
            return Ok(t);
        }
        delay_us(1);
    }
    Err(acked(m, req, ack, mask))
}

/// Ring doorbell `id`.
pub fn ring_doorbell(m: &impl Mmio, gpu: usize, id: u32) {
    m.write32(gpu + doorbell(id), 1);
}

/// `panthor_fw_ring_csg_doorbells`: toggle the groups' bits of the GLB
/// `doorbell_req` and ring the global doorbell.
pub fn ring_csg_doorbells(m: &impl Mmio, gpu: usize, iface: &Iface, csg_mask: u32) {
    toggle_reqs(
        m,
        iface.glb_input + offset_of!(GlbInput, doorbell_req),
        iface.glb_output + offset_of!(GlbOutput, doorbell_ack),
        csg_mask,
    );
    ring_doorbell(m, gpu, GLB_DOORBELL_ID);
}

/// `panthor_fw_conv_timeout`: `us` microseconds in ticks of the arch timer
/// at `timer_hz`, in units of 1024 ticks, rounded up. panthor falls back to
/// the GPU's cycle counter when there is no arch timer, and to the largest
/// value when it has no rate at all; the RK3588 has the arch timer, so only
/// the last case is kept, for a rate of 0.
pub const fn conv_timeout(us: u32, timer_hz: u32) -> u32 {
    if timer_hz == 0 {
        return GLB_TIMER_VAL_MASK | GLB_TIMER_SOURCE_GPU_COUNTER;
    }
    let div = 1_000_000u64 << 10;
    let ticks = (us as u64 * timer_hz as u64).div_ceil(div);
    if ticks > GLB_TIMER_VAL_MASK as u64 {
        GLB_TIMER_VAL_MASK
    } else {
        ticks as u32
    }
}

/// The global configuration's inputs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GlobalConfig {
    /// `core_en_mask`: the shader cores the firmware may power, all of
    /// `SHADER_PRESENT` (0x50005 on the G610 of the T6).
    pub shader_present: u64,
    /// The arch timer's rate (CNTFRQ_EL0): [`RK3588_TIMER_HZ`].
    pub timer_hz: u32,
}

/// What [`global_init`] wrote.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GlobalInit {
    pub core_en_mask: u64,
    pub poweroff_timer: u32,
    pub progress_timer: u32,
    pub idle_timer: u32,
    pub ack_irq_mask: u32,
    /// The request word after the update and the toggle.
    pub req: u32,
}

/// The toggles [`global_init`] asks for, which its caller waits on.
pub const GLB_INIT_TOGGLES: u32 = GLB_CFG_ALLOC_EN | GLB_CFG_POWEROFF_TIMER | GLB_CFG_PROGRESS_TIMER;

/// `panthor_fw_init_global_iface`: every core enabled, the three timers,
/// the acknowledge interrupts panthor wants, `GLB_IDLE_EN` set, the
/// configuration toggles asked for, and the global doorbell. On the T6 at
/// 24 MHz: power-off 235 (10 ms), progress 0x271000, idle 19 (800 µs),
/// `ack_irq_mask` 0x0400050e.
pub fn global_init(m: &impl Mmio, gpu: usize, iface: &Iface, cfg: &GlobalConfig) -> GlobalInit {
    let i = iface.glb_input;
    let o = iface.glb_output;
    let out = GlobalInit {
        core_en_mask: cfg.shader_present,
        poweroff_timer: conv_timeout(PWROFF_HYSTERESIS_US, cfg.timer_hz),
        progress_timer: (PROGRESS_TIMEOUT_CYCLES >> PROGRESS_TIMEOUT_SCALE_SHIFT) as u32,
        idle_timer: conv_timeout(IDLE_HYSTERESIS_US, cfg.timer_hz),
        ack_irq_mask: GLB_CFG_ALLOC_EN
            | GLB_PING
            | GLB_CFG_PROGRESS_TIMER
            | GLB_CFG_POWEROFF_TIMER
            | GLB_IDLE_EN
            | GLB_IDLE,
        req: 0,
    };
    write64(m, i + offset_of!(GlbInput, core_en_mask), out.core_en_mask);
    m.write32(i + offset_of!(GlbInput, poweroff_timer), out.poweroff_timer);
    m.write32(i + offset_of!(GlbInput, progress_timer), out.progress_timer);
    m.write32(i + offset_of!(GlbInput, idle_timer), out.idle_timer);
    m.write32(i + offset_of!(GlbInput, ack_irq_mask), out.ack_irq_mask);
    let req = i + offset_of!(GlbInput, req);
    update_reqs(m, req, GLB_IDLE_EN, GLB_IDLE_EN);
    toggle_reqs(m, req, o + offset_of!(GlbOutput, ack), GLB_INIT_TOGGLES);
    ring_doorbell(m, gpu, GLB_DOORBELL_ID);
    GlobalInit { req: m.read32(req), ..out }
}

/// `panthor_fw_ping_work` once: toggle `GLB_PING`, ring the global
/// doorbell, wait up to 100 ms for the acknowledge. The microseconds it
/// took. The kernel side repeats it every [`PING_INTERVAL_MS`].
pub fn ping(m: &impl Mmio, gpu: usize, iface: &Iface, delay_us: &mut impl FnMut(u32)) -> Result<u32, Refused> {
    let req = iface.glb_input + offset_of!(GlbInput, req);
    let ack = iface.glb_output + offset_of!(GlbOutput, ack);
    toggle_reqs(m, req, ack, GLB_PING);
    ring_doorbell(m, gpu, GLB_DOORBELL_ID);
    wait_acks(m, req, ack, GLB_PING, GLB_ACK_WAIT_US, delay_us).map_err(|a| (CsfRefusal::PingTimeout, a))
}

/// What [`boot`] saw: Z3's record.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Booted {
    pub start: Start,
    pub iface: Iface,
    /// Group 0 and its stream 0, the ones GL uses (the design page: one
    /// group, one stream).
    pub csg0: Csg,
    pub cs0: Cs,
    pub init: GlobalInit,
    /// Microseconds until the configuration toggles were acknowledged.
    pub init_ack_us: u32,
    /// Microseconds until the `PING` was acknowledged.
    pub ping_ack_us: u32,
}

/// The firmware brought up, in `panthor_fw_init`'s order from the load
/// on: load the sections; start the MCU; find the interface (version 0
/// refused); write the global configuration; then, which panthor does not
/// do at this point, wait for its toggles' acknowledge (so a record says
/// the firmware took the configuration) and send one `PING`.
///
/// Before it: the GPU powered with its L2 (Z1, [`super::l2_power_on`]),
/// and AS0 enabled with every section of `img` mapped (Z2), which
/// `mem` reports.
pub fn boot(
    m: &impl Mmio,
    mem: &impl McuMemory,
    gpu: usize,
    img: &Image,
    cfg: &GlobalConfig,
    delay_us: &mut impl FnMut(u32),
) -> Result<Booted, Refused> {
    load_sections(m, mem, img)?;
    let start = start_mcu(m, gpu, delay_us)?;
    let iface = discover(m, mem, img)?;
    let csg0 = iface.csg(m, 0)?;
    let cs0 = iface.cs(m, &csg0, 0)?;
    let init = global_init(m, gpu, &iface, cfg);
    let init_ack_us = wait_acks(
        m,
        iface.glb_input + offset_of!(GlbInput, req),
        iface.glb_output + offset_of!(GlbOutput, ack),
        GLB_INIT_TOGGLES,
        GLB_ACK_WAIT_US,
        delay_us,
    )
    .map_err(|a| (CsfRefusal::GlbConfigAckTimeout, a))?;
    let ping_ack_us = ping(m, gpu, &iface, delay_us)?;
    Ok(Booted { start, iface, csg0, cs0, init, init_ack_us, ping_ack_us })
}

// The layouts' sizes, as panthor's structures have them (the u64 members
// round the input blocks up to 8).
const _: () = assert!(size_of::<GlbControl>() == 0x20);
const _: () = assert!(size_of::<GlbInput>() == 0x88);
const _: () = assert!(size_of::<GlbOutput>() == 0x1c);
const _: () = assert!(size_of::<CsgControl>() == 0x1c);
const _: () = assert!(size_of::<CsgInput>() == 0x58);
const _: () = assert!(size_of::<CsgOutput>() == 0x20);
const _: () = assert!(size_of::<CsControl>() == 0xc);
const _: () = assert!(size_of::<CsInput>() == 0x58);
const _: () = assert!(size_of::<CsOutput>() == 0xd8);

#[cfg(test)]
mod tests {
    use super::super::fw::{self, synth};
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;

    const GPU: usize = 0xfb00_0000;
    /// The mock maps AS0 at one CPU window: MCU address `va` is CPU
    /// address `MEM + va`.
    const MEM: usize = 0x1_0000_0000;

    #[test]
    fn the_layouts_are_panthors() {
        // kbase's GLB_* register offsets, which panthor's structures lay out
        assert_eq!(
            [
                offset_of!(GlbControl, group_num),
                offset_of!(GlbControl, group_stride),
                offset_of!(GlbControl, instr_features)
            ],
            [0x10, 0x14, 0x1c]
        );
        assert_eq!(
            [
                offset_of!(GlbInput, ack_irq_mask),
                offset_of!(GlbInput, doorbell_req),
                offset_of!(GlbInput, progress_timer),
                offset_of!(GlbInput, poweroff_timer),
                offset_of!(GlbInput, core_en_mask),
                offset_of!(GlbInput, perfcnt_as),
                offset_of!(GlbInput, perfcnt_base),
                offset_of!(GlbInput, perfcnt_extract),
                offset_of!(GlbInput, perfcnt_config),
                offset_of!(GlbInput, perfcnt_fw_enable),
                offset_of!(GlbInput, perfcnt_mmu_l2_enable),
                offset_of!(GlbInput, idle_timer),
            ],
            [0x4, 0x8, 0x10, 0x14, 0x18, 0x24, 0x28, 0x30, 0x40, 0x48, 0x5c, 0x80]
        );
        assert_eq!(
            [
                offset_of!(GlbOutput, doorbell_ack),
                offset_of!(GlbOutput, halt_status),
                offset_of!(GlbOutput, perfcnt_insert)
            ],
            [0x8, 0x10, 0x18]
        );
        assert_eq!(
            [
                offset_of!(CsgInput, cs_irq_ack),
                offset_of!(CsgInput, allow_compute),
                offset_of!(CsgInput, allow_fragment),
                offset_of!(CsgInput, allow_other),
                offset_of!(CsgInput, endpoint_req),
                offset_of!(CsgInput, suspend_buf),
                offset_of!(CsgInput, protm_suspend_buf),
                offset_of!(CsgInput, config),
                offset_of!(CsgInput, iter_trace_config),
            ],
            [0xc, 0x20, 0x28, 0x30, 0x34, 0x40, 0x48, 0x50, 0x54]
        );
        assert_eq!(
            [
                offset_of!(CsgOutput, cs_irq_req),
                offset_of!(CsgOutput, status_state),
                offset_of!(CsgOutput, resource_dep)
            ],
            [0xc, 0x18, 0x1c]
        );
        assert_eq!(
            [
                offset_of!(CsInput, config),
                offset_of!(CsInput, ack_irq_mask),
                offset_of!(CsInput, ringbuf_base),
                offset_of!(CsInput, ringbuf_size),
                offset_of!(CsInput, heap_start),
                offset_of!(CsInput, heap_end),
                offset_of!(CsInput, ringbuf_input),
                offset_of!(CsInput, ringbuf_output),
                offset_of!(CsInput, instr_config),
                offset_of!(CsInput, instrbuf_base),
                offset_of!(CsInput, instrbuf_offset_ptr),
            ],
            [0x4, 0xc, 0x10, 0x18, 0x20, 0x28, 0x30, 0x38, 0x40, 0x48, 0x50]
        );
        assert_eq!(
            [
                offset_of!(CsOutput, status_cmd_ptr),
                offset_of!(CsOutput, status_wait),
                offset_of!(CsOutput, status_wait_sync_ptr),
                offset_of!(CsOutput, status_scoreboards),
                offset_of!(CsOutput, status_blocked_reason),
                offset_of!(CsOutput, fault),
                offset_of!(CsOutput, fatal),
                offset_of!(CsOutput, fault_info),
                offset_of!(CsOutput, fatal_info),
                offset_of!(CsOutput, heap_vt_start),
                offset_of!(CsOutput, heap_frag_end),
                offset_of!(CsOutput, heap_address),
            ],
            [0x40, 0x48, 0x50, 0x5c, 0x60, 0x80, 0x84, 0x88, 0x90, 0xc0, 0xcc, 0xd0]
        );
        assert_eq!([doorbell(0), doorbell(1), doorbell(8)], [0x8_0000, 0x9_0000, 0x10_0000]);
        assert_eq!(iface_version(1, 5, 0), 0x0105_0000);
        assert_eq!((version_major(0x0105_0000), version_minor(0x0105_0000), version_patch(0x0105_0000)), (1, 5, 0));
        assert_eq!(cs_config(0xf, 1), 0x10f);
        assert_eq!(csg_ep_req(8, 8, 1, 3), 0x3001_0808);
        assert_eq!((cs_exception_type(0x1234_5678), cs_exception_data(0x1234_5678)), (0x78, 0x12_3456));
        assert_eq!(GLB_REQ_MASK, 0x7ff);
        assert_eq!(GLB_EVT_MASK, (1 << 27) - (1 << 20));
    }

    #[test]
    fn the_timers_are_24_mhz_ticks_in_units_of_1024() {
        // 10 ms: 240000 ticks / 1024 = 234.4, rounded up
        assert_eq!(conv_timeout(PWROFF_HYSTERESIS_US, RK3588_TIMER_HZ), 235);
        // 800 us: 19200 / 1024 = 18.75
        assert_eq!(conv_timeout(IDLE_HYSTERESIS_US, RK3588_TIMER_HZ), 19);
        assert_eq!(conv_timeout(1024, 1_000_000), 1);
        assert_eq!(conv_timeout(1025, 1_000_000), 2);
        assert_eq!(conv_timeout(u32::MAX, u32::MAX), GLB_TIMER_VAL_MASK);
        assert_eq!(conv_timeout(1, 0), 0xffff_ffff);
        assert_eq!((PROGRESS_TIMEOUT_CYCLES >> PROGRESS_TIMEOUT_SCALE_SHIFT) as u32, 0x27_1000);
    }

    #[test]
    fn a_toggle_flips_only_its_bits_against_the_ack() {
        let g = Mcu::new(synth::small());
        let (req, ack) = (MEM + 0x10, MEM + 0x20);
        g.mem.borrow_mut().insert(req, 0x0000_0401);
        g.mem.borrow_mut().insert(ack, 0x0000_0101);
        toggle_reqs(&g, req, ack, GLB_PING | GLB_CFG_ALLOC_EN);
        // PING: ack 1 -> req 0; ALLOC_EN: ack 0 -> req 1; IDLE_EN and HALT kept
        assert_eq!(g.word(req), 0x0000_0405);
        assert_eq!(acked(&g, req, ack, GLB_PING | GLB_CFG_ALLOC_EN | GLB_HALT), GLB_HALT);
        update_reqs(&g, req, 0, GLB_IDLE_EN);
        assert_eq!(g.word(req), 0x0000_0005);
        // asked twice before an acknowledge: the request stays asked once
        toggle_reqs(&g, req, ack, GLB_PING);
        assert_eq!(g.word(req) & GLB_PING, 0);
        // the wait reports what was acknowledged when it gives up
        assert_eq!(wait_acks(&g, req, ack, GLB_PING | GLB_HALT, 2, &mut |_| {}), Err(GLB_HALT));
    }

    /// A model of the G610's MCU: boots `lag` polls after `MCU_CONTROL` =
    /// `AUTO` if the code section was loaded and cleaned, then writes what
    /// the board's firmware is predicted to write (version 1.5.0, the
    /// streams' features); acknowledges a global request when the global
    /// doorbell rings, unless `deaf` says which bits it never acknowledges.
    struct Mcu {
        regs: RefCell<HashMap<usize, u32>>,
        mem: RefCell<HashMap<usize, u32>>,
        writes: RefCell<Vec<(usize, u32)>>,
        cleaned: RefCell<Vec<(usize, usize)>>,
        image: Vec<u8>,
        /// MCU addresses AS0 does not map.
        unmapped: Option<u32>,
        lag: u32,
        polls: RefCell<u32>,
        starting: RefCell<bool>,
        never_boots: bool,
        version: u32,
        deaf: u32,
        group_num: u32,
    }

    impl Mcu {
        fn new(image: Vec<u8>) -> Mcu {
            Mcu {
                regs: RefCell::new(HashMap::new()),
                mem: RefCell::new(HashMap::new()),
                writes: RefCell::new(vec![]),
                cleaned: RefCell::new(vec![]),
                image,
                unmapped: None,
                lag: 3,
                polls: RefCell::new(0),
                starting: RefCell::new(false),
                never_boots: false,
                version: iface_version(1, 5, 0),
                deaf: 0,
                group_num: 8,
            }
        }
        fn word(&self, a: usize) -> u32 {
            *self.mem.borrow().get(&a).unwrap_or(&0)
        }
        fn put(&self, va: u32, v: u32) {
            self.mem.borrow_mut().insert(MEM + va as usize, v);
        }
        /// The firmware's boot: the code it runs must be in memory and
        /// cleaned, the shared section's control block as loaded (the
        /// pages and strides stay the image's; the firmware writes the
        /// version and the stream features).
        fn boot(&self) -> bool {
            let img = fw::parse(&self.image).unwrap();
            let code = img.sections().iter().find(|s| s.executable()).unwrap();
            let d = code.data(&self.image);
            let first = u32::from_le_bytes([d[0], d[1], d[2], d[3]]);
            let cpu = MEM + code.va_start as usize;
            if self.never_boots
                || self.word(cpu) != first
                || !self.cleaned.borrow().contains(&(cpu, code.va_size() as usize))
            {
                return false;
            }
            let base = MCU_SHARED_REGION_START;
            self.put(base, self.version);
            self.put(base + 16, self.group_num);
            for i in 0..self.group_num {
                let csg = base + GROUP_CONTROL_OFFSET + i * 0xa0;
                self.put(csg + 4, base + 0x3000 + i * 0x80);
                self.put(csg + 8, base + 0x7000 + i * 0x40);
                self.put(csg + 20, 8);
                self.put(csg + 24, 0xc);
                for j in 0..8 {
                    let cs = csg + STREAM_CONTROL_OFFSET + j * 0xc;
                    // 96 work registers, 8 scoreboards, every kind
                    self.put(cs, 95 | 8 << 8 | CS_FEATURES_COMPUTE | CS_FEATURES_FRAGMENT | CS_FEATURES_TILER);
                    self.put(cs + 4, base + 0x3400 + i * 0x400 + j * 0x80);
                    self.put(cs + 8, base + 0x7200 + i * 0x800 + j * 0x100);
                }
            }
            true
        }
    }

    impl McuMemory for Mcu {
        fn cpu(&self, va: u32, len: u32) -> Option<usize> {
            match self.unmapped {
                Some(u) if u >= va && u < va + len => None,
                _ => Some(MEM + va as usize),
            }
        }
        fn clean(&self, cpu: usize, len: usize) {
            self.cleaned.borrow_mut().push((cpu, len));
        }
    }

    impl Mmio for Mcu {
        fn read32(&self, addr: usize) -> u32 {
            if addr >= MEM {
                return self.word(addr);
            }
            let off = addr - GPU;
            if off == JOB_INT_RAWSTAT && *self.starting.borrow() {
                let mut p = self.polls.borrow_mut();
                *p += 1;
                if *p > self.lag {
                    *self.starting.borrow_mut() = false;
                    if self.boot() {
                        *self.regs.borrow_mut().entry(JOB_INT_RAWSTAT).or_insert(0) |= JOB_INT_GLOBAL_IF;
                        self.regs.borrow_mut().insert(MCU_STATUS, MCU_STATUS_ENABLED);
                    }
                }
            }
            *self.regs.borrow().get(&off).unwrap_or(&0)
        }
        fn write32(&self, addr: usize, val: u32) {
            if addr >= MEM {
                self.mem.borrow_mut().insert(addr, val);
                return;
            }
            let off = addr - GPU;
            self.writes.borrow_mut().push((off, val));
            match off {
                JOB_INT_CLEAR => *self.regs.borrow_mut().entry(JOB_INT_RAWSTAT).or_insert(0) &= !val,
                MCU_CONTROL if val == MCU_CONTROL_AUTO => *self.starting.borrow_mut() = true,
                o if o == doorbell(GLB_DOORBELL_ID) => {
                    let (i, o) = (MEM + 0x0400_2000, MEM + 0x0400_6000);
                    let req = self.word(i);
                    let ack = self.word(o);
                    let mask = GLB_REQ_MASK & !self.deaf;
                    self.mem.borrow_mut().insert(o, (ack & !mask) | (req & mask));
                }
                _ => {
                    self.regs.borrow_mut().insert(off, val);
                }
            }
        }
        fn read8(&self, _: usize) -> u8 {
            unreachable!()
        }
        fn write8(&self, addr: usize, val: u8) {
            let a = addr & !3;
            let sh = (addr & 3) * 8;
            let w = (self.word(a) & !(0xff << sh)) | u32::from(val) << sh;
            self.mem.borrow_mut().insert(a, w);
        }
    }

    const T6: GlobalConfig = GlobalConfig { shader_present: 0x50005, timer_hz: RK3588_TIMER_HZ };

    #[test]
    fn the_sections_land_where_as0_maps_them_zeroed_and_cleaned() {
        let b = synth::small();
        let img = fw::parse(&b).unwrap();
        let g = Mcu::new(b.clone());
        // stale words where zeroes must go
        g.put(0x200_0000 + 0x100, 0xdead_beef);
        g.put(0x400_0000 + 0x3000, 0xdead_beef);
        load_sections(&g, &g, &img).unwrap();
        for s in img.sections() {
            let d = s.data(&b);
            for (k, w) in d.chunks(4).enumerate() {
                let mut v = [0u8; 4];
                v[..w.len()].copy_from_slice(w);
                assert_eq!(g.word(MEM + s.va_start as usize + 4 * k), u32::from_le_bytes(v), "{s:x?} word {k}");
            }
        }
        assert_eq!(g.word(MEM + 0x200_0100), 0);
        assert_eq!(g.word(MEM + 0x400_3000), 0);
        // the shared section's GLB control block as the image has it
        assert_eq!([8, 12, 16, 20].map(|o| g.word(MEM + 0x400_0000 + o)), [0x0400_2000, 0x0400_6000, 8, 0xa0]);
        let cleaned: Vec<(usize, usize)> =
            img.sections().iter().map(|s| (MEM + s.va_start as usize, s.va_size() as usize)).collect();
        assert_eq!(*g.cleaned.borrow(), cleaned);
        // the protected section is not loaded
        assert!(g.mem.borrow().keys().all(|&a| !(MEM + 0x300_0000..MEM + 0x304_0000).contains(&a)));
        assert!(g.writes.borrow().is_empty(), "no GPU register is written by the load");
    }

    #[test]
    fn a_section_as0_does_not_map_stops_the_load() {
        let b = synth::small();
        let img = fw::parse(&b).unwrap();
        let mut g = Mcu::new(b.clone());
        g.unmapped = Some(MCU_SHARED_REGION_START + 0x8000);
        assert_eq!(load_sections(&g, &g, &img), Err((CsfRefusal::SectionUnmapped, MCU_SHARED_REGION_START)));
    }

    /// The whole of Z3 over the model: GLB version 1.5.0 and the ping
    /// acknowledged, with every GPU register write in order.
    #[test]
    fn the_boot_reads_version_1_5_0_and_the_ping_is_acknowledged() {
        let b = synth::small();
        let img = fw::parse(&b).unwrap();
        let g = Mcu::new(b.clone());
        let mut slept = 0u32;
        let r = boot(&g, &g, GPU, &img, &T6, &mut |us| slept += us).unwrap();
        assert_eq!(
            r.start,
            Start { mcu_status_before: 0, job_raw_before: 0, waited_us: 3, job_raw: JOB_INT_GLOBAL_IF, mcu_status: 1 }
        );
        let v = r.iface.version;
        assert_eq!((version_major(v), version_minor(v), version_patch(v)), (1, 5, 0));
        assert_eq!((r.iface.features, r.iface.instr_features), (0, 0x71));
        assert_eq!((r.iface.group_num, r.iface.group_stride), (8, 0xa0));
        assert_eq!((r.iface.glb_input, r.iface.glb_output), (MEM + 0x400_2000, MEM + 0x400_6000));
        assert_eq!((r.csg0.stream_num, r.csg0.stream_stride, r.csg0.input), (8, 0xc, MEM + 0x400_3000));
        assert_eq!((cs_features_work_regs(r.cs0.features), cs_features_scoreboards(r.cs0.features)), (96, 8));
        assert_eq!(cs_features_work_regs(r.cs0.features) - UNPRESERVED_REG_COUNT, 92);
        assert_eq!(r.cs0.control, MEM + 0x400_1040);
        assert_eq!(
            r.init,
            GlobalInit {
                core_en_mask: 0x50005,
                poweroff_timer: 235,
                progress_timer: 0x27_1000,
                idle_timer: 19,
                ack_irq_mask: 0x0400_050e,
                req: GLB_IDLE_EN | GLB_INIT_TOGGLES,
            }
        );
        assert_eq!((r.init_ack_us, r.ping_ack_us), (0, 0));
        let i = MEM + 0x400_2000;
        assert_eq!(
            [0x18, 0x1c, 0x10, 0x14, 0x80, 0x4].map(|o| g.word(i + o)),
            [0x50005, 0, 0x27_1000, 235, 19, 0x0400_050e]
        );
        // after the ping: PING asked and acknowledged, the rest as left
        assert_eq!(g.word(i), GLB_IDLE_EN | GLB_INIT_TOGGLES | GLB_PING);
        assert_eq!(g.word(MEM + 0x400_6000), GLB_IDLE_EN | GLB_INIT_TOGGLES | GLB_PING);
        assert_eq!(
            *g.writes.borrow(),
            vec![
                (JOB_INT_CLEAR, JOB_INT_GLOBAL_IF),
                (MCU_CONTROL, MCU_CONTROL_AUTO),
                (JOB_INT_CLEAR, JOB_INT_GLOBAL_IF),
                (doorbell(0), 1),
                (doorbell(0), 1),
            ]
        );
        assert_eq!(slept, 3);
    }

    #[test]
    fn an_mcu_that_never_answers_times_out_after_1_s_with_its_status() {
        let b = synth::small();
        let img = fw::parse(&b).unwrap();
        let mut g = Mcu::new(b.clone());
        g.never_boots = true;
        g.regs.borrow_mut().insert(MCU_STATUS, 0);
        let mut slept = 0u64;
        assert_eq!(
            boot(&g, &g, GPU, &img, &T6, &mut |us| slept += u64::from(us)),
            Err((CsfRefusal::BootTimeout, MCU_STATUS_DISABLED))
        );
        assert_eq!(slept, u64::from(BOOT_WAIT_US) + 1);
    }

    #[test]
    fn code_not_loaded_is_no_boot() {
        // the model boots only from the loaded, cleaned code: starting the
        // MCU without the load times out
        let b = synth::small();
        let g = Mcu::new(b.clone());
        assert_eq!(start_mcu(&g, GPU, &mut |_| {}).unwrap_err().0, CsfRefusal::BootTimeout);
    }

    #[test]
    fn a_running_mcu_or_a_stuck_interrupt_is_refused_before_the_start() {
        let b = synth::small();
        let img = fw::parse(&b).unwrap();
        let g = Mcu::new(b.clone());
        g.regs.borrow_mut().insert(MCU_STATUS, MCU_STATUS_HALT);
        assert_eq!(boot(&g, &g, GPU, &img, &T6, &mut |_| {}), Err((CsfRefusal::McuNotDisabled, MCU_STATUS_HALT)));
        assert!(g.writes.borrow().is_empty());

        struct Sticky(Mcu);
        impl Mmio for Sticky {
            fn read32(&self, a: usize) -> u32 {
                if a == GPU + JOB_INT_RAWSTAT {
                    return JOB_INT_GLOBAL_IF;
                }
                self.0.read32(a)
            }
            fn write32(&self, a: usize, v: u32) {
                self.0.write32(a, v)
            }
            fn read8(&self, a: usize) -> u8 {
                self.0.read8(a)
            }
            fn write8(&self, a: usize, v: u8) {
                self.0.write8(a, v)
            }
        }
        let s = Sticky(Mcu::new(b));
        assert_eq!(start_mcu(&s, GPU, &mut |_| {}), Err((CsfRefusal::BootIrqStuck, JOB_INT_GLOBAL_IF)));
        assert!(s.0.writes.borrow().iter().all(|w| w.0 != MCU_CONTROL));
    }

    #[test]
    fn a_version_of_0_is_a_failed_boot() {
        let b = synth::small();
        let img = fw::parse(&b).unwrap();
        let mut g = Mcu::new(b.clone());
        g.version = 0;
        assert_eq!(boot(&g, &g, GPU, &img, &T6, &mut |_| {}), Err((CsfRefusal::VersionZero, 0)));
        // nothing is written into the interface after it
        assert_eq!(g.word(MEM + 0x400_2000), 0);
    }

    #[test]
    fn an_interface_before_1_1_0_has_no_instr_features() {
        let b = synth::small();
        let img = fw::parse(&b).unwrap();
        let mut g = Mcu::new(b.clone());
        g.version = iface_version(1, 0, 3);
        let r = boot(&g, &g, GPU, &img, &T6, &mut |_| {}).unwrap();
        assert_eq!((r.iface.version, r.iface.instr_features), (0x0100_0003, 0));
    }

    #[test]
    fn bad_counts_blocks_and_pages_outside_the_shared_section_are_refused() {
        let b = synth::small();
        let img = fw::parse(&b).unwrap();
        let mut g = Mcu::new(b.clone());
        g.group_num = 2;
        assert_eq!(boot(&g, &g, GPU, &img, &T6, &mut |_| {}), Err((CsfRefusal::GroupCount, 2)));
        let mut g = Mcu::new(b.clone());
        g.group_num = 32;
        assert_eq!(boot(&g, &g, GPU, &img, &T6, &mut |_| {}), Err((CsfRefusal::GroupCount, 32)));

        let g = Mcu::new(b.clone());
        load_sections(&g, &g, &img).unwrap();
        start_mcu(&g, GPU, &mut |_| {}).unwrap();
        g.put(MCU_SHARED_REGION_START + 12, 0x0401_0000);
        assert_eq!(discover(&g, &g, &img), Err((CsfRefusal::IfaceOutsideShared, 0x0401_0000)));
        g.put(MCU_SHARED_REGION_START + 12, 0x0400_6000);
        // group 7's stream 7 output page past the section
        g.put(MCU_SHARED_REGION_START + 0x1000 + 7 * 0xa0 + 0x40 + 7 * 0xc + 8, 0x0300_0000);
        assert_eq!(discover(&g, &g, &img), Err((CsfRefusal::IfaceOutsideShared, 0x0300_0000)));
        g.put(MCU_SHARED_REGION_START + 0x1000 + 7 * 0xa0 + 0x40 + 7 * 0xc + 8, 0x0400_7200 + 7 * 0x800 + 7 * 0x100);
        // a control block must lie whole inside the section
        let sh = Shared { cpu: MEM, va: MCU_SHARED_REGION_START, size: 0x1_0000 };
        assert_eq!(sh.block(0xffe4, size_of::<CsgControl>()), Ok(MEM + 0xffe4));
        assert_eq!(sh.block(0xffe8, size_of::<CsgControl>()), Err((CsfRefusal::IfaceOutsideShared, 0x0400_ffe8)));
        assert_eq!(sh.block(0xffff_fff0, size_of::<CsControl>()), Err((CsfRefusal::IfaceOutsideShared, 0x03ff_fff0)));
        g.put(MCU_SHARED_REGION_START + 0x1000 + 3 * 0xa0 + 20, 33);
        assert_eq!(discover(&g, &g, &img), Err((CsfRefusal::StreamCount, 33)));
        g.put(MCU_SHARED_REGION_START + 0x1000 + 3 * 0xa0 + 20, 7);
        assert_eq!(discover(&g, &g, &img), Err((CsfRefusal::StreamCount, 7)));
    }

    #[test]
    fn groups_and_streams_that_differ_from_the_first_are_refused() {
        let b = synth::small();
        let img = fw::parse(&b).unwrap();
        let g = Mcu::new(b.clone());
        load_sections(&g, &g, &img).unwrap();
        start_mcu(&g, GPU, &mut |_| {}).unwrap();
        discover(&g, &g, &img).unwrap();
        // group 5's suspend size
        g.put(MCU_SHARED_REGION_START + 0x1000 + 5 * 0xa0 + 12, 28672);
        assert_eq!(discover(&g, &g, &img), Err((CsfRefusal::CsgSlotsDiffer, 5)));
        g.put(MCU_SHARED_REGION_START + 0x1000 + 5 * 0xa0 + 12, 0);
        // group 2's stream 6 features
        g.put(MCU_SHARED_REGION_START + 0x1000 + 2 * 0xa0 + 0x40 + 6 * 0xc, 63 | 8 << 8);
        assert_eq!(discover(&g, &g, &img), Err((CsfRefusal::CsSlotsDiffer, 2 << 8 | 6)));
    }

    #[test]
    fn unacknowledged_requests_time_out_after_100_ms() {
        let b = synth::small();
        let img = fw::parse(&b).unwrap();
        let mut g = Mcu::new(b.clone());
        g.deaf = GLB_CFG_POWEROFF_TIMER;
        let mut slept = 0u32;
        assert_eq!(
            boot(&g, &g, GPU, &img, &T6, &mut |us| slept += us),
            Err((CsfRefusal::GlbConfigAckTimeout, GLB_CFG_ALLOC_EN | GLB_CFG_PROGRESS_TIMER))
        );
        assert_eq!(slept, 3 + GLB_ACK_WAIT_US + 1);

        let mut g = Mcu::new(b.clone());
        g.deaf = GLB_PING;
        assert_eq!(boot(&g, &g, GPU, &img, &T6, &mut |_| {}), Err((CsfRefusal::PingTimeout, 0)));
    }

    #[test]
    fn a_csg_doorbell_toggles_doorbell_req_and_rings_the_global_one() {
        let b = synth::small();
        let img = fw::parse(&b).unwrap();
        let g = Mcu::new(b.clone());
        let r = boot(&g, &g, GPU, &img, &T6, &mut |_| {}).unwrap();
        g.put(0x0400_6000 + 8, 0b10);
        ring_csg_doorbells(&g, GPU, &r.iface, 0b11);
        assert_eq!(g.word(MEM + 0x0400_2000 + 8), 0b01);
        assert_eq!(*g.writes.borrow().last().unwrap(), (doorbell(0), 1));
    }

    /// The board's file through the boot over the model (`MALI_CSFFW`; run
    /// by tools/check.sh with it, or a gap).
    #[test]
    #[ignore = "needs MALI_CSFFW, the board's mali_csffw.bin (tools/check.sh runs it, or reports a gap)"]
    fn the_boards_file_boots_over_the_model() {
        let path = std::env::var("MALI_CSFFW").expect("MALI_CSFFW: the path of the board's mali_csffw.bin");
        let b = std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
        let img = fw::parse(&b).unwrap();
        let g = Mcu::new(b.clone());
        let r = boot(&g, &g, GPU, &img, &T6, &mut |_| {}).unwrap();
        assert_eq!(r.iface.version, iface_version(1, 5, 0));
        assert_eq!(r.iface.version, img.header.version_hash, "the header's interface version is what boots");
        // the image's own words, which the model does not write
        assert_eq!((r.iface.features, r.iface.instr_features), (0, 0x71));
        assert_eq!((r.iface.glb_input, r.iface.glb_output), (MEM + 0x400_2000, MEM + 0x400_6000));
        assert_eq!((r.csg0.input, r.csg0.output), (MEM + 0x400_3000, MEM + 0x400_7000));
        assert_eq!((r.cs0.input, r.cs0.output), (MEM + 0x400_3400, MEM + 0x400_7200));
        assert_eq!(g.word(MEM + 0x400_6000) & GLB_PING, GLB_PING);
        // the zeroed tail of every section past its data reads 0 (the shared
        // one's is the GLB input page, written since)
        for s in img.sections().iter().filter(|s| s.zeroed() && !s.shared()) {
            let tail = (s.va_start + s.data_size()).next_multiple_of(4);
            assert_eq!(g.word(MEM + tail as usize), 0, "{s:x?}");
        }
    }
}
