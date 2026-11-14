// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! A GL_RGB16F texture as a colour attachment, through the GL API over the V3D
//! host back end (t-rgb16f-attach-v3d; docs/notes/v3d-fbo-render.md section 5,
//! D5, the 12 `fbo.render` rows `color.{tex2d,texcube,blit.tex2d,sample,
//! tex2darray,tex3d,resize,recreate_color}.rgb16f`, `shared_colorbuffer` x3,
//! `shared_colorbuffer_clear`): the real GL core, the real V3D back end, the
//! job builder faked (tests/es3_invalidate_v3d.rs's rig), so the tests see
//! the target, render-target row and load words of every job.
//!
//! A GL_RGB16F texture is held in the RGBA16F class and is a colour
//! attachment as the renderbuffer of the same format is (ES 3.0 table 3.13:
//! RGB16F is colour-renderable with GL_EXT_color_buffer_half_float): complete
//! on 2D, cube, 2D-array and 3D targets, drawing and clearing as the same
//! typed RGBA16F RT with the alpha forced to 1 at the load, a read-back whose
//! alpha is 1 (`framebuffer::attachment_storage` lets the format through and
//! `fb_target_alpha_bits_n` answers 0 for it).
//!
//! The fake GPU draws nothing, so a read-back sees the texels the test put in
//! the texture, not a rendered image; a rendered image is a board result.
//!
//! Run: `cargo test --no-default-features --features
//! host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_rgb16f_attach_v3d`.
//! It compiles to nothing under the ES 2.0 report (every case here asks for an
//! ES 3.0 format) and on a target that is not a Linux host (the rig builds C).

#![cfg(all(
    feature = "host-test",
    feature = "gpu-v3d",
    feature = "shaderc-online",
    not(feature = "report-es2"),
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
/* the fakes and trampolines take the C functions' parameter lists as they are */
#![allow(clippy::too_many_arguments)]

use core::ffi::{c_char, c_int, c_void, CStr};
use std::collections::BTreeMap;
use std::ffi::CString;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Mutex, MutexGuard, Once, OnceLock};

use yakogl::gl::consts::*;
use yakogl::gl::context::{self, GlContext};
use yakogl::gl::draw::glDrawArrays;
use yakogl::gl::es3::glClearBufferfv;
use yakogl::gl::framebuffer::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps, OsalV3dCpuOf};
use yakogl::v3d::backend::*;
use yakogl::v3d::ffi::*;
use yakogl::v3d::kick::V3dKick;

/* ---- the real layout code, built from driver/src/v3d/v3d_tex.c ------------- */

extern "C" {
    fn dlopen(path: *const c_char, flags: c_int) -> *mut c_void;
    fn dlsym(h: *mut c_void, name: *const c_char) -> *mut c_void;
}

/// `driver/src/v3d/v3d_tex.c` and the Mesa tiling it includes, as a shared
/// object of this test's own; its symbols are reached through the handle
/// only (RTLD_LOCAL), so the trampolines below are the ones the library
/// links.
fn lib() -> *mut c_void {
    static L: OnceLock<usize> = OnceLock::new();
    *L.get_or_init(|| {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("v3d_depth_storage");
        std::fs::create_dir_all(&dir).expect("the scratch directory");
        let so = dir.join(format!("v3d_tex-{}.so", std::process::id()));
        /* the compiler's own temporary files next to the object: a host
         * whose /tmp is full must not fail the check */
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
/// (src/v3d/ffi.rs), each handed to the real C. `v3d_tex_pack_state` also
/// records the state it packed, by the record's address, for the checks.
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

/* The signatures are src/v3d/ffi.rs's. Kept as written: a macro call wider
 * than the width is wrapped by some rustfmt releases and left by others, so
 * the gate's `cargo fmt --check` must not depend on the host's release. */
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

/// Every texture state packed, by the record's CPU address.
static STATES: Mutex<Vec<(usize, V3dTexState)>> = Mutex::new(Vec::new());

#[no_mangle]
unsafe extern "C" fn v3d_tex_pack_state(out: *mut u8, s: *const V3dTexState) -> bool {
    // SAFETY: the C function's signature.
    let f: unsafe extern "C" fn(*mut u8, *const V3dTexState) -> bool =
        unsafe { std::mem::transmute(sym("v3d_tex_pack_state")) };
    // SAFETY: the library's call, passed on; `s` is its live state.
    let ok = unsafe { f(out, s) };
    if ok {
        // SAFETY: the backend's live state.
        STATES.lock().unwrap_or_else(|e| e.into_inner()).push((out as usize, unsafe { *s }));
    }
    ok
}

/* ---- the job builder, stood in for (driver/src/v3d/v3d_job.c) -------------- */

#[derive(Clone, Debug)]
struct SeenJob {
    target: V3dJobTarget,
    clear: Option<V3dJobClear>,
    /// The FS texture records of each draw, in order.
    tmu: Vec<Vec<V3dJobTmu>>,
}

static JOBS: Mutex<Vec<SeenJob>> = Mutex::new(Vec::new());

fn jobs() -> MutexGuard<'static, Vec<SeenJob>> {
    JOBS.lock().unwrap_or_else(|e| e.into_inner())
}

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
    jobs().push(SeenJob { target: t, clear: None, tmu: Vec::new() });
    t.width != 0 && t.height != 0
}

#[no_mangle]
unsafe extern "C" fn v3d_job_draw(j: *mut V3dJob, d: *const V3dJobDraw, _layout: *mut c_void) -> i32 {
    // SAFETY: the backend passes its live job and a draw whose pointers live for the call.
    let (j, d) = unsafe { (&mut *j, &*d) };
    let tmu = if d.tmu.is_null() || d.num_tmu == 0 {
        Vec::new()
    } else {
        // SAFETY: `num_tmu` records, live for the call.
        unsafe { std::slice::from_raw_parts(d.tmu, d.num_tmu as usize) }.to_vec()
    };
    j.draws += 1;
    // SAFETY: the job's BCL is live.
    unsafe { (*j.bcl).used += 120 };
    if let Some(job) = jobs().last_mut() {
        job.tmu.push(tmu);
    }
    V3D_JOB_OK
}

#[no_mangle]
unsafe extern "C" fn v3d_job_end(
    j: *mut V3dJob,
    c: *const V3dJobClear,
    _tile_list_start: *mut u32,
    _tile_list_end: *mut u32,
) -> bool {
    // SAFETY: the backend passes its live job and clear.
    let (j, c) = unsafe { (&mut *j, *c) };
    // SAFETY: the job's lists are live.
    unsafe {
        (*j.bcl).used += 1;
        (*j.rcl).used = 200;
    }
    if let Some(job) = jobs().last_mut() {
        job.clear = Some(c);
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

/// The tile-size table of v3d_frame.c (Mesa's v3d_choose_tile_size), as
/// the backend's unit tests fake it.
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
unsafe extern "C" fn v3d_frame_alloc_sizes(t: *const V3dTiling, _draws: u32, _page: u32, ta: *mut u32, ts: *mut u32) {
    // SAFETY: the backend's parameters.
    unsafe {
        let n = (*t).tiles_x * (*t).tiles_y;
        *ta = n * 128 + 8192;
        *ts = n * 256;
    }
}

/* ---- the platform ------------------------------------------------------------ */

/// GPU addresses are 32-bit (the V3D's MMU), from here up.
const GPU_BASE: u64 = 0x0100_0000;

struct Alloc {
    /// u64 backing: every u32 view of it is aligned.
    mem: Box<[u64]>,
    size: usize,
}

#[derive(Default)]
struct Platform {
    next: u64,
    allocs: BTreeMap<u64, Alloc>,
    kicks: Vec<V3dKick>,
    log: Vec<String>,
    /// Bytes of the live allocations and their high-water since the last
    /// `mark_peak`: the carve-out use as the GL core and the back end make it.
    live: usize,
    peak: usize,
}

static PLATFORM: Mutex<Option<Platform>> = Mutex::new(None);

fn with_platform<R>(f: impl FnOnce(&mut Platform) -> R) -> R {
    let mut g = PLATFORM.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(Platform::default))
}

/// The D24S8 word the "GPU" stores at (x, y) in the job of fence `fence`:
/// a depth that differs per texel and per frame in bits 31..8, and a
/// stencil byte the texture unit and the resolve must both ignore.
fn depth_word(fence: usize, x: u32, y: u32) -> u32 {
    let d = (x * 7 + y * 13 + fence as u32 * 1009) & 0x00ff_ffff;
    (d << 8) | 0x3c
}

/// The CPU view of `n` bytes at GPU address `gpu`.
fn platform_cpu(p: &mut Platform, gpu: u64, n: usize) -> *mut u8 {
    let (base, a) = p.allocs.range_mut(..=gpu).next_back().expect("an allocation below the address");
    let off = (gpu - base) as usize;
    assert!(off + n <= a.size, "0x{gpu:x}+{n} is past its allocation");
    // SAFETY: inside the allocation.
    unsafe { (a.mem.as_mut_ptr() as *mut u8).add(off) }
}

/// A one-level D24S8 layout as the backend lays out a depth/stencil buffer
/// (`zs_buf`: `v3d_tex_layout(w, h, 1, 4)` with the UIF XOR dropped), from
/// the real C.
fn zs_layout(w: u32, h: u32) -> V3dTexLayout {
    let mut l = V3dTexLayout::default();
    // SAFETY: a live layout to fill.
    assert!(unsafe { v3d_tex_layout(w, h, 1, 4, &mut l) }, "{w}x{h} lays out");
    if l.slices[0].tiling == V3D_TEX_TILING_UIF_XOR {
        l.slices[0].tiling = V3D_TEX_TILING_UIF_NO_XOR;
    }
    l
}

unsafe extern "C" fn p_gpu_alloc(_heap: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
    with_platform(|p| {
        if size == 0 || size > 256 << 20 {
            return -12;
        }
        let mut mem = vec![0u64; size.div_ceil(8)].into_boxed_slice();
        let gpu = GPU_BASE + p.next;
        p.next += (size as u64).div_ceil(4096) * 4096;
        let cpu = mem.as_mut_ptr() as *mut c_void;
        // SAFETY: the caller passes a writable descriptor.
        unsafe { *out = OsalGpuMem { cpu, gpu, phys: gpu, size, impl_: cpu } };
        p.allocs.insert(gpu, Alloc { mem, size });
        p.live += size;
        p.peak = p.peak.max(p.live);
        0
    })
}

unsafe extern "C" fn p_gpu_free(mem: *mut OsalGpuMem) -> i32 {
    // SAFETY: the caller passes its descriptor.
    let m = unsafe { *mem };
    if !m.impl_.is_null() {
        with_platform(|p| {
            if let Some(a) = p.allocs.remove(&m.gpu) {
                p.live -= a.size;
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

/// The "GPU": every job completes inside its submit (as src/osal/linux.c),
/// and a job that stores its depth/stencil buffer leaves `depth_word` in
/// every texel of it, tiled as the TLB tiles it (the layout `zs_buf` gave
/// the buffer, which the job's clear names).
unsafe extern "C" fn p_submit(kick: *const c_void, len: usize) -> i64 {
    if len != size_of::<V3dKick>() {
        return -22;
    }
    // SAFETY: the backend's live descriptor of that size.
    let k = unsafe { *(kick as *const V3dKick) };
    let fence = with_platform(|p| {
        p.kicks.push(k);
        p.kicks.len()
    });
    let job = jobs().get(fence - 1).cloned().expect("the job this kick runs was begun and ended");
    let c = job.clear.expect("the job was ended");
    if c.zs_store && c.zs_address != 0 {
        let (w, h) = (job.target.width, job.target.height);
        let l = zs_layout(w, h);
        assert_eq!(c.zs_memory_format, l.slices[0].tiling, "the store's memory format is the layout's tiling");
        let words: Vec<u32> = (0..w * h).map(|i| depth_word(fence, i % w, i / w)).collect();
        with_platform(|p| {
            let image = platform_cpu(p, u64::from(c.zs_address), l.size as usize);
            // SAFETY: the buffer's live mapping holds the laid-out level
            // (platform_cpu checked l.size bytes); `words` is h rows of w words.
            unsafe { v3d_tex_store_level(image, &l, 0, words.as_ptr() as *const u8, w * 4) };
        });
    }
    fence as i64
}

unsafe extern "C" fn p_wait(_fence: i64, _timeout_us: i64) -> i32 {
    0
}

unsafe extern "C" fn p_completed() -> i64 {
    with_platform(|p| p.kicks.len() as i64)
}

unsafe extern "C" fn p_query(what: u32, out: *mut c_void, out_sz: usize) -> i32 {
    use yakogl::osal::{
        OSAL_QUERY_V3D_BIN_POOL, OSAL_QUERY_V3D_CPU_OF_GPU, OSAL_QUERY_V3D_JOB_OPEN, OSAL_QUERY_V3D_VPM_BYTES,
    };
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
    if what == OSAL_QUERY_V3D_JOB_OPEN && out_sz == 4 {
        return 0;
    }
    if what == OSAL_QUERY_V3D_BIN_POOL {
        return -61; /* ENODATA, as zephyr_v3d.c answers before the first job */
    }
    -22 /* no TFU, the default budgets, no clock */
}

/// The library logs every line as `log("%s", line)` (`osal::log_str`), so a
/// two-pointer function takes it; on x86-64 and AArch64 Linux a variadic call
/// passes those two arguments as a plain one does.
unsafe extern "C" fn p_log2(_fmt: *const c_char, line: *const c_char) {
    // SAFETY: the library passes a NUL-terminated line.
    let s = unsafe { CStr::from_ptr(line) }.to_string_lossy().into_owned();
    with_platform(|p| p.log.push(s));
}

type LogFn = unsafe extern "C" fn(*const c_char, ...);

unsafe extern "C" fn p_init(_platform_arg: *mut c_void) -> i32 {
    0
}

/// The platform's table (made once, at run time: the log hook is a cast).
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

/* ---- the context ------------------------------------------------------------- */

/// The default framebuffer's size.
const CANVAS: (i32, i32) = (40, 30);

static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();
static mut CTX: *mut GlContext = core::ptr::null_mut();

/// The one context and backend of the file, set up once; the tests take
/// turns.
fn rig() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    ONCE.call_once(|| {
        // SAFETY: statics; the lock serialises every GL call of the file.
        unsafe {
            osal_set_ops(osal());
            yakogl::backend::g_backend = &yakogl_backend_v3d;
            assert_eq!(yakogl::backend::init(), 0, "{:?}", logs());
            CTX = context::gl_context_create(CANVAS.0 as u32, CANVAS.1 as u32);
            assert!(!CTX.is_null(), "{:?}", logs());
        }
    });
    // SAFETY: plain GL calls.
    unsafe {
        glFinish();
        while glGetError() != GL_NO_ERROR {}
    }
    g
}

fn logs() -> Vec<String> {
    with_platform(|p| p.log.clone())
}

fn shader(type_: GLenum, src: &str) -> GLuint {
    let c = CString::new(src).unwrap();
    // SAFETY: plain GL calls; the string lives for the call.
    unsafe {
        let s = glCreateShader(type_);
        let srcs = [c.as_ptr()];
        glShaderSource(s, 1, srcs.as_ptr(), core::ptr::null());
        glCompileShader(s);
        let mut ok: GLint = 0;
        glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
        assert_eq!(ok, 1, "compile 0x{type_:x}: {src}\n{:?}", logs());
        s
    }
}

fn program(vs: &str, fs: &str) -> GLuint {
    let (v, f) = (shader(GL_VERTEX_SHADER, vs), shader(GL_FRAGMENT_SHADER, fs));
    let name = CString::new("a_position").unwrap();
    // SAFETY: plain GL calls on valid handles.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glBindAttribLocation(p, 0, name.as_ptr());
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "link failed: {:?}", logs());
        p
    }
}

/* ---- the cases --------------------------------------------------------------- */

const SIZE: i32 = 16;
const VS3: &str = "#version 300 es\nin vec4 a_position;\nvoid main() { gl_Position = a_position; }\n";
const FS3: &str = concat!(
    "#version 300 es\nprecision mediump float;\n",
    "layout(location = 0) out vec4 c0;\n",
    "void main() { c0 = vec4(0.25, 0.5, 0.75, 0.125); }\n"
);
const FS3_SAMPLE: &str = concat!(
    "#version 300 es\nprecision mediump float;\nuniform sampler2D u_tex;\n",
    "layout(location = 0) out vec4 c0;\n",
    "void main() { c0 = texelFetch(u_tex, ivec2(gl_FragCoord.xy), 0); }\n"
);

/// IEEE half: 0.25, 0.5 and 0.75, the three channels the texture has.
const HALF: [u16; 3] = [0x3400, 0x3800, 0x3a00];

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

/// What the framebuffer's colour image is.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Att {
    /// The control: a GL_RGB16F renderbuffer.
    Rb,
    Tex2d,
    /// Face +X.
    Cube,
    /// Layer 1 of 3.
    Array,
    /// Layer 1 of 3.
    Tex3d,
}

const TEXTURES: [Att; 4] = [Att::Tex2d, Att::Cube, Att::Array, Att::Tex3d];

/// Specify the storage of a GL_RGB16F texture of `att` at `size` x `size`
/// (3 layers for the layered ones), every texel `HALF`.
fn specify(att: Att, size: i32) {
    let texels: Vec<u16> = (0..size * size).flat_map(|_| HALF).collect();
    // SAFETY: `texels` holds the 3-channel half image every call reads.
    unsafe {
        match att {
            Att::Cube => {
                for f in 0..6 {
                    glTexImage2D(
                        GL_TEXTURE_CUBE_MAP_POSITIVE_X + f,
                        0,
                        GL_RGB16F as GLint,
                        size,
                        size,
                        0,
                        GL_RGB,
                        GL_HALF_FLOAT,
                        texels.as_ptr() as *const c_void,
                    );
                }
            }
            Att::Array | Att::Tex3d => {
                let all: Vec<u16> = (0..3).flat_map(|_| texels.iter().copied()).collect();
                let target = if att == Att::Array { GL_TEXTURE_2D_ARRAY } else { GL_TEXTURE_3D };
                glTexImage3D(
                    target,
                    0,
                    GL_RGB16F as GLint,
                    size,
                    size,
                    3,
                    0,
                    GL_RGB,
                    GL_HALF_FLOAT,
                    all.as_ptr() as *const c_void,
                );
            }
            _ => glTexImage2D(
                GL_TEXTURE_2D,
                0,
                GL_RGB16F as GLint,
                size,
                size,
                0,
                GL_RGB,
                GL_HALF_FLOAT,
                texels.as_ptr() as *const c_void,
            ),
        }
    }
}

fn tex_target(att: Att) -> GLenum {
    match att {
        Att::Cube => GL_TEXTURE_CUBE_MAP,
        Att::Array => GL_TEXTURE_2D_ARRAY,
        Att::Tex3d => GL_TEXTURE_3D,
        _ => GL_TEXTURE_2D,
    }
}

fn rgb16f_texture(att: Att, size: i32) -> GLuint {
    let mut t = 0;
    let target = tex_target(att);
    // SAFETY: room for one name.
    unsafe {
        glGenTextures(1, &mut t);
        glBindTexture(target, t);
    }
    specify(att, size);
    // SAFETY: plain GL calls.
    unsafe {
        glTexParameteri(target, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
        glTexParameteri(target, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
    }
    assert_eq!(err(), GL_NO_ERROR, "{att:?} GL_RGB16F storage: {:?}", logs());
    t
}

fn rgb16f_renderbuffer(size: i32) -> GLuint {
    let mut r = 0;
    // SAFETY: room for one name.
    unsafe {
        glGenRenderbuffers(1, &mut r);
        glBindRenderbuffer(GL_RENDERBUFFER, r);
        glRenderbufferStorage(GL_RENDERBUFFER, GL_RGB16F, size, size);
    }
    assert_eq!(err(), GL_NO_ERROR, "GL_RGB16F renderbuffer storage");
    r
}

/// Attach the colour image `name` of `att` to colour attachment 0 of the
/// bound framebuffer.
fn attach(att: Att, name: GLuint) {
    // SAFETY: plain GL calls on live names.
    unsafe {
        match att {
            Att::Rb => glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, name),
            Att::Tex2d => glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, name, 0),
            Att::Cube => {
                glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_CUBE_MAP_POSITIVE_X, name, 0)
            }
            Att::Array | Att::Tex3d => glFramebufferTextureLayer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, name, 0, 1),
        }
    }
    assert_eq!(err(), GL_NO_ERROR, "{att:?} attach");
}

fn image(att: Att, size: i32) -> GLuint {
    if att == Att::Rb {
        rgb16f_renderbuffer(size)
    } else {
        rgb16f_texture(att, size)
    }
}

/// A framebuffer, bound, with `name` as its colour and (`zs`) a depth or a
/// stencil renderbuffer.
fn fbo_of(att: Att, name: GLuint, zs: Option<GLenum>) -> GLuint {
    let mut f = 0;
    // SAFETY: room for one name.
    unsafe {
        glGenFramebuffers(1, &mut f);
        glBindFramebuffer(GL_FRAMEBUFFER, f);
    }
    attach(att, name);
    if let Some(fmt) = zs {
        let mut z = 0;
        let point = if fmt == GL_STENCIL_INDEX8 { GL_STENCIL_ATTACHMENT } else { GL_DEPTH_ATTACHMENT };
        // SAFETY: room for one name.
        unsafe {
            glGenRenderbuffers(1, &mut z);
            glBindRenderbuffer(GL_RENDERBUFFER, z);
            glRenderbufferStorage(GL_RENDERBUFFER, fmt, SIZE, SIZE);
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, point, GL_RENDERBUFFER, z);
        }
        assert_eq!(err(), GL_NO_ERROR);
    }
    f
}

fn status() -> GLenum {
    // SAFETY: plain GL call.
    let s = unsafe { glCheckFramebufferStatus(GL_FRAMEBUFFER) };
    assert_eq!(err(), GL_NO_ERROR);
    s
}

fn program3(fs: &str) -> GLuint {
    program(VS3, fs)
}

/* a quad over the whole target at z = 0, (x, y, z) per vertex */
static QUAD: [f32; 18] = [
    -1.0, -1.0, 0.0, 1.0, -1.0, 0.0, -1.0, 1.0, 0.0, //
    -1.0, 1.0, 0.0, 1.0, -1.0, 0.0, 1.0, 1.0, 0.0,
];

/// A draw of the quad with `prog`; the GL error it raised.
fn draw_quad(prog: GLuint) -> GLenum {
    // SAFETY: plain GL calls; the array is a static.
    unsafe {
        glUseProgram(prog);
        glVertexAttribPointer(0, 3, GL_FLOAT, 0, 12, QUAD.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glDrawArrays(GL_TRIANGLES, 0, 6);
        let e = glGetError();
        glFinish();
        e
    }
}

fn clear_all() -> GLenum {
    // SAFETY: plain GL calls.
    unsafe {
        glClearColor(0.5, 0.25, 0.75, 0.5);
        glClear(GL_COLOR_BUFFER_BIT);
        let e = glGetError();
        glFinish();
        e
    }
}

fn clear_buffer() -> GLenum {
    let v = [0.5f32, 0.25, 0.75, 0.5];
    // SAFETY: plain GL calls; four floats.
    unsafe {
        glClearBufferfv(GL_COLOR, 0, v.as_ptr());
        let e = glGetError();
        glFinish();
        e
    }
}

fn mark() -> usize {
    jobs().len()
}

/// What the jobs begun after `m` say about their colour image.
fn since(m: usize) -> Vec<Summary> {
    jobs()[m..].iter().map(summary).collect()
}

/// Size, typed, the RT 0 output format, the alpha forced to 1 at the load.
type Summary = (u32, u32, bool, u32, bool);

fn summary(j: &SeenJob) -> Summary {
    let c = j.clear.expect("the job was ended");
    (j.target.width, j.target.height, j.target.typed, j.target.rt_format[0], c.force_alpha_1)
}

/// The summary RGB16F draws with: a typed 16F RGBA RT with the alpha forced
/// to 1 (the renderbuffer's, `color.blend.rgb16f_src_over`).
fn rgb16f_job(size: i32) -> Summary {
    (size as u32, size as u32, true, V3D_OUTPUT_IMAGE_FORMAT_RGBA16F, true)
}

fn teardown() {
    // SAFETY: plain GL call.
    unsafe { glBindFramebuffer(GL_FRAMEBUFFER, 0) };
    while err() != GL_NO_ERROR {}
}

/// Pixel (1, 1) of the bound framebuffer as floats.
fn read_px() -> [f32; 4] {
    let mut px = [-1.0f32; 4];
    // SAFETY: room for one RGBA float pixel.
    unsafe { glReadPixels(1, 1, 1, 1, GL_RGBA, GL_FLOAT, px.as_mut_ptr() as *mut c_void) };
    assert_eq!(err(), GL_NO_ERROR, "glReadPixels: {:?}", logs());
    px
}

/* ---- scenarios, each returning what the GL core and the back end said -------- */

/// Completeness of the framebuffer with `att` as colour 0.
fn completeness(att: Att) -> GLenum {
    let _g = rig();
    let name = image(att, SIZE);
    fbo_of(att, name, None);
    let s = status();
    teardown();
    s
}

/// A draw: (GL error, the jobs it made).
fn draw_case(att: Att) -> (GLenum, Vec<Summary>) {
    let _g = rig();
    let name = image(att, SIZE);
    fbo_of(att, name, None);
    let p = program3(FS3);
    let m = mark();
    let e = draw_quad(p);
    let made = since(m);
    teardown();
    (e, made)
}

/// glClear and glClearBufferfv: (their errors, the jobs they made).
fn clear_case(att: Att) -> (GLenum, GLenum, Vec<Summary>) {
    let _g = rig();
    let name = image(att, SIZE);
    fbo_of(att, name, None);
    let m = mark();
    let e1 = clear_all();
    let e2 = clear_buffer();
    let made = since(m);
    teardown();
    (e1, e2, made)
}

/// The read-back of the texels the test uploaded (the fake GPU renders
/// nothing): (status, pixel).
fn readback_case(att: Att) -> (GLenum, [f32; 4]) {
    let _g = rig();
    let name = image(att, SIZE);
    fbo_of(att, name, None);
    let s = status();
    let px = if s == GL_FRAMEBUFFER_COMPLETE { read_px() } else { [-1.0; 4] };
    teardown();
    (s, px)
}

/// dEQP `shared_colorbuffer`: two framebuffers over one colour image, each
/// with its own depth or stencil renderbuffer, used in turn (A cleared or
/// drawn, B drawn). (status A, status B, error A, error B, jobs)
fn shared_case(att: Att, clear_first: bool) -> (GLenum, GLenum, GLenum, GLenum, Vec<Summary>) {
    let _g = rig();
    let name = image(att, SIZE);
    let a = fbo_of(att, name, Some(GL_DEPTH_COMPONENT16));
    let sa = status();
    let b = fbo_of(att, name, Some(GL_STENCIL_INDEX8));
    let sb = status();
    let p = program3(FS3);
    let m = mark();
    // SAFETY: live names.
    unsafe { glBindFramebuffer(GL_FRAMEBUFFER, a) };
    let ea = if clear_first { clear_all() } else { draw_quad(p) };
    // SAFETY: live names.
    unsafe { glBindFramebuffer(GL_FRAMEBUFFER, b) };
    let eb = draw_quad(p);
    let made = since(m);
    teardown();
    (sa, sb, ea, eb, made)
}

/// dEQP `resize`: the colour image re-specified at another size under the
/// same framebuffer, then drawn. (status, error, jobs)
fn resize_case(att: Att) -> (GLenum, GLenum, Vec<Summary>) {
    let _g = rig();
    let name = image(att, SIZE);
    fbo_of(att, name, None);
    // SAFETY: plain GL calls on live names.
    unsafe {
        if att == Att::Rb {
            glBindRenderbuffer(GL_RENDERBUFFER, name);
            glRenderbufferStorage(GL_RENDERBUFFER, GL_RGB16F, SIZE + 8, SIZE + 8);
        } else {
            glBindTexture(tex_target(att), name);
        }
    }
    if att != Att::Rb {
        specify(att, SIZE + 8);
    }
    assert_eq!(err(), GL_NO_ERROR);
    let s = status();
    let p = program3(FS3);
    let m = mark();
    let e = draw_quad(p);
    let made = since(m);
    teardown();
    (s, e, made)
}

/// dEQP `recreate_color`: the colour image deleted and a new one attached to
/// the same framebuffer. (status, error, jobs)
fn recreate_case(att: Att) -> (GLenum, GLenum, Vec<Summary>) {
    let _g = rig();
    let name = image(att, SIZE);
    fbo_of(att, name, None);
    // SAFETY: a live name.
    unsafe {
        if att == Att::Rb {
            glDeleteRenderbuffers(1, &name);
        } else {
            glDeleteTextures(1, &name);
        }
    }
    let name2 = image(att, SIZE);
    attach(att, name2);
    let s = status();
    let p = program3(FS3);
    let m = mark();
    let e = draw_quad(p);
    let made = since(m);
    teardown();
    (s, e, made)
}

/// dEQP `blit.tex2d`: a blit from a GL_RGB16F framebuffer into another.
/// (read status, draw status, error)
fn blit_case(att: Att) -> (GLenum, GLenum, GLenum) {
    let _g = rig();
    let (src, dst) = (image(att, SIZE), image(att, SIZE));
    let fs = fbo_of(att, src, None);
    let sr = status();
    let fd = fbo_of(att, dst, None);
    let sd = status();
    // SAFETY: live names.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, fs);
        glBindFramebuffer(GL_DRAW_FRAMEBUFFER, fd);
        glBlitFramebuffer(0, 0, SIZE, SIZE, 0, 0, SIZE, SIZE, GL_COLOR_BUFFER_BIT, GL_NEAREST);
    }
    let e = err();
    teardown();
    (sr, sd, e)
}

/// dEQP `sample`: a draw into the GL_RGB16F texture, then the texture
/// sampled into an RGBA8 framebuffer. (the sampling draw's error, the jobs
/// of both draws)
fn sample_case() -> (GLenum, Vec<Summary>) {
    let _g = rig();
    let t = rgb16f_texture(Att::Tex2d, SIZE);
    fbo_of(Att::Tex2d, t, None);
    let p = program3(FS3);
    let m = mark();
    draw_quad(p);
    let (mut rb, mut f) = (0, 0);
    // SAFETY: room for names; live names.
    unsafe {
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorage(GL_RENDERBUFFER, GL_RGBA8, SIZE, SIZE);
        glGenFramebuffers(1, &mut f);
        glBindFramebuffer(GL_FRAMEBUFFER, f);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb);
        glBindTexture(GL_TEXTURE_2D, t);
    }
    assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE);
    let ps = program3(FS3_SAMPLE);
    let u = CString::new("u_tex").unwrap();
    // SAFETY: a linked program.
    unsafe {
        glUseProgram(ps);
        glUniform1i(glGetUniformLocation(ps, u.as_ptr()), 0);
    }
    let e = draw_quad(ps);
    let made = since(m);
    teardown();
    (e, made)
}

/* ---- today: the control, the renderbuffer ------------------------------------- */

#[test]
fn renderbuffer_is_complete_and_draws_rgba16f_with_alpha_forced() {
    assert_eq!(completeness(Att::Rb), GL_FRAMEBUFFER_COMPLETE);
    assert_eq!(draw_case(Att::Rb), (GL_NO_ERROR, vec![rgb16f_job(SIZE)]));
}

#[test]
fn renderbuffer_clears_reach_the_back_end() {
    let (e1, e2, made) = clear_case(Att::Rb);
    assert_eq!((e1, e2), (GL_NO_ERROR, GL_NO_ERROR));
    assert!(!made.is_empty(), "a clear is a job");
    assert!(made.iter().all(|j| *j == rgb16f_job(SIZE)), "{made:?}");
}

#[test]
fn renderbuffer_shared_by_two_framebuffers() {
    for clear_first in [false, true] {
        let (sa, sb, ea, eb, made) = shared_case(Att::Rb, clear_first);
        assert_eq!((sa, sb, ea, eb), (GL_FRAMEBUFFER_COMPLETE, GL_FRAMEBUFFER_COMPLETE, GL_NO_ERROR, GL_NO_ERROR));
        assert!(!made.is_empty() && made.iter().all(|j| *j == rgb16f_job(SIZE)), "{made:?}");
    }
}

#[test]
fn renderbuffer_resized_and_recreated() {
    assert_eq!(resize_case(Att::Rb), (GL_FRAMEBUFFER_COMPLETE, GL_NO_ERROR, vec![rgb16f_job(SIZE + 8)]));
    assert_eq!(recreate_case(Att::Rb), (GL_FRAMEBUFFER_COMPLETE, GL_NO_ERROR, vec![rgb16f_job(SIZE)]));
}

#[test]
fn renderbuffer_blit_into_the_same_format() {
    assert_eq!(blit_case(Att::Rb), (GL_FRAMEBUFFER_COMPLETE, GL_FRAMEBUFFER_COMPLETE, GL_NO_ERROR));
}

/* ---- the texture draws as the renderbuffer does ------------------------------- */

const WHY: &str = "a RGBA16F texture of format GL_RGB16F is a colour attachment (GL_EXT_color_buffer_half_float)";

#[test]
fn texture_is_complete_on_every_target() {
    for att in TEXTURES {
        assert_eq!(completeness(att), GL_FRAMEBUFFER_COMPLETE, "{att:?}: {WHY}");
    }
}

#[test]
fn texture_draws_and_clears_as_the_renderbuffer_does() {
    for att in TEXTURES {
        assert_eq!(draw_case(att), (GL_NO_ERROR, vec![rgb16f_job(SIZE)]), "{att:?} draw: {WHY}");
        let (e1, e2, made) = clear_case(att);
        assert_eq!((e1, e2), (GL_NO_ERROR, GL_NO_ERROR), "{att:?} clear");
        assert!(!made.is_empty() && made.iter().all(|j| *j == rgb16f_job(SIZE)), "{att:?} {made:?}");
    }
}

/// The alpha reads as 1 although the texture holds none (ES 3.0 4.3.2: a
/// missing alpha reads 1), and the colour is what was uploaded.
#[test]
fn texture_readback_has_alpha_one() {
    for att in TEXTURES {
        assert_eq!(readback_case(att), (GL_FRAMEBUFFER_COMPLETE, [0.25, 0.5, 0.75, 1.0]), "{att:?}: {WHY}");
    }
}

#[test]
fn texture_shared_colorbuffer_as_the_renderbuffer() {
    let ok = GL_FRAMEBUFFER_COMPLETE;
    for att in TEXTURES {
        for clear_first in [false, true] {
            let (sa, sb, ea, eb, made) = shared_case(att, clear_first);
            assert_eq!((sa, sb, ea, eb), (ok, ok, GL_NO_ERROR, GL_NO_ERROR), "{att:?} clear_first {clear_first}");
            assert!(!made.is_empty() && made.iter().all(|j| *j == rgb16f_job(SIZE)), "{att:?} {made:?}");
        }
    }
}

#[test]
fn texture_resize_recreate_blit_as_the_renderbuffer() {
    let ok = GL_FRAMEBUFFER_COMPLETE;
    for att in TEXTURES {
        assert_eq!(resize_case(att), (ok, GL_NO_ERROR, vec![rgb16f_job(SIZE + 8)]), "{att:?} resize");
        assert_eq!(recreate_case(att), (ok, GL_NO_ERROR, vec![rgb16f_job(SIZE)]), "{att:?} recreate");
        assert_eq!(blit_case(att), (ok, ok, GL_NO_ERROR), "{att:?} blit");
    }
}

/// `sample`: the draw into the texture, then its sampling; both reach the
/// back end (2 jobs).
#[test]
fn texture_sample_after_render() {
    let (e, made) = sample_case();
    assert_eq!(e, GL_NO_ERROR);
    assert_eq!(made.len(), 2, "{made:?}");
    assert_eq!(made[0], rgb16f_job(SIZE));
}
