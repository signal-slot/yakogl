// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The Vivante GC7000UL of the i.MX 8M Plus: the kernel side (feature
//! `vivante`), the driver's only kernel side (`CONFIG_YAKOGL_DRV_GPU_VIVANTE`).
//!
//! What the deleted C kernel side (`driver/src/vivante/gc_bringup.c`,
//! `gc_hw.c`, `gc_mmu.c`, `gc_cmdbuf.c`) did, rewritten from permissive
//! sources only
//! (driver/docs/vivante-kernel-rust.md has the table, one row per sequence):
//! - register names, offsets, fields and command words: the etna_viv rnndb
//!   headers (MIT) in `driver/src/vivante/hw/` ([`regs`], checked against
//!   them by a test);
//! - the order of the reset, the hardware init, the MMU v2 tables and the
//!   front end's WAIT/LINK ring: NXP's galcore kernel driver
//!   (`drivers/mxc/gpu-viv` of linux-imx, dual MIT/GPL, read under MIT as
//!   documentation; cited by file and function, nothing copied);
//! - the SoC side (GPC power domains, CCM roots and gates): the i.MX 8M Plus
//!   reference manual through [`crate::soc::imx8mp::gpc`] and
//!   [`crate::soc::imx8mp::ccm`];
//! - values where those sources differ from what runs on the board: the
//!   board's own measurements, said so where they are used.
//!
//! The Linux etnaviv kernel driver (GPL) is not a source of this module.
//!
//! The modules:
//! - [`power`]: the GPU's clock roots, gates and power domains;
//! - [`hw`]: identify, reset, clock gating, the init after reset, idle;
//! - [`mmu`]: the MMU v2 page tables and the secure-mode enable;
//! - [`cmd`]: command-stream words; [`cmdbuf`]: the ring, the front end,
//!   one submit;
//! - [`irq`]: events, fences and the interrupt's work.
//!
//! **State.** The C composition code (`gc_render.c`, `gc_draw.c`) keeps the
//! driver's state in `struct gc_dev` (`gc_internal.h`) and reads its fields.
//! That struct stays C's; [`GcCore`] mirrors the part of it before the
//! kernel objects (both sides pin the offsets: `gc_kernel_rust.c`'s
//! `_Static_assert`s, the `const` asserts in [`crate::ffi`], and the host
//! test against the C layout recorded in [`c_golden`]). Kernel objects (the
//! spinlock, the event semaphore, the submit mutex) stay in C: the shim
//! takes the lock around the calls that need it.
//!
//! **Memory.** The carveout is Normal non-cacheable on the board. Every
//! store the GPU may read goes through [`Mem`] (volatile), and the stores
//! whose order the front end observes are separated by [`dmb_sy`], the
//! barrier Zephyr's `barrier_dmem_fence_full()` is on arm64.

pub mod cmd;
pub mod cmdbuf;
pub mod hw;
pub mod irq;
pub mod mmu;
pub mod power;
#[allow(non_upper_case_globals)] // the rnndb names, MMUv2_* included
pub mod regs;

#[cfg(test)]
// Generated from the C by driver/tools/gclab/kerneltest, which was deleted
// with the C (4c25beb0; `git show 4c25beb0^:driver/tools/gclab/kerneltest/kerneltest.c`).
// Kept as recorded data: the replay test holds the Rust to it byte for byte.
#[rustfmt::skip]
mod c_golden;
#[cfg(test)]
mod tests;

use core::ffi::c_void;

/// `GC_PAGE_SIZE` and the fixed layout at the head of the carveout
/// (`gc_internal.h`; ours). The page tables come first because the MMU is
/// built before any allocator exists.
pub const PAGE_SIZE: u32 = 0x1000;
pub const OFF_MTLB: usize = 0x00_0000;
pub const OFF_PTA: usize = 0x00_1000;
pub const OFF_SAFE_PAGE: usize = 0x00_2000;
pub const OFF_MMU_CMD: usize = 0x00_3000;
pub const OFF_STLB: usize = 0x00_4000;
pub const STLB_COUNT: u32 = 64;
pub const OFF_RING: usize = 0x04_4000;
pub const RING_SIZE: usize = 0x01_0000;
pub const OFF_POOL: usize = 0x06_0000;
/// The window `gc_render_map_phys()` hands VAs from.
pub const IMPORT_VA_BASE: u32 = 0x1000_0000;
/// Event ids: 30 (INTR_ACKNOWLEDGE bits 29:0; 30 and 31 are the errors).
pub const EVENT_COUNT: u32 = 30;
pub const EVENT_ALL: u32 = (1 << EVENT_COUNT) - 1;
/// `GC_CMDBUF_TAIL` (gc_render.h): the LINK the ring writes past a caller's
/// command buffer.
pub const CMDBUF_TAIL: u32 = 8;

/// `struct gc_identity` (gc.h), the layout pinned on both sides.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GcIdentity {
    pub model: u32,
    pub revision: u32,
    pub product_id: u32,
    pub customer_id: u32,
    pub eco_id: u32,
    pub date: u32,
    pub time: u32,
    pub features: u32,
    pub minor_features: [u32; 12],
    pub secure_ahb: bool,
    pub from_table: bool,
    pub stream_count: u32,
    pub register_max: u32,
    pub thread_count: u32,
    pub shader_core_count: u32,
    pub vertex_cache_size: u32,
    pub vertex_output_buffer_size: u32,
    pub pixel_pipes: u32,
    pub instruction_count: u32,
    pub num_constants: u32,
    pub varyings_count: u32,
}

/// `struct gc_submit` (gc_render.h).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GcSubmit {
    pub cmd_gpu: u32,
    pub cmd_bytes: u32,
    pub flags: u32,
}

/// `GC_SUBMIT_*` (gc_render.h).
pub mod submit {
    pub const FLUSH_COLOR: u32 = 1 << 0;
    pub const FLUSH_DEPTH: u32 = 1 << 1;
    pub const FLUSH_TEX: u32 = 1 << 2;
    pub const NO_PIPE_SEL: u32 = 1 << 3;
}

/// `struct gc_dev` (gc_internal.h) up to and including `dump_seen`: every
/// field before the kernel objects. The C side owns the struct; this is its
/// view. Fields the interrupt writes and others poll (`fence_done` and the
/// counters C declares `volatile`) are accessed with volatile operations.
#[repr(C)]
#[derive(Debug)]
pub struct GcCore {
    pub dev: *const c_void,
    pub id: *const GcIdentity,
    pub regs: usize,
    pub carveout: *mut u8,
    pub carveout_phys: u64,
    pub carveout_size: usize,
    pub mtlb: *mut u32,
    pub pta: *mut u64,
    pub stlb_used: u32,
    pub import_va: u32,
    pub mmu_on: bool,
    pub ring: *mut u32,
    pub ring_gpu: u32,
    pub ring_pos: u32,
    pub wait_pos: u32,
    pub fe_running: bool,
    pub event_free: u32,
    pub event_fence: [u32; EVENT_COUNT as usize],
    pub fence_next: u32,
    pub fence_done: u32,
    pub irq_errors: u32,
    pub fence_timeouts: u32,
    pub max_quiet_ms: u32,
    pub fence_lost: u32,
    pub resets: u32,
    pub idle_resets: u32,
    pub event_busy: u32,
    pub wait_ms: u32,
    pub event_t0_ns: [u64; EVENT_COUNT as usize],
    pub perf_last_done_ns: u64,
    pub perf_busy_ns: u64,
    pub perf_kicks_done: u32,
    pub perf_wait_ns: u64,
    pub perf_waits: u32,
    pub perf_wait_sleeps: u32,
    pub pool_double_frees: u32,
    pub dump_idle: u32,
    pub dump_fence: u32,
    pub dump_seen: bool,
}

impl GcCore {
    /// A zeroed core over `regs` and a carveout (the host tests' start).
    pub fn new(regs: usize, carveout: *mut u8, carveout_phys: u64, carveout_size: usize) -> GcCore {
        GcCore {
            dev: core::ptr::null(),
            id: core::ptr::null(),
            regs,
            carveout,
            carveout_phys,
            carveout_size,
            mtlb: core::ptr::null_mut(),
            pta: core::ptr::null_mut(),
            stlb_used: 0,
            import_va: 0,
            mmu_on: false,
            ring: core::ptr::null_mut(),
            ring_gpu: 0,
            ring_pos: 0,
            wait_pos: 0,
            fe_running: false,
            event_free: 0,
            event_fence: [0; EVENT_COUNT as usize],
            fence_next: 0,
            fence_done: 0,
            irq_errors: 0,
            fence_timeouts: 0,
            max_quiet_ms: 0,
            fence_lost: 0,
            resets: 0,
            idle_resets: 0,
            event_busy: 0,
            wait_ms: 0,
            event_t0_ns: [0; EVENT_COUNT as usize],
            perf_last_done_ns: 0,
            perf_busy_ns: 0,
            perf_kicks_done: 0,
            perf_wait_ns: 0,
            perf_waits: 0,
            perf_wait_sleeps: 0,
            pool_double_frees: 0,
            dump_idle: 0,
            dump_fence: 0,
            dump_seen: false,
        }
    }

    /// The identity the bring-up published, if any.
    pub fn identity(&self) -> Option<&GcIdentity> {
        // SAFETY: `id` is null or points at the bring-up's identity, which
        // lives as long as the device (the shim's static gc_data).
        unsafe { self.id.as_ref() }
    }

    /// The carveout as GPU-visible memory.
    pub fn mem(&self) -> Mem {
        Mem { base: self.carveout, len: self.carveout_size }
    }

    /// `gc_cpu_of()` (gc_render.c): the carveout offset of a GPU VA, or
    /// `None` outside it. The carveout is identity-mapped.
    pub fn carveout_offset(&self, gpu_va: u32) -> Option<usize> {
        let va = u64::from(gpu_va);
        if self.carveout.is_null() || va < self.carveout_phys || va >= self.carveout_phys + self.carveout_size as u64 {
            return None;
        }
        Some((va - self.carveout_phys) as usize)
    }

    pub(crate) fn set_fence_done(&mut self, v: u32) {
        // SAFETY: a field of `self`; volatile because gc_render_wait() polls
        // it outside the lock (C declares it volatile).
        unsafe { core::ptr::write_volatile(&raw mut self.fence_done, v) }
    }

    pub(crate) fn fence_done(&self) -> u32 {
        // SAFETY: as above.
        unsafe { core::ptr::read_volatile(&raw const self.fence_done) }
    }

    pub(crate) fn bump_irq_errors(&mut self) {
        // SAFETY: as above (C: volatile).
        unsafe { core::ptr::write_volatile(&raw mut self.irq_errors, self.irq_errors.wrapping_add(1)) }
    }
}

/// The carveout's CPU view: bounds-checked volatile word access. On the
/// board this is Normal non-cacheable memory the GPU reads concurrently, so
/// no store may be merged, elided or reordered by the compiler; the order
/// between stores the hardware observes is [`dmb_sy`]'s.
#[derive(Clone, Copy, Debug)]
pub struct Mem {
    base: *mut u8,
    len: usize,
}

impl Mem {
    /// # Safety
    /// `base` must be valid for reads and writes of `len` bytes, 8-byte
    /// aligned, for as long as the `Mem` is used.
    pub unsafe fn new(base: *mut u8, len: usize) -> Mem {
        Mem { base, len }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn w32(&self, off: usize, v: u32) {
        assert!(off.is_multiple_of(4) && off + 4 <= self.len, "carveout store at {off:#x}");
        // SAFETY: in bounds and aligned (checked); `Mem::new`'s contract.
        unsafe { core::ptr::write_volatile(self.base.add(off).cast::<u32>(), v) }
    }

    pub fn r32(&self, off: usize) -> u32 {
        assert!(off.is_multiple_of(4) && off + 4 <= self.len, "carveout load at {off:#x}");
        // SAFETY: as in `w32`.
        unsafe { core::ptr::read_volatile(self.base.add(off).cast::<u32>()) }
    }

    pub fn w64(&self, off: usize, v: u64) {
        assert!(off.is_multiple_of(8) && off + 8 <= self.len, "carveout store at {off:#x}");
        // SAFETY: as in `w32`.
        unsafe { core::ptr::write_volatile(self.base.add(off).cast::<u64>(), v) }
    }
}

/// `dmb sy`: Zephyr's `barrier_dmem_fence_full()` on arm64, the barrier the
/// ring's two-store commits need. Elsewhere (host tests) a SeqCst fence.
#[inline(always)]
pub fn dmb_sy() {
    #[cfg(target_arch = "aarch64")]
    // SAFETY: a barrier; no memory is accessed.
    unsafe {
        core::arch::asm!("dmb sy", options(nostack, preserves_flags));
    }
    #[cfg(not(target_arch = "aarch64"))]
    core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
}
