// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Draws that alternate between a 4x framebuffer, a 1x framebuffer and the 4x
//! window on the V3D (plan item v3d-msaa-switch-tests), through the GL API,
//! the real GL core and the real V3D back end (`draw_call`), on the host. The
//! harness is tests/es3_msaa_v3d_mrt.rs's: the job builder is stood in for
//! (it records what the back end hands it and draws nothing), the tile rule
//! is the real driver/src/v3d/v3d_frame.c, and the test plays the GPU's
//! resolve store by filling a target's raster memory with its colour.
//!
//! The sequence 4x FBO, 1x FBO, window, 4x FBO again, twice over:
//!
//! - each draw reaches the back end with its own target's sample count (4, 1,
//!   4, 4) and its own `target_id`/`target_gen`; the 4x job is a `msaa` job,
//!   the 1x job is not (no tile-buffer msaa flag left from the job before);
//! - `GL_SAMPLES` follows the bound draw framebuffer;
//! - the 4x FBO's samples survive the jobs of the other targets: its second
//!   visit loads them (`load_rt0`), the first visit of each target does not,
//!   and the 1x FBO never loads 4x samples;
//! - after the switches each target resolves (or reads back) to its own
//!   colour, never another target's.
//!
//! What a host test cannot see: the pixels the GPU writes and the control
//! list. BOARD-UNVERIFIED: no Pi 4 run.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table,msaa-fbo --test es3_msaa_v3d_switch
//! ```

#![cfg(all(
    feature = "host-test",
    feature = "shaderc-online",
    feature = "gpu-v3d",
    feature = "msaa-fbo",
    not(feature = "report-es2")
))]

use core::ffi::{c_char, c_int, c_void, CStr};
use std::collections::BTreeMap;
use std::ffi::CString;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Mutex, MutexGuard, Once, OnceLock};
use yakogl::backend::{GlesDraw, GLES_MAX_TARGETS, GLES_SURF_RGBA8888};
use yakogl::gl::buffer::glBindBuffer;
use yakogl::gl::consts::*;
use yakogl::gl::context;
use yakogl::gl::context::gl_context_bind_surface;
use yakogl::gl::draw::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::{glEnableVertexAttribArray, glVertexAttribPointer};
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps, OsalV3dCpuOf};
use yakogl::v3d::backend::yakogl_backend_v3d;
use yakogl::v3d::ffi::*;
use yakogl::v3d::kick::V3dKick;

/// One draw as the V3D back end was handed it, and what it answered.
#[derive(Clone, Debug)]
struct GlSeen {
    /// An application draw (vertices), not a clear alone.
    draws: bool,
    /// `targets[..num_targets]`: (gpu, format).
    targets: Vec<(u64, u8)>,
    samples: u8,
    /// `target_id`, `target_gen` and the per-target arrays.
    id: (u32, u32),
    /// `draw_call`'s answer (0, or the refusal's -errno).
    rc: i32,
}

/* ---- the job builder, the tile rule and the platform (tests/es3_four_rt_v3d.rs) ---- */

#[derive(Default)]
struct Seen {
    jobs: Vec<V3dJobTarget>,
    clears: Vec<V3dJobClear>,
    /// The vertex count of each draw the job builder was handed.
    draws: Vec<u32>,
    tilings: Vec<Tiling>,
    /// What the V3D back end was handed by each draw, and its answer.
    gl: Vec<GlSeen>,
    log: Vec<String>,
}

/// One call of the tile rule: what was asked, the tile it answered.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Tiling {
    rts: u32,
    msaa: bool,
    max_bpp: u32,
    tile: (u32, u32),
}

static SEEN: Mutex<Seen> = Mutex::new(Seen {
    jobs: Vec::new(),
    clears: Vec::new(),
    draws: Vec::new(),
    tilings: Vec::new(),
    gl: Vec::new(),
    log: Vec::new(),
});

fn seen<R>(f: impl FnOnce(&mut Seen) -> R) -> R {
    f(&mut SEEN.lock().unwrap_or_else(|e| e.into_inner()))
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
    seen(|s| s.jobs.push(t));
    t.width != 0 && t.height != 0
}

#[no_mangle]
unsafe extern "C" fn v3d_job_draw(j: *mut V3dJob, d: *const V3dJobDraw, _layout: *mut c_void) -> i32 {
    // SAFETY: the backend passes its live job and a draw whose pointers live for the call.
    let (j, d) = unsafe { (&mut *j, &*d) };
    j.draws += 1;
    // SAFETY: the job's BCL is live; the draw's slices are as long as it says.
    unsafe {
        (*j.bcl).used += 120;
        seen(|s| s.draws.push(d.count));
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
    // SAFETY: the backend passes its live job and clear; its lists are live.
    unsafe {
        seen(|s| s.clears.push(*c));
        (*(*j).bcl).used += 1;
        (*(*j).rcl).used = 200;
    }
    true
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
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("v3d_msaa");
        std::fs::create_dir_all(&dir).expect("the scratch directory");
        let so = dir.join(format!("v3d_msaa-{}.so", std::process::id()));
        /* the compiler's own temporary files next to the object */
        let st = Command::new("cc")
            .current_dir(&root)
            .env("TMPDIR", &dir)
            .args(["-std=c11", "-O1", "-shared", "-fPIC", "-Wall", "-Wextra", "-Werror"])
            .args(["-Idriver/src/v3d", "-Idriver/src/v3d/generated"])
            .args(["-Ithird_party/mesa-26.2.1/src/broadcom", "-Ithird_party/mesa-26.2.1/src"])
            .arg("-Ithird_party/mesa-26.2.1/include")
            .args([
                "driver/src/v3d/v3d_tex.c",
                "driver/src/v3d/v3d_tiling_mesa.c",
                "driver/src/v3d/v3d_frame.c",
                "-lm",
                "-o",
            ])
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
    trampoline!(v3d_frame_alloc_sizes(t: *const V3dTiling, draws: u32, page: u32, ta: *mut u32, ts: *mut u32));
}

/// The real tile rule (driver/src/v3d/v3d_frame.c), each call recorded:
/// the render-target count, MSAA and largest internal bpp the back end
/// asked for, and the tile it got.
fn tiling(rts: u32, msaa: bool, max_bpp: u32, t: *const V3dTiling) {
    // SAFETY: the library filled `t` just before.
    let t = unsafe { *t };
    seen(|s| s.tilings.push(Tiling { rts, msaa, max_bpp, tile: (t.tile_w, t.tile_h) }));
}

#[no_mangle]
unsafe extern "C" fn v3d_frame_tiling(width: u32, height: u32, t: *mut V3dTiling) {
    // SAFETY: the C function's signature, filling the backend's out-parameter.
    let f: unsafe extern "C" fn(u32, u32, *mut V3dTiling) = unsafe { std::mem::transmute(sym("v3d_frame_tiling")) };
    // SAFETY: as above.
    unsafe { f(width, height, t) };
    tiling(1, false, 0, t);
}

#[no_mangle]
unsafe extern "C" fn v3d_frame_tiling_rts(width: u32, height: u32, rts: u32, t: *mut V3dTiling) {
    // SAFETY: the C function's signature, filling the backend's out-parameter.
    let f: unsafe extern "C" fn(u32, u32, u32, *mut V3dTiling) =
        unsafe { std::mem::transmute(sym("v3d_frame_tiling_rts")) };
    // SAFETY: as above.
    unsafe { f(width, height, rts, t) };
    tiling(rts, false, 0, t);
}

#[no_mangle]
unsafe extern "C" fn v3d_frame_tiling_ms(width: u32, height: u32, rts: u32, msaa: bool, t: *mut V3dTiling) {
    // SAFETY: the C function's signature, filling the backend's out-parameter.
    let f: unsafe extern "C" fn(u32, u32, u32, bool, *mut V3dTiling) =
        unsafe { std::mem::transmute(sym("v3d_frame_tiling_ms")) };
    // SAFETY: as above.
    unsafe { f(width, height, rts, msaa, t) };
    tiling(rts, msaa, 0, t);
}

#[no_mangle]
unsafe extern "C" fn v3d_frame_tiling_bpp(
    width: u32,
    height: u32,
    rts: u32,
    msaa: bool,
    max_bpp: u32,
    t: *mut V3dTiling,
) {
    // SAFETY: the C function's signature, filling the backend's out-parameter.
    let f: unsafe extern "C" fn(u32, u32, u32, bool, u32, *mut V3dTiling) =
        unsafe { std::mem::transmute(sym("v3d_frame_tiling_bpp")) };
    // SAFETY: as above.
    unsafe { f(width, height, rts, msaa, max_bpp, t) };
    tiling(rts, msaa, max_bpp, t);
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

struct Alloc {
    mem: Box<[u64]>,
    size: usize,
}
struct Platform {
    next: u64,
    allocs: BTreeMap<u64, Alloc>,
    submitted: usize,
}
static PLATFORM: Mutex<Option<Platform>> = Mutex::new(None);

fn with_platform<R>(f: impl FnOnce(&mut Platform) -> R) -> R {
    let mut g = PLATFORM.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(|| Platform { next: 0, allocs: BTreeMap::new(), submitted: 0 }))
}

unsafe extern "C" fn p_gpu_alloc(_heap: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
    with_platform(|p| {
        if size == 0 || size > 256 << 20 {
            return -12;
        }
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
        with_platform(|p| p.allocs.remove(&m.gpu));
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

/// The V3D back end with its draw entry watched: what the GL core hands it,
/// then the real `draw_call`'s answer.
unsafe extern "C" fn spy_draw(d: *const GlesDraw) -> i32 {
    // SAFETY: the GL core hands a live draw.
    let dd = unsafe { &*d };
    // SAFETY: the real back end's entry, with the same draw.
    let rc = unsafe { (yakogl_backend_v3d.draw.expect("the V3D draws"))(d) };
    let n = (dd.num_targets.max(1) as usize).min(GLES_MAX_TARGETS);
    seen(|s| {
        s.gl.push(GlSeen {
            draws: dd.index_count != 0,
            targets: dd.targets[..n].iter().map(|t| (t.gpu, t.format)).collect(),
            samples: dd.samples,
            id: (dd.target_id, dd.target_gen),
            rc,
        })
    });
    rc
}

fn spy_backend() -> &'static yakogl::backend::YakoglBackend {
    let b = yakogl::backend::YakoglBackend { draw: Some(spy_draw), ..yakogl_backend_v3d };
    Box::leak(Box::new(b))
}

static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

/// The one context and back end of the file; the tests take turns and each
/// starts with nothing recorded and nothing pending.
fn context() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    ONCE.call_once(|| {
        // SAFETY: statics; the lock serialises every GL call of the file.
        unsafe {
            osal_set_ops(osal());
            /* the four-target stand-in (tests/es3_mrt4_msaa_v3d.rs) */
            yakogl::backend::HOST_DRAW_TARGETS.store(4, std::sync::atomic::Ordering::Relaxed);
            yakogl::backend::g_backend = spy_backend();
            assert_eq!(yakogl::backend::init(), 0, "{:?}", seen(|s| s.log.clone()));
            assert!(!context::gl_context_create(8, 8).is_null(), "{:?}", seen(|s| s.log.clone()));
        }
    });
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glFinish();
        while glGetError() != GL_NO_ERROR {}
    }
    seen(|s| *s = Seen::default());
    g
}

/// GL_NO_ERROR, or the first error of the queue (the queue is drained).
fn err() -> GLenum {
    // SAFETY: plain GL calls.
    unsafe {
        let e = glGetError();
        while glGetError() != GL_NO_ERROR {}
        e
    }
}

/* ---- framebuffers, the program, the draw ------------------------------- */

struct Fbo {
    fb: GLuint,
    rbs: Vec<GLuint>,
}

impl Drop for Fbo {
    fn drop(&mut self) {
        // SAFETY: names this test made.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteFramebuffers(1, &self.fb);
            glDeleteRenderbuffers(self.rbs.len() as GLsizei, self.rbs.as_ptr());
            glUseProgram(0);
        }
    }
}

/// The error of `glRenderbufferStorageMultisample(.., samples, format, 8, 8)`
/// on a fresh renderbuffer, and the renderbuffer.
fn storage(samples: GLsizei, format: GLenum) -> (GLenum, GLuint) {
    let mut rb = 0;
    // SAFETY: plain GL calls.
    unsafe {
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        while glGetError() != GL_NO_ERROR {}
        glRenderbufferStorageMultisample(GL_RENDERBUFFER, samples, format, 8, 8);
    }
    (err(), rb)
}

/// A bound 8x8 framebuffer with a renderbuffer of `samples` on each of
/// `formats`' colour attachments, all draw buffers. A storage error comes back
/// as `Err(error)`.
fn framebuffer(samples: GLsizei, formats: &[GLenum]) -> Result<Fbo, GLenum> {
    let mut fb = 0;
    // SAFETY: room for one name; plain GL call.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
    }
    let mut fbo = Fbo { fb, rbs: Vec::new() };
    for (i, f) in formats.iter().enumerate() {
        let (e, rb) = storage(samples, *f);
        fbo.rbs.push(rb);
        if e != GL_NO_ERROR {
            return Err(e);
        }
        // SAFETY: a renderbuffer this test made.
        unsafe { glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0 + i as GLenum, GL_RENDERBUFFER, rb) };
    }
    let bufs: Vec<GLenum> = (0..formats.len()).map(|i| GL_COLOR_ATTACHMENT0 + i as GLenum).collect();
    // SAFETY: plain GL calls.
    unsafe {
        glDrawBuffers(bufs.len() as GLsizei, bufs.as_ptr());
        glViewport(0, 0, 8, 8);
        assert_eq!(err(), GL_NO_ERROR);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "{formats:#x?}");
    }
    Ok(fbo)
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

/// The current program: `n` float outputs, output `k` a constant of its own.
fn program(n: usize) {
    let mut fs = String::from("#version 300 es\nprecision highp float;\n");
    for i in 0..n {
        fs += &format!("layout(location = {i}) out vec4 o{i};\n");
    }
    fs += "void main() {\n";
    for i in 0..n {
        let (r, g, b) = (0.125 * (i + 1) as f32, 1.0 - 0.125 * i as f32, 0.5);
        fs += &format!("o{i} = vec4({r:?}, {g:?}, {b:?}, 1.0);\n");
    }
    fs += "}\n";
    let v = shader(GL_VERTEX_SHADER, "#version 300 es\nin vec4 p;\nvoid main() { gl_Position = p; }\n");
    let f = shader(GL_FRAGMENT_SHADER, &fs);
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1);
        glUseProgram(p);
    }
}

/// Draw one triangle, flushed; what the back end was handed for it (and
/// answered), and what the job builder recorded.
fn draw() -> (GlSeen, Seen) {
    seen(|s| *s = Seen::default());
    draw_keeping()
}

/// [`draw`] without forgetting what was recorded before it (a clear that ran
/// a job of its own).
fn draw_keeping() -> (GlSeen, Seen) {
    static TRI: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 3.0, -1.0, 0.0, 1.0, -1.0, 3.0, 0.0, 1.0];
    // SAFETY: TRI is static and outlives the draw.
    unsafe {
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, TRI.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glDrawArrays(GL_TRIANGLES, 0, 3);
        glFinish();
    }
    let s = seen(core::mem::take);
    assert_eq!(err(), GL_NO_ERROR, "{:?}", s.log);
    let d = s.gl.iter().rev().find(|d| d.draws).cloned().expect("the draw reached the back end");
    (d, s)
}

/* ---- the targets --------------------------------------------------------- */

/// The memory of the window: 8x8 RGBA8 in the platform's GPU memory, leaked.
fn window_mem() -> &'static mut OsalGpuMem {
    static MEM: Mutex<usize> = Mutex::new(0);
    let mut m = MEM.lock().unwrap_or_else(|e| e.into_inner());
    if *m == 0 {
        let mut mem = OsalGpuMem { cpu: core::ptr::null_mut(), gpu: 0, phys: 0, size: 0, impl_: core::ptr::null_mut() };
        // SAFETY: the platform's allocator writes `mem`.
        assert_eq!(unsafe { p_gpu_alloc(0, 8 * 8 * 4, 0, &mut mem) }, 0);
        *m = Box::into_raw(Box::new(mem)) as usize;
    }
    // SAFETY: leaked and never freed; the tests take turns (LOCK).
    unsafe { &mut *(*m as *mut OsalGpuMem) }
}

/// The 4x window bound as the default framebuffer; one sample again when
/// dropped.
struct Window;

impl Window {
    fn new() -> Window {
        // SAFETY: the memory is leaked and holds 8x8 RGBA8888 pixels.
        let r = unsafe { gl_context_bind_surface(window_mem(), 8, 8, GLES_SURF_RGBA8888, 4) };
        assert_eq!(r, 0);
        Window
    }
}

impl Drop for Window {
    fn drop(&mut self) {
        // SAFETY: as in `new`.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glFinish();
            gl_context_bind_surface(window_mem(), 8, 8, GLES_SURF_RGBA8888, 1);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Target {
    Fbo4,
    Fbo1,
    Window,
}

/// The colour each target's pixels end up with: the stand-in for what the
/// GPU would draw and resolve.
fn colour(t: Target) -> [u8; 4] {
    match t {
        Target::Fbo4 => [200, 10, 10, 255],
        Target::Fbo1 => [10, 200, 10, 255],
        Target::Window => [10, 10, 200, 255],
    }
}

/// What a draw into a target must say: its sample count and `GL_SAMPLES`.
fn want_samples(t: Target) -> (u8, GLint) {
    match t {
        Target::Fbo4 | Target::Window => (4, 4),
        Target::Fbo1 => (1, 0),
    }
}

/// Fill the memory of the allocation holding `gpu` with `px` repeated: the
/// stand-in for the GPU's resolve (or plain) store into the target.
fn fill(gpu: u64, px: [u8; 4]) {
    with_platform(|p| {
        let (_, a) = p.allocs.range_mut(..=gpu).next_back().expect("an allocation holds the target");
        // SAFETY: the allocation's own words, viewed as bytes.
        let bytes = unsafe { core::slice::from_raw_parts_mut(a.mem.as_mut_ptr() as *mut u8, a.size) };
        for c in bytes.chunks_exact_mut(4) {
            c.copy_from_slice(&px);
        }
    });
}

fn geti(pname: GLenum) -> GLint {
    let mut v = -1;
    // SAFETY: one value.
    unsafe { glGetIntegerv(pname, &mut v) };
    v
}

/// The bytes (8x8 RGBA) of the target `fb` after its own resolve (a blit into
/// a fresh 1x framebuffer, for a 4x one) or plain readback (a 1x one), and
/// the blit's error.
fn read_target(fb: GLuint, four_x: bool) -> (GLenum, Vec<u8>) {
    let mut px = vec![0u8; 8 * 8 * 4];
    let mut e = GL_NO_ERROR;
    let mut tmp = None;
    let mut src = fb;
    if four_x {
        let dst = framebuffer(0, &[GL_RGBA8]).expect("a single-sample target");
        // SAFETY: plain GL calls on names this test made.
        unsafe {
            glBindFramebuffer(GL_READ_FRAMEBUFFER, fb);
            glBindFramebuffer(GL_DRAW_FRAMEBUFFER, dst.fb);
            while glGetError() != GL_NO_ERROR {}
            glBlitFramebuffer(0, 0, 8, 8, 0, 0, 8, 8, GL_COLOR_BUFFER_BIT, GL_NEAREST);
        }
        e = err();
        src = dst.fb;
        tmp = Some(dst);
    }
    // SAFETY: as above; `px` holds 8x8 RGBA.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, src);
        glReadPixels(0, 0, 8, 8, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr() as *mut c_void);
    }
    assert_eq!(err(), GL_NO_ERROR);
    drop(tmp);
    (e, px)
}

/// Every pixel within `tol` of `want` per channel (a small edge tolerance).
fn close(px: &[u8], want: [u8; 4], tol: u8) -> bool {
    px.chunks_exact(4).all(|p| p.iter().zip(want).all(|(a, b)| a.abs_diff(b) <= tol))
}

struct Rig {
    f4: Fbo,
    f1: Fbo,
    _w: Window,
}

impl Rig {
    fn new() -> Rig {
        let _w = Window::new();
        let f4 = framebuffer(4, &[GL_RGBA8]).expect("4 samples are legal storage");
        let f1 = framebuffer(0, &[GL_RGBA8]).expect("plain storage");
        program(1);
        Rig { f4, f1, _w }
    }
    fn fb(&self, t: Target) -> GLuint {
        match t {
            Target::Fbo4 => self.f4.fb,
            Target::Fbo1 => self.f1.fb,
            Target::Window => 0,
        }
    }
    /// Bind `t` for drawing and draw one triangle into it.
    fn draw_into(&self, t: Target) -> (GlSeen, Seen) {
        /* the scratch framebuffers of a readback drop the program */
        program(1);
        // SAFETY: plain GL calls.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, self.fb(t));
            glViewport(0, 0, 8, 8);
        }
        assert_eq!(geti(GL_SAMPLES), want_samples(t).1, "{t:?}: GL_SAMPLES follows the bound framebuffer");
        draw()
    }
}

/* ---- the sequences ------------------------------------------------------- */

const ORDER: [Target; 4] = [Target::Fbo4, Target::Fbo1, Target::Window, Target::Fbo4];

/// `rounds` times the sequence 4x FBO, 1x FBO, window, 4x FBO: what each draw
/// and job say, and what each target holds after its own resolve at the end.
fn sequence(rounds: usize) {
    let _g = context();
    let rig = Rig::new();
    let mut ident: [Option<((u32, u32), u64)>; 3] = [None; 3];
    for round in 0..rounds {
        for (step, &t) in ORDER.iter().enumerate() {
            let (d, s) = rig.draw_into(t);
            let tag = format!("round {round} step {step} {t:?}");
            assert_eq!((d.samples, d.targets.len(), d.rc), (want_samples(t).0, 1, 0), "{tag}: {d:?} {:?}", s.log);
            assert_eq!(s.jobs.len(), 1, "{tag}: one job per draw");
            /* the tile-buffer msaa flag is the target's, not the job before's */
            assert_eq!(s.jobs[0].msaa, want_samples(t).0 == 4, "{tag}: job msaa flag {:?}", s.jobs[0]);
            let tile = s.tilings.last().expect("the job asked the tile rule");
            assert_eq!(tile.msaa, want_samples(t).0 == 4, "{tag}: tile rule {tile:?}");
            assert_eq!(tile.tile, if tile.msaa { (32, 32) } else { (64, 64) }, "{tag}: {tile:?}");
            let c = s.clears.last().expect("the job ended");
            let visited = round > 0 || step == 3;
            if t == Target::Fbo4 {
                assert_eq!(c.load_rt0, visited, "{tag}: 4x samples kept across other targets' jobs: {c:?}");
            }
            if t == Target::Window {
                assert_eq!(d.targets[0].0, window_mem().gpu, "{tag}: the window's own memory");
            }
            /* the identity is the target's own, the same on every visit */
            let slot = match t {
                Target::Fbo4 => 0,
                Target::Fbo1 => 1,
                Target::Window => 2,
            };
            let now = (d.id, d.targets[0].0);
            match ident[slot] {
                None => ident[slot] = Some(now),
                Some(before) => assert_eq!(before, now, "{tag}: same target, same identity"),
            }
            fill(d.targets[0].0, colour(t));
        }
    }
    let gpus: std::collections::BTreeSet<u64> = ident.iter().map(|i| i.unwrap().1).collect();
    assert_eq!(gpus.len(), 3, "three targets, three memories: {ident:?}");
    for t in [Target::Fbo4, Target::Fbo1, Target::Window] {
        let (e, px) = read_target(rig.fb(t), t != Target::Fbo1);
        assert_eq!(e, GL_NO_ERROR, "{t:?}");
        assert!(close(&px, colour(t), 2), "{t:?} keeps its own contents: want {:?}, got {:?}", colour(t), &px[..8]);
    }
}

#[test]
fn four_one_window_four_once() {
    sequence(1);
}

/// Twice over: the second round finds every kept sample buffer and no stale
/// per-job state of the first.
#[test]
fn four_one_window_four_twice() {
    sequence(2);
}

/// Each target is read back right after its own draw, while the others have
/// other colours: nothing leaks from one into the next.
#[test]
fn each_target_resolves_to_its_own_colour_after_each_switch() {
    let _g = context();
    let rig = Rig::new();
    let mut mem = std::collections::BTreeMap::new();
    for round in 0..2 {
        for t in [Target::Fbo4, Target::Fbo1, Target::Window, Target::Fbo4, Target::Window, Target::Fbo1] {
            let (d, _) = rig.draw_into(t);
            assert_eq!(d.rc, 0, "round {round} {t:?}");
            let gpu = d.targets[0].0;
            mem.insert(format!("{t:?}"), gpu);
            fill(gpu, colour(t));
            let (e, px) = read_target(rig.fb(t), t != Target::Fbo1);
            assert_eq!(e, GL_NO_ERROR, "round {round} {t:?}");
            assert!(close(&px, colour(t), 2), "round {round} {t:?}: got {:?}", &px[..8]);
        }
    }
    assert_eq!(mem.len(), 3);
}

/// `GL_SAMPLES` and `GL_SAMPLE_BUFFERS` of the bound draw framebuffer, and a
/// read framebuffer of another kind changes neither.
#[test]
fn gl_samples_follow_the_bound_draw_framebuffer() {
    let _g = context();
    let rig = Rig::new();
    let samples = |t: Target| -> (GLint, GLint) {
        // SAFETY: plain GL calls.
        unsafe { glBindFramebuffer(GL_DRAW_FRAMEBUFFER, rig.fb(t)) };
        (geti(GL_SAMPLE_BUFFERS), geti(GL_SAMPLES))
    };
    for _ in 0..2 {
        assert_eq!(samples(Target::Fbo4), (1, 4));
        assert_eq!(samples(Target::Fbo1), (0, 0));
        assert_eq!(samples(Target::Window), (1, 4));
        // SAFETY: plain GL call.
        unsafe { glBindFramebuffer(GL_READ_FRAMEBUFFER, rig.f1.fb) };
        assert_eq!(geti(GL_SAMPLES), 4, "the read framebuffer does not count");
        assert_eq!(samples(Target::Fbo4), (1, 4));
        assert_eq!(err(), GL_NO_ERROR);
    }
}
