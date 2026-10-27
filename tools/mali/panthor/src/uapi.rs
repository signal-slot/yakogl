// SPDX-License-Identifier: Apache-2.0 AND MIT
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// SPDX-FileCopyrightText: 2023 Collabora Ltd.
// SPDX-FileCopyrightText: 1999 Precision Insight, Inc.
// SPDX-FileCopyrightText: 2000 VA Linux Systems, Inc.
// Transcribed from Linux v6.18 include/uapi/drm/panthor_drm.h (MIT) and, for
// the DRM core ioctls, Mesa 26.2.1 include/drm-uapi/drm.h (MIT). The upstream
// part stays under its licence (LICENSES/MIT.txt); our changes are Apache-2.0.
//! The panthor uAPI: the structures and ioctl numbers this crate uses.
//!
//! Each struct names its line in `panthor_drm.h` (Linux v6.18). The sizes and
//! the ioctl numbers are pinned by the tests below against the values a C
//! compiler gives for the same headers (aarch64 and x86_64 agree: both LP64).
//! Only what the crate calls is here; `BO_SET_LABEL` and
//! `SET_USER_MMIO_OFFSET` are left out (nothing uses them: the submit passes
//! `latest_flush = 0`, which the header defines as "flush unconditionally",
//! panthor_drm.h:856-867).

/// `_IOC` for the DRM ioctls (`'d'`), as `asm-generic/ioctl.h` packs it:
/// direction in bits 31:30, size in 29:16, type in 15:8, number in 7:0.
const fn ioc(dir: u32, nr: u32, size: usize) -> u32 {
    assert!(size < (1 << 14));
    (dir << 30) | ((size as u32) << 16) | ((b'd' as u32) << 8) | nr
}
const IOC_WRITE: u32 = 1;
const IOC_READWRITE: u32 = 3;
const fn iowr<T>(nr: u32) -> u32 {
    ioc(IOC_READWRITE, nr, core::mem::size_of::<T>())
}
const fn iow<T>(nr: u32) -> u32 {
    ioc(IOC_WRITE, nr, core::mem::size_of::<T>())
}

/// drm.h:1297.
pub const DRM_COMMAND_BASE: u32 = 0x40;

// ---- DRM core (drm.h) ----

/// `struct drm_version` (drm.h:137).
#[repr(C)]
#[derive(Debug)]
pub struct DrmVersion {
    pub version_major: i32,
    pub version_minor: i32,
    pub version_patchlevel: i32,
    pub name_len: usize,
    pub name: *mut u8,
    pub date_len: usize,
    pub date: *mut u8,
    pub desc_len: usize,
    pub desc: *mut u8,
}

/// `struct drm_gem_close` (drm.h:599).
#[repr(C)]
#[derive(Debug, Default)]
pub struct DrmGemClose {
    pub handle: u32,
    pub pad: u32,
}

/// `struct drm_syncobj_create` (drm.h:894).
#[repr(C)]
#[derive(Debug, Default)]
pub struct DrmSyncobjCreate {
    pub handle: u32,
    pub flags: u32,
}

/// `struct drm_syncobj_destroy` (drm.h:900).
#[repr(C)]
#[derive(Debug, Default)]
pub struct DrmSyncobjDestroy {
    pub handle: u32,
    pub pad: u32,
}

/// drm.h:924-925.
pub const DRM_SYNCOBJ_WAIT_FLAGS_WAIT_ALL: u32 = 1 << 0;
pub const DRM_SYNCOBJ_WAIT_FLAGS_WAIT_FOR_SUBMIT: u32 = 1 << 1;

/// `struct drm_syncobj_wait` (drm.h:928). `timeout_nsec` is absolute
/// CLOCK_MONOTONIC.
#[repr(C)]
#[derive(Debug, Default)]
pub struct DrmSyncobjWait {
    pub handles: u64,
    pub timeout_nsec: i64,
    pub count_handles: u32,
    pub flags: u32,
    pub first_signaled: u32,
    pub pad: u32,
    pub deadline_nsec: u64,
}

/// drm.h:1041, 1063, 1219, 1220, 1223.
pub const DRM_IOCTL_VERSION: u32 = iowr::<DrmVersion>(0x00);
pub const DRM_IOCTL_GEM_CLOSE: u32 = iow::<DrmGemClose>(0x09);
pub const DRM_IOCTL_SYNCOBJ_CREATE: u32 = iowr::<DrmSyncobjCreate>(0xbf);
pub const DRM_IOCTL_SYNCOBJ_DESTROY: u32 = iowr::<DrmSyncobjDestroy>(0xc0);
pub const DRM_IOCTL_SYNCOBJ_WAIT: u32 = iowr::<DrmSyncobjWait>(0xc3);

// ---- panthor (panthor_drm.h) ----

/// `enum drm_panthor_ioctl_id` (panthor_drm.h:85-147).
pub const DRM_PANTHOR_DEV_QUERY: u32 = 0;
pub const DRM_PANTHOR_VM_CREATE: u32 = 1;
pub const DRM_PANTHOR_VM_DESTROY: u32 = 2;
pub const DRM_PANTHOR_VM_BIND: u32 = 3;
pub const DRM_PANTHOR_VM_GET_STATE: u32 = 4;
pub const DRM_PANTHOR_BO_CREATE: u32 = 5;
pub const DRM_PANTHOR_BO_MMAP_OFFSET: u32 = 6;
pub const DRM_PANTHOR_GROUP_CREATE: u32 = 7;
pub const DRM_PANTHOR_GROUP_DESTROY: u32 = 8;
pub const DRM_PANTHOR_GROUP_SUBMIT: u32 = 9;
pub const DRM_PANTHOR_GROUP_GET_STATE: u32 = 10;
pub const DRM_PANTHOR_TILER_HEAP_CREATE: u32 = 11;
pub const DRM_PANTHOR_TILER_HEAP_DESTROY: u32 = 12;

/// `struct drm_panthor_obj_array` (panthor_drm.h:164).
#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
pub struct DrmPanthorObjArray {
    pub stride: u32,
    pub count: u32,
    pub array: u64,
}

impl DrmPanthorObjArray {
    /// `DRM_PANTHOR_OBJ_ARRAY(cnt, ptr)` (panthor_drm.h:183-184).
    pub fn of<T>(items: &[T]) -> Self {
        Self {
            stride: core::mem::size_of::<T>() as u32,
            count: items.len() as u32,
            array: if items.is_empty() { 0 } else { items.as_ptr() as u64 },
        }
    }
}

/// `enum drm_panthor_sync_op_flags` (panthor_drm.h:189-207).
pub const DRM_PANTHOR_SYNC_OP_HANDLE_TYPE_SYNCOBJ: u32 = 0;
pub const DRM_PANTHOR_SYNC_OP_HANDLE_TYPE_TIMELINE_SYNCOBJ: u32 = 1;
pub const DRM_PANTHOR_SYNC_OP_WAIT: u32 = 0;
pub const DRM_PANTHOR_SYNC_OP_SIGNAL: u32 = 1 << 31;

/// `struct drm_panthor_sync_op` (panthor_drm.h:212).
#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
pub struct DrmPanthorSyncOp {
    pub flags: u32,
    pub handle: u32,
    pub timeline_value: u64,
}

/// `enum drm_panthor_dev_query_type` (panthor_drm.h:232-246).
pub const DRM_PANTHOR_DEV_QUERY_GPU_INFO: u32 = 0;
pub const DRM_PANTHOR_DEV_QUERY_CSIF_INFO: u32 = 1;
pub const DRM_PANTHOR_DEV_QUERY_TIMESTAMP_INFO: u32 = 2;
pub const DRM_PANTHOR_DEV_QUERY_GROUP_PRIORITIES_INFO: u32 = 3;

/// `struct drm_panthor_gpu_info` (panthor_drm.h:253); the field decoders are
/// the header's `DRM_PANTHOR_*(x)` macros (panthor_drm.h:256-287).
#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
pub struct DrmPanthorGpuInfo {
    pub gpu_id: u32,
    pub gpu_rev: u32,
    pub csf_id: u32,
    pub l2_features: u32,
    pub tiler_features: u32,
    pub mem_features: u32,
    pub mmu_features: u32,
    pub thread_features: u32,
    pub max_threads: u32,
    pub thread_max_workgroup_size: u32,
    pub thread_max_barrier_size: u32,
    pub coherency_features: u32,
    pub texture_features: [u32; 4],
    pub as_present: u32,
    pub pad0: u32,
    pub shader_present: u64,
    pub l2_present: u64,
    pub tiler_present: u64,
    pub core_features: u32,
    pub pad: u32,
    pub gpu_features: u64,
}

impl DrmPanthorGpuInfo {
    pub fn arch_major(&self) -> u32 {
        self.gpu_id >> 28
    }
    pub fn arch_minor(&self) -> u32 {
        (self.gpu_id >> 24) & 0xf
    }
    pub fn arch_rev(&self) -> u32 {
        (self.gpu_id >> 20) & 0xf
    }
    pub fn product_major(&self) -> u32 {
        (self.gpu_id >> 16) & 0xf
    }
    pub fn version_major(&self) -> u32 {
        (self.gpu_id >> 12) & 0xf
    }
    pub fn version_minor(&self) -> u32 {
        (self.gpu_id >> 4) & 0xff
    }
    pub fn version_status(&self) -> u32 {
        self.gpu_id & 0xf
    }
    pub fn cshw_major(&self) -> u32 {
        (self.csf_id >> 26) & 0x3f
    }
    pub fn cshw_minor(&self) -> u32 {
        (self.csf_id >> 20) & 0x3f
    }
    pub fn cshw_rev(&self) -> u32 {
        (self.csf_id >> 16) & 0xf
    }
    pub fn mcu_major(&self) -> u32 {
        (self.csf_id >> 10) & 0x3f
    }
    pub fn mcu_minor(&self) -> u32 {
        (self.csf_id >> 4) & 0x3f
    }
    pub fn mcu_rev(&self) -> u32 {
        self.csf_id & 0xf
    }
    pub fn mmu_va_bits(&self) -> u32 {
        self.mmu_features & 0xff
    }
}

/// `struct drm_panthor_csif_info` (panthor_drm.h:340).
#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
pub struct DrmPanthorCsifInfo {
    pub csg_slot_count: u32,
    pub cs_slot_count: u32,
    pub cs_reg_count: u32,
    pub scoreboard_slot_count: u32,
    pub unpreserved_cs_reg_count: u32,
    pub pad: u32,
}

/// `struct drm_panthor_timestamp_info` (panthor_drm.h:374).
#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
pub struct DrmPanthorTimestampInfo {
    pub timestamp_frequency: u64,
    pub current_timestamp: u64,
    pub timestamp_offset: u64,
}

/// `struct drm_panthor_group_priorities_info` (panthor_drm.h:393).
#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
pub struct DrmPanthorGroupPrioritiesInfo {
    pub allowed_mask: u8,
    pub pad: [u8; 3],
}

/// `struct drm_panthor_dev_query` (panthor_drm.h:408).
#[repr(C)]
#[derive(Debug, Default)]
pub struct DrmPanthorDevQuery {
    pub type_: u32,
    pub size: u32,
    pub pointer: u64,
}

/// `struct drm_panthor_vm_create` (panthor_drm.h:436).
#[repr(C)]
#[derive(Debug, Default)]
pub struct DrmPanthorVmCreate {
    pub flags: u32,
    pub id: u32,
    pub user_va_range: u64,
}

/// `struct drm_panthor_vm_destroy` (panthor_drm.h:469).
#[repr(C)]
#[derive(Debug, Default)]
pub struct DrmPanthorVmDestroy {
    pub id: u32,
    pub pad: u32,
}

/// `enum drm_panthor_vm_bind_op_flags` (panthor_drm.h:480-522).
pub const DRM_PANTHOR_VM_BIND_OP_MAP_READONLY: u32 = 1 << 0;
pub const DRM_PANTHOR_VM_BIND_OP_MAP_NOEXEC: u32 = 1 << 1;
pub const DRM_PANTHOR_VM_BIND_OP_MAP_UNCACHED: u32 = 1 << 2;
pub const DRM_PANTHOR_VM_BIND_OP_TYPE_MAP: u32 = 0 << 28;
pub const DRM_PANTHOR_VM_BIND_OP_TYPE_UNMAP: u32 = 1 << 28;

/// `struct drm_panthor_vm_bind_op` (panthor_drm.h:527).
#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
pub struct DrmPanthorVmBindOp {
    pub flags: u32,
    pub bo_handle: u32,
    pub bo_offset: u64,
    pub va: u64,
    pub size: u64,
    pub syncs: DrmPanthorObjArray,
}

/// `struct drm_panthor_vm_bind` (panthor_drm.h:582). `flags` 0 is a
/// synchronous bind: the mapping exists when the ioctl returns
/// (`DRM_PANTHOR_VM_BIND_ASYNC`, panthor_drm.h:571-577, is not used).
#[repr(C)]
#[derive(Debug, Default)]
pub struct DrmPanthorVmBind {
    pub vm_id: u32,
    pub flags: u32,
    pub ops: DrmPanthorObjArray,
}

/// `enum drm_panthor_vm_state` (panthor_drm.h:596-618).
pub const DRM_PANTHOR_VM_STATE_USABLE: u32 = 0;

/// `struct drm_panthor_vm_get_state` (panthor_drm.h:623).
#[repr(C)]
#[derive(Debug, Default)]
pub struct DrmPanthorVmGetState {
    pub vm_id: u32,
    pub state: u32,
}

/// `struct drm_panthor_bo_create` (panthor_drm.h:646). The returned `size`
/// is the page-aligned size actually allocated.
#[repr(C)]
#[derive(Debug, Default)]
pub struct DrmPanthorBoCreate {
    pub size: u64,
    pub flags: u32,
    pub exclusive_vm_id: u32,
    pub handle: u32,
    pub pad: u32,
}

/// `struct drm_panthor_bo_mmap_offset` (panthor_drm.h:682).
#[repr(C)]
#[derive(Debug, Default)]
pub struct DrmPanthorBoMmapOffset {
    pub handle: u32,
    pub pad: u32,
    pub offset: u64,
}

/// `struct drm_panthor_queue_create` (panthor_drm.h:696).
#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
pub struct DrmPanthorQueueCreate {
    pub priority: u8,
    pub pad: [u8; 3],
    pub ringbuf_size: u32,
}

/// `enum drm_panthor_group_priority` (panthor_drm.h:713-733).
pub const PANTHOR_GROUP_PRIORITY_MEDIUM: u8 = 1;

/// `struct drm_panthor_group_create` (panthor_drm.h:738).
#[repr(C)]
#[derive(Debug, Default)]
pub struct DrmPanthorGroupCreate {
    pub queues: DrmPanthorObjArray,
    pub max_compute_cores: u8,
    pub max_fragment_cores: u8,
    pub max_tiler_cores: u8,
    pub priority: u8,
    pub pad: u32,
    pub compute_core_mask: u64,
    pub fragment_core_mask: u64,
    pub tiler_core_mask: u64,
    pub vm_id: u32,
    pub group_handle: u32,
}

/// `struct drm_panthor_group_destroy` (panthor_drm.h:816).
#[repr(C)]
#[derive(Debug, Default)]
pub struct DrmPanthorGroupDestroy {
    pub group_handle: u32,
    pub pad: u32,
}

/// `struct drm_panthor_queue_submit` (panthor_drm.h:831): `stream_size` a
/// multiple of 8, `stream_addr` 64-byte aligned (panthor_drm.h:836-854).
#[repr(C)]
#[derive(Debug, Default)]
pub struct DrmPanthorQueueSubmit {
    pub queue_index: u32,
    pub stream_size: u32,
    pub stream_addr: u64,
    pub latest_flush: u32,
    pub pad: u32,
    pub syncs: DrmPanthorObjArray,
}

/// `struct drm_panthor_group_submit` (panthor_drm.h:879).
#[repr(C)]
#[derive(Debug, Default)]
pub struct DrmPanthorGroupSubmit {
    pub group_handle: u32,
    pub pad: u32,
    pub queue_submits: DrmPanthorObjArray,
}

/// `enum drm_panthor_group_state_flags` (panthor_drm.h:893-916).
pub const DRM_PANTHOR_GROUP_STATE_TIMEDOUT: u32 = 1 << 0;
pub const DRM_PANTHOR_GROUP_STATE_FATAL_FAULT: u32 = 1 << 1;
pub const DRM_PANTHOR_GROUP_STATE_INNOCENT: u32 = 1 << 2;

/// `struct drm_panthor_group_get_state` (panthor_drm.h:924).
#[repr(C)]
#[derive(Debug, Default)]
pub struct DrmPanthorGroupGetState {
    pub group_handle: u32,
    pub state: u32,
    pub fatal_queues: u32,
    pub pad: u32,
}

/// `struct drm_panthor_tiler_heap_create` (panthor_drm.h:944): chunk size
/// page-aligned in [128 KiB, 8 MiB], at least one initial chunk, `max_chunks`
/// at least the initial count (panthor_drm.h:948-962).
#[repr(C)]
#[derive(Debug, Default)]
pub struct DrmPanthorTilerHeapCreate {
    pub vm_id: u32,
    pub initial_chunk_count: u32,
    pub chunk_size: u32,
    pub max_chunks: u32,
    pub target_in_flight: u32,
    pub handle: u32,
    pub tiler_heap_ctx_gpu_va: u64,
    pub first_heap_chunk_gpu_va: u64,
}

/// `struct drm_panthor_tiler_heap_destroy` (panthor_drm.h:991).
#[repr(C)]
#[derive(Debug, Default)]
pub struct DrmPanthorTilerHeapDestroy {
    pub handle: u32,
    pub pad: u32,
}

/// `DRM_IOCTL_PANTHOR(WR, ...)` (panthor_drm.h:1054-1089).
const fn panthor<T>(id: u32) -> u32 {
    iowr::<T>(DRM_COMMAND_BASE + id)
}
pub const DRM_IOCTL_PANTHOR_DEV_QUERY: u32 = panthor::<DrmPanthorDevQuery>(DRM_PANTHOR_DEV_QUERY);
pub const DRM_IOCTL_PANTHOR_VM_CREATE: u32 = panthor::<DrmPanthorVmCreate>(DRM_PANTHOR_VM_CREATE);
pub const DRM_IOCTL_PANTHOR_VM_DESTROY: u32 = panthor::<DrmPanthorVmDestroy>(DRM_PANTHOR_VM_DESTROY);
pub const DRM_IOCTL_PANTHOR_VM_BIND: u32 = panthor::<DrmPanthorVmBind>(DRM_PANTHOR_VM_BIND);
pub const DRM_IOCTL_PANTHOR_VM_GET_STATE: u32 = panthor::<DrmPanthorVmGetState>(DRM_PANTHOR_VM_GET_STATE);
pub const DRM_IOCTL_PANTHOR_BO_CREATE: u32 = panthor::<DrmPanthorBoCreate>(DRM_PANTHOR_BO_CREATE);
pub const DRM_IOCTL_PANTHOR_BO_MMAP_OFFSET: u32 = panthor::<DrmPanthorBoMmapOffset>(DRM_PANTHOR_BO_MMAP_OFFSET);
pub const DRM_IOCTL_PANTHOR_GROUP_CREATE: u32 = panthor::<DrmPanthorGroupCreate>(DRM_PANTHOR_GROUP_CREATE);
pub const DRM_IOCTL_PANTHOR_GROUP_DESTROY: u32 = panthor::<DrmPanthorGroupDestroy>(DRM_PANTHOR_GROUP_DESTROY);
pub const DRM_IOCTL_PANTHOR_GROUP_SUBMIT: u32 = panthor::<DrmPanthorGroupSubmit>(DRM_PANTHOR_GROUP_SUBMIT);
pub const DRM_IOCTL_PANTHOR_GROUP_GET_STATE: u32 = panthor::<DrmPanthorGroupGetState>(DRM_PANTHOR_GROUP_GET_STATE);
pub const DRM_IOCTL_PANTHOR_TILER_HEAP_CREATE: u32 =
    panthor::<DrmPanthorTilerHeapCreate>(DRM_PANTHOR_TILER_HEAP_CREATE);
pub const DRM_IOCTL_PANTHOR_TILER_HEAP_DESTROY: u32 =
    panthor::<DrmPanthorTilerHeapDestroy>(DRM_PANTHOR_TILER_HEAP_DESTROY);

#[cfg(test)]
mod tests {
    use super::*;
    use core::mem::{offset_of, size_of};

    // The pins are in uapi_pins.expected, the output of uapi_pins.c (built
    // against Linux v6.18 panthor_drm.h and the DRM core drm.h with
    // aarch64-linux-gnu-gcc and run under qemu-aarch64; the x86_64 host gcc
    // prints the same). Regenerate and compare with:
    //
    //   tools/mali/panthor/uapi_pins.sh <dir with v6.18 panthor_drm.h> <dir with drm.h>
    //
    // Each test below lists the Rust side of one kind of line, in the
    // file's order, and the whole file must equal the three lists.
    const PINS: &str = include_str!("../uapi_pins.expected");

    fn pinned(kind: &str) -> Vec<&'static str> {
        PINS.lines().filter(|l| l.split(' ').next() == Some(kind)).collect()
    }

    fn check(kind: &str, got: Vec<String>) {
        let want = pinned(kind);
        assert_eq!(
            got.len(),
            want.len(),
            "{kind}: Rust lists {} pins, uapi_pins.expected holds {}",
            got.len(),
            want.len()
        );
        for (g, w) in got.iter().zip(&want) {
            assert_eq!(g, w, "{kind} pin: Rust `{g}`, C compiler `{w}`");
        }
    }

    #[test]
    fn struct_sizes_are_the_c_compilers() {
        let got = [
            ("drm_version", size_of::<DrmVersion>()),
            ("drm_gem_close", size_of::<DrmGemClose>()),
            ("drm_syncobj_create", size_of::<DrmSyncobjCreate>()),
            ("drm_syncobj_destroy", size_of::<DrmSyncobjDestroy>()),
            ("drm_syncobj_wait", size_of::<DrmSyncobjWait>()),
            ("obj_array", size_of::<DrmPanthorObjArray>()),
            ("sync_op", size_of::<DrmPanthorSyncOp>()),
            ("gpu_info", size_of::<DrmPanthorGpuInfo>()),
            ("csif_info", size_of::<DrmPanthorCsifInfo>()),
            ("timestamp_info", size_of::<DrmPanthorTimestampInfo>()),
            ("group_priorities_info", size_of::<DrmPanthorGroupPrioritiesInfo>()),
            ("dev_query", size_of::<DrmPanthorDevQuery>()),
            ("vm_create", size_of::<DrmPanthorVmCreate>()),
            ("vm_destroy", size_of::<DrmPanthorVmDestroy>()),
            ("vm_bind_op", size_of::<DrmPanthorVmBindOp>()),
            ("vm_bind", size_of::<DrmPanthorVmBind>()),
            ("vm_get_state", size_of::<DrmPanthorVmGetState>()),
            ("bo_create", size_of::<DrmPanthorBoCreate>()),
            ("bo_mmap_offset", size_of::<DrmPanthorBoMmapOffset>()),
            ("queue_create", size_of::<DrmPanthorQueueCreate>()),
            ("group_create", size_of::<DrmPanthorGroupCreate>()),
            ("group_destroy", size_of::<DrmPanthorGroupDestroy>()),
            ("queue_submit", size_of::<DrmPanthorQueueSubmit>()),
            ("group_submit", size_of::<DrmPanthorGroupSubmit>()),
            ("group_get_state", size_of::<DrmPanthorGroupGetState>()),
            ("tiler_heap_create", size_of::<DrmPanthorTilerHeapCreate>()),
            ("tiler_heap_destroy", size_of::<DrmPanthorTilerHeapDestroy>()),
        ];
        check("size", got.iter().map(|(n, v)| format!("size   {n} {v}")).collect());
    }

    #[test]
    fn field_offsets_are_the_c_compilers() {
        let got = [
            ("struct drm_panthor_gpu_info.shader_present", offset_of!(DrmPanthorGpuInfo, shader_present)),
            ("struct drm_panthor_gpu_info.gpu_features", offset_of!(DrmPanthorGpuInfo, gpu_features)),
            ("struct drm_panthor_group_create.compute_core_mask", offset_of!(DrmPanthorGroupCreate, compute_core_mask)),
            ("struct drm_panthor_group_create.vm_id", offset_of!(DrmPanthorGroupCreate, vm_id)),
            (
                "struct drm_panthor_tiler_heap_create.tiler_heap_ctx_gpu_va",
                offset_of!(DrmPanthorTilerHeapCreate, tiler_heap_ctx_gpu_va),
            ),
            ("struct drm_panthor_vm_bind_op.syncs", offset_of!(DrmPanthorVmBindOp, syncs)),
            ("struct drm_panthor_queue_submit.syncs", offset_of!(DrmPanthorQueueSubmit, syncs)),
        ];
        check("offset", got.iter().map(|(n, v)| format!("offset {n} {v}")).collect());
    }

    #[test]
    fn ioctl_numbers_are_the_c_compilers() {
        let got = [
            ("VERSION", DRM_IOCTL_VERSION),
            ("GEM_CLOSE", DRM_IOCTL_GEM_CLOSE),
            ("SYNCOBJ_CREATE", DRM_IOCTL_SYNCOBJ_CREATE),
            ("SYNCOBJ_DESTROY", DRM_IOCTL_SYNCOBJ_DESTROY),
            ("SYNCOBJ_WAIT", DRM_IOCTL_SYNCOBJ_WAIT),
            ("PANTHOR_DEV_QUERY", DRM_IOCTL_PANTHOR_DEV_QUERY),
            ("PANTHOR_VM_CREATE", DRM_IOCTL_PANTHOR_VM_CREATE),
            ("PANTHOR_VM_DESTROY", DRM_IOCTL_PANTHOR_VM_DESTROY),
            ("PANTHOR_VM_BIND", DRM_IOCTL_PANTHOR_VM_BIND),
            ("PANTHOR_VM_GET_STATE", DRM_IOCTL_PANTHOR_VM_GET_STATE),
            ("PANTHOR_BO_CREATE", DRM_IOCTL_PANTHOR_BO_CREATE),
            ("PANTHOR_BO_MMAP_OFFSET", DRM_IOCTL_PANTHOR_BO_MMAP_OFFSET),
            ("PANTHOR_GROUP_CREATE", DRM_IOCTL_PANTHOR_GROUP_CREATE),
            ("PANTHOR_GROUP_DESTROY", DRM_IOCTL_PANTHOR_GROUP_DESTROY),
            ("PANTHOR_GROUP_SUBMIT", DRM_IOCTL_PANTHOR_GROUP_SUBMIT),
            ("PANTHOR_GROUP_GET_STATE", DRM_IOCTL_PANTHOR_GROUP_GET_STATE),
            ("PANTHOR_TILER_HEAP_CREATE", DRM_IOCTL_PANTHOR_TILER_HEAP_CREATE),
            ("PANTHOR_TILER_HEAP_DESTROY", DRM_IOCTL_PANTHOR_TILER_HEAP_DESTROY),
        ];
        check("ioctl", got.iter().map(|(n, v)| format!("ioctl  {n} {v:#010x}")).collect());
        assert!(PINS.lines().any(|l| l == "const SYNC_OP_SIGNAL 0x80000000"));
        assert_eq!(DRM_PANTHOR_SYNC_OP_SIGNAL, 0x8000_0000);
    }

    #[test]
    fn gpu_id_decodes_as_the_design_page_reads_it() {
        // GPU_ID 0xa8670005 = arch 10.8, product major 7 (Mali-G610), from
        // the board's dmesg (driver/docs/nanopc-t6-mali.md, "The GPU").
        let g = DrmPanthorGpuInfo { gpu_id: 0xa867_0005, ..Default::default() };
        assert_eq!((g.arch_major(), g.arch_minor(), g.arch_rev(), g.product_major()), (10, 8, 6, 7));
        assert_eq!((g.version_major(), g.version_minor(), g.version_status()), (0, 0, 5));
    }
}
