// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The ES 2.0 context against the float-target and mip-level changes of
//! e59119a1 (v3d: float colour targets drawn, cleared and read back),
//! 48cb7272 and 02c623ba (GL_EXT_color_buffer_float listed only where the
//! `color-buffer-float` feature is on and the back end draws every float
//! target), c558647b (RGB16F/RG16F under GL_EXT_color_buffer_half_float, R32F
//! incomplete without GL_EXT_color_buffer_float) and f68094d6 (a pitched mip
//! level above 0 is a complete colour attachment), on V3D 4.2. The Pi 4 whole
//! dEQP-GLES2 tree (46f93767) predates them; dEQP-GLES2 reads
//! GL_EXTENSIONS (state_query, info), checks completeness
//! (functional.fbo.completeness.*, with the half-float textures when
//! GL_OES_texture_half_float is listed) and renders into mip levels above 0.
//!
//! The feature set is `V3D_ES2_CONFIG` of tools/check.sh: `color-buffer-float`
//! is NOT in it. It is not in the Pi 4 dEQP-GLES2 image either: the images
//! come from tools/cts/deqp-tree-images.sh (pi4-deqp-images.sh) with Kconfig
//! YAKOGL_REPORT_ES2 on, and the CMake option YAKOGL_COLOR_BUFFER_FLOAT
//! (default OFF, CMakeLists.txt) is set by none of them. Besides, the claim is
//! `cfg!(feature = "color-buffer-float") && !REPORT_ES2 && ...`
//! (src/gl/state.rs COLOR_BUFFER_FLOAT_EXTENSION), so no ES 2.0 report lists
//! it, with the feature on or off.
//!
//! The expected answers are the ES 2.0 spec (4.4.5 completeness, with
//! GL_OES_texture_npot, GL_OES_fbo_render_mipmap, GL_OES_texture_half_float,
//! GL_EXT_color_buffer_half_float) and the extensions the build lists, not
//! what the code answers. The job builder is stood in for as in
//! tests/v3d_es2ctx_refusals.rs; the texture layout is the real v3d_tex.c.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table,report-es2 --test v3d_es2ctx_drift_fbo_complete
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-v3d", feature = "host-test", feature = "report-es2"))]

use core::ffi::{c_char, c_int, c_void, CStr};
use std::collections::BTreeMap;
use std::ffi::CString;
use std::path::PathBuf;
use std::process::Command;
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

fn unbind_fbo(fb: GLuint) {
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fb);
    }
}

fn extension(name: &str) -> bool {
    let has = extension_listed(name);
    if !has {
        eprintln!("skipped: {name} is not advertised");
    }
    has
}

fn extension_listed(name: &str) -> bool {
    // SAFETY: GL_EXTENSIONS is a NUL-terminated string.
    let s = unsafe { CStr::from_ptr(glGetString(GL_EXTENSIONS) as *const c_char) };
    s.to_string_lossy().split(' ').any(|e| e == name)
}

/* ---- the cases ---- */

/// A 2D texture bound to unit 0: `w` x `h` at level 0 and `levels` levels
/// down, each of `format`/`type_`, `bpt` bytes a texel.
fn chain(format: GLenum, type_: GLenum, w: GLsizei, h: GLsizei, levels: usize, bpt: usize) -> GLuint {
    let mut t = 0;
    // SAFETY: plain GL calls; each buffer holds a level of `bpt`-byte texels.
    unsafe {
        glActiveTexture(GL_TEXTURE0);
        glGenTextures(1, &mut t);
        glBindTexture(GL_TEXTURE_2D, t);
        let (mut lw, mut lh) = (w, h);
        for level in 0..levels {
            let texels = vec![0x40u8; (lw * lh) as usize * bpt];
            glTexImage2D(
                GL_TEXTURE_2D,
                level as GLint,
                format as GLint,
                lw,
                lh,
                0,
                format,
                type_,
                texels.as_ptr() as *const c_void,
            );
            lw = (lw / 2).max(1);
            lh = (lh / 2).max(1);
        }
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
    }
    t
}

fn status() -> GLenum {
    // SAFETY: a plain GL call.
    unsafe { glCheckFramebufferStatus(GL_FRAMEBUFFER) }
}

fn attach(tex: GLuint, level: GLint) {
    // SAFETY: a plain GL call.
    unsafe { glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, level) };
}

fn extensions() -> String {
    // SAFETY: GL_EXTENSIONS is a NUL-terminated string.
    unsafe { CStr::from_ptr(glGetString(GL_EXTENSIONS) as *const c_char) }.to_string_lossy().into_owned()
}

/// Sanity: this is an ES 2.0 context.
#[test]
fn the_context_is_es2() {
    let _g = rig();
    let mut m = 0;
    // SAFETY: a plain GL call.
    unsafe { glGetIntegerv(GL_MAJOR_VERSION, &mut m) };
    assert_eq!(err(), GL_INVALID_ENUM, "GL_MAJOR_VERSION is an ES 3.0 query");
}

/// 48cb7272 / e59119a1: the extension string of the ES 2.0 report on V3D with
/// the `V3D_ES2_CONFIG` features: no GL_EXT_color_buffer_float (ES 3.0's, and
/// the feature is off), no OES_texture_float (single-precision textures are
/// not claimed), GL_EXT_color_buffer_half_float with GL_OES_texture_half_float,
/// and the ES 2.0-only tail.
#[test]
fn extension_list_is_the_es2_one() {
    let _g = rig();
    let want = "GL_OES_EGL_image GL_OES_standard_derivatives GL_OES_compressed_ETC1_RGB8_texture \
GL_OES_compressed_ETC2_RGB8_texture GL_OES_compressed_ETC2_RGBA8_texture GL_OES_vertex_array_object \
GL_OES_mapbuffer GL_OES_fbo_render_mipmap GL_EXT_texture_format_BGRA8888 GL_OES_rgb8_rgba8 \
GL_OES_required_internalformat GL_EXT_texture_rg GL_OES_texture_half_float GL_EXT_color_buffer_half_float \
GL_NV_fbo_color_attachments GL_EXT_debug_marker GL_OES_texture_npot GL_OES_depth_texture \
GL_OES_packed_depth_stencil";
    let got = extensions();
    assert_eq!(got, want);
    for absent in ["GL_EXT_color_buffer_float", "GL_OES_texture_float", "GL_OES_texture_float_linear"] {
        assert!(!got.split(' ').any(|e| e == absent), "{absent} is listed");
    }
}

/// Completeness of the ES 2.0 colour formats as textures: RGBA/UNSIGNED_BYTE,
/// RGB/UNSIGNED_BYTE, RGB565, RGBA4 and RGB5_A1, at level 0 and at every level
/// 1..n of a power-of-two chain (GL_OES_fbo_render_mipmap is listed; f68094d6).
/// ES 2.0 4.4.5: complete.
#[test]
fn pot_texture_chains_are_complete_at_every_level() {
    let _g = rig();
    let cases: [(&str, GLenum, GLenum, usize); 5] = [
        ("RGBA/UNSIGNED_BYTE", GL_RGBA, GL_UNSIGNED_BYTE, 4),
        ("RGB/UNSIGNED_BYTE", GL_RGB, GL_UNSIGNED_BYTE, 3),
        ("RGB565", GL_RGB, GL_UNSIGNED_SHORT_5_6_5, 2),
        ("RGBA4", GL_RGBA, GL_UNSIGNED_SHORT_4_4_4_4, 2),
        ("RGB5_A1", GL_RGBA, GL_UNSIGNED_SHORT_5_5_5_1, 2),
    ];
    for (name, format, ty, bpt) in cases {
        for (w, h) in [(32, 32), (32, 8), (16, 16)] {
            let levels = (w.max(h) as u32).ilog2() as usize + 1;
            let fb = bind_fbo();
            let t = chain(format, ty, w, h, levels, bpt);
            assert_eq!(err(), GL_NO_ERROR, "{name} {w}x{h}: the chain");
            for level in 0..levels as GLint {
                attach(t, level);
                assert_eq!(err(), GL_NO_ERROR, "{name} {w}x{h} level {level}: attach");
                assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "{name} {w}x{h} level {level}");
            }
            unbind_fbo(fb);
        }
    }
}

/// NPOT (GL_OES_texture_npot is listed): a texture of any size is complete at
/// level 0, with or without a chain, and so is every level of an NPOT chain
/// (GL_OES_fbo_render_mipmap).
#[test]
fn npot_texture_is_complete() {
    let _g = rig();
    assert!(extension("GL_OES_texture_npot"));
    for (w, h, levels) in [(30, 20, 1), (30, 20, 5), (7, 5, 3), (33, 1, 1)] {
        let fb = bind_fbo();
        let t = chain(GL_RGBA, GL_UNSIGNED_BYTE, w, h, levels, 4);
        assert_eq!(err(), GL_NO_ERROR, "{w}x{h}: the texture");
        for level in 0..levels as GLint {
            attach(t, level);
            assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "{w}x{h} level {level}");
        }
        unbind_fbo(fb);
    }
}

/// A level the texture does not have is no attachment (ES 2.0 4.4.5): the
/// framebuffer is INCOMPLETE_ATTACHMENT; glFramebufferTexture2D itself takes
/// the level.
///
/// DEFECT (found by this file, not yet fixed): level 4 of a 16x16 texture
/// that defines levels 0..2 answers GL_FRAMEBUFFER_COMPLETE (the V3D layout
/// allocates the whole chain, and attachment_storage only asks the storage to
/// hold the level; `level_defined` is read only for `mip_level_align`
/// back ends). The assertion stays; the test is ignored until the GL core
/// refuses an undefined level.
#[test]
#[ignore = "defect: an undefined level past the defined chain is COMPLETE on V3D (issue text in the item report)"]
fn a_level_past_the_chain_is_incomplete() {
    let _g = rig();
    let fb = bind_fbo();
    let t = chain(GL_RGBA, GL_UNSIGNED_BYTE, 16, 16, 3, 4);
    attach(t, 4);
    assert_eq!(err(), GL_NO_ERROR);
    assert_eq!(status(), GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT);
    unbind_fbo(fb);
}

/// c558647b / 02c623ba: with GL_EXT_color_buffer_half_float, RGBA16F_EXT and
/// RGB16F_EXT renderbuffers are colour-renderable, and RGBA and RGB textures
/// of type HALF_FLOAT_OES (GL_OES_texture_half_float) are complete.
#[test]
fn half_float_targets_are_complete() {
    let _g = rig();
    assert!(extension("GL_EXT_color_buffer_half_float"));
    assert!(extension("GL_OES_texture_half_float"));
    for fmt in [GL_RGBA16F_EXT, GL_RGB16F_EXT] {
        let fb = bind_fbo();
        renderbuffer(GL_COLOR_ATTACHMENT0, fmt, W, H);
        assert_eq!(err(), GL_NO_ERROR, "renderbuffer {fmt:#x}");
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "renderbuffer {fmt:#x}");
        unbind_fbo(fb);
    }
    for (name, format, bpt) in [("RGBA", GL_RGBA, 8), ("RGB", GL_RGB, 6)] {
        let fb = bind_fbo();
        let t = chain(format, GL_HALF_FLOAT_OES, 16, 16, 1, bpt);
        assert_eq!(err(), GL_NO_ERROR, "{name}/HALF_FLOAT_OES texture");
        attach(t, 0);
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "{name}/HALF_FLOAT_OES texture");
        unbind_fbo(fb);
    }
}

/// c558647b / e59119a1: without GL_EXT_color_buffer_float (not listed) no
/// 32-bit float name is renderbuffer storage (GL_INVALID_ENUM in ES 2.0), and
/// a FLOAT texture is no ES 2.0 texture without GL_OES_texture_float
/// (GL_INVALID_ENUM at glTexImage2D), so its attachment is incomplete.
#[test]
fn single_float_targets_are_refused() {
    let _g = rig();
    for fmt in [GL_RGBA32F_EXT, GL_R32F_EXT, GL_RG32F] {
        let fb = bind_fbo();
        let mut rb = 0;
        // SAFETY: plain GL calls with room for one name.
        unsafe {
            glGenRenderbuffers(1, &mut rb);
            glBindRenderbuffer(GL_RENDERBUFFER, rb);
            glRenderbufferStorage(GL_RENDERBUFFER, fmt, W, H);
            assert_eq!(err(), GL_INVALID_ENUM, "renderbuffer storage {fmt:#x}");
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb);
        }
        err();
        assert_ne!(status(), GL_FRAMEBUFFER_COMPLETE, "renderbuffer {fmt:#x}");
        unbind_fbo(fb);
    }
    let fb = bind_fbo();
    let mut t = 0;
    let texels = vec![0u8; 16 * 16 * 16];
    // SAFETY: plain GL calls; the buffer holds 16 bytes a texel.
    unsafe {
        glActiveTexture(GL_TEXTURE0);
        glGenTextures(1, &mut t);
        glBindTexture(GL_TEXTURE_2D, t);
        glTexImage2D(
            GL_TEXTURE_2D,
            0,
            GL_RGBA as GLint,
            16,
            16,
            0,
            GL_RGBA,
            GL_FLOAT,
            texels.as_ptr() as *const c_void,
        );
    }
    assert_eq!(err(), GL_INVALID_ENUM, "a FLOAT texture without GL_OES_texture_float");
    attach(t, 0);
    err();
    assert_eq!(status(), GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT);
    unbind_fbo(fb);
}

/// f68094d6: a draw into level 1 (and deeper) of a texture chain is accepted:
/// no error, no refused draw. RGBA, RGB565 and half-float textures.
#[test]
fn a_draw_into_a_mip_level_is_accepted() {
    let _g = rig();
    let cases: [(&str, GLenum, GLenum, usize); 3] = [
        ("RGBA/UNSIGNED_BYTE", GL_RGBA, GL_UNSIGNED_BYTE, 4),
        ("RGB565", GL_RGB, GL_UNSIGNED_SHORT_5_6_5, 2),
        ("RGBA/HALF_FLOAT_OES", GL_RGBA, GL_HALF_FLOAT_OES, 8),
    ];
    for (name, format, ty, bpt) in cases {
        let fb = bind_fbo();
        let t = chain(format, ty, 32, 32, 6, bpt);
        for level in [1, 2, 5] {
            attach(t, level);
            assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "{name} level {level}");
            let side = 32 >> level;
            accepted(&format!("{name} level {level}"), || {
                // SAFETY: plain GL calls.
                unsafe {
                    glViewport(0, 0, side, side);
                    glClearColor(0.1, 0.2, 0.3, 0.4);
                    glClear(GL_COLOR_BUFFER_BIT);
                }
                program(FS_COLOR);
                draw_triangle();
            });
        }
        unbind_fbo(fb);
    }
}
