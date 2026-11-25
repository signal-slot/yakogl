// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! ES 2.0 context guard for 654b0620 (GL_IMPLEMENTATION_COLOR_READ_FORMAT/TYPE
//! for RGB10_A2 read buffers) and 5e94999a (glLinkProgram captures the program
//! binary; GL_NUM_PROGRAM_BINARY_FORMATS answers 1 in an ES 3.0 build), on the
//! V3D 4.2 (Raspberry Pi 4). The whole dEQP-GLES2 tree of the Raspberry Pi 4
//! (46f93767) predates both, and its stage-1 promise is 0 Fail and only the 34
//! NotSupported of tools/cts/es2-accepted.txt. dEQP-GLES2 asks for the
//! implementation read pair in functional.read_pixels.* and
//! functional.state_query.*, and checks the binary-format state against
//! GL_EXTENSIONS. The expected answers come from ES 2.0.25 and the extensions
//! GL_EXTENSIONS lists, not from the current output:
//!
//! - GL_IMPLEMENTATION_COLOR_READ_FORMAT/TYPE (ES 2.0 4.3.1, table 6.20) is a
//!   pair glReadPixels accepts on the read buffer; RGBA/UNSIGNED_BYTE always
//!   works. Pinned for the default framebuffer and for FBOs with RGBA4,
//!   RGB5_A1 and RGB565 renderbuffers (RGBA8 only if GL_OES_rgb8_rgba8 is
//!   listed). 654b0620's RGB10_A2 pair is ES 3.0: no such storage, and the
//!   packed type is refused.
//! - GL_NUM_SHADER_BINARY_FORMATS is 0 (no shader binary format is listed) and
//!   GL_SHADER_BINARY_FORMATS then writes nothing.
//! - GL_NUM_PROGRAM_BINARY_FORMATS and GL_PROGRAM_BINARY_FORMATS are
//!   GL_INVALID_ENUM unless GL_OES_get_program_binary is listed; when it is,
//!   glGetProgramBinaryOES / glProgramBinaryOES round-trip a linked GLSL ES
//!   1.00 program.
//! - glLinkProgram keeps answering GL_LINK_STATUS and the info log: a normal
//!   program links with GL_LINK_STATUS 1 and draws (no GL error,
//!   `v3d_refused_draws()` unchanged); a failing one (a varying the fragment
//!   shader reads and the vertex shader does not write) is 0 with a non-empty
//!   log, and cannot be used.
//!
//! The rig is the one of tests/v3d_es2ctx_refusals.rs: the job builder stood
//! in for, the real GL core (a `report-es2` build), V3D back end and compiler.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table,report-es2 --test v3d_es2ctx_drift_read_format
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

use yakogl::gl::buffer::glBindBuffer;
use yakogl::gl::draw::*;
use yakogl::gl::es3::{glGetProgramBinary, glProgramBinary};
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::vertex::{glEnableVertexAttribArray, glVertexAttribPointer};
use yakogl::v3d::backend::v3d_refused_draws;

const W: GLsizei = 32;
const H: GLsizei = 32;

/* ---- GL plumbing ---- */

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

fn extensions() -> Vec<String> {
    // SAFETY: plain GL call; the string is NUL-terminated and static.
    let s = unsafe { CStr::from_ptr(glGetString(GL_EXTENSIONS) as *const c_char) };
    s.to_str().unwrap().split(' ').filter(|e| !e.is_empty()).map(String::from).collect()
}

fn listed(name: &str) -> bool {
    extensions().iter().any(|e| e == name)
}

/// glGetIntegerv into a sentinel-filled array: (error, values written).
fn geti(pname: GLenum) -> (GLenum, Vec<GLint>) {
    const UNSET: GLint = 0x5a5a_5a5a;
    let mut v = [UNSET; 8];
    // SAFETY: plain GL call; eight slots.
    unsafe { glGetIntegerv(pname, v.as_mut_ptr()) };
    let e = err();
    (e, v.iter().copied().take_while(|&x| x != UNSET).collect())
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
        assert_eq!(ok, 1, "shader compiles:\n{text:?}");
        s
    }
}

/// The program of `vs`/`fs` after glLinkProgram: (name, GL_LINK_STATUS, info log).
fn link(vs: &str, fs: &str) -> (GLuint, GLint, String) {
    let (v, f) = (shader(GL_VERTEX_SHADER, vs), shader(GL_FRAGMENT_SHADER, fs));
    // SAFETY: plain GL calls; the log buffer is 512 bytes.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glBindAttribLocation(p, 0, c"p".as_ptr());
        glLinkProgram(p);
        let mut ok: GLint = -1;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        let mut buf = [0u8; 512];
        let mut n: GLsizei = 0;
        glGetProgramInfoLog(p, 512, &mut n, buf.as_mut_ptr() as *mut c_char);
        (p, ok, String::from_utf8_lossy(&buf[..n as usize]).into_owned())
    }
}

const VS100: &str = "attribute vec4 p;\nvarying vec4 v;\nvoid main() { v = p; gl_Position = p; }\n";
const FS100: &str = "precision mediump float;\nvarying vec4 v;\nvoid main() { gl_FragColor = v; }\n";
/* reads a varying the vertex shader does not write: ES 2.0 2.10.4 link error */
const FS_BAD: &str = "precision mediump float;\nvarying vec4 nowhere;\nvoid main() { gl_FragColor = nowhere; }\n";

static TRI: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 3.0, -1.0, 0.0, 1.0, -1.0, 3.0, 0.0, 1.0];

/// One triangle with program `p` on the current framebuffer: (GL error, draws the back end refused).
fn draw(p: GLuint) -> (GLenum, u64) {
    let base = v3d_refused_draws();
    // SAFETY: plain GL calls; TRI is static and outlives the draw.
    unsafe {
        glUseProgram(p);
        glViewport(0, 0, W, H);
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, TRI.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glDrawArrays(GL_TRIANGLES, 0, 3);
        glFinish();
        glUseProgram(0);
    }
    (err(), v3d_refused_draws().wrapping_sub(base) as u64)
}

/* ---- the implementation read format ---- */

fn read_pair() -> (GLenum, GLenum) {
    let (mut f, mut t): (GLint, GLint) = (0, 0);
    // SAFETY: plain GL calls; one slot each.
    unsafe {
        glGetIntegerv(GL_IMPLEMENTATION_COLOR_READ_FORMAT, &mut f);
        glGetIntegerv(GL_IMPLEMENTATION_COLOR_READ_TYPE, &mut t);
    }
    assert_eq!(err(), GL_NO_ERROR, "the read format/type queries are ES 2.0 state");
    (f as GLenum, t as GLenum)
}

fn read_ok(f: GLenum, t: GLenum) -> bool {
    let mut px = [0u8; 8 * 8 * 8];
    // SAFETY: plain GL call; 8x8 pixels of at most 8 bytes each.
    unsafe { glReadPixels(0, 0, 8, 8, f, t, px.as_mut_ptr() as *mut c_void) };
    err() == GL_NO_ERROR
}

/// The pair is one of ES 2.0's (table 3.4: RGBA or RGB with a type that fits
/// it) and glReadPixels takes it on the current read buffer; RGBA/UNSIGNED_BYTE too.
fn pin_read_format(what: &str) {
    let (f, t) = read_pair();
    assert!(
        matches!(
            (f, t),
            (GL_RGBA, GL_UNSIGNED_BYTE)
                | (GL_RGBA, GL_UNSIGNED_SHORT_4_4_4_4)
                | (GL_RGBA, GL_UNSIGNED_SHORT_5_5_5_1)
                | (GL_RGB, GL_UNSIGNED_BYTE)
                | (GL_RGB, GL_UNSIGNED_SHORT_5_6_5)
        ),
        "{what}: 0x{f:x}/0x{t:x} is not an ES 2.0 read pair"
    );
    assert!(read_ok(f, t), "{what}: glReadPixels refuses the implementation pair 0x{f:x}/0x{t:x}");
    assert!(read_ok(GL_RGBA, GL_UNSIGNED_BYTE), "{what}: RGBA/UNSIGNED_BYTE always works");
}

/// A 16x16 renderbuffer of `format` on a new FBO: (renderbuffer, FBO, status), the FBO bound.
fn rbo_fbo(format: GLenum) -> (GLuint, GLuint, GLenum) {
    // SAFETY: plain GL calls on the one context.
    unsafe {
        let (mut rb, mut fb) = (0, 0);
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorage(GL_RENDERBUFFER, format, 16, 16);
        let e = err();
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb);
        assert_eq!(e, GL_NO_ERROR, "storage 0x{format:x}");
        (rb, fb, glCheckFramebufferStatus(GL_FRAMEBUFFER))
    }
}

#[test]
fn read_format_default_framebuffer() {
    let _g = rig();
    pin_read_format("default framebuffer");
}

#[test]
fn read_format_fbo_colour_formats() {
    let _g = rig();
    let mut formats = vec![GL_RGBA4, GL_RGB5_A1, GL_RGB565];
    if listed("GL_OES_rgb8_rgba8") {
        formats.push(GL_RGBA8_OES);
    }
    for f in formats {
        let (rb, fb, status) = rbo_fbo(f);
        // ES 2.0 4.4.5: RGBA4, RGB5_A1 and RGB565 are colour-renderable
        assert_eq!(status, GL_FRAMEBUFFER_COMPLETE, "0x{f:x} renderbuffer FBO");
        pin_read_format(&format!("FBO 0x{f:x}"));
        // SAFETY: plain GL calls.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteFramebuffers(1, &fb);
            glDeleteRenderbuffers(1, &rb);
        }
        assert_eq!(err(), GL_NO_ERROR);
    }
    pin_read_format("default framebuffer again");
}

#[test]
fn rgb10_a2_is_no_es2_format() {
    let _g = rig();
    // 654b0620's RGB10_A2 pair belongs to ES 3.0: ES 2.0 has no such storage
    // and glReadPixels refuses the packed type.
    // SAFETY: plain GL calls.
    unsafe {
        let mut rb = 0;
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorage(GL_RENDERBUFFER, GL_RGB10_A2, 16, 16);
        assert_eq!(err(), GL_INVALID_ENUM, "RGB10_A2 is not an ES 2.0 renderbuffer format");
        glBindRenderbuffer(GL_RENDERBUFFER, 0);
        glDeleteRenderbuffers(1, &rb);
    }
    assert!(!read_ok(GL_RGBA, GL_UNSIGNED_INT_2_10_10_10_REV));
    let (f, t) = read_pair();
    assert_ne!(t, GL_UNSIGNED_INT_2_10_10_10_REV, "format 0x{f:x}");
}

/* ---- binary formats ---- */

#[test]
fn shader_binary_formats_are_none() {
    let _g = rig();
    // no shader binary extension is listed, so the count is 0
    assert!(!extensions().iter().any(|e| e.contains("shader_binary")), "{:?}", extensions());
    assert_eq!(geti(GL_NUM_SHADER_BINARY_FORMATS), (GL_NO_ERROR, vec![0]));
    let mut c: GLboolean = 0;
    // SAFETY: plain GL call.
    unsafe { glGetBooleanv(GL_SHADER_COMPILER, &mut c) };
    assert_eq!((c, err()), (1, GL_NO_ERROR), "an online compiler is there");
}

/// ES 2.0.25 6.2, table 6.20: GL_SHADER_BINARY_FORMATS is core state, a list
/// of GL_NUM_SHADER_BINARY_FORMATS (here 0) values: glGetIntegerv writes
/// nothing and sets no error. DEFECT (GL core, the same one
/// tests/gc_es2ctx_drift_read_format.rs holds): `get_integerv`
/// (src/gl/state.rs) has no arm for 0x8DF8 and answers GL_INVALID_ENUM.
/// Ignored until fixed.
#[test]
#[ignore = "defect: glGetIntegerv(GL_SHADER_BINARY_FORMATS) is GL_INVALID_ENUM, ES 2.0 says an empty list and no error"]
fn shader_binary_formats_list_is_empty() {
    let _g = rig();
    assert_eq!(geti(GL_SHADER_BINARY_FORMATS), (GL_NO_ERROR, vec![]));
}

#[test]
fn program_binary_formats_follow_the_extension() {
    let _g = rig();
    if !listed("GL_OES_get_program_binary") {
        // the enums exist only with the extension: INVALID_ENUM, nothing written
        assert_eq!(geti(GL_NUM_PROGRAM_BINARY_FORMATS_OES), (GL_INVALID_ENUM, vec![]));
        assert_eq!(geti(GL_PROGRAM_BINARY_FORMATS_OES), (GL_INVALID_ENUM, vec![]));
        let (p, ok, log) = link(VS100, FS100);
        assert_eq!(ok, 1, "{log}");
        let mut n: GLint = 0x5a5a;
        // SAFETY: plain GL calls.
        unsafe {
            glGetProgramiv(p, GL_PROGRAM_BINARY_LENGTH_OES, &mut n);
            assert_eq!((n, err()), (0x5a5a, GL_INVALID_ENUM), "GL_PROGRAM_BINARY_LENGTH_OES needs the extension");
            glDeleteProgram(p);
        }
        return;
    }
    // GL_OES_get_program_binary: glGetProgramBinaryOES and glProgramBinaryOES are
    // the entry points glGetProgramBinary and glProgramBinary here
    let (e, nfmt) = geti(GL_NUM_PROGRAM_BINARY_FORMATS_OES);
    assert_eq!(e, GL_NO_ERROR);
    assert!(nfmt[0] >= 1, "the extension is listed, so a linked program has a binary format");
    let (e, fmts) = geti(GL_PROGRAM_BINARY_FORMATS_OES);
    assert_eq!((e, fmts.len() as GLint), (GL_NO_ERROR, nfmt[0]));
    let (p, ok, log) = link(VS100, FS100);
    assert_eq!(ok, 1, "{log}");
    // SAFETY: plain GL calls; the buffer is as long as the length asked.
    unsafe {
        let mut n: GLint = 0;
        glGetProgramiv(p, GL_PROGRAM_BINARY_LENGTH_OES, &mut n);
        assert!(n > 0 && err() == GL_NO_ERROR);
        let mut buf = vec![0u8; n as usize];
        let (mut got, mut fmt): (GLsizei, GLenum) = (0, 0);
        glGetProgramBinary(p, n, &mut got, &mut fmt, buf.as_mut_ptr() as *mut c_void);
        assert_eq!((got, err()), (n, GL_NO_ERROR));
        assert!(fmts.contains(&(fmt as GLint)));
        let q = glCreateProgram();
        glProgramBinary(q, fmt, buf.as_ptr() as *const c_void, n);
        let mut lk: GLint = 0;
        glGetProgramiv(q, GL_LINK_STATUS, &mut lk);
        assert_eq!((lk, err()), (1, GL_NO_ERROR), "the binary links again");
        assert_eq!(draw(q), (GL_NO_ERROR, 0), "the loaded program draws; {:?}", log_lines());
        glDeleteProgram(q);
        glDeleteProgram(p);
    }
}

#[test]
fn link_status_and_log() {
    let _g = rig();
    let (p, ok, log) = link(VS100, FS100);
    assert_eq!(ok, 1, "a normal program links: {log}");
    assert_eq!(err(), GL_NO_ERROR);
    // the link's captured binary (5e94999a) leaves the program drawable
    assert_eq!(draw(p), (GL_NO_ERROR, 0), "the linked program draws; {:?}", log_lines());

    let (b, ok, log) = link(VS100, FS_BAD);
    assert_eq!(ok, 0, "a fragment varying nothing writes is a link error (ES 2.0 2.10.4)");
    assert!(!log.is_empty(), "the failed link leaves an info log");
    assert_eq!(err(), GL_NO_ERROR, "a failed link is no GL error");
    // a failed program cannot be used (ES 2.0 2.10.3), a good one can
    // SAFETY: plain GL calls.
    unsafe {
        glUseProgram(b);
        assert_eq!(err(), GL_INVALID_OPERATION);
        glUseProgram(p);
        assert_eq!(err(), GL_NO_ERROR);
        glUseProgram(0);
        glLinkProgram(b);
        let mut ok2: GLint = -1;
        glGetProgramiv(b, GL_LINK_STATUS, &mut ok2);
        assert_eq!((ok2, err()), (0, GL_NO_ERROR));
        // relinking the good program keeps it linked and drawable
        glLinkProgram(p);
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok2);
        assert_eq!((ok2, err()), (1, GL_NO_ERROR));
        assert_eq!(draw(p), (GL_NO_ERROR, 0), "the relinked program draws; {:?}", log_lines());
        glDeleteProgram(b);
        glDeleteProgram(p);
    }
}
