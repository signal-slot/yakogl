// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The Mali-G610's part of the OSAL contract (driver/docs/nanopc-t6-mali.md,
//! "Toward L2"): what the `gpu-mali` backend (src/mali/backend.rs) asks of an
//! OSAL beyond GPU memory, through the ops every GPU uses.
//!
//! | need | op | shape |
//! | --- | --- | --- |
//! | a buffer the CPU writes and the GPU reads, at a GPU VA | `gpu_alloc` | [`super::OsalGpuMem`]: `cpu` and `gpu` (shader code with [`super::OSAL_MEM_GPU_EXEC`]) |
//! | the device's counts | `query(OSAL_QUERY_MALI_GPU)` | [`OsalMaliGpu`] |
//! | the tiler heap the kernel side made | `query(OSAL_QUERY_MALI_TILER_HEAP)` | [`OsalMaliHeap`] |
//! | run a command stream | `submit(&OsalMaliKick, size)` | a fence (>= 0) or a negative errno |
//! | its completion | `wait(fence, timeout_us)` | 0, > 0 on timeout, < 0 on a fault |
//!
//! The OSAL owns the address space, the queue and the heap; the backend
//! owns what it writes into its buffers. An OSAL answers the two queries
//! only when it can submit to a Mali queue: the development OSAL over
//! panthor (tools/mali/panthor, `yakogl_mali_panthor::osal`), later the
//! Zephyr kernel side (driver/rust/src/gpu/mali, milestone Z4). The C
//! mirrors are `struct osal_mali_gpu`, `osal_mali_heap` and `osal_mali_kick`
//! in osal_internal.h, beside the query ids; `yakogl-tools abi-layout` pins
//! both sides (src/abi_layout.rs, src/abi_layout_check.c), and the test
//! below pins the numbers here as well.

/// out = [`OsalMaliGpu`]: the device's identity and command-stream counts.
pub const OSAL_QUERY_MALI_GPU: u32 = 15;
/// out = [`OsalMaliHeap`]: the tiler heap the OSAL made for the backend's
/// queue (the same heap at every query while the OSAL is up).
pub const OSAL_QUERY_MALI_TILER_HEAP: u32 = 16;

/// The device facts the backend builds from (panthor: `DRM_PANTHOR_DEV_QUERY_GPU_INFO`
/// and `_CSIF_INFO`, panthor_drm.h).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OsalMaliGpu {
    /// `GPU_ID` (0xa8670005 on the T6).
    pub gpu_id: u32,
    /// `TILER_FEATURES` (bits 11:8 are `max_levels`).
    pub tiler_features: u32,
    /// The registers of one command stream, and those the kernel keeps
    /// (`cs_reg_count`, `unpreserved_cs_reg_count`).
    pub cs_reg_count: u32,
    pub unpreserved_cs_reg_count: u32,
}

/// The tiler heap (`DRM_IOCTL_PANTHOR_TILER_HEAP_CREATE`'s results): the
/// heap context `HEAP_SET` names, the first chunk and the chunk size the
/// heap descriptor names.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OsalMaliHeap {
    pub ctx_gpu: u64,
    pub first_chunk_gpu: u64,
    pub chunk_size: u32,
    pub reserved: u32,
}

/// The kick descriptor `submit` takes on a `gpu-mali` build: one command
/// stream in GPU memory. Its words name absolute GPU addresses, so the
/// stream runs where it was built.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OsalMaliKick {
    /// The stream's first word (64-byte aligned).
    pub stream_gpu: u64,
    /// Its length in bytes (a non-zero multiple of 8).
    pub stream_bytes: u32,
    pub reserved: u32,
}

#[cfg(test)]
mod tests {
    use super::{OsalMaliGpu, OsalMaliHeap, OsalMaliKick};
    use core::mem::{offset_of, size_of};

    /// The three shapes, field by field: the numbers of `struct
    /// osal_mali_gpu`, `osal_mali_heap` and `osal_mali_kick` in
    /// osal_internal.h (src/abi_layout_check.c asserts them on the C side).
    #[test]
    fn the_mali_osal_structs_have_the_c_layout() {
        assert_eq!(size_of::<OsalMaliGpu>(), 16);
        assert_eq!(
            [
                offset_of!(OsalMaliGpu, gpu_id),
                offset_of!(OsalMaliGpu, tiler_features),
                offset_of!(OsalMaliGpu, cs_reg_count),
                offset_of!(OsalMaliGpu, unpreserved_cs_reg_count),
            ],
            [0, 4, 8, 12]
        );
        assert_eq!(size_of::<OsalMaliHeap>(), 24);
        assert_eq!(
            [
                offset_of!(OsalMaliHeap, ctx_gpu),
                offset_of!(OsalMaliHeap, first_chunk_gpu),
                offset_of!(OsalMaliHeap, chunk_size),
                offset_of!(OsalMaliHeap, reserved),
            ],
            [0, 8, 16, 20]
        );
        assert_eq!(size_of::<OsalMaliKick>(), 16);
        assert_eq!(
            [
                offset_of!(OsalMaliKick, stream_gpu),
                offset_of!(OsalMaliKick, stream_bytes),
                offset_of!(OsalMaliKick, reserved)
            ],
            [0, 8, 12]
        );
        /* and the C header declares them */
        let internal = include_str!("osal_internal.h");
        for s in ["struct osal_mali_gpu {", "struct osal_mali_heap {", "struct osal_mali_kick {"] {
            assert!(internal.contains(s), "osal_internal.h has {s:?}");
        }
    }

    /// The C headers reserve the same ids and the same memory flag.
    #[test]
    fn the_c_headers_reserve_the_mali_ids_and_the_exec_flag() {
        let internal = include_str!("osal_internal.h");
        for line in [
            format!("#define OSAL_QUERY_MALI_GPU {}u\n", super::OSAL_QUERY_MALI_GPU),
            format!("#define OSAL_QUERY_MALI_TILER_HEAP {}u\n", super::OSAL_QUERY_MALI_TILER_HEAP),
        ] {
            assert!(internal.contains(&line), "osal_internal.h has {line:?}");
        }
        let public = include_str!("../../include/gles_osal.h");
        assert_eq!(crate::osal::OSAL_MEM_GPU_EXEC, 1 << 3);
        assert!(public.contains("\tOSAL_MEM_GPU_EXEC  = 1 << 3,"), "gles_osal.h has OSAL_MEM_GPU_EXEC = 1 << 3");
    }
}
