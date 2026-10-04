// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! OS/platform abstraction (`include/gles_osal.h`).
//!
//! This is the only interface the GL core and the GPU backend use to touch
//! the outside world: GPU-visible memory + MMU, cache maintenance, submit and
//! completion, registers, platform queries, logging. One implementation per
//! platform (in C, `src/osal/*.c`) fills an [`OsalOps`] table; `g_osal`
//! points at the active one.
//!
//! The table is a C ABI: an application (or the on-silicon test suite) may
//! install its own with `osal_set_ops()`.

use core::ffi::{c_char, c_void};
use core::sync::atomic::{AtomicU32, Ordering};

pub mod render;

/// A chunk of GPU-visible memory (`struct osal_gpu_mem`). `cpu` may be null
/// if the platform cannot CPU-map it (then use `gpu_write`/`gpu_read`).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct OsalGpuMem {
    pub cpu: *mut c_void,
    pub gpu: u64,
    pub phys: u64,
    pub size: usize,
    pub impl_: *mut c_void,
}

impl OsalGpuMem {
    pub const ZERO: OsalGpuMem =
        OsalGpuMem { cpu: core::ptr::null_mut(), gpu: 0, phys: 0, size: 0, impl_: core::ptr::null_mut() };

    /// The CPU mapping as bytes, or `None` when there is none.
    ///
    /// # Safety
    /// The allocation must be live, and the GPU must not be writing it.
    pub unsafe fn bytes_mut<'a>(&self) -> Option<&'a mut [u8]> {
        if self.cpu.is_null() {
            None
        } else {
            Some(core::slice::from_raw_parts_mut(self.cpu as *mut u8, self.size))
        }
    }
}

impl Default for OsalGpuMem {
    fn default() -> Self {
        Self::ZERO
    }
}

/* enum osal_mem_flags */
pub const OSAL_MEM_DATA: u32 = 0;
pub const OSAL_MEM_CPU_CACHED: u32 = 1 << 0;
pub const OSAL_MEM_GPU_RO: u32 = 1 << 1;
pub const OSAL_MEM_UNCACHED: u32 = 1 << 2;

/// The longest line glog! hands the OSAL's log(), NUL included
/// (`OSAL_LOG_LINE_MAX` in gles_osal.h; the test below pins the two). An OSAL
/// that formats into its own buffer sizes it this (src/osal/osal_log.h).
pub const OSAL_LOG_LINE_MAX: usize = 256;

/* query() ids (osal_internal.h) */
/// out = [`render::PvrRenderStatic`].
pub const OSAL_QUERY_RENDER_STATIC: u32 = 1;
/// out = `u32`: the V3D's VPM in bytes, as the OSAL identified it.
pub const OSAL_QUERY_V3D_VPM_BYTES: u32 = 2;
/// in/out = [`OsalV3dCpuOf`]: the CPU view of V3D memory at a GPU address.
pub const OSAL_QUERY_V3D_CPU_OF_GPU: u32 = 3;
/// out = [`OsalV3dBinPool`]: the binner's pool after the last job's binning.
pub const OSAL_QUERY_V3D_BIN_POOL: u32 = 4;
/// out = `u32`: the draws one V3D job is sized for.
pub const OSAL_QUERY_V3D_DRAWS_PER_JOB: u32 = 5;
/// out = `u32`: the bytes of tiled texture copies the V3D may keep.
pub const OSAL_QUERY_V3D_TILED_BUDGET: u32 = 6;
/// in = `u32` through the out pointer: 1 when the V3D backend opens a job, 0
/// when none is open; the OSAL frees at once without an open job.
pub const OSAL_QUERY_V3D_JOB_OPEN: u32 = 7;
/// out = `u32`: 1 when the V3D backend keeps depth and stencil apart
/// (`v3d_split_zs`), read at init.
pub const OSAL_QUERY_V3D_SPLIT_ZS: u32 = 8;
/// in = `u32` through the out pointer: 1 when the GC7000 backend opens a
/// frame, 0 once it has submitted it; without an open frame the OSAL holds a
/// free only for the last submitted kick (osal_internal.h).
pub const OSAL_QUERY_GC_FRAME_OPEN: u32 = 9;
/// out = `u64`: a monotonic clock in nanoseconds; the V3D backend times its
/// CPU texture tiling with it (`v3d_tiling_time`).
pub const OSAL_QUERY_V3D_CLOCK_NS: u32 = 10;
/// out = `u32`: the smallest tiled image the V3D backend hands to the TFU
/// (0 or no answer: none; the CPU tiles everything).
pub const OSAL_QUERY_V3D_TFU_MIN_BYTES: u32 = 11;
/// in/out = [`OsalV3dTfu`]: run one TFU job; -ENOTSUP without TFU jobs.
pub const OSAL_QUERY_V3D_TFU_RUN: u32 = 12;
/// out = `u32`: 1 when the V3D backend keeps stencil halves in S8 buffers
/// (`V3dState::s8_stencil`), read at init.
pub const OSAL_QUERY_V3D_S8_STENCIL: u32 = 13;

/// `struct osal_v3d_tfu` (osal_internal.h): one TFU job's registers, in
/// `struct v3d_tfu_job`'s order, and its result and time.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct OsalV3dTfu {
    pub icfg: u32,
    pub iia: u32,
    pub iis: u32,
    pub ica: u32,
    pub iua: u32,
    pub ioa: u32,
    pub ios: u32,
    pub coef0: u32,
    pub rc: i32,
    pub us: u32,
}
const _: () = assert!(core::mem::size_of::<OsalV3dTfu>() == 40);

/// `struct osal_v3d_bin_pool` (osal_internal.h): the raw registers after the
/// last job that binned without error, and the pool it was given.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct OsalV3dBinPool {
    pub jobs: u32,
    pub pool: u32,
    pub pool_size: u32,
    pub bpca: u32,
    pub bpcs: u32,
    pub qma: u32,
    pub qms: u32,
    /// Overflow blocks the job took (each time the pool ran out); with any
    /// taken, `bpca` points into the last one, not into the pool.
    pub overflow_used: u32,
    /// The overflow blocks there are, and their size.
    pub overflow_blocks: u32,
    pub overflow_block_size: u32,
}
const _: () = assert!(core::mem::size_of::<OsalV3dBinPool>() == 40);

/// `struct osal_v3d_cpu_of` (osal_internal.h).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct OsalV3dCpuOf {
    pub gpu: u64,
    pub bytes: u64,
    pub cpu: *mut core::ffi::c_void,
}
#[cfg(target_pointer_width = "64")]
const _: () = assert!(core::mem::size_of::<OsalV3dCpuOf>() == 24);

/* GPU-MMU L0 (leaf) PTE flags (osal_internal.h) */
pub const OSAL_L0_SLC_BYPASS: u64 = 0x0000_0000_0000_0008;
pub const OSAL_L0_PM_META_PROTECT: u64 = 0x4000_0000_0000_0000;
pub const OSAL_L0_FLAGS_DATA: u64 = OSAL_L0_SLC_BYPASS;

/// `struct osal_ops`. Functions return 0 on success and a negative errno on
/// failure unless noted. Optional entries may be `None`.
#[repr(C)]
pub struct OsalOps {
    pub init: Option<unsafe extern "C" fn(platform_arg: *mut c_void) -> i32>,
    pub shutdown: Option<unsafe extern "C" fn()>,
    pub gpu_alloc: Option<unsafe extern "C" fn(heap_base: u64, size: usize, flags: u32, out: *mut OsalGpuMem) -> i32>,
    /// 0, or a negative errno when the OSAL refused the free (see [`gpu_free`]).
    pub gpu_free: Option<unsafe extern "C" fn(mem: *mut OsalGpuMem) -> i32>,
    pub mmu_map: Option<unsafe extern "C" fn(gpu_va: u64, phys: u64, size: usize, flags: u32) -> i32>,
    pub map_phys:
        Option<unsafe extern "C" fn(heap_base: u64, phys: u64, size: usize, flags: u32, out: *mut OsalGpuMem) -> i32>,
    pub gpu_write: Option<unsafe extern "C" fn(mem: *mut OsalGpuMem, off: usize, src: *const c_void, n: usize) -> i32>,
    pub gpu_read: Option<unsafe extern "C" fn(mem: *mut OsalGpuMem, off: usize, dst: *mut c_void, n: usize) -> i32>,
    pub cache_flush: Option<unsafe extern "C" fn(cpu: *mut c_void, n: usize)>,
    pub cache_invd: Option<unsafe extern "C" fn(cpu: *mut c_void, n: usize)>,
    /// Returns a fence id (>= 0) or a negative errno.
    pub submit: Option<unsafe extern "C" fn(kick_desc: *const c_void, kick_desc_len: usize) -> i64>,
    /// 0 on completion, > 0 on timeout, < 0 on error/HWR.
    pub wait: Option<unsafe extern "C" fn(fence: i64, timeout_us: i64) -> i32>,
    /// The highest completed fence, without waiting (0: none yet).
    pub completed: Option<unsafe extern "C" fn() -> i64>,
    pub reg_read: Option<unsafe extern "C" fn(off: u32) -> u32>,
    pub reg_write: Option<unsafe extern "C" fn(off: u32, val: u32)>,
    pub query: Option<unsafe extern "C" fn(what: u32, out: *mut c_void, out_sz: usize) -> i32>,
    /// printf-style. Rust callers only ever pass `"%s"` + one string (see
    /// [`log_str`]); the variadic type is the C contract.
    pub log: Option<unsafe extern "C" fn(fmt: *const c_char, ...)>,
    /// Block until the display starts a new scan-out period, so that
    /// `eglSwapBuffers` can honour its swap interval (EGL 1.5 3.10.3; the
    /// default is one vertical period per swap). 0, -ETIMEDOUT or -ENOTSUP.
    ///
    /// None on a platform with no vertical sync: presentation is then
    /// immediate, as it was everywhere before this op.
    pub wait_vblank: Option<unsafe extern "C" fn(timeout_us: i64) -> i32>,
    /// A mutex the library takes around each EGL entry point. It does NOT
    /// make a second thread safe: one thread issues every GL and EGL call
    /// (`gles_osal.h`, "threads"), and no GL entry point takes it. Not
    /// recursive. None on the Zephyr and bare-metal OSALs, together with
    /// `unlock`; a table with only one of the two is treated as having
    /// neither.
    pub lock: Option<unsafe extern "C" fn()>,
    pub unlock: Option<unsafe extern "C" fn()>,
    /// A non-zero number that names the calling thread, the same number
    /// every time on one thread and different on two live threads; 0 when
    /// the platform cannot tell. The library uses it only to refuse calls
    /// from a second thread (see [`crate::gl::context::foreign_thread`]).
    /// None: the check is off.
    pub thread_id: Option<unsafe extern "C" fn() -> u32>,
}

/* The platform OSALs stay in C (src/osal/{zephyr,baremetal,linux}.c): they
 * are thin glue over C APIs (Zephyr macros and inlines, weak board hooks, a
 * variadic log) that stable Rust cannot define. Each exports its ops table;
 * the build selects one as the compile-time default of g_osal. With none
 * selected g_osal starts null and the application calls osal_set_ops() first. */
#[allow(dead_code)]
extern "C" {
    static osal_zephyr_ops: OsalOps;
    static osal_baremetal_ops: OsalOps;
    static osal_linux_ops: OsalOps;
}

#[cfg(feature = "osal-zephyr")]
const DEFAULT_OPS: *const OsalOps = &raw const osal_zephyr_ops;
#[cfg(all(feature = "osal-baremetal", not(feature = "osal-zephyr")))]
const DEFAULT_OPS: *const OsalOps = &raw const osal_baremetal_ops;
#[cfg(all(feature = "osal-linux", not(feature = "osal-zephyr"), not(feature = "osal-baremetal")))]
const DEFAULT_OPS: *const OsalOps = &raw const osal_linux_ops;
#[cfg(not(any(feature = "osal-zephyr", feature = "osal-baremetal", feature = "osal-linux")))]
const DEFAULT_OPS: *const OsalOps = core::ptr::null();

/// The active platform ops (`extern const struct osal_ops *g_osal;`).
#[no_mangle]
pub static mut g_osal: *const OsalOps = DEFAULT_OPS;

#[no_mangle]
pub unsafe extern "C" fn osal_set_ops(ops: *const OsalOps) {
    g_osal = ops;
}

/// The active ops table, or `None` before one is installed.
#[inline]
pub fn ops() -> Option<&'static OsalOps> {
    // SAFETY: g_osal is null or points at a table that outlives its use
    // (a static, per the osal_set_ops contract).
    unsafe { g_osal.as_ref() }
}

/* ---- wrappers the core uses ------------------------------------------------ */

/// Allocate GPU memory in `heap`. Returns 0 or a negative errno.
pub fn gpu_alloc(heap: u64, size: usize, flags: u32, out: &mut OsalGpuMem) -> i32 {
    match ops().and_then(|o| o.gpu_alloc) {
        // SAFETY: `out` is a valid exclusive reference.
        Some(f) => unsafe { f(heap, size, flags, out) },
        None => -crate::util::ENODEV,
    }
}

/// Frees the OSAL refused since the library loaded (see [`gpu_free`]).
static REFUSED_FREES: AtomicU32 = AtomicU32::new(0);

/// Release an allocation (no-op for an empty `mem`); `mem` is cleared.
///
/// The OSAL may refuse the free with a negative errno: its pending-free
/// queue was full and the bounded wait for the oldest kick failed, or was not
/// opted in (`src/osal/pending_frees.h`). The memory is then kept for good.
/// The GL object is deleted all the same (a delete has no error to report),
/// and every caller here is such a delete, so the refusal stops here: it is
/// counted ([`refused_frees`]) and the first one is logged. The OSAL counts
/// it too and prints its queue's counters at close.
pub fn gpu_free(mem: &mut OsalGpuMem) {
    if let Some(f) = ops().and_then(|o| o.gpu_free) {
        let size = mem.size;
        // SAFETY: `mem` is a valid exclusive reference.
        let rc = unsafe { f(mem) };
        if rc < 0 && REFUSED_FREES.fetch_add(1, Ordering::Relaxed) == 0 {
            crate::glog!(
                "osal: a free of {size} bytes was refused ({rc}); the memory is kept, later refusals are only counted"
            );
        }
    }
}

/// Live pool memory of one owner, for the run-end owner lines
/// (`CONFIG_YAKOGL_PVR_POOL_OWNERS`, driver/docs/pvr-render-pool.md):
/// blocks and page-rounded bytes per device-VA heap, the heap read from the
/// block's address (the backend's GLES_HEAP_* bases, each a 1 GiB window).
#[derive(Clone, Copy, Default)]
pub struct OwnerTally {
    pub blocks: u32,
    /// [data (general), program (pds), shader (usc), elsewhere]
    pub bytes: [u64; 4],
}

impl OwnerTally {
    /// Count `m` when it is a live allocation of the pool's (callers pass
    /// only memory the library allocated, not a mapped window).
    pub fn add(&mut self, m: &OsalGpuMem) {
        if m.impl_.is_null() || m.size == 0 {
            return;
        }
        let pages = (m.size as u64).div_ceil(4096) * 4096;
        let heaps =
            [crate::backend::GLES_HEAP_DATA, crate::backend::GLES_HEAP_PROGRAM, crate::backend::GLES_HEAP_SHADER];
        let i = heaps
            .iter()
            .position(|&h| {
                let base = crate::backend::heap_base(h);
                base != 0 && m.gpu >= base && m.gpu - base < 0x4000_0000
            })
            .unwrap_or(3);
        self.blocks += 1;
        self.bytes[i] += pages;
    }

    /// Fold another tally into this one.
    pub fn merge(&mut self, o: &OwnerTally) {
        self.blocks += o.blocks;
        for (a, b) in self.bytes.iter_mut().zip(o.bytes) {
            *a += b;
        }
    }

    pub fn kib(&self) -> u64 {
        self.bytes.iter().sum::<u64>() / 1024
    }

    /// The owner's line, "pool owner <name>: N blocks, K KiB (general a,
    /// pds b, usc c, elsewhere d)"; returns K.
    pub fn report(&self, name: &str) -> u64 {
        let k = |i: usize| self.bytes[i] / 1024;
        crate::glog!(
            "pool owner {}: {} blocks, {} KiB (general {}, pds {}, usc {}, elsewhere {})",
            name,
            self.blocks,
            self.kib(),
            k(0),
            k(1),
            k(2),
            k(3)
        );
        self.kib()
    }
}

/// How many frees the OSAL refused (see [`gpu_free`]).
pub fn refused_frees() -> u32 {
    REFUSED_FREES.load(Ordering::Relaxed)
}

/// The lock and unlock pair, when the platform has both (see [`OsalOps`]).
/// A table with one of the two is treated as having neither: half a mutex
/// deadlocks the first thread that meets it.
fn lock_ops() -> Option<(unsafe extern "C" fn(), unsafe extern "C" fn())> {
    let o = ops()?;
    Some((o.lock?, o.unlock?))
}

/// Whether this platform supplies the EGL entry-point lock.
pub fn is_locking() -> bool {
    lock_ops().is_some()
}

/// Take the shared-state lock. Prefer [`Lock::take`], which releases it.
pub fn lock() {
    if let Some((f, _)) = lock_ops() {
        // SAFETY: the platform's mutex; the library never holds it twice.
        unsafe { f() };
    }
}

/// Release the shared-state lock taken by [`lock`].
pub fn unlock() {
    if let Some((_, f)) = lock_ops() {
        // SAFETY: as above; called once per `lock`.
        unsafe { f() };
    }
}

/// The calling thread's id from the OSAL, or 0 when the platform has no
/// `thread_id` or cannot tell (the thread check is then off).
#[inline]
pub fn thread_id() -> u32 {
    match ops().and_then(|o| o.thread_id) {
        // SAFETY: the platform's function; it takes nothing and has no
        // precondition.
        Some(f) => unsafe { f() },
        None => 0,
    }
}

/// Holds the shared-state lock for a scope. An entry point starts with
/// `let _lock = Lock::take();`, and the lock goes when it returns -- by any
/// path, including a `?` or an early return in the middle of an error check.
pub struct Lock;

impl Lock {
    pub fn take() -> Lock {
        lock();
        Lock
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        unlock();
    }
}

/// CPU writes -> memory, before the GPU reads them.
///
/// # Safety
/// `cpu..cpu+n` must be inside a live CPU mapping (or `cpu` null / `n` 0).
pub unsafe fn cache_flush(cpu: *mut c_void, n: usize) {
    if cpu.is_null() || n == 0 {
        return;
    }
    if let Some(f) = ops().and_then(|o| o.cache_flush) {
        f(cpu, n);
    }
}

/// Memory -> CPU, before the CPU reads what the GPU wrote.
///
/// # Safety
/// As [`cache_flush`].
pub unsafe fn cache_invd(cpu: *mut c_void, n: usize) {
    if cpu.is_null() || n == 0 {
        return;
    }
    if let Some(f) = ops().and_then(|o| o.cache_invd) {
        f(cpu, n);
    }
}

/// Log one already-formatted line.
/// Wait for the display to start a new scan-out period. Returns 0 when one
/// passed, or a negative errno; -ENOTSUP when the platform has no vertical
/// sync, which is what tells [`egl`](crate::gl::egl) that presentation is
/// immediate and the swap interval cannot be honoured.
pub fn wait_vblank(timeout_us: i64) -> i32 {
    match ops().and_then(|o| o.wait_vblank) {
        // SAFETY: the platform's own function pointer, called as declared.
        Some(f) => unsafe { f(timeout_us) },
        None => -crate::util::ENOTSUP,
    }
}

pub fn log_str(line: &core::ffi::CStr) {
    if let Some(f) = ops().and_then(|o| o.log) {
        // SAFETY: "%s" consumes exactly the one string argument passed.
        unsafe { f(c"%s".as_ptr(), line.as_ptr()) };
    }
}

/// `glog!("fmt {}", args)`: format with `core::fmt` and hand the line to the
/// OSAL's log hook (lines longer than [`OSAL_LOG_LINE_MAX`]` - 1` bytes are
/// truncated).
#[macro_export]
macro_rules! glog {
    ($($arg:tt)*) => {{
        use core::fmt::Write as _;
        let mut line = $crate::util::FmtBuf::<{ $crate::osal::OSAL_LOG_LINE_MAX }>::new();
        let _ = write!(line, $($arg)*);
        $crate::osal::log_str(line.as_cstr());
    }};
}

#[cfg(test)]
mod tests {
    /// glog!'s buffer and the C header's OSAL_LOG_LINE_MAX (which the Zephyr
    /// OSALs size their log line by) are the same number: a smaller C buffer
    /// cut the PowerVR depth/stencil refusal line on the AM62P.
    #[test]
    fn log_line_max_matches_the_c_header() {
        let header = include_str!("../../include/gles_osal.h");
        let value = header
            .lines()
            .find_map(|l| l.trim().strip_prefix("#define OSAL_LOG_LINE_MAX"))
            .expect("gles_osal.h defines OSAL_LOG_LINE_MAX");
        assert_eq!(value.trim().parse::<usize>(), Ok(super::OSAL_LOG_LINE_MAX));
    }

    /// Every Zephyr OSAL defines `osal_mem_line` (osal_internal.h) once,
    /// and each words it with its own prefix, so a console line names the
    /// OSAL that printed it (glmark2's "glmark2-mem:" line, one string per
    /// OSAL). The C files are not compiled on the host; this reads them.
    #[test]
    fn each_zephyr_osal_has_its_own_mem_line() {
        let osals = [
            ("zephyr_v3d.c", include_str!("zephyr_v3d.c"), "osal(v3d) mem:"),
            ("zephyr.c", include_str!("zephyr.c"), "osal(zephyr) mem:"),
            ("zephyr_gc.c", include_str!("zephyr_gc.c"), "osal(gc) mem:"),
        ];
        for (name, src, prefix) in osals {
            assert_eq!(src.matches("\nvoid osal_mem_line(char *buf, size_t n)\n").count(), 1, "{name} defines it once");
            let body = &src[src.find("void osal_mem_line(").unwrap()..];
            let body = &body[..body.find("\n}\n").expect("the function's end")];
            assert!(body.contains(&format!("\"{prefix} ")), "{name}: its line starts {prefix:?}");
            for (other, _, p) in osals.iter().filter(|o| o.0 != name) {
                assert!(!body.contains(p), "{name} uses {other}'s prefix {p:?}");
            }
        }
        let header = include_str!("osal_internal.h");
        assert!(header.contains("void osal_mem_line(char *buf, size_t n);"));
    }
}
