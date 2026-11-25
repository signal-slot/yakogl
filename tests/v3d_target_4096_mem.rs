// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Do 4096-wide V3D targets fit the Pi 4 dEQP image's default carve-out?
//! (#1261, docs/notes/v3d-large-targets.md.) Since 6d2e7d0d the V3D back end
//! draws targets up to 4096 a side: dEQP's shader executor (Pi 4 images
//! S03-S08) draws 4096 x 2-6 targets and the fbo GL_MAX_RENDERBUFFER_SIZE
//! cases a 4096 x 4096 one. The rig is tests/es3_large_target_v3d.rs's (the
//! real GL core and V3D back end, job builder and GPU stood in for) with a
//! platform whose allocator holds the board's carve-out: the heap is
//! `CONFIG_YAKOGL_V3D_CARVEOUT_BYTES` of driver/samples/zephyr/deqp/boards/rpi_4b.conf
//! (50,331,648 bytes, 48 MiB, also zephyr/Kconfig's default) less the page
//! table and scratch page at its front (src/osal/zephyr_v3d.c) and the
//! binner's two overflow blocks; blocks are counted in whole 256-byte
//! granules and a request past what is free answers -ENOMEM as
//! `fail_alloc_from` does in src/v3d/backend_tests.rs. Fragmentation is not
//! modelled, so a fit here is a lower bound on the need.
//!
//! Asserted, each with no refusal, no GL error and a read-back of the clear
//! and of the draw:
//! (a) a 4096 x 4 RGBA8 target (the shader executor's shape);
//! (b) a 4096 x 4096 RGBA8 colour target with a DEPTH24_STENCIL8
//!     attachment, depth test on. Colour alone is 64 MiB, so it does not fit
//!     48 MiB: the test is ignored. The same draw with an unlimited heap
//!     peaks at 153,286,144 bytes (a carve-out of 154,338,816, 147 MiB);
//!     `the_4096_square_target_needs` measures it and asserts the default
//!     carve-out is short of it.
//!
//! The high-water (granule-rounded bytes in use, the page table not
//! counted) is part of each assertion message and is printed with
//! `--nocapture`.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d_target_4096_mem -- --include-ignored
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-v3d", feature = "host-test", not(feature = "report-es2")))]

use core::ffi::{c_char, c_int, c_void, CStr};
use std::collections::BTreeMap;
use std::ffi::CString;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, Once, OnceLock};
use yakogl::gl::buffer::glBindBuffer;
use yakogl::gl::consts::*;
use yakogl::gl::context;
use yakogl::gl::draw::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::{glEnableVertexAttribArray, glVertexAttribPointer};
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps, OsalV3dCpuOf};
use yakogl::v3d::backend::{v3d_refused_draws, yakogl_backend_v3d};
use yakogl::v3d::ffi::*;
use yakogl::v3d::kick::V3dKick;

/* ---- the job builder and the GPU: records, fills the raster store ---- */

#[derive(Default)]
struct Seen {
    jobs: Vec<V3dJobTarget>,
    log: Vec<String>,
    /// Raster stores the stand-in GPU would have written past their allocation.
    overruns: u32,
}

static SEEN: Mutex<Seen> = Mutex::new(Seen { jobs: Vec::new(), log: Vec::new(), overruns: 0 });

fn seen<R>(f: impl FnOnce(&mut Seen) -> R) -> R {
    f(&mut SEEN.lock().unwrap_or_else(|e| e.into_inner()))
}

fn log_lines() -> Vec<String> {
    seen(|s| s.log.clone())
}

/// What `FS` writes, vec4(0.25, 0.5, 0.75, 1.0), as RGBA8.
const DRAW: [u8; 4] = [64, 128, 191, 255];

#[no_mangle]
unsafe extern "C" fn v3d_job_begin(
    j: *mut V3dJob,
    t: *const V3dJobTarget,
    bcl: *mut V3dCl,
    rcl: *mut V3dCl,
    ind: *mut V3dCl,
) -> bool {
    // SAFETY: the backend passes its live job, target and lists.
    let (j, t) = unsafe { (&mut *j, *t) };
    *j = V3dJob { bcl, rcl, ind, t, draws: 0, defaults: 0, broken: false, pack: V3dJobPack::ZERO };
    // SAFETY: as above.
    unsafe {
        (*bcl).used = 16;
        (*ind).used = 256;
    }
    seen(|s| s.jobs.push(t));
    t.width != 0 && t.height != 0
}

#[no_mangle]
unsafe extern "C" fn v3d_job_draw(j: *mut V3dJob, _d: *const V3dJobDraw, _layout: *mut c_void) -> i32 {
    // SAFETY: the backend passes its live job.
    unsafe {
        (*j).draws += 1;
        (*(*j).bcl).used += 120;
    }
    V3D_JOB_OK
}

/// `w` x `h` pixels of `px` at `addr`, `stride` bytes a row, inside the
/// platform allocation that holds `addr`.
fn fill(addr: u32, stride: u32, w: u32, h: u32, px: [u8; 4]) {
    if addr == 0 || w == 0 || h == 0 {
        return;
    }
    let ok = with_platform(|p| match p.allocs.range_mut(..=addr as u64).next_back() {
        Some((base, a)) => {
            let off = (addr as u64 - base) as usize;
            let last = off + (h as usize - 1) * stride as usize + w as usize * 4;
            if last > a.size {
                return false;
            }
            // SAFETY: the allocation is `size` bytes of u64s and `last` is inside it.
            let bytes = unsafe { core::slice::from_raw_parts_mut(a.mem.as_mut_ptr() as *mut u8, a.size) };
            for y in 0..h as usize {
                for x in 0..w as usize {
                    let o = off + y * stride as usize + x * 4;
                    bytes[o..o + 4].copy_from_slice(&px);
                }
            }
            true
        }
        None => true,
    });
    if !ok {
        seen(|s| s.overruns += 1);
    }
}

#[no_mangle]
unsafe extern "C" fn v3d_job_end(
    j: *mut V3dJob,
    c: *const V3dJobClear,
    _tile_list_start: *mut u32,
    _tile_list_end: *mut u32,
) -> bool {
    // SAFETY: the backend passes its live job and clear; its lists are live.
    let (t, c, drew) = unsafe {
        (*(*j).bcl).used += 1;
        (*(*j).rcl).used = 200;
        ((*j).t, *c, (*j).draws > 0)
    };
    /* RGBA8, RT 0 only (this file's targets) */
    if !c.load_rt0 {
        fill(t.address, t.stride, t.width, t.height, c.rgba8.to_le_bytes());
    }
    if drew {
        fill(t.address, t.stride, t.width, t.height, DRAW);
    }
    true
}

#[no_mangle]
unsafe extern "C" fn v3d_frame_tiling(width: u32, height: u32, t: *mut V3dTiling) {
    // SAFETY: the backend's out-parameter, passed on.
    unsafe { v3d_frame_tiling_bpp(width, height, 1, false, 0, t) }
}

#[no_mangle]
unsafe extern "C" fn v3d_frame_tiling_rts(width: u32, height: u32, rts: u32, t: *mut V3dTiling) {
    // SAFETY: the backend's out-parameter, passed on.
    unsafe { v3d_frame_tiling_bpp(width, height, rts, false, 0, t) }
}

#[no_mangle]
unsafe extern "C" fn v3d_frame_tiling_ms(width: u32, height: u32, rts: u32, msaa: bool, t: *mut V3dTiling) {
    // SAFETY: the backend's out-parameter, passed on.
    unsafe { v3d_frame_tiling_bpp(width, height, rts, msaa, 0, t) }
}

/// The tile-size table of v3d_frame.c, as the backend's unit tests fake it.
#[no_mangle]
unsafe extern "C" fn v3d_frame_tiling_bpp(
    width: u32,
    height: u32,
    rts: u32,
    msaa: bool,
    max_bpp: u32,
    t: *mut V3dTiling,
) {
    const SIZES: [(u32, u32); 7] = [(64, 64), (64, 32), (32, 32), (32, 16), (16, 16), (16, 8), (8, 8)];
    let idx = match rts {
        0 | 1 => 0,
        2 => 1,
        _ => 2,
    } + if msaa { 2 } else { 0 }
        + max_bpp.min(2) as usize;
    let (tw, th) = SIZES[idx];
    let tiles = V3dTiling {
        tile_w: tw,
        tile_h: th,
        tiles_x: width.div_ceil(tw),
        tiles_y: height.div_ceil(th),
        supertile_w: 1,
        supertile_h: 1,
        frame_w_supertiles: width.div_ceil(tw),
        frame_h_supertiles: height.div_ceil(th),
    };
    // SAFETY: the backend's out-parameter.
    unsafe { *t = tiles };
}

#[no_mangle]
unsafe extern "C" fn v3d_frame_alloc_sizes(t: *const V3dTiling, _d: u32, _page: u32, ta: *mut u32, ts: *mut u32) {
    // SAFETY: the backend's parameters.
    unsafe {
        let n = (*t).tiles_x * (*t).tiles_y;
        *ta = n * 128 + 8192;
        *ts = n * 256;
    }
}

extern "C" {
    fn dlopen(path: *const c_char, flags: c_int) -> *mut c_void;
    fn dlsym(h: *mut c_void, name: *const c_char) -> *mut c_void;
}

/// `driver/src/v3d/v3d_tex.c` and the Mesa tiling it includes, as a shared
/// object of this test's own, reached through the handle only (RTLD_LOCAL).
fn lib() -> *mut c_void {
    static L: OnceLock<usize> = OnceLock::new();
    *L.get_or_init(|| {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("v3d_target_4096_mem");
        std::fs::create_dir_all(&dir).expect("the scratch directory");
        let so = dir.join(format!("v3d_tex-{}.so", std::process::id()));
        /* the compiler's own temporary files next to the object */
        let st = Command::new("cc")
            .current_dir(&root)
            .env("TMPDIR", &dir)
            .args(["-std=c11", "-O1", "-shared", "-fPIC", "-Wall", "-Wextra", "-Werror"])
            .args(["-Idriver/src/v3d", "-Idriver/src/v3d/generated"])
            .args(["-Ithird_party/mesa-26.2.1/src/broadcom", "-Ithird_party/mesa-26.2.1/src"])
            .arg("-Ithird_party/mesa-26.2.1/include")
            .args(["driver/src/v3d/v3d_tex.c", "driver/src/v3d/v3d_tiling_mesa.c", "-lm", "-o"])
            .arg(&so)
            .status()
            .expect("cc runs");
        assert!(st.success(), "driver/src/v3d/v3d_tex.c builds");
        let path = CString::new(so.to_str().expect("a utf-8 path")).unwrap();
        // SAFETY: a path of a library this test just built; RTLD_NOW.
        let h = unsafe { dlopen(path.as_ptr(), 2) };
        assert!(!h.is_null(), "the layout code loads");
        let _ = std::fs::remove_file(&so);
        h as usize
    }) as *mut c_void
}

fn sym(name: &str) -> *mut c_void {
    let c = CString::new(name).unwrap();
    // SAFETY: a handle `lib` made and a NUL-terminated name.
    let p = unsafe { dlsym(lib(), c.as_ptr()) };
    assert!(!p.is_null(), "{name} in v3d_tex.c");
    p
}

/// The trampolines: the library's `extern "C"` texture functions
/// (src/v3d/ffi.rs), each handed to the real C.
macro_rules! trampoline {
    ($name:ident ( $($arg:ident : $ty:ty),* ) $(-> $ret:ty)?) => {
        #[no_mangle]
        unsafe extern "C" fn $name($($arg: $ty),*) $(-> $ret)? {
            // SAFETY: the C function's own signature, as src/v3d/ffi.rs declares it.
            let f: unsafe extern "C" fn($($ty),*) $(-> $ret)? =
                unsafe { std::mem::transmute(sym(stringify!($name))) };
            // SAFETY: the library's call, passed on.
            unsafe { f($($arg),*) }
        }
    };
}

/* The signatures are src/v3d/ffi.rs's, kept as written (see
 * tests/depth_resolve_v3d.rs: rustfmt releases wrap these differently). */
#[rustfmt::skip]
mod trampolines {
    use super::*;

    trampoline!(v3d_tex_layout(width: u32, height: u32, levels: u32, cpp: u32, l: *mut V3dTexLayout) -> bool);
    trampoline!(v3d_tex_layout_msaa(width: u32, height: u32, cpp: u32, l: *mut V3dTexLayout) -> bool);
    trampoline!(v3d_tex_store_level(image: *mut u8, l: *const V3dTexLayout, level: u32, src: *const u8, stride: u32));
    trampoline!(v3d_tex_tfu_pack(l: *const V3dTexLayout, ttype: u32, src: u32, stride: u32, dst: u32,
                                 j: *mut V3dTexTfuJob) -> bool);
    trampoline!(v3d_tex_tfu_pack_mipmap(l: *const V3dTexLayout, ttype: u32, image: u32, j: *mut V3dTexTfuJob) -> bool);
    trampoline!(v3d_tex_layer_stride(l: *const V3dTexLayout) -> u32);
    trampoline!(v3d_tex_layers_size(l: *const V3dTexLayout, layers: u32, out: *mut u32) -> bool);
    trampoline!(v3d_tex_store_box(image: *mut u8, l: *const V3dTexLayout, level: u32, x: u32, y: u32, w: u32, h: u32,
                                  src: *const u8, stride: u32) -> bool);
    trampoline!(v3d_tex_load_box(image: *const u8, l: *const V3dTexLayout, level: u32, x: u32, y: u32, w: u32, h: u32,
                                 dst: *mut u8, stride: u32) -> bool);
    trampoline!(v3d_tex_state_from_layout(s: *mut V3dTexState, l: *const V3dTexLayout));
    trampoline!(v3d_tex_pack_sampler(out: *mut u8, s: *const V3dSampler) -> bool);
    trampoline!(v3d_tex_pack_sampler_compare(out: *mut u8, s: *const V3dSampler, compare_op: u32) -> bool);
}

#[no_mangle]
unsafe extern "C" fn v3d_tex_pack_state(out: *mut u8, s: *const V3dTexState) -> bool {
    // SAFETY: the C function's signature.
    let f: unsafe extern "C" fn(*mut u8, *const V3dTexState) -> bool =
        unsafe { std::mem::transmute(sym("v3d_tex_pack_state")) };
    // SAFETY: the library's call, passed on; `s` is its live state.
    unsafe { f(out, s) }
}

/* ---- the platform: memory that answers, jobs that finish at once ---- */

const GRANULE: u64 = 256;
/// The heap the carve-out leaves, in bytes (`set_heap`).
static HEAP: AtomicU64 = AtomicU64::new(u64::MAX);

/// The deqp sample's board file: every Pi 4 image's configuration.
const BOARD_CONF: &str = include_str!("../driver/samples/zephyr/deqp/boards/rpi_4b.conf");

/// `CONFIG_YAKOGL_V3D_CARVEOUT_BYTES` of rpi_4b.conf.
fn carveout() -> u64 {
    let want = "CONFIG_YAKOGL_V3D_CARVEOUT_BYTES=";
    let v = BOARD_CONF.lines().find_map(|l| l.trim().strip_prefix(want));
    v.and_then(|v| v.trim().parse().ok()).expect("rpi_4b.conf sets the carve-out")
}

/// The allocator's first byte: the GPU page table of the 512 MiB address
/// space (zephyr_v3d.c PT_BYTES) and the scratch page.
const FIRST_FREE: u64 = (0x2000_0000 / 4096) * 4 + 4096;
/// The binner's two 256 KiB overflow blocks, taken at init.
const OVERFLOW: u64 = 2 * 256 * 1024;

/// The heap a carve-out of `carve` bytes leaves.
fn heap_of(carve: u64) -> u64 {
    carve - FIRST_FREE - OVERFLOW
}

/// Limit the platform's heap and restart the high-water at what is in use.
fn set_heap(bytes: u64) {
    HEAP.store(bytes, Ordering::Relaxed);
    with_platform(|p| p.peak = p.in_use);
}

fn peak() -> u64 {
    with_platform(|p| p.peak)
}

struct Alloc {
    mem: Box<[u64]>,
    size: usize,
}
struct Platform {
    /// Granule-rounded bytes in use, and the most since `set_heap`.
    in_use: u64,
    peak: u64,
    next: u64,
    allocs: BTreeMap<u64, Alloc>,
    submitted: usize,
}
static PLATFORM: Mutex<Option<Platform>> = Mutex::new(None);

fn with_platform<R>(f: impl FnOnce(&mut Platform) -> R) -> R {
    let mut g = PLATFORM.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(|| Platform { in_use: 0, peak: 0, next: 0, allocs: BTreeMap::new(), submitted: 0 }))
}

unsafe extern "C" fn p_gpu_alloc(_heap: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
    with_platform(|p| {
        if size == 0 || size > 256 << 20 {
            return -12;
        }
        let rounded = (size as u64).div_ceil(GRANULE) * GRANULE;
        if p.in_use + rounded > HEAP.load(Ordering::Relaxed) {
            return -12;
        }
        p.in_use += rounded;
        p.peak = p.peak.max(p.in_use);
        let mut mem = vec![0u64; size.div_ceil(8)].into_boxed_slice();
        let gpu = 0x0100_0000 + p.next;
        p.next += (size as u64).div_ceil(4096) * 4096;
        let cpu = mem.as_mut_ptr() as *mut c_void;
        // SAFETY: the caller passes a writable descriptor.
        unsafe { *out = OsalGpuMem { cpu, gpu, phys: gpu, size, impl_: cpu } };
        p.allocs.insert(gpu, Alloc { mem, size });
        0
    })
}
unsafe extern "C" fn p_gpu_free(mem: *mut OsalGpuMem) -> i32 {
    // SAFETY: the caller passes its descriptor.
    let m = unsafe { *mem };
    if !m.impl_.is_null() {
        with_platform(|p| {
            if let Some(a) = p.allocs.remove(&m.gpu) {
                p.in_use -= (a.size as u64).div_ceil(GRANULE) * GRANULE;
            }
        });
    }
    0
}
unsafe extern "C" fn p_gpu_write(mem: *mut OsalGpuMem, off: usize, src: *const c_void, n: usize) -> i32 {
    // SAFETY: the caller passes its descriptor.
    let m = unsafe { *mem };
    if m.cpu.is_null() || off.checked_add(n).is_none_or(|e| e > m.size) {
        return -22;
    }
    // SAFETY: inside the allocation (checked above).
    unsafe { core::ptr::copy_nonoverlapping(src as *const u8, (m.cpu as *mut u8).add(off), n) };
    0
}
unsafe extern "C" fn p_cache(_cpu: *mut c_void, _n: usize) {}
unsafe extern "C" fn p_submit(_kick: *const c_void, len: usize) -> i64 {
    if len != size_of::<V3dKick>() {
        return -22;
    }
    with_platform(|p| {
        p.submitted += 1;
        p.submitted as i64
    })
}
unsafe extern "C" fn p_wait(_fence: i64, _timeout_us: i64) -> i32 {
    0
}
unsafe extern "C" fn p_completed() -> i64 {
    with_platform(|p| p.submitted as i64)
}
unsafe extern "C" fn p_query(what: u32, out: *mut c_void, out_sz: usize) -> i32 {
    use yakogl::osal::{OSAL_QUERY_V3D_BIN_POOL, OSAL_QUERY_V3D_CPU_OF_GPU, OSAL_QUERY_V3D_VPM_BYTES};
    if what == OSAL_QUERY_V3D_VPM_BYTES && out_sz == 4 {
        // SAFETY: a u32 out-parameter.
        unsafe { *(out as *mut u32) = 65536 };
        return 0;
    }
    if what == OSAL_QUERY_V3D_CPU_OF_GPU && out_sz == size_of::<OsalV3dCpuOf>() {
        // SAFETY: the backend's in/out struct of that size.
        let q = unsafe { &mut *(out as *mut OsalV3dCpuOf) };
        return with_platform(|p| match p.allocs.range(..=q.gpu).next_back() {
            Some((base, a)) if q.gpu + q.bytes <= base + a.size as u64 => {
                // SAFETY: inside the allocation.
                q.cpu = unsafe { (a.mem.as_ptr() as *mut u8).add((q.gpu - base) as usize) } as *mut c_void;
                0
            }
            _ => -22,
        });
    }
    if what == OSAL_QUERY_V3D_BIN_POOL {
        return -61;
    }
    -22
}
/// The library logs every line as `log("%s", line)` (`osal::log_str`).
unsafe extern "C" fn p_log2(_fmt: *const c_char, line: *const c_char) {
    // SAFETY: the library passes a NUL-terminated line.
    let s = unsafe { CStr::from_ptr(line) }.to_string_lossy().into_owned();
    seen(|x| x.log.push(s));
}
unsafe extern "C" fn p_init(_platform_arg: *mut c_void) -> i32 {
    0
}

type LogFn = unsafe extern "C" fn(*const c_char, ...);

fn osal() -> &'static OsalOps {
    // SAFETY: see p_log2: the library only ever calls it with ("%s", line).
    let log = unsafe { core::mem::transmute::<unsafe extern "C" fn(*const c_char, *const c_char), LogFn>(p_log2) };
    Box::leak(Box::new(OsalOps {
        init: Some(p_init),
        shutdown: None,
        gpu_alloc: Some(p_gpu_alloc),
        gpu_free: Some(p_gpu_free),
        mmu_map: None,
        map_phys: None,
        gpu_write: Some(p_gpu_write),
        gpu_read: None,
        cache_flush: Some(p_cache),
        cache_invd: Some(p_cache),
        submit: Some(p_submit),
        wait: Some(p_wait),
        completed: Some(p_completed),
        reg_read: None,
        reg_write: None,
        query: Some(p_query),
        log: Some(log),
        wait_vblank: None,
        present: None,
        lock: None,
        unlock: None,
        thread_id: None,
        display_busy: None,
    }))
}

/* ---- the context ---- */

static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

/// The one context and back end of the module; the tests take turns and
/// each starts with nothing recorded and nothing pending.
fn rig() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    ONCE.call_once(|| {
        // SAFETY: statics; the lock serialises every GL call of the module.
        unsafe {
            osal_set_ops(osal());
            yakogl::backend::g_backend = &yakogl_backend_v3d;
            assert_eq!(yakogl::backend::init(), 0, "{:?}", log_lines());
            assert!(!context::gl_context_create(32, 32).is_null(), "{:?}", log_lines());
        }
    });
    // SAFETY: plain GL calls.
    unsafe {
        glFinish();
        while glGetError() != GL_NO_ERROR {}
    }
    seen(|s| *s = Seen::default());
    g
}

/* ---- helpers ---- */

const VS: &str = "#version 300 es\nin vec4 p;\nvoid main() { gl_Position = p; }\n";
const FS: &str = "#version 300 es\nprecision mediump float;\nlayout(location = 0) out vec4 c;\n\
                  void main() { c = vec4(0.25, 0.5, 0.75, 1.0); }\n";

/// The error flag, drained: the first error since the last call.
fn err() -> GLenum {
    // SAFETY: plain GL calls.
    unsafe {
        let e = glGetError();
        while glGetError() != GL_NO_ERROR {}
        e
    }
}

fn shader(ty: GLenum, text: &str) -> GLuint {
    let text = CString::new(text).unwrap();
    // SAFETY: plain GL calls; one NUL-terminated source.
    unsafe {
        let s = glCreateShader(ty);
        let p: [*const c_char; 1] = [text.as_ptr()];
        glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
        glCompileShader(s);
        let mut ok: GLint = 0;
        glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
        assert_eq!(ok, 1, "{text:?} did not compile");
        s
    }
}

/// Make `FS` (with `VS`) the current program.
fn program() {
    let (v, f) = (shader(GL_VERTEX_SHADER, VS), shader(GL_FRAGMENT_SHADER, FS));
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glBindAttribLocation(p, 0, c"p".as_ptr());
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1);
        glUseProgram(p);
    }
}

fn draw_triangle() {
    static TRI: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 3.0, -1.0, 0.0, 1.0, -1.0, 3.0, 0.0, 1.0];
    // SAFETY: TRI is static and outlives the draw.
    unsafe {
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, TRI.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glDrawArrays(GL_TRIANGLES, 0, 3);
    }
}

/// The bound framebuffer's colour attachment 0, `w` x `h` RGBA8 pixels.
fn read(w: u32, h: u32) -> Vec<u8> {
    let mut px = vec![0xEEu8; (w * h * 4) as usize];
    // SAFETY: plain GL calls; `px` holds `w` x `h` RGBA8 pixels.
    unsafe {
        glReadPixels(0, 0, w as GLsizei, h as GLsizei, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr() as *mut c_void);
    }
    assert_eq!(err(), GL_NO_ERROR, "glReadPixels {w}x{h}");
    px
}

/* ---- the targets ---- */

/// A framebuffer of one `w` x `h` RGBA8 renderbuffer and, with `depth`, a
/// DEPTH24_STENCIL8 one: (framebuffer, renderbuffers, the storage's error).
fn target(w: GLsizei, h: GLsizei, depth: bool) -> (GLuint, [GLuint; 2], GLenum) {
    let (mut fb, mut rb) = (0, [0; 2]);
    let mut e = GL_NO_ERROR;
    // SAFETY: plain GL calls with room for the names.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glGenRenderbuffers(2, rb.as_mut_ptr());
        glBindRenderbuffer(GL_RENDERBUFFER, rb[0]);
        glRenderbufferStorage(GL_RENDERBUFFER, GL_RGBA8, w, h);
        e = e.max(err());
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb[0]);
        if depth {
            glBindRenderbuffer(GL_RENDERBUFFER, rb[1]);
            glRenderbufferStorage(GL_RENDERBUFFER, GL_DEPTH24_STENCIL8, w, h);
            e = e.max(err());
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_DEPTH_STENCIL_ATTACHMENT, GL_RENDERBUFFER, rb[1]);
        }
        glViewport(0, 0, w, h);
    }
    (fb, rb, e)
}

fn drop_target(fb: GLuint, rb: [GLuint; 2]) {
    // SAFETY: plain GL calls on names made here.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fb);
        glDeleteRenderbuffers(2, rb.as_ptr());
        glFinish();
    }
}

/// Clear, draw and read back a `w` x `h` target with the heap at `heap`
/// bytes: the peak (granule-rounded bytes in use) when nothing was refused,
/// else the failure text with the peak.
fn run(w: u32, h: u32, depth: bool, heap: u64) -> Result<u64, String> {
    let _ = err();
    program();
    set_heap(heap);
    seen(|s| *s = Seen::default());
    let base = v3d_refused_draws();
    let (fb, rb, e) = target(w as GLsizei, h as GLsizei, depth);
    let r = (|| {
        let what = format!("{w}x{h} depth={depth} heap={heap}");
        if e != GL_NO_ERROR {
            return Err(format!("{what}: storage error {e:#x}"));
        }
        // SAFETY: plain GL calls.
        unsafe {
            if glCheckFramebufferStatus(GL_FRAMEBUFFER) != GL_FRAMEBUFFER_COMPLETE {
                return Err(format!("{what}: incomplete"));
            }
            glClearColor(1.0, 0.0, 0.0, 1.0);
            glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT);
            if depth {
                glEnable(GL_DEPTH_TEST);
            }
            glFinish();
        }
        let ge = err();
        if ge != GL_NO_ERROR {
            return Err(format!("{what}: clear error {ge:#x}: {:?}", log_lines()));
        }
        if !read(w, h).chunks(4).all(|p| p == [255, 0, 0, 255]) {
            return Err(format!("{what}: clear read back wrong"));
        }
        draw_triangle();
        // SAFETY: plain GL calls.
        unsafe {
            glFinish();
            glDisable(GL_DEPTH_TEST);
        }
        let ge = err();
        let refused = v3d_refused_draws() - base;
        if ge != GL_NO_ERROR || refused != 0 {
            return Err(format!("{what}: draw error {ge:#x}, refused {refused}: {:?}", log_lines()));
        }
        if !read(w, h).chunks(4).all(|p| p == DRAW) {
            return Err(format!("{what}: draw read back wrong"));
        }
        let (overruns, log) = seen(|s| (s.overruns, s.log.clone()));
        if overruns != 0 || log.iter().any(|l| l.contains("draw refused") || l.contains("E2BIG")) {
            return Err(format!("{what}: overruns {overruns}: {log:?}"));
        }
        Ok(())
    })();
    let pk = peak();
    drop_target(fb, rb);
    set_heap(u64::MAX);
    r.map(|()| pk).map_err(|m| format!("{m} (high-water {pk} bytes of {heap})"))
}

/// (a) the shader executor's 4096 x 4 target fits the default carve-out.
#[test]
fn a_4096x4_target_fits_the_default_carveout() {
    let _g = rig();
    let heap = heap_of(carveout());
    let pk = run(4096, 4, false, heap).unwrap_or_else(|m| panic!("{m}"));
    println!("4096x4 RGBA8: high-water {pk} bytes of a {heap}-byte heap (carve-out {})", carveout());
    assert!(pk <= heap, "4096x4 high-water {pk} of {heap}");
}

/// (b) the fbo cases' 4096 x 4096 colour + DEPTH24_STENCIL8 target.
/// Ignored: measured need 153,286,144 bytes of heap (colour 64 MiB, depth
/// 64 MiB, the rest the tile memory), a carve-out of at least 154,338,816
/// bytes, against 50,331,648; `the_4096_square_target_needs` pins it.
#[test]
#[ignore = "4096x4096 RGBA8 + DEPTH24_STENCIL8 needs 153,286,144 bytes of heap (147 MiB; carve-out >= 154,338,816), not 48 MiB; see the_4096_square_target_needs"]
fn a_4096x4096_colour_depth_target_fits_the_default_carveout() {
    let _g = rig();
    let heap = heap_of(carveout());
    let pk = run(4096, 4096, true, heap).unwrap_or_else(|m| panic!("{m}"));
    println!("4096x4096 RGBA8+D24S8: high-water {pk} bytes of a {heap}-byte heap");
}

/// The need of (b), measured with an unlimited heap: the draw works, and
/// its high-water is past the heap of the default carve-out.
#[test]
fn the_4096_square_target_needs() {
    let _g = rig();
    let heap = heap_of(carveout());
    let pk = run(4096, 4096, true, u64::MAX).unwrap_or_else(|m| panic!("{m}"));
    let need = pk + FIRST_FREE + OVERFLOW;
    println!("4096x4096 RGBA8+D24S8: high-water {pk} bytes; carve-out needed >= {need} (have {})", carveout());
    assert!(pk > heap, "high-water {pk} fits a {heap}-byte heap: un-ignore the (b) test");
}
