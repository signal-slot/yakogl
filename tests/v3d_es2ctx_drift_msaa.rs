// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The ES 2.0 context against the multisample counts of 5e2a9ad5 (renderbuffer
//! sample counts, completeness and draw sample state in src/gl/draw.rs,
//! framebuffer.rs, msaa.rs) and 8eb22b62 (`YakoglBackend::max_samples`, 0 on
//! every back end) on V3D 4.2. dEQP-GLES2 asks for a single-sampled surface:
//! GL_SAMPLES and GL_SAMPLE_BUFFERS read 0 (state_query, multisample.*), the
//! renderbuffer queries and FBO completeness answer as ES 2.0.25 says
//! (functional.fbo.completeness.*, fbo.render.*), and no job is multisampled.
//! The expected answers are ES 2.0.25's and dEQP-GLES2's, not the current
//! output; where the spec leaves a size implementation-defined ("at least"),
//! the test asserts the least. Rig, job-builder stand-in and platform as in
//! tests/v3d_es2ctx_refusals.rs; the stand-in also records every job target's
//! `msaa` flag and every draw's SAMPLE_STATE mask (0xf in a single-sampled job).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table,report-es2 --test v3d_es2ctx_drift_msaa
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-v3d", feature = "host-test", feature = "report-es2"))]
#![allow(dead_code)]

use core::ffi::{c_char, c_int, c_void, CStr};
use std::collections::BTreeMap;
use std::ffi::CString;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering::SeqCst};
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
/// Jobs begun on a multisampled target, jobs begun, draws seen, draws whose
/// SAMPLE_STATE mask is not 0xf.
static MSAA_JOBS: AtomicU32 = AtomicU32::new(0);
static JOBS: AtomicU32 = AtomicU32::new(0);
static DRAWS: AtomicU32 = AtomicU32::new(0);
static ODD_MASKS: AtomicU32 = AtomicU32::new(0);

fn log_lines() -> Vec<String> {
    LOG.lock().unwrap_or_else(|e| e.into_inner()).clone()
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
    JOBS.fetch_add(1, SeqCst);
    if t.msaa || t.ms_address != 0 {
        MSAA_JOBS.fetch_add(1, SeqCst);
    }
    *j = V3dJob { bcl, rcl, ind, t, draws: 0, defaults: 0, broken: false, pack: V3dJobPack::ZERO };
    // SAFETY: as above.
    unsafe {
        (*bcl).used = 16;
        (*ind).used = 256;
    }
    t.width != 0 && t.height != 0
}

#[no_mangle]
unsafe extern "C" fn v3d_job_draw(j: *mut V3dJob, d: *const V3dJobDraw, _layout: *mut c_void) -> i32 {
    DRAWS.fetch_add(1, SeqCst);
    // SAFETY: the backend passes its live draw and state.
    if unsafe { (*(*d).st).sample_mask } != 0xf {
        ODD_MASKS.fetch_add(1, SeqCst);
    }
    // SAFETY: the backend passes its live job.
    unsafe {
        (*j).draws += 1;
        (*(*j).bcl).used += 120;
    }
    V3D_JOB_OK
}

#[no_mangle]
unsafe extern "C" fn v3d_job_end(
    j: *mut V3dJob,
    _c: *const V3dJobClear,
    _tile_list_start: *mut u32,
    _tile_list_end: *mut u32,
) -> bool {
    // SAFETY: the backend passes its live job; its lists are live.
    unsafe {
        (*(*j).bcl).used += 1;
        (*(*j).rcl).used = 200;
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

const VS: &str = "attribute vec4 p;\nvarying vec2 t;\nvoid main() { gl_Position = p; t = p.xy * 0.5 + 0.5; }\n";
const FS_COLOR: &str = "precision mediump float;\nvarying vec2 t;\nvoid main() { gl_FragColor = vec4(t, 0.5, 1.0); }\n";
const FS_FRAGDATA: &str =
    "precision mediump float;\nvarying vec2 t;\nvoid main() { gl_FragData[0] = vec4(t, 0.5, 1.0); }\n";
const FS_TEX: &str = "precision mediump float;\nvarying vec2 t;\nuniform sampler2D s;\n\
                      void main() { gl_FragColor = texture2D(s, t); }\n";
const FS_CUBE: &str = "precision mediump float;\nvarying vec2 t;\nuniform samplerCube s;\n\
                       void main() { gl_FragColor = textureCube(s, vec3(t, 0.5)); }\n";

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

/// The one rule of the file: what `f` does is accepted. No GL error and the
/// back end refused no draw (`v3d_refused_draws` is read as a difference).
fn accepted(what: &str, f: impl FnOnce()) {
    let base = v3d_refused_draws();
    f();
    // SAFETY: a plain GL call.
    unsafe { glFinish() };
    let (e, n) = (err(), v3d_refused_draws().wrapping_sub(base));
    assert_eq!((e, n), (GL_NO_ERROR, 0), "{what}: error, refused draws; {:?}", log_lines());
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

fn bind_fbo() -> GLuint {
    let mut fb = 0;
    // SAFETY: plain GL calls with room for one name.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glViewport(0, 0, W, H);
    }
    fb
}

fn renderbuffer(attachment: GLenum, format: GLenum, w: GLsizei, h: GLsizei) {
    // SAFETY: plain GL calls with room for one name.
    unsafe {
        let mut rb = 0;
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorage(GL_RENDERBUFFER, format, w, h);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, attachment, GL_RENDERBUFFER, rb);
    }
}

fn complete() -> bool {
    // SAFETY: a plain GL call.
    let ok = unsafe { glCheckFramebufferStatus(GL_FRAMEBUFFER) == GL_FRAMEBUFFER_COMPLETE };
    if !ok {
        eprintln!("skipped: the framebuffer is not complete (the GL core's answer, not a back-end refusal)");
    }
    ok
}

fn unbind_fbo(fb: GLuint) {
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fb);
    }
}

/* ---- the tests ---- */

/// A 2D texture bound to unit 0: `w` x `h` of `internal`/`format`/`type_`,
/// filled with 0x7f, the filter given.
fn texture(internal: GLenum, format: GLenum, type_: GLenum, w: GLsizei, h: GLsizei, min: GLenum) -> GLuint {
    let mut t = 0;
    let texels = vec![0x7fu8; (w * h) as usize * 4];
    // SAFETY: plain GL calls; the buffer holds 4 bytes a texel.
    unsafe {
        glActiveTexture(GL_TEXTURE0);
        glGenTextures(1, &mut t);
        glBindTexture(GL_TEXTURE_2D, t);
        glTexImage2D(GL_TEXTURE_2D, 0, internal as GLint, w, h, 0, format, type_, texels.as_ptr() as *const c_void);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, min as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, min as GLint);
    }
    t
}

fn geti(pname: GLenum) -> GLint {
    let mut v = -12345;
    // SAFETY: plain GL call with room for one value.
    unsafe { glGetIntegerv(pname, &mut v) };
    v
}

/// The GL core's value of `pname`, with no error.
fn int(what: &str, pname: GLenum) -> GLint {
    let v = geti(pname);
    assert_eq!(err(), GL_NO_ERROR, "{what}");
    v
}

fn rbo(pname: GLenum) -> GLint {
    let mut v = -12345;
    // SAFETY: plain GL call with room for one value.
    unsafe { glGetRenderbufferParameteriv(GL_RENDERBUFFER, pname, &mut v) };
    assert_eq!(err(), GL_NO_ERROR, "renderbuffer query {pname:#x}");
    v
}

fn status() -> GLenum {
    // SAFETY: a plain GL call.
    unsafe { glCheckFramebufferStatus(GL_FRAMEBUFFER) }
}

/// Every sample query reads zero in the framebuffer bound now.
fn single_sampled(what: &str) {
    assert_eq!(int(what, GL_SAMPLES), 0, "{what}: GL_SAMPLES");
    assert_eq!(int(what, GL_SAMPLE_BUFFERS), 0, "{what}: GL_SAMPLE_BUFFERS");
}

/// Draw a clear and a triangle; accepted, and no job is multisampled.
fn draws_single_sampled(what: &str) {
    let (m0, d0, o0) = (MSAA_JOBS.load(SeqCst), DRAWS.load(SeqCst), ODD_MASKS.load(SeqCst));
    accepted(what, || {
        // SAFETY: plain GL calls.
        unsafe {
            glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT);
        }
        program(FS_COLOR);
        draw_triangle();
    });
    assert!(DRAWS.load(SeqCst) > d0, "{what}: the draw reached the job");
    assert_eq!(MSAA_JOBS.load(SeqCst), m0, "{what}: a multisampled job");
    assert_eq!(ODD_MASKS.load(SeqCst), o0, "{what}: a SAMPLE_STATE mask other than 0xf");
}

#[test]
fn default_framebuffer_is_single_sampled() {
    let _g = rig();
    single_sampled("default framebuffer");
    assert!(int("max", GL_MAX_RENDERBUFFER_SIZE) > 0);
    draws_single_sampled("default framebuffer");
    assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE);
    assert_eq!(err(), GL_NO_ERROR);
}

/// ES 2.0 has no renderbuffer sample count and no GL_MAX_SAMPLES: both are
/// GL_INVALID_ENUM and the out-parameter is left alone.
#[test]
fn sample_counts_are_invalid_enum() {
    let _g = rig();
    let fb = bind_fbo();
    renderbuffer(GL_COLOR_ATTACHMENT0, GL_RGBA4, 8, 8);
    for (what, pname, is_rbo) in
        [("GL_RENDERBUFFER_SAMPLES", GL_RENDERBUFFER_SAMPLES, true), ("GL_MAX_SAMPLES", GL_MAX_SAMPLES, false)]
    {
        let mut v: GLint = 0x5a5a;
        // SAFETY: plain GL calls with room for one value.
        unsafe {
            if is_rbo {
                glGetRenderbufferParameteriv(GL_RENDERBUFFER, pname, &mut v);
            } else {
                glGetIntegerv(pname, &mut v);
            }
        }
        assert_eq!(err(), GL_INVALID_ENUM, "{what}");
        assert_eq!(v, 0x5a5a, "{what} wrote its answer");
    }
    let mut f = [7.0f32; 1];
    // SAFETY: plain GL call.
    unsafe { glGetFloatv(GL_MAX_SAMPLES, f.as_mut_ptr()) };
    assert_eq!(err(), GL_INVALID_ENUM, "glGetFloatv GL_MAX_SAMPLES");
    assert_eq!(f[0], 7.0);
    unbind_fbo(fb);
}

/// The renderbuffer queries of ES 2.0.25 6.1.14 and dEQP-GLES2
/// state_query.rbo: width, height, internal format exact; component sizes at
/// least the format's (the core may store more).
#[test]
fn renderbuffer_queries() {
    let _g = rig();
    let fb = bind_fbo();
    // (format, attachment, [r, g, b, a], depth, stencil)
    let cases: [(GLenum, GLenum, [GLint; 4], GLint, GLint); 5] = [
        (GL_RGBA4, GL_COLOR_ATTACHMENT0, [4, 4, 4, 4], 0, 0),
        (GL_RGB5_A1, GL_COLOR_ATTACHMENT0, [5, 5, 5, 1], 0, 0),
        (GL_RGB565, GL_COLOR_ATTACHMENT0, [5, 6, 5, 0], 0, 0),
        (GL_DEPTH_COMPONENT16, GL_DEPTH_ATTACHMENT, [0; 4], 16, 0),
        (GL_STENCIL_INDEX8, GL_STENCIL_ATTACHMENT, [0; 4], 0, 8),
    ];
    for (i, (fmt, att, rgba, depth, stencil)) in cases.into_iter().enumerate() {
        let (w, h) = (3 + i as GLsizei * 5, 7 + i as GLsizei * 2);
        renderbuffer(att, fmt, w, h);
        assert_eq!(err(), GL_NO_ERROR, "{fmt:#x} storage");
        assert_eq!(rbo(GL_RENDERBUFFER_WIDTH), w, "{fmt:#x} width");
        assert_eq!(rbo(GL_RENDERBUFFER_HEIGHT), h, "{fmt:#x} height");
        assert_eq!(rbo(GL_RENDERBUFFER_INTERNAL_FORMAT), fmt as GLint, "{fmt:#x} internal format");
        let sizes = [
            GL_RENDERBUFFER_RED_SIZE,
            GL_RENDERBUFFER_GREEN_SIZE,
            GL_RENDERBUFFER_BLUE_SIZE,
            GL_RENDERBUFFER_ALPHA_SIZE,
        ];
        for (k, p) in sizes.into_iter().enumerate() {
            assert!(rbo(p) >= rgba[k], "{fmt:#x} colour size {k}");
        }
        assert!(rbo(GL_RENDERBUFFER_DEPTH_SIZE) >= depth, "{fmt:#x} depth size");
        assert!(rbo(GL_RENDERBUFFER_STENCIL_SIZE) >= stencil, "{fmt:#x} stencil size");
    }
    /* an unknown pname, a wrong target and an unbound renderbuffer */
    let mut v: GLint = 0x5a5a;
    // SAFETY: plain GL calls with room for one value.
    unsafe {
        glGetRenderbufferParameteriv(GL_RENDERBUFFER, 0x1234, &mut v);
        assert_eq!(err(), GL_INVALID_ENUM);
        glGetRenderbufferParameteriv(GL_FRAMEBUFFER, GL_RENDERBUFFER_WIDTH, &mut v);
        assert_eq!(err(), GL_INVALID_ENUM);
        glBindRenderbuffer(GL_RENDERBUFFER, 0);
        glGetRenderbufferParameteriv(GL_RENDERBUFFER, GL_RENDERBUFFER_WIDTH, &mut v);
        assert_eq!(err(), GL_INVALID_OPERATION);
    }
    assert_eq!(v, 0x5a5a);
    unbind_fbo(fb);
}

/// Renderbuffer-backed FBOs: every ES 2.0 colour format with and without the
/// depth and stencil formats: complete, GL_SAMPLES / GL_SAMPLE_BUFFERS 0,
/// a draw accepted in a single-sampled job.
#[test]
fn renderbuffer_fbos_are_single_sampled() {
    let _g = rig();
    for color in [GL_RGBA4, GL_RGB5_A1, GL_RGB565] {
        for (depth, stencil) in [(false, false), (true, false), (false, true), (true, true)] {
            let fb = bind_fbo();
            renderbuffer(GL_COLOR_ATTACHMENT0, color, W, H);
            if depth {
                renderbuffer(GL_DEPTH_ATTACHMENT, GL_DEPTH_COMPONENT16, W, H);
            }
            if stencil {
                renderbuffer(GL_STENCIL_ATTACHMENT, GL_STENCIL_INDEX8, W, H);
            }
            let what = format!("colour {color:#x} depth {depth} stencil {stencil}");
            assert_eq!(err(), GL_NO_ERROR, "{what}");
            assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "{what}");
            single_sampled(&what);
            draws_single_sampled(&what);
            unbind_fbo(fb);
        }
    }
}

/// ES 2.0.25 4.4.5: an FBO with no image attached is
/// INCOMPLETE_MISSING_ATTACHMENT; one image of any kind (depth or stencil
/// alone) is enough, no colour attachment is required.
#[test]
fn missing_attachment() {
    let _g = rig();
    let fb = bind_fbo();
    assert_eq!(status(), GL_FRAMEBUFFER_INCOMPLETE_MISSING_ATTACHMENT, "empty FBO");
    renderbuffer(GL_DEPTH_ATTACHMENT, GL_DEPTH_COMPONENT16, W, H);
    assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "depth only");
    renderbuffer(GL_STENCIL_ATTACHMENT, GL_STENCIL_INDEX8, W, H);
    assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "depth and stencil only");
    assert_eq!(err(), GL_NO_ERROR);
    unbind_fbo(fb);
}

/// INCOMPLETE_ATTACHMENT: an attached renderbuffer with no storage;
/// INCOMPLETE_DIMENSIONS: attachments of different sizes.
#[test]
fn incomplete_attachment_and_dimensions() {
    let _g = rig();
    let fb = bind_fbo();
    // SAFETY: plain GL calls with room for one name.
    unsafe {
        let mut rb = 0;
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb);
    }
    assert_eq!(status(), GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT, "a renderbuffer without storage");
    renderbuffer(GL_COLOR_ATTACHMENT0, GL_RGBA4, 16, 16);
    assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE);
    renderbuffer(GL_DEPTH_ATTACHMENT, GL_DEPTH_COMPONENT16, 8, 16);
    assert_eq!(status(), GL_FRAMEBUFFER_INCOMPLETE_DIMENSIONS, "colour 16x16, depth 8x16");
    renderbuffer(GL_DEPTH_ATTACHMENT, GL_DEPTH_COMPONENT16, 16, 16);
    assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE);
    renderbuffer(GL_STENCIL_ATTACHMENT, GL_STENCIL_INDEX8, 16, 4);
    assert_eq!(status(), GL_FRAMEBUFFER_INCOMPLETE_DIMENSIONS, "stencil 16x4");
    assert_eq!(err(), GL_NO_ERROR);
    unbind_fbo(fb);
}

/// Texture attachments (RGBA and RGB, 8 bits, and the 16-bit packed types):
/// complete, single-sampled, drawn into without a refusal; a texture with no
/// level 0 image is INCOMPLETE_ATTACHMENT.
#[test]
fn texture_fbos_are_single_sampled() {
    let _g = rig();
    for (internal, format, ty) in [
        (GL_RGBA, GL_RGBA, GL_UNSIGNED_BYTE),
        (GL_RGB, GL_RGB, GL_UNSIGNED_BYTE),
        (GL_RGBA, GL_RGBA, GL_UNSIGNED_SHORT_4_4_4_4),
        (GL_RGBA, GL_RGBA, GL_UNSIGNED_SHORT_5_5_5_1),
        (GL_RGB, GL_RGB, GL_UNSIGNED_SHORT_5_6_5),
    ] {
        let fb = bind_fbo();
        let t = texture(internal, format, ty, W, H, GL_NEAREST);
        // SAFETY: plain GL call.
        unsafe { glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, t, 0) };
        let what = format!("texture {internal:#x}/{ty:#x}");
        assert_eq!(err(), GL_NO_ERROR, "{what}");
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "{what}");
        single_sampled(&what);
        draws_single_sampled(&what);
        /* with a depth and a stencil renderbuffer beside it */
        renderbuffer(GL_DEPTH_ATTACHMENT, GL_DEPTH_COMPONENT16, W, H);
        renderbuffer(GL_STENCIL_ATTACHMENT, GL_STENCIL_INDEX8, W, H);
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "{what} + depth + stencil");
        single_sampled(&what);
        draws_single_sampled(&format!("{what} + depth + stencil"));
        unbind_fbo(fb);
    }
    /* a texture with no level 0 image */
    let fb = bind_fbo();
    let mut t = 0;
    // SAFETY: plain GL calls with room for one name.
    unsafe {
        glGenTextures(1, &mut t);
        glBindTexture(GL_TEXTURE_2D, t);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, t, 0);
    }
    assert_eq!(status(), GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT, "a texture without an image");
    assert_eq!(err(), GL_NO_ERROR);
    unbind_fbo(fb);
}

/// glSampleCoverage, GL_SAMPLE_COVERAGE and GL_SAMPLE_ALPHA_TO_COVERAGE are
/// ES 2.0 state: they toggle without an error, read back, and a draw with
/// them on is neither refused nor multisampled.
#[test]
fn coverage_state_toggles() {
    let _g = rig();
    for cap in [GL_SAMPLE_COVERAGE, GL_SAMPLE_ALPHA_TO_COVERAGE] {
        // SAFETY: plain GL calls.
        unsafe {
            assert_eq!(glIsEnabled(cap), 0, "{cap:#x} starts off");
            glEnable(cap);
            assert_eq!(err(), GL_NO_ERROR);
            assert_eq!(glIsEnabled(cap), 1, "{cap:#x} on");
        }
    }
    // SAFETY: plain GL calls.
    unsafe {
        glSampleCoverage(0.25, 1);
        assert_eq!(err(), GL_NO_ERROR);
        let mut v = [0.0f32; 1];
        glGetFloatv(GL_SAMPLE_COVERAGE_VALUE, v.as_mut_ptr());
        assert_eq!(v[0], 0.25);
        assert_eq!(geti(GL_SAMPLE_COVERAGE_INVERT), 1);
        glSampleCoverage(7.0, 0);
        glGetFloatv(GL_SAMPLE_COVERAGE_VALUE, v.as_mut_ptr());
        assert_eq!(v[0], 1.0, "the value is clamped");
        assert_eq!(err(), GL_NO_ERROR);
    }
    draws_single_sampled("coverage on, default framebuffer");
    let fb = bind_fbo();
    renderbuffer(GL_COLOR_ATTACHMENT0, GL_RGBA4, W, H);
    renderbuffer(GL_DEPTH_ATTACHMENT, GL_DEPTH_COMPONENT16, W, H);
    assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE);
    draws_single_sampled("coverage on, FBO");
    unbind_fbo(fb);
    // SAFETY: plain GL calls.
    unsafe {
        for cap in [GL_SAMPLE_COVERAGE, GL_SAMPLE_ALPHA_TO_COVERAGE] {
            glDisable(cap);
            assert_eq!(glIsEnabled(cap), 0);
        }
        glSampleCoverage(1.0, 0);
    }
    assert_eq!(err(), GL_NO_ERROR);
}
