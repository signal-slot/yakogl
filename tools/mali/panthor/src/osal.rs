// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The `OsalOps` table of the development OSAL (driver/docs/nanopc-t6-mali.md,
//! L2): YakoGL's GL core and its `gpu-mali` backend (src/mali/backend.rs)
//! reach the Mali-G610 through panthor with it.
//!
//! The library is linked as a crate (no `osal-*` feature, so `g_osal`
//! starts null) and [`install`] points `g_osal` at [`OPS`]; the C OSALs of
//! src/osal/ are not built. This crate is the place because it already
//! holds the panthor objects (`dev.rs`), and an OSAL over a Linux uAPI is a
//! std program: the product's OSAL is the Zephyr kernel side (Z1-Z4).
//!
//! What each op does, on the objects `init` makes once (a VM, a group with
//! one queue, a tiler heap of Mesa's defaults):
//!
//! | op | panthor |
//! | --- | --- |
//! | `gpu_alloc` | a BO, CPU-mapped (write-combined), bound at the next free VA; `OSAL_MEM_GPU_EXEC` read-only executable, else no-exec |
//! | `gpu_alloc` (`phys`) | the BO's own GPU VA: a development OSAL has no physical addresses, so the address the OSAL's own buffers go by is this identity. Only this OSAL gives it meaning, and only [`map_phys`](OPS) reads it |
//! | `map_phys` | a window onto pool memory the caller owns (a gles_native_window's `phys`, which a caller takes from its own `gpu_alloc`): the live BO that holds `[phys, phys + size)` mapped a second time, at a fresh VA and a CPU address of its own ([`crate::dev::Bo::alias`]). Each holds the pages for as long as it lasts, whatever happens to the caller's BO. A `phys` no live BO holds is refused |
//! | `gpu_free` | held until the kick being recorded has completed (the OSAL contract, include/gles_osal.h: "held until the kicks that may read it complete"; the fence `submitted + 1` that src/osal/zephyr.c gives a free), then unbind, unmap, close |
//! | `submit` | `GROUP_SUBMIT` of the [`OsalMaliKick`]'s stream with a syncobj to signal; the fence is a counter |
//! | `wait` | the syncobj wait, then the group state: 0, 1 on timeout, -EIO on a timed-out or faulted group |
//! | `completed` | the highest fence whose syncobj and every earlier one signalled (a zero-timeout wait) |
//! | `query` | `OSAL_QUERY_MALI_GPU` from the GPU and CSIF queries, `OSAL_QUERY_MALI_TILER_HEAP` from TILER_HEAP_CREATE |
//! | `cache_flush` / `cache_invd` | a fence: the mapping is write-combined and the kernel cleans the GPU caches around each job |
//! | `log` | a line on stdout, "yakogl: ..." |
//!
//! `init` also lends the in-process compiler its scratch and keep regions
//! (the library's default sizes), as a Zephyr OSAL does at boot.
//!
//! Single-threaded, as the library's contract is: the state lives behind a
//! mutex only so that a second thread is caught, not served.

use crate::dev::{Alias, Bo, Device, Error, Fence, Group, Map, TilerHeap, Vm, Waited};
use core::ffi::{c_char, c_void};
use std::collections::BTreeMap;
use std::sync::atomic::{fence, Ordering};
use std::sync::Mutex;
use std::time::Duration;
use yakogl::osal::mali::{OsalMaliGpu, OsalMaliHeap, OsalMaliKick, OSAL_QUERY_MALI_GPU, OSAL_QUERY_MALI_TILER_HEAP};
use yakogl::osal::{OsalGpuMem, OsalOps, OSAL_MEM_GPU_EXEC};

/// The render node `init` opens: `YAKOGL_MALI_DEV`, else this.
pub const DEFAULT_DEV: &str = "/dev/dri/renderD128";
/// Mesa's tiler heap: 2 MiB chunks, 5 initial, 64 at most, 65535 in flight
/// (driinfo_panfrost.h:16-21, Mesa 25.0.7 pan_csf.c:1420-1426).
const HEAP_CHUNK: u32 = 2 << 20;
const HEAP_INITIAL: u32 = 5;
const HEAP_MAX: u32 = 64;
const HEAP_IN_FLIGHT: u32 = 65535;
const RING_BYTES: u32 = 64 * 1024;

struct State {
    dev: &'static Device,
    vm: &'static Vm<'static>,
    group: Group<'static>,
    heap: TilerHeap<'static>,
    bos: BTreeMap<usize, Bo<'static>>,
    /// The `map_phys` windows, by the same ids as `bos`.
    windows: BTreeMap<usize, Alias<'static>>,
    windows_high_water: usize,
    /// Buffers and windows freed while a kick that may read them has not
    /// completed.
    pending: Held<Freed>,
    next_bo: usize,
    fences: BTreeMap<i64, Fence<'static>>,
    submitted: i64,
    completed: i64,
    /// Buffers live now and at most, and their bytes.
    live: usize,
    live_high_water: usize,
    bytes: u64,
    bytes_high_water: u64,
}

/// `State` holds raw CPU mappings; it is only ever touched under the mutex
/// by the one thread the library allows.
struct Cell(Option<State>);
// SAFETY: see above; the mutex serialises every access.
unsafe impl Send for Cell {}

static STATE: Mutex<Cell> = Mutex::new(Cell(None));

fn with<R>(f: impl FnOnce(&mut State) -> R) -> Option<R> {
    let mut g = STATE.lock().unwrap_or_else(|e| e.into_inner());
    g.0.as_mut().map(f)
}

const EIO: i32 = libc::EIO;
const EINVAL: i32 = libc::EINVAL;
const ENODEV: i32 = libc::ENODEV;
const ENOTSUP: i32 = libc::ENOTSUP;

fn say(what: &str, e: &Error) {
    println!("yakogl-mali-panthor: {what}: {e}");
}

unsafe extern "C" fn o_init(_arg: *mut c_void) -> i32 {
    let mut g = STATE.lock().unwrap_or_else(|e| e.into_inner());
    if g.0.is_some() {
        return 0;
    }
    let path = std::env::var("YAKOGL_MALI_DEV").unwrap_or_else(|_| DEFAULT_DEV.into());
    let made = (|| -> Result<State, Error> {
        let dev: &'static Device = Box::leak(Box::new(Device::open(&path)?));
        let vm: &'static Vm<'static> = Box::leak(Box::new(Vm::create(dev)?));
        let group = Group::new(vm, RING_BYTES)?;
        let heap = TilerHeap::new(vm, HEAP_CHUNK, HEAP_INITIAL, HEAP_MAX, HEAP_IN_FLIGHT)?;
        Ok(State {
            dev,
            vm,
            group,
            heap,
            bos: BTreeMap::new(),
            windows: BTreeMap::new(),
            windows_high_water: 0,
            pending: Held::default(),
            next_bo: 1,
            fences: BTreeMap::new(),
            submitted: 0,
            completed: 0,
            live: 0,
            live_high_water: 0,
            bytes: 0,
            bytes_high_water: 0,
        })
    })();
    /* the in-process compiler's two regions, lent once as a Zephyr OSAL
     * lends its statics at boot (src/osal/zephyr_shaderc_scratch.c), at the
     * library's default sizes */
    let made = made.and_then(|s| {
        use yakogl::shaderc::online::store;
        let scratch: &'static mut [u8] = Box::leak(vec![0u8; store::SCRATCH_BYTES].into_boxed_slice());
        let keep: &'static mut [u64] = Box::leak(vec![0u64; store::KEEP_BYTES / 8].into_boxed_slice());
        // SAFETY: both are leaked, so they outlive every compile, and only
        // the compiler touches them.
        let (rs, rk) = unsafe {
            (
                store::shc_online_lend_scratch(scratch.as_mut_ptr(), scratch.len()),
                store::shc_online_lend_keep(keep.as_mut_ptr().cast(), store::KEEP_BYTES),
            )
        };
        if rs != 0 || rk != 0 {
            return Err(Error::Refused(format!("the compiler refused its regions (scratch {rs}, keep {rk})")));
        }
        Ok(s)
    });
    match made {
        Ok(s) => {
            println!(
                "yakogl-mali-panthor: {} {} {}.{}.{}, gpu_id {:#010x}, tiler_features {:#x}, csif cs_reg_count {} unpreserved {}; vm {}, group {}, tiler heap ctx {:#x} first chunk {:#x} chunk {:#x}",
                path,
                s.dev.driver.name,
                s.dev.driver.major,
                s.dev.driver.minor,
                s.dev.driver.patch,
                s.dev.gpu.gpu_id,
                s.dev.gpu.tiler_features,
                s.dev.csif.cs_reg_count,
                s.dev.csif.unpreserved_cs_reg_count,
                s.vm.id,
                s.group.handle,
                s.heap.ctx_va,
                s.heap.first_chunk_va,
                s.heap.chunk_size
            );
            g.0 = Some(s);
            0
        }
        Err(e) => {
            say("init", &e);
            -ENODEV
        }
    }
}

unsafe extern "C" fn o_alloc(_heap: u64, size: usize, flags: u32, out: *mut OsalGpuMem) -> i32 {
    if out.is_null() || size == 0 {
        return -EINVAL;
    }
    let map = if flags & OSAL_MEM_GPU_EXEC != 0 { Map::Code } else { Map::Data };
    let r = with(|s| {
        let mut bo = Bo::new(s.vm, if map == Map::Code { "osal code" } else { "osal data" }, size as u64, map, 4096)?;
        let id = s.next_bo;
        s.next_bo += 1;
        let m = OsalGpuMem {
            cpu: bo.bytes().as_mut_ptr().cast(),
            gpu: bo.va(),
            /* the dev-only identity `map_phys` reads (module docs) */
            phys: bo.va(),
            size: bo.size as usize,
            impl_: id as *mut c_void,
        };
        s.live += 1;
        s.bytes += bo.size;
        s.live_high_water = s.live_high_water.max(s.live);
        s.bytes_high_water = s.bytes_high_water.max(s.bytes);
        s.bos.insert(id, bo);
        Ok::<_, Error>(m)
    });
    match r {
        Some(Ok(m)) => {
            // SAFETY: the caller's out-parameter.
            unsafe { *out = m };
            0
        }
        Some(Err(e)) => {
            say(&format!("gpu_alloc of {size} bytes"), &e);
            -libc::ENOMEM
        }
        None => -ENODEV,
    }
}

unsafe extern "C" fn o_free(mem: *mut OsalGpuMem) -> i32 {
    // SAFETY: the caller's descriptor.
    let Some(m) = (unsafe { mem.as_mut() }) else { return -EINVAL };
    if m.impl_.is_null() {
        return 0;
    }
    let id = m.impl_ as usize;
    /* the GL core frees an object's storage right after the backend saw
     * its last draw, which may sit in the frame being recorded or in one in
     * flight: the BO is held until the kick being recorded now (the newest
     * that may read it) has completed, as every OSAL that defers frees
     * holds it (src/osal/pending_frees.h, zephyr.c z_gpu_free) */
    let freed = with(|s| {
        let gone = match s.bos.remove(&id) {
            Some(bo) => Some(Freed::Bo(bo)),
            None => s.windows.remove(&id).map(|a| Freed::Window { _mapping: a }),
        };
        gone.map(|f| {
            s.pending.push(f, hold_fence(s.submitted));
            drain_pending(s);
        })
    });
    *m = OsalGpuMem::ZERO;
    match freed {
        Some(Some(())) => 0,
        _ => -EINVAL,
    }
}

/// What `gpu_free` holds until its fence: a buffer, or a `map_phys`
/// window (the GPU may still read through either).
enum Freed {
    Bo(Bo<'static>),
    /// Held for its Drop, which unmaps it.
    Window {
        _mapping: Alias<'static>,
    },
}

/// The page-aligned part of a buffer at `[va, va + len)` that holds
/// `[phys, phys + size)`: (its offset in the buffer, its bytes, `phys`'s
/// offset in it). None when the buffer does not hold the whole range.
pub fn window_in(va: u64, len: u64, phys: u64, size: u64) -> Option<(u64, u64, u64)> {
    const PAGE: u64 = 4096;
    let end = phys.checked_add(size)?;
    if size == 0 || phys < va || end > va.checked_add(len)? {
        return None;
    }
    let first = (phys - va) & !(PAGE - 1);
    let span = (end - va).next_multiple_of(PAGE).min(len) - first;
    Some((first, span, phys - va - first))
}

unsafe extern "C" fn o_map_phys(_heap: u64, phys: u64, size: usize, flags: u32, out: *mut OsalGpuMem) -> i32 {
    if out.is_null() || size == 0 {
        return -EINVAL;
    }
    let map = if flags & OSAL_MEM_GPU_EXEC != 0 { Map::Code } else { Map::Data };
    let r = with(|s| {
        /* one VM, so the heap names no VA range of its own */
        let Some((&owner, bo, (first, span, at))) =
            s.bos.iter().find_map(|(id, bo)| Some((id, bo, window_in(bo.va(), bo.size, phys, size as u64)?)))
        else {
            return Err(Error::Refused(format!(
                "no live buffer holds [{phys:#x}, +{size:#x}) (a phys here is a gpu_alloc's own VA)"
            )));
        };
        let a = bo.alias(&format!("map_phys of osal buffer {owner}"), first, span, map)?;
        let id = s.next_bo;
        s.next_bo += 1;
        let m = OsalGpuMem {
            // SAFETY: `at` is inside the alias's `span` bytes (window_in).
            cpu: unsafe { a.cpu().add(at as usize) }.cast(),
            gpu: a.va() + at,
            phys,
            size,
            impl_: id as *mut c_void,
        };
        s.windows.insert(id, a);
        s.windows_high_water = s.windows_high_water.max(s.windows.len());
        Ok(m)
    });
    match r {
        Some(Ok(m)) => {
            // SAFETY: the caller's out-parameter.
            unsafe { *out = m };
            0
        }
        Some(Err(e)) => {
            say(&format!("map_phys of [{phys:#x}, +{size:#x})"), &e);
            -EINVAL
        }
        None => -ENODEV,
    }
}

/// The fence a buffer freed now is held to: the kick being recorded, the
/// newest that may read it (`s_last_submitted + 1` in zephyr.c z_gpu_free),
/// whether or not anything is recorded into it yet.
pub fn hold_fence(submitted: i64) -> i64 {
    submitted + 1
}

/// The highest fence whose kick and every earlier one completed: a
/// zero-timeout look at the oldest fences not yet seen signalled.
fn completed_now(s: &mut State) -> i64 {
    loop {
        let Some((&id, f)) = s.fences.iter().next() else {
            s.completed = s.submitted;
            break;
        };
        match f.syncobj.wait(Duration::ZERO) {
            Ok(Waited::Signalled(_)) => {
                s.fences.remove(&id);
            }
            _ => {
                s.completed = id - 1;
                break;
            }
        }
    }
    s.completed
}

/// Give back every held buffer whose fence has completed (it is then
/// unbound, unmapped and closed).
fn drain_pending(s: &mut State) {
    let done = completed_now(s);
    for f in s.pending.drain(done) {
        if let Freed::Bo(bo) = f {
            s.live -= 1;
            s.bytes -= bo.size;
        }
    }
}

/// Things held until a fence completes, oldest first (the queue
/// src/osal/pending_frees.h keeps for the Zephyr OSALs).
#[derive(Debug)]
pub struct Held<T> {
    items: Vec<(T, i64)>,
    high_water: usize,
}

impl<T> Default for Held<T> {
    fn default() -> Self {
        Held { items: Vec::new(), high_water: 0 }
    }
}

impl<T> Held<T> {
    /// Hold `t` until fence `fence` has completed.
    pub fn push(&mut self, t: T, fence: i64) {
        self.items.push((t, fence));
        self.high_water = self.high_water.max(self.items.len());
    }

    /// Take out every item whose fence is at or below `completed`.
    pub fn drain(&mut self, completed: i64) -> Vec<T> {
        let (gone, kept): (Vec<_>, Vec<_>) = self.items.drain(..).partition(|(_, f)| *f <= completed);
        self.items = kept;
        gone.into_iter().map(|(t, _)| t).collect()
    }

    /// Held now, and at most.
    pub fn counts(&self) -> (usize, usize) {
        (self.items.len(), self.high_water)
    }
}

unsafe extern "C" fn o_cache(_cpu: *mut c_void, _n: usize) {
    fence(Ordering::SeqCst);
}

unsafe extern "C" fn o_submit(kick: *const c_void, len: usize) -> i64 {
    if kick.is_null() || len != core::mem::size_of::<OsalMaliKick>() {
        return -i64::from(EINVAL);
    }
    // SAFETY: the backend's OsalMaliKick of the size checked.
    let k = unsafe { *kick.cast::<OsalMaliKick>() };
    fence(Ordering::SeqCst);
    let r = with(|s| {
        let f = s.group.submit(k.stream_gpu, k.stream_bytes)?;
        s.submitted += 1;
        s.fences.insert(s.submitted, f);
        Ok::<_, Error>(s.submitted)
    });
    match r {
        Some(Ok(id)) => id,
        Some(Err(e)) => {
            say(&format!("submit of {} bytes at {:#x}", k.stream_bytes, k.stream_gpu), &e);
            -i64::from(EIO)
        }
        None => -i64::from(ENODEV),
    }
}

unsafe extern "C" fn o_wait(id: i64, timeout_us: i64) -> i32 {
    let r = with(|s| {
        let Some(f) = s.fences.get(&id) else {
            /* waited for already, or never submitted */
            return Ok(if id <= s.submitted { 0 } else { -EINVAL });
        };
        let t = Duration::from_micros(timeout_us.max(0) as u64);
        match s.group.wait(f, t)? {
            (Waited::Signalled(_), _) => {
                s.fences.remove(&id);
                drain_pending(s);
                Ok(0)
            }
            (Waited::TimedOut, _) => Ok(1),
        }
    });
    fence(Ordering::SeqCst);
    match r {
        Some(Ok(rc)) => rc,
        Some(Err(e)) => {
            say(&format!("wait for fence {id}"), &e);
            /* the job is lost: its fence is not waited for again */
            with(|s| s.fences.remove(&id));
            -EIO
        }
        None => -ENODEV,
    }
}

unsafe extern "C" fn o_completed() -> i64 {
    with(|s| {
        drain_pending(s);
        s.completed
    })
    .unwrap_or(0)
}

unsafe extern "C" fn o_query(what: u32, out: *mut c_void, sz: usize) -> i32 {
    if out.is_null() {
        return -EINVAL;
    }
    let r = with(|s| match what {
        OSAL_QUERY_MALI_GPU if sz == core::mem::size_of::<OsalMaliGpu>() => {
            let g = OsalMaliGpu {
                gpu_id: s.dev.gpu.gpu_id,
                tiler_features: s.dev.gpu.tiler_features,
                cs_reg_count: s.dev.csif.cs_reg_count,
                unpreserved_cs_reg_count: s.dev.csif.unpreserved_cs_reg_count,
            };
            // SAFETY: an out-parameter of this size.
            unsafe { *out.cast::<OsalMaliGpu>() = g };
            0
        }
        OSAL_QUERY_MALI_TILER_HEAP if sz == core::mem::size_of::<OsalMaliHeap>() => {
            let h = OsalMaliHeap {
                ctx_gpu: s.heap.ctx_va,
                first_chunk_gpu: s.heap.first_chunk_va,
                chunk_size: s.heap.chunk_size,
                reserved: 0,
            };
            // SAFETY: as above.
            unsafe { *out.cast::<OsalMaliHeap>() = h };
            0
        }
        _ => -ENOTSUP,
    });
    r.unwrap_or(-ENODEV)
}

/// The log hook: the two arguments `osal::log_str` passes (`"%s"`, line).
extern "C" fn o_log(_fmt: *const c_char, line: *const c_char) {
    if line.is_null() {
        return;
    }
    // SAFETY: osal::log_str passes a NUL-terminated line.
    println!("yakogl: {}", unsafe { std::ffi::CStr::from_ptr(line) }.to_string_lossy());
}

/* `log` is a C variadic function, which stable Rust cannot define, and
 * calling a non-variadic function through a variadic pointer is undefined
 * behaviour. So the hook is a one-instruction trampoline declared to Rust as
 * the variadic foreign function it is, which tail-jumps to o_log with the
 * argument registers untouched: ABI-correct where the variadic convention
 * passes the leading pointer arguments as a non-variadic call does, AArch64
 * AAPCS64 on Linux (x0, x1) and x86-64 System V (rdi, rsi). The same device
 * as the library's test log hook (src/testutil.rs). */
#[cfg(all(target_os = "linux", target_arch = "aarch64"))]
core::arch::global_asm!(
    ".pushsection .text.yakogl_panthor_log,\"ax\",%progbits",
    ".globl yakogl_panthor_log",
    ".hidden yakogl_panthor_log",
    ".type yakogl_panthor_log,%function",
    "yakogl_panthor_log:",
    "b {log}",
    ".size yakogl_panthor_log, . - yakogl_panthor_log",
    ".popsection",
    log = sym o_log,
);
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
core::arch::global_asm!(
    ".pushsection .text.yakogl_panthor_log,\"ax\",@progbits",
    ".globl yakogl_panthor_log",
    ".hidden yakogl_panthor_log",
    ".type yakogl_panthor_log,@function",
    "yakogl_panthor_log:",
    "jmp {log}",
    ".size yakogl_panthor_log, . - yakogl_panthor_log",
    ".popsection",
    log = sym o_log,
);

#[cfg(all(target_os = "linux", any(target_arch = "x86_64", target_arch = "aarch64")))]
extern "C" {
    fn yakogl_panthor_log(fmt: *const c_char, ...);
}

#[cfg(all(target_os = "linux", any(target_arch = "x86_64", target_arch = "aarch64")))]
const LOG: Option<unsafe extern "C" fn(*const c_char, ...)> = Some(yakogl_panthor_log);
#[cfg(not(all(target_os = "linux", any(target_arch = "x86_64", target_arch = "aarch64"))))]
const LOG: Option<unsafe extern "C" fn(*const c_char, ...)> = None;

/// The table.
pub static OPS: OsalOps = OsalOps {
    init: Some(o_init),
    shutdown: None,
    gpu_alloc: Some(o_alloc),
    gpu_free: Some(o_free),
    mmu_map: None,
    map_phys: Some(o_map_phys),
    gpu_write: None,
    gpu_read: None,
    cache_flush: Some(o_cache),
    cache_invd: Some(o_cache),
    submit: Some(o_submit),
    wait: Some(o_wait),
    completed: Some(o_completed),
    reg_read: None,
    reg_write: None,
    query: Some(o_query),
    log: LOG,
    wait_vblank: None,
    present: None,
    lock: None,
    unlock: None,
    thread_id: None,
    display_busy: None,
};

/// Point the library's `g_osal` at [`OPS`] (before the first EGL or GL
/// call).
pub fn install() {
    // SAFETY: OPS is a static that outlives every use.
    unsafe { yakogl::osal::osal_set_ops(&OPS) };
}

/// The OSAL's counts so far: (submits, fences still unwaited, live BOs and
/// their high-water, live bytes and their high-water); None before `init`.
/// Buffers and windows held for a kick now and at most (`gpu_free`'s deferral); buffers
/// are counted live too until they go.
pub fn pending() -> Option<(usize, usize)> {
    with(|s| {
        drain_pending(s);
        s.pending.counts()
    })
}

pub fn counts() -> Option<(i64, usize, usize, usize, u64, u64)> {
    with(|s| (s.submitted, s.fences.len(), s.live, s.live_high_water, s.bytes, s.bytes_high_water))
}

/// The `map_phys` windows live now and at most; None before `init`.
pub fn windows() -> Option<(usize, usize)> {
    with(|s| (s.windows.len(), s.windows_high_water))
}

/// The group's state now (`DRM_IOCTL_PANTHOR_GROUP_GET_STATE`): (state,
/// fatal queues); None before `init`.
pub fn group_state() -> Option<Result<(u32, u32), Error>> {
    with(|s| s.group.state().map(|g| (g.state, g.fatal_queues)))
}

/// The VM is still usable (the kernel marks it unusable after a fault).
pub fn vm_usable() -> Option<Result<bool, Error>> {
    with(|s| s.vm.usable())
}

#[cfg(test)]
mod tests {
    use super::{hold_fence, window_in, Held};

    /// A window is the page-aligned part of the one buffer that holds the
    /// whole range, cut to the buffer; a range the buffer does not hold
    /// whole is no window.
    #[test]
    fn a_window_is_the_pages_of_the_buffer_that_holds_the_range() {
        let (va, len) = (0x0100_0000, 0x3000);
        assert_eq!(window_in(va, len, va, 0x3000), Some((0, 0x3000, 0)), "the whole buffer");
        assert_eq!(window_in(va, len, va + 0x1100, 0x100), Some((0x1000, 0x1000, 0x100)), "inside one page");
        assert_eq!(window_in(va, len, va + 0xf00, 0x200), Some((0, 0x2000, 0xf00)), "across a page boundary");
        assert_eq!(window_in(va, len, va + 0x2f00, 0x100), Some((0x2000, 0x1000, 0xf00)), "up to the end");
        assert_eq!(window_in(va, len, va + 0x2f00, 0x101), None, "past the end");
        assert_eq!(window_in(va, len, va - 1, 2), None, "before the start");
        assert_eq!(window_in(va, len, va, 0), None, "no bytes");
        assert_eq!(window_in(va, len, u64::MAX, 2), None, "wraps");
    }

    /// A buffer freed after kick 4 was submitted, while kick 5 is recorded:
    /// held to 5, so it stays when 4 completes (the frame being recorded may
    /// read it) and goes when 5 does.
    #[test]
    fn a_free_is_held_to_the_kick_being_recorded() {
        let mut h = Held::default();
        h.push("deleted texture", hold_fence(4));
        assert_eq!(h.drain(4), Vec::<&str>::new(), "kick 4 completed; the recorded kick 5 may still read it");
        assert_eq!(h.counts(), (1, 1));
        assert_eq!(h.drain(5), ["deleted texture"]);
    }

    /// A free while fence 3 is being recorded is held until 3 completes, not
    /// given back when 2 does; the high-water counts every item held at
    /// once.
    #[test]
    fn a_held_item_goes_only_once_its_fence_completes() {
        let mut h = Held::default();
        h.push("open frame's target", 3);
        h.push("in flight", 2);
        assert_eq!(h.drain(1), Vec::<&str>::new(), "nothing completed that either waits for");
        assert_eq!(h.drain(2), ["in flight"]);
        assert_eq!(h.counts(), (1, 2));
        assert_eq!(h.drain(2), Vec::<&str>::new(), "fence 3 is not complete");
        assert_eq!(h.drain(3), ["open frame's target"]);
        assert_eq!(h.counts(), (0, 2));
    }
}
