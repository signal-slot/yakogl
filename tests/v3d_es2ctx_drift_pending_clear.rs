// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The ES 2.0 context against the pending clear that stays on its
//! framebuffer across a bind (4255adf1: a full clear of the window or of an
//! FBO with renderbuffer attachments is parked when another framebuffer is
//! bound, and handed back at the bind to it) and 0db66553 (an emptied
//! framebuffer's clear is not parked). The Pi 4 (V3D 4.2) whole dEQP-GLES2
//! tree (46f93767) predates both, and they change when a clear reaches a
//! job: dEQP-GLES2 functional.fbo.render.*, fbo.api.*, read_pixels and the
//! texture-from-FBO cases depend on it. This file replays the ES 2.0
//! sequences through the real GL core (a `report-es2` build, GLSL ES 1.00
//! shaders) and the real V3D back end and asserts, from the stand-in job
//! builder, which records every job (its target, its draws, whether it
//! clears the colour and to what, and whether a kick went to the platform
//! after it ended), what GL semantics need, not what the code does today:
//!
//! - clear A, bind B, draw, bind A, glReadPixels: A's clear is in one job,
//!   submitted by the time the read returns; B's draw job did not take it;
//! - clear A, bind B, glTexImage2D / glDeleteTextures on A's colour texture
//!   (or glRenderbufferStorage / glDeleteRenderbuffers on A's renderbuffer),
//!   bind A: no job clears the new storage and the clear reached the old one
//!   at most once; no GL error at the bind;
//! - clear A, detach every attachment of A, attach a new renderbuffer: the
//!   clear is in at most one job and no GL error (a clear still pending on
//!   the bound A may be carried onto the new renderbuffer: its contents are
//!   undefined, GL ES 2.0 4.4.3);
//! - clear A (a texture), sample A's texture in a draw into B: A's clear job
//!   ended before the sampling job began;
//! - clear the default framebuffer, bind an FBO (with or without a draw),
//!   glFinish: the window's clear is in a submitted job.
//!
//! Every sequence asserts GL_NO_ERROR and `v3d_refused_draws()` unchanged.
//! The colours are 0 or 1 a channel, so every colour format holds them
//! exactly. The texture layout code is the real driver/src/v3d/v3d_tex.c.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table,report-es2 --test v3d_es2ctx_drift_pending_clear
//! ```
//!
//! The feature set is `V3D_ES2_CONFIG` of tools/check.sh. The first run
//! compiles driver/src/v3d/v3d_tex.c with `cc`; a cold build of the crate
//! takes 30 to 60 minutes.
#![cfg(all(feature = "shaderc-online", feature = "gpu-v3d", feature = "host-test", feature = "report-es2"))]

use core::ffi::{c_char, c_int, c_void, CStr};
use std::collections::BTreeMap;
use std::ffi::CString;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, Once, OnceLock};
use yakogl::gl::consts::*;
use yakogl::gl::context;
use yakogl::gl::framebuffer::*;
use yakogl::gl::state::*;
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps, OsalV3dCpuOf};
use yakogl::v3d::backend::yakogl_backend_v3d;
use yakogl::v3d::ffi::*;
use yakogl::v3d::kick::V3dKick;

/* ---- the job builder: records, draws nothing ---- */

static LOG: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn log_lines() -> Vec<String> {
    LOG.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// One job the back end opened.
#[derive(Clone, Debug)]
struct Job {
    begin: u64,
    /// 0 while the job is open.
    end: u64,
    address: u32,
    draws: u32,
    /// The TLB loads the colour (false: the job clears it).
    load_rt0: bool,
    rgba8: u32,
    /// A kick went to the platform after the job ended.
    submitted: bool,
}

static JOBS: Mutex<Vec<Job>> = Mutex::new(Vec::new());
static SEQ: AtomicU64 = AtomicU64::new(1);

fn jobs() -> Vec<Job> {
    JOBS.lock().unwrap_or_else(|e| e.into_inner()).clone()
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
    JOBS.lock().unwrap_or_else(|e| e.into_inner()).push(Job {
        begin: SEQ.fetch_add(1, Ordering::SeqCst),
        end: 0,
        address: t.address,
        draws: 0,
        load_rt0: true,
        rgba8: 0,
        submitted: false,
    });
    t.width != 0 && t.height != 0
}

#[no_mangle]
unsafe extern "C" fn v3d_job_draw(j: *mut V3dJob, _d: *const V3dJobDraw, _layout: *mut c_void) -> i32 {
    // SAFETY: the backend passes its live job.
    unsafe {
        (*j).draws += 1;
        (*(*j).bcl).used += 120;
    }
    if let Some(last) = JOBS.lock().unwrap_or_else(|e| e.into_inner()).last_mut() {
        last.draws += 1;
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
    let c = unsafe {
        (*(*j).bcl).used += 1;
        (*(*j).rcl).used = 200;
        &*c
    };
    if let Some(last) = JOBS.lock().unwrap_or_else(|e| e.into_inner()).last_mut() {
        last.end = SEQ.fetch_add(1, Ordering::SeqCst);
        (last.load_rt0, last.rgba8) = (c.load_rt0, c.rgba8);
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
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("v3d_copy_reasons");
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
    for j in JOBS.lock().unwrap_or_else(|e| e.into_inner()).iter_mut() {
        if j.end != 0 {
            j.submitted = true;
        }
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
    use yakogl::osal::{
        OSAL_QUERY_V3D_BIN_POOL, OSAL_QUERY_V3D_CPU_OF_GPU, OSAL_QUERY_V3D_TILED_BUDGET, OSAL_QUERY_V3D_VPM_BYTES,
    };
    /* the kept tiled copies of image T5 (driver/samples/zephyr/deqp/texture-2048.conf),
     * the one Pi 4 image that samples the 2048x2048 textures of texture.size.2d */
    if what == OSAL_QUERY_V3D_TILED_BUDGET && out_sz == 4 {
        // SAFETY: a u32 out-parameter.
        unsafe { *(out as *mut u32) = 25_165_824 };
        return 0;
    }
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
    LOG.lock().unwrap_or_else(|e| e.into_inner()).push(s);
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
        /* a test that failed halfway leaves its state behind */
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        for cap in [GL_BLEND, GL_DEPTH_TEST, GL_STENCIL_TEST, GL_SCISSOR_TEST, GL_CULL_FACE] {
            glDisable(cap);
        }
        glColorMask(1, 1, 1, 1);
        glViewport(0, 0, W, H);
        while glGetError() != GL_NO_ERROR {}
    }
    LOG.lock().unwrap_or_else(|e| e.into_inner()).clear();
    JOBS.lock().unwrap_or_else(|e| e.into_inner()).clear();
    g
}

/* ---- helpers ---- */

use yakogl::gl::buffer::glBindBuffer;
use yakogl::gl::draw::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::texture::*;
use yakogl::gl::vertex::{glEnableVertexAttribArray, glVertexAttribPointer};
use yakogl::v3d::backend::v3d_refused_draws;

const W: GLsizei = 32;
const H: GLsizei = 32;

const RED: [f32; 4] = [1.0, 0.0, 0.0, 1.0];
const GREEN: [f32; 4] = [0.0, 1.0, 0.0, 1.0];
const YELLOW: [f32; 4] = [1.0, 1.0, 0.0, 1.0];
/// The clear colour as the job's word (R in the low byte).
const RED_W: u32 = 0xff00_00ff;
const GREEN_W: u32 = 0xff00_ff00;
const YELLOW_W: u32 = 0xff00_ffff;

const VS: &str = "attribute vec4 p;\nvarying vec2 t;\nvoid main() { gl_Position = p; t = p.xy * 0.5 + 0.5; }\n";
const FS_COLOR: &str = "precision mediump float;\nvarying vec2 t;\nvoid main() { gl_FragColor = vec4(t, 0.5, 1.0); }\n";
const FS_TEX: &str = "precision mediump float;\nvarying vec2 t;\nuniform sampler2D s;\n\
                      void main() { gl_FragColor = texture2D(s, t); }\n";

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

/// Make `fs` (with `VS`) the current program; sampler `s`, if any, reads unit 0.
fn program(fs: &str) {
    let (v, f) = (shader(GL_VERTEX_SHADER, VS), shader(GL_FRAGMENT_SHADER, fs));
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
        let s = glGetUniformLocation(p, c"s".as_ptr());
        if s >= 0 {
            glUniform1i(s, 0);
        }
    }
}

static TRI: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 3.0, -1.0, 0.0, 1.0, -1.0, 3.0, 0.0, 1.0];

fn draw_triangle() {
    // SAFETY: TRI is static and outlives the draw.
    unsafe {
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, TRI.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glDrawArrays(GL_TRIANGLES, 0, 3);
    }
}

/// The one rule of the file: what `f` does is accepted. No GL error and the
/// back end refused no draw (`v3d_refused_draws` is read as a difference).
fn accepted(what: &str, f: impl FnOnce()) {
    let base = v3d_refused_draws();
    f();
    let (e, n) = (err(), v3d_refused_draws().wrapping_sub(base));
    assert_eq!((e, n), (GL_NO_ERROR, 0), "{what}: error, refused draws; {:?}", log_lines());
}

fn finish() {
    // SAFETY: a plain GL call.
    unsafe { glFinish() };
}

/// A framebuffer object: an RGBA4 renderbuffer colour (its clear is parked
/// across a bind) or an RGBA texture colour (its clear goes out at the bind).
struct Fbo {
    fb: GLuint,
    /// The colour image's name: a renderbuffer, or a texture when `texture`.
    color: GLuint,
    texture: bool,
}

impl Fbo {
    fn new(texture: bool) -> Fbo {
        let mut f = Fbo { fb: 0, color: 0, texture };
        // SAFETY: plain GL calls with room for one name each.
        unsafe {
            glGenFramebuffers(1, &mut f.fb);
            glBindFramebuffer(GL_FRAMEBUFFER, f.fb);
            if texture {
                f.color = new_texture();
                glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, f.color, 0);
            } else {
                f.color = new_renderbuffer();
                glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, f.color);
            }
            assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
        }
        assert_eq!(err(), GL_NO_ERROR, "framebuffer setup");
        f
    }

    fn bind(&self) {
        // SAFETY: plain GL calls on a name this test made.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, self.fb);
            glViewport(0, 0, W, H);
        }
    }
}

impl Drop for Fbo {
    fn drop(&mut self) {
        // SAFETY: names this test made.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteFramebuffers(1, &self.fb);
            if self.texture {
                glDeleteTextures(1, &self.color);
            } else {
                glDeleteRenderbuffers(1, &self.color);
            }
        }
    }
}

fn new_renderbuffer() -> GLuint {
    let mut rb = 0;
    // SAFETY: plain GL calls with room for one name.
    unsafe {
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorage(GL_RENDERBUFFER, GL_RGBA4, W, H);
    }
    rb
}

fn texture_image() {
    // SAFETY: plain GL calls on the bound texture.
    unsafe {
        let null = core::ptr::null();
        glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA as GLint, W, H, 0, GL_RGBA, GL_UNSIGNED_BYTE, null);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
    }
}

fn new_texture() -> GLuint {
    let mut t = 0;
    // SAFETY: plain GL calls with room for one name.
    unsafe {
        glActiveTexture(GL_TEXTURE0);
        glGenTextures(1, &mut t);
        glBindTexture(GL_TEXTURE_2D, t);
    }
    texture_image();
    t
}

/// glClear(COLOR) of the bound framebuffer: every channel, no scissor.
fn clear(c: [f32; 4]) {
    // SAFETY: plain GL calls.
    unsafe {
        glDisable(GL_SCISSOR_TEST);
        glColorMask(1, 1, 1, 1);
        glClearColor(c[0], c[1], c[2], c[3]);
        glClear(GL_COLOR_BUFFER_BIT);
    }
    assert_eq!(err(), GL_NO_ERROR, "glClear");
}

fn read_pixel() {
    let mut px = [0u8; 4];
    // SAFETY: room for one RGBA8 pixel.
    unsafe { glReadPixels(0, 0, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr() as *mut c_void) };
    assert_eq!(err(), GL_NO_ERROR, "glReadPixels");
}

/// The jobs that clear the colour to `word`.
fn clears_to(word: u32) -> Vec<Job> {
    jobs().into_iter().filter(|j| !j.load_rt0 && j.rgba8 == word).collect()
}

/// The jobs with at least one draw.
fn drawing() -> Vec<Job> {
    jobs().into_iter().filter(|j| j.draws > 0).collect()
}

/// The one-job form of a parked clear: `word` is in exactly one job, ended
/// and kicked, which no draw of another target carries.
fn the_clear(what: &str, word: u32) -> Job {
    let c = clears_to(word);
    assert_eq!(c.len(), 1, "{what}: one job clears to {word:#x}: {:?}", jobs());
    assert!(c[0].end != 0, "{what}: the clear's job ended: {:?}", jobs());
    c[0].clone()
}

/* ---- the cases ---- */

/// Sanity: this is an ES 2.0 context (GL_VERSION says ES 2.0, the Pi 4 tree's
/// build, `report-es2`).
#[test]
fn the_context_is_es2() {
    let _g = rig();
    // SAFETY: plain GL calls.
    let major = unsafe {
        let mut m = 0;
        glGetIntegerv(GL_MAJOR_VERSION, &mut m);
        m
    };
    assert_eq!(err(), GL_INVALID_ENUM, "GL_MAJOR_VERSION is an ES 3.0 query");
    assert_eq!(major, 0);
}

/// Clear A, bind B, draw, bind A, glReadPixels: A's clear is in a job
/// submitted before the read returns, and B's draw job does not carry it. For
/// a renderbuffer A (the clear is parked across the bind) and a texture A.
#[test]
fn a_clear_reaches_a_job_before_the_read_of_its_framebuffer() {
    let _g = rig();
    for texture in [false, true] {
        let what = format!("A is a {}", if texture { "texture" } else { "renderbuffer" });
        JOBS.lock().unwrap_or_else(|e| e.into_inner()).clear();
        let (a, b) = (Fbo::new(texture), Fbo::new(false));
        program(FS_COLOR);
        accepted(&what, || {
            a.bind();
            clear(RED);
            b.bind();
            draw_triangle();
            a.bind();
            read_pixel();
        });
        let c = the_clear(&what, RED_W);
        assert!(c.submitted, "{what}: A's clear is in a submitted job when the read returns: {:?}", jobs());
        let d = drawing();
        assert!(!d.is_empty(), "{what}: B's draw reached a job: {:?}", jobs());
        assert!(
            d.iter().all(|j| j.load_rt0 || j.address == c.address),
            "{what}: B's draw job carries no clear of A's colour: {:?}",
            jobs()
        );
        assert!(d.iter().all(|j| j.address != c.address), "{what}: the draw into B is not A's job: {:?}", jobs());
    }
}

/// Clear A, bind B, replace A's colour storage (glTexImage2D, or
/// glRenderbufferStorage), bind A: the clear was for the old storage; no job
/// clears the new one, and the bind is no error.
#[test]
fn a_parked_clear_does_not_land_on_new_storage() {
    let _g = rig();
    for texture in [false, true] {
        let what = format!("A is a {}", if texture { "texture" } else { "renderbuffer" });
        JOBS.lock().unwrap_or_else(|e| e.into_inner()).clear();
        let (a, b) = (Fbo::new(texture), Fbo::new(false));
        program(FS_COLOR);
        accepted(&what, || {
            a.bind();
            clear(RED);
            b.bind();
            draw_triangle();
            // SAFETY: plain GL calls on names this test made.
            unsafe {
                if texture {
                    glBindTexture(GL_TEXTURE_2D, a.color);
                    texture_image();
                } else {
                    glBindRenderbuffer(GL_RENDERBUFFER, a.color);
                    glRenderbufferStorage(GL_RENDERBUFFER, GL_RGBA4, W, H);
                }
            }
            a.bind();
            finish();
        });
        let old = jobs();
        assert!(clears_to(RED_W).len() <= 1, "{what}: the clear is emitted at most once: {old:?}");
        let old_addr = clears_to(RED_W).first().map(|j| j.address);
        /* a draw into A now: its job loads or clears nothing red; the new
         * storage is not the clear's target */
        accepted(&what, || {
            draw_triangle();
            finish();
        });
        let all = jobs();
        let after: Vec<_> = all.iter().filter(|j| j.begin > old.last().map_or(0, |l| l.begin)).collect();
        assert!(
            after.iter().all(|j| j.load_rt0 || j.rgba8 != RED_W),
            "{what}: no job clears the new storage to the old clear colour: {all:?}"
        );
        let _ = old_addr;
    }
}

/// Clear A, bind B, delete A's colour (glDeleteTextures, or
/// glDeleteRenderbuffers), bind A: nothing is an error and the clear is in at
/// most one job; a new attachment then takes no clear.
#[test]
fn a_deleted_colour_takes_the_clear_with_it() {
    let _g = rig();
    for texture in [false, true] {
        let what = format!("A is a {}", if texture { "texture" } else { "renderbuffer" });
        JOBS.lock().unwrap_or_else(|e| e.into_inner()).clear();
        let a = Fbo::new(texture);
        let b = Fbo::new(false);
        program(FS_COLOR);
        accepted(&what, || {
            a.bind();
            clear(RED);
            b.bind();
            draw_triangle();
            // SAFETY: names this test made.
            unsafe {
                if texture {
                    glDeleteTextures(1, &a.color);
                } else {
                    glDeleteRenderbuffers(1, &a.color);
                }
            }
            a.bind();
            // SAFETY: a plain GL call.
            /* a deleted object stays attached to a framebuffer that is not
             * bound (GL ES 2.0 4.4.2.1), so A is still complete */
            let _ = unsafe { glCheckFramebufferStatus(GL_FRAMEBUFFER) };
            finish();
        });
        assert!(clears_to(RED_W).len() <= 1, "{what}: at most one clear job: {:?}", jobs());
        /* a new attachment: nothing stale on it */
        // SAFETY: plain GL calls.
        accepted(&what, || unsafe {
            if texture {
                let t = new_texture();
                glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, t, 0);
            } else {
                let rb = new_renderbuffer();
                glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb);
            }
            assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
        });
        let before = clears_to(RED_W).len();
        accepted(&what, || {
            draw_triangle();
            finish();
        });
        assert_eq!(clears_to(RED_W).len(), before, "{what}: no clear on the new attachment: {:?}", jobs());
        // the names are gone: keep Drop from deleting them again
        std::mem::forget(a);
        // SAFETY: plain GL call.
        unsafe { glBindFramebuffer(GL_FRAMEBUFFER, 0) };
    }
}

/// Clear A, detach every attachment of A, attach a new renderbuffer, draw:
/// accepted, and the clear is in at most one job (the new renderbuffer's
/// contents are undefined, so a pending clear carried onto it is allowed).
#[test]
fn a_detached_framebuffer_keeps_no_clear() {
    let _g = rig();
    for texture in [false, true] {
        let what = format!("A is a {}", if texture { "texture" } else { "renderbuffer" });
        JOBS.lock().unwrap_or_else(|e| e.into_inner()).clear();
        let a = Fbo::new(texture);
        program(FS_COLOR);
        accepted(&what, || {
            a.bind();
            clear(RED);
            // SAFETY: plain GL calls on names this test made.
            unsafe {
                glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, 0);
                glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_RENDERBUFFER, 0);
                glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_STENCIL_ATTACHMENT, GL_RENDERBUFFER, 0);
                let rb = new_renderbuffer();
                glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb);
                assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
            }
            draw_triangle();
            finish();
        });
        assert!(!drawing().is_empty(), "{what}: the draw into the new attachment reached a job: {:?}", jobs());
        let n = clears_to(RED_W).len();
        assert!(n <= 1, "{what}: the old image's clear is emitted at most once: {:?}", jobs());
    }
}

/// Clear A (a texture), sample A's texture in a draw into B: A's clear job
/// ended before the sampling job began.
#[test]
fn a_clear_precedes_the_job_that_samples_it() {
    let _g = rig();
    JOBS.lock().unwrap_or_else(|e| e.into_inner()).clear();
    let (a, b) = (Fbo::new(true), Fbo::new(false));
    accepted("sample A", || {
        a.bind();
        clear(GREEN);
        b.bind();
        program(FS_TEX);
        // SAFETY: plain GL calls on a name this test made.
        unsafe {
            glActiveTexture(GL_TEXTURE0);
            glBindTexture(GL_TEXTURE_2D, a.color);
        }
        draw_triangle();
        finish();
    });
    let c = the_clear("sample A", GREEN_W);
    let sampling: Vec<_> = drawing().into_iter().filter(|j| j.address != c.address).collect();
    assert!(!sampling.is_empty(), "the sampling draw reached a job: {:?}", jobs());
    assert!(
        sampling.iter().all(|j| c.end < j.begin),
        "A's clear job ends before the job that samples A begins: {:?}",
        jobs()
    );
}

/// Clear the default framebuffer, bind an FBO (and draw into it, or not),
/// glFinish: the window's clear is in a submitted job.
#[test]
fn the_window_clear_is_submitted_by_the_finish() {
    let _g = rig();
    for draw in [false, true] {
        let what = format!("draw: {draw}");
        JOBS.lock().unwrap_or_else(|e| e.into_inner()).clear();
        let a = Fbo::new(false);
        program(FS_COLOR);
        accepted(&what, || {
            // SAFETY: a plain GL call.
            unsafe { glBindFramebuffer(GL_FRAMEBUFFER, 0) };
            clear(YELLOW);
            a.bind();
            if draw {
                draw_triangle();
            }
            finish();
        });
        let c = the_clear(&what, YELLOW_W);
        assert!(c.submitted, "{what}: the window's clear is in a submitted job after glFinish: {:?}", jobs());
        assert!(
            drawing().iter().all(|j| j.address != c.address),
            "{what}: the FBO's draw did not go into the window's job: {:?}",
            jobs()
        );
    }
}
