// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! C entry points of the GC7000 kernel side (gpu::vivante); the shim is
//! driver/drivers/gpu/gpu_vivante_gc_rust.c, which implements the
//! `gc_internal.h` / `gc.h` functions the composition code calls on top of
//! these. Declarations: include/yakogl_drivers.h ("Vivante GC7000").
//!
//! `core` is the shim's `struct gc_dev` (its prefix is [`GcCore`], pinned
//! here and in the shim); `id` is a `struct gc_identity`; `env` the shim's
//! `struct yd_env`. The shim serialises calls the way the C did: the submit
//! mutex around the ring, the spinlock around [`yd_gc_event_claim`] and
//! [`yd_gc_irq_handle`].

use crate::env::YdEnv;
use crate::gpu::vivante::{self as gc, cmdbuf, hw, irq, mmu, power, GcCore, GcIdentity, GcSubmit};
use crate::mmio::Hw;
use core::ffi::c_void;
use core::mem::{offset_of, size_of};

fn hwio() -> Hw {
    // SAFETY: every address the entry points touch is a register of a block
    // the shim device_map()ed: the GPU's (core->regs / regs), the GPC's and
    // the CCM's.
    unsafe { Hw::new() }
}

fn rc(r: Result<(), i32>) -> i32 {
    match r {
        Ok(()) => 0,
        Err(e) => e,
    }
}

/// Clocks and power domains (gpu::vivante::power). 0 or a negative errno.
/// `mark`, when not null, is called with `ctx` and a `power::step` number
/// before each sub-step (`power::power_on_marked`); it must not touch the
/// CCM, the GPC or the GPU.
///
/// # Safety
/// `ccm` and `gpc` are the mapped CCM and GPC blocks, `env` a valid table,
/// `mark` null or callable with `ctx`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yd_gc_power_on(
    ccm: usize,
    gpc: usize,
    env: *const YdEnv,
    mark: Option<unsafe extern "C" fn(ctx: *mut c_void, step: u32)>,
    ctx: *mut c_void,
) -> i32 {
    // SAFETY: the caller's contract.
    let env = unsafe { &*env };
    let mut call = |n: u32| {
        if let Some(f) = mark {
            // SAFETY: the caller's contract.
            unsafe { f(ctx, n) }
        }
    };
    match power::power_on_marked(&hwio(), ccm, gpc, env, &mut call) {
        Ok(_) => 0,
        Err(e) => e,
    }
}

/// Identify the GPU at `regs` into `*id`.
///
/// # Safety
/// `regs` is the mapped GPU block; `id` and `env` are valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yd_gc_identify(regs: usize, env: *const YdEnv, id: *mut GcIdentity) -> i32 {
    // SAFETY: the caller's contract.
    let (env, id) = unsafe { (&*env, &mut *id) };
    match hw::identify(&hwio(), regs, env) {
        Ok(v) => {
            *id = v;
            0
        }
        Err(e) => e,
    }
}

/// Reset the GPU at `regs` to idle (`gc_bringup_reset`).
///
/// # Safety
/// As [`yd_gc_identify`]; `id` points at an identity.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yd_gc_reset(regs: usize, env: *const YdEnv, id: *const GcIdentity) -> i32 {
    // SAFETY: the caller's contract.
    let (env, id) = unsafe { (&*env, &*id) };
    rc(hw::reset(&hwio(), regs, env, id))
}

/// `gc_hw_init`.
///
/// # Safety
/// `core` is the shim's `struct gc_dev` with `regs` set; `env` valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yd_gc_hw_init(core: *mut GcCore, env: *const YdEnv) -> i32 {
    // SAFETY: the caller's contract.
    let (c, env) = unsafe { (&*core, &*env) };
    hw::init(&hwio(), c.regs, env);
    0
}

/// `gc_rd_fe`.
///
/// # Safety
/// As [`yd_gc_hw_init`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yd_gc_rd_fe(core: *const GcCore, off: u32) -> u32 {
    // SAFETY: the caller's contract.
    let c = unsafe { &*core };
    hw::rd_fe(&hwio(), c.regs, off)
}

/// `gc_idle_state_decode`.
///
/// # Safety
/// `env` valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yd_gc_idle_state_decode(env: *const YdEnv, idle: u32) {
    // SAFETY: the caller's contract.
    hw::log_idle_state(unsafe { &*env }, idle)
}

/// `gc_mmu_init`.
///
/// # Safety
/// `core` has the carveout set (CPU view valid for `carveout_size` bytes,
/// 8-byte aligned); `env` valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yd_gc_mmu_init(core: *mut GcCore, env: *const YdEnv) -> i32 {
    // SAFETY: the caller's contract.
    let (c, env) = unsafe { (&mut *core, &*env) };
    rc(mmu::init(c, env))
}

/// `gc_mmu_map`.
///
/// # Safety
/// As [`yd_gc_mmu_init`], after it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yd_gc_mmu_map(
    core: *mut GcCore,
    env: *const YdEnv,
    gpu_va: u32,
    phys: u64,
    size: usize,
) -> i32 {
    // SAFETY: the caller's contract.
    let (c, env) = unsafe { (&mut *core, &*env) };
    rc(mmu::map(c, env, gpu_va, phys, size))
}

/// `gc_mmu_enable`.
///
/// # Safety
/// As [`yd_gc_mmu_map`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yd_gc_mmu_enable(core: *mut GcCore, env: *const YdEnv) -> i32 {
    // SAFETY: the caller's contract.
    let (c, env) = unsafe { (&mut *core, &*env) };
    rc(mmu::enable(c, &hwio(), env))
}

/// `gc_mmu_dump`.
///
/// # Safety
/// As [`yd_gc_hw_init`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yd_gc_mmu_dump(core: *const GcCore, env: *const YdEnv) {
    // SAFETY: the caller's contract.
    let (c, env) = unsafe { (&*core, &*env) };
    mmu::dump(c, &hwio(), env)
}

/// `gc_fe_start`.
///
/// # Safety
/// As [`yd_gc_hw_init`]; `core->id` null or valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yd_gc_fe_start(core: *const GcCore, address: u32, prefetch: u32) {
    // SAFETY: the caller's contract.
    cmdbuf::fe_start(unsafe { &*core }, &hwio(), address, prefetch)
}

/// `gc_fe_wait_idle`.
///
/// # Safety
/// As [`yd_gc_hw_init`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yd_gc_fe_wait_idle(core: *const GcCore, env: *const YdEnv, timeout_ms: u32) -> i32 {
    // SAFETY: the caller's contract.
    let (c, env) = unsafe { (&*core, &*env) };
    rc(cmdbuf::fe_wait_idle(c, &hwio(), env, timeout_ms))
}

/// `gc_cmd_build_pta_config`.
///
/// # Safety
/// As [`yd_gc_mmu_init`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yd_gc_cmd_build_pta_config(core: *const GcCore, index: u32) -> u32 {
    // SAFETY: the caller's contract.
    mmu::build_pta_config(unsafe { &*core }, index)
}

/// `gc_ring_init`; `clock_hz` is the core clock (devicetree).
///
/// # Safety
/// As [`yd_gc_mmu_init`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yd_gc_ring_init(core: *mut GcCore, env: *const YdEnv, clock_hz: u32) -> i32 {
    // SAFETY: the caller's contract.
    let (c, env) = unsafe { (&mut *core, &*env) };
    cmdbuf::ring_init(c, &hwio(), env, clock_hz);
    0
}

/// `gc_ring_submit`.
///
/// # Safety
/// As [`yd_gc_ring_init`], after it; `s` valid; the caller holds the submit
/// mutex.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yd_gc_ring_submit(
    core: *mut GcCore,
    env: *const YdEnv,
    s: *const GcSubmit,
    event: u32,
    fence: u32,
    clock_hz: u32,
) -> i32 {
    // SAFETY: the caller's contract.
    let (c, env, s) = unsafe { (&mut *core, &*env, &*s) };
    rc(cmdbuf::ring_submit(c, env, s, event, fence, clock_hz))
}

/// `gc_ring_dump`.
///
/// # Safety
/// As [`yd_gc_hw_init`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yd_gc_ring_dump(core: *const GcCore, env: *const YdEnv) {
    // SAFETY: the caller's contract.
    let (c, env) = unsafe { (&*core, &*env) };
    cmdbuf::ring_dump(c, &hwio(), env)
}

/// Claim an event for `fence` under the caller's lock: the id, -EAGAIN when
/// all are in flight, -EIO for a corrupt bitmap.
///
/// # Safety
/// `core` valid; the caller holds the spinlock.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yd_gc_event_claim(core: *mut GcCore, fence: u32) -> i32 {
    // SAFETY: the caller's contract.
    match irq::claim(unsafe { &mut *core }, fence) {
        Ok(Some(id)) => id as i32,
        Ok(None) => -crate::errno::EAGAIN,
        Err(e) => e,
    }
}

/// The interrupt's work under the caller's lock; `*freed` gets the number of
/// events freed (the shim gives the semaphore that often). True when this
/// GPU raised the interrupt.
///
/// # Safety
/// `core` valid with `regs` mapped; `freed` valid; the caller holds the
/// spinlock.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yd_gc_irq_handle(core: *mut GcCore, now_ns: u64, freed: *mut u32) -> bool {
    // SAFETY: the caller's contract.
    let (c, freed) = unsafe { (&mut *core, &mut *freed) };
    let h = irq::handle(c, &hwio(), now_ns);
    *freed = h.freed;
    h.ours
}

// The layouts include/yakogl_drivers.h and the shim pin (LP64): the shim
// asserts offsetof(struct gc_dev, ...) and offsetof(struct gc_identity,
// ...) to the same numbers.
const _: () = {
    assert!(size_of::<GcIdentity>() == 124);
    assert!(offset_of!(GcIdentity, minor_features) == 32);
    assert!(offset_of!(GcIdentity, secure_ahb) == 80);
    assert!(offset_of!(GcIdentity, from_table) == 81);
    assert!(offset_of!(GcIdentity, stream_count) == 84);
    assert!(offset_of!(GcIdentity, varyings_count) == 120);
    assert!(size_of::<GcSubmit>() == 12);
    assert!(offset_of!(GcCore, regs) == 16);
    assert!(offset_of!(GcCore, carveout) == 24);
    assert!(offset_of!(GcCore, carveout_phys) == 32);
    assert!(offset_of!(GcCore, carveout_size) == 40);
    assert!(offset_of!(GcCore, mtlb) == 48);
    assert!(offset_of!(GcCore, pta) == 56);
    assert!(offset_of!(GcCore, stlb_used) == 64);
    assert!(offset_of!(GcCore, import_va) == 68);
    assert!(offset_of!(GcCore, mmu_on) == 72);
    assert!(offset_of!(GcCore, ring) == 80);
    assert!(offset_of!(GcCore, ring_gpu) == 88);
    assert!(offset_of!(GcCore, ring_pos) == 92);
    assert!(offset_of!(GcCore, wait_pos) == 96);
    assert!(offset_of!(GcCore, fe_running) == 100);
    assert!(offset_of!(GcCore, event_free) == 104);
    assert!(offset_of!(GcCore, event_fence) == 108);
    assert!(offset_of!(GcCore, fence_next) == 228);
    assert!(offset_of!(GcCore, fence_done) == 232);
    assert!(offset_of!(GcCore, irq_errors) == 236);
    assert!(offset_of!(GcCore, event_t0_ns) == 272);
    assert!(offset_of!(GcCore, perf_last_done_ns) == 512);
    assert!(offset_of!(GcCore, perf_busy_ns) == 520);
    assert!(offset_of!(GcCore, perf_kicks_done) == 528);
    assert!(offset_of!(GcCore, dump_seen) == 564);
    let _ = gc::EVENT_ALL;
};
