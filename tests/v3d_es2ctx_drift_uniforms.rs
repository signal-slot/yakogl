// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The ES 2.0 context against 3cbecbb9 ("Records and the GL program take more
//! than 255 active uniforms") on V3D 4.2. That commit made the shader record's
//! uniform count 16 bits (src/shaderc/online/v3d_abi.rs), moved a program's
//! locations into a per-context pool and changed a location to
//! `index | element << 16` (src/gl/program.rs, src/gl/draw.rs). The Pi 4
//! whole dEQP-GLES2 tree (46f93767) predates it, and its stage-1 promise is
//! 0 Fail. dEQP-GLES2 functional.uniform_api.* (random.*, value.*,
//! info_query.*) reads exactly what changed: locations, glGetActiveUniform,
//! glGetUniform* values, GL_ACTIVE_UNIFORMS, GL_ACTIVE_UNIFORM_MAX_LENGTH.
//!
//! The rig is tests/v3d_es2ctx_refusals.rs's: a `report-es2` build, GLSL ES
//! 1.00 shaders, the real GL core and V3D back end, the real
//! driver/src/v3d/v3d_tex.c; the job builder is stood in for and, as in
//! tests/v3d_uniform_host.rs, records the two uniform blocks of every draw.
//!
//! Expected values are ES 2.0.25 (section 2.10.4 and the glGetActiveUniform,
//! glGetUniform and glUniform entries) and dEQP-GLES2's uniform_api checks,
//! not the current output:
//!
//! * `glGetActiveUniform` names an array `a[0]` with its size, a struct
//!   member `s.m` and an array of structs `t[i].m` with size 1 each; the
//!   count is GL_ACTIVE_UNIFORMS, the longest name plus its NUL is
//!   GL_ACTIVE_UNIFORM_MAX_LENGTH; a uniform of both stages counts once.
//! * `glGetUniformLocation` answers one location for `a` and `a[0]`, another
//!   for each element, -1 for an element past the size, an inactive name, and
//!   a `gl_` name.
//! * glUniform* of the wrong type or count is INVALID_OPERATION (a bool takes
//!   glUniform1i/1f), a transposed matrix INVALID_VALUE, location -1 is
//!   ignored without an error, no current program is INVALID_OPERATION;
//!   glGetUniform* of a bad location is INVALID_OPERATION; a bool reads back
//!   as 0 or 1.
//! * a draw is not refused and the draw's blocks hold the values set.
//! * a program of 580 active uniforms (two stages of 64 and 65 structs of
//!   five and four vec4 members, 320 and 260 vectors, under the 384 each the
//!   back end reports), the path past 255 records, links and does the same.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table,report-es2 --test v3d_es2ctx_drift_uniforms
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

/// The two uniform blocks of one draw, as the back end hands them over.
#[derive(Clone, Debug, Default)]
struct Seen {
    vs: Vec<u32>,
    fs: Vec<u32>,
}

static DRAWS: Mutex<Vec<Seen>> = Mutex::new(Vec::new());

fn block(s: &V3dUniformStorage) -> Vec<u32> {
    if s.words.is_null() || s.count == 0 {
        Vec::new()
    } else {
        // SAFETY: the backend's live host array, `count` dwords.
        unsafe { std::slice::from_raw_parts(s.words, s.count as usize) }.to_vec()
    }
}

#[no_mangle]
unsafe extern "C" fn v3d_job_draw(j: *mut V3dJob, d: *const V3dJobDraw, _layout: *mut c_void) -> i32 {
    // SAFETY: the backend passes its live job and a draw whose pointers live for the call.
    let (j, d) = unsafe { (&mut *j, &*d) };
    j.draws += 1;
    // SAFETY: the job's BCL is live.
    unsafe { (*j.bcl).used += 120 };
    DRAWS.lock().unwrap_or_else(|e| e.into_inner()).push(Seen { vs: block(&d.vs_storage), fs: block(&d.fs_storage) });
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

use yakogl::gl::draw::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::vertex::{glEnableVertexAttribArray, glVertexAttribPointer};
use yakogl::v3d::backend::v3d_refused_draws;

const W: GLsizei = 32;
const H: GLsizei = 32;

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
        let mut log = vec![0u8; 512];
        glGetShaderInfoLog(s, 512, core::ptr::null_mut(), log.as_mut_ptr() as *mut c_char);
        let log = String::from_utf8_lossy(&log).trim_end_matches('\0').to_string();
        assert_eq!(ok, 1, "a {ty:#x} shader of {} bytes did not compile: {log}", text.as_bytes().len());
        s
    }
}

/// Link `vs` and `fs` (attribute `p` at 0) and make the program current.
fn link(vs: &str, fs: &str) -> GLuint {
    let (v, f) = (shader(GL_VERTEX_SHADER, vs), shader(GL_FRAGMENT_SHADER, fs));
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glBindAttribLocation(p, 0, c"p".as_ptr());
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "link: {:?}", log_lines());
        glUseProgram(p);
        p
    }
}

static TRI: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 3.0, -1.0, 0.0, 1.0, -1.0, 3.0, 0.0, 1.0];

/// One draw, which must be accepted: no GL error, no refused draw, and one
/// draw handed to the job builder. Returns the blocks it saw.
fn draw_accepted(what: &str) -> Seen {
    let base = v3d_refused_draws();
    let before = DRAWS.lock().unwrap_or_else(|e| e.into_inner()).len();
    // SAFETY: TRI is static and outlives the draw.
    unsafe {
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, TRI.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glDrawArrays(GL_TRIANGLES, 0, 3);
        glFinish();
    }
    let (e, n) = (err(), v3d_refused_draws().wrapping_sub(base));
    assert_eq!((e, n), (GL_NO_ERROR, 0), "{what}: error, refused draws; {:?}", log_lines());
    let d = DRAWS.lock().unwrap_or_else(|e| e.into_inner());
    assert_eq!(d.len(), before + 1, "{what}: one draw reached the job builder; {:?}", log_lines());
    d[before].clone()
}

use yakogl::gl::buffer::glBindBuffer;

fn loc(p: GLuint, name: &str) -> GLint {
    let c = CString::new(name).unwrap();
    // SAFETY: a valid program and a NUL-terminated name.
    unsafe { glGetUniformLocation(p, c.as_ptr()) }
}

fn loc_ok(p: GLuint, name: &str) -> GLint {
    let l = loc(p, name);
    assert!(l >= 0, "{name} has a location");
    l
}

fn prog_i(p: GLuint, pname: GLenum) -> GLint {
    let mut v = -1;
    // SAFETY: a plain GL call with room for one value.
    unsafe { glGetProgramiv(p, pname, &mut v) };
    assert_eq!(err(), GL_NO_ERROR);
    v
}

/// Every active uniform: (name, size, type), in index order.
fn actives(p: GLuint) -> Vec<(String, GLint, GLenum)> {
    let n = prog_i(p, GL_ACTIVE_UNIFORMS) as u32;
    let max = prog_i(p, GL_ACTIVE_UNIFORM_MAX_LENGTH);
    (0..n)
        .map(|i| {
            let mut buf = vec![0x7fu8; max as usize + 8];
            let (mut len, mut size, mut ty) = (-1, -1, 0);
            // SAFETY: the buffer is the length passed.
            unsafe { glGetActiveUniform(p, i, max, &mut len, &mut size, &mut ty, buf.as_mut_ptr() as *mut c_char) };
            assert_eq!(err(), GL_NO_ERROR, "glGetActiveUniform({i})");
            let name = String::from_utf8(buf[..len as usize].to_vec()).unwrap();
            assert_eq!(buf[len as usize], 0, "{name}: NUL-terminated");
            (name, size, ty)
        })
        .collect()
}

fn getf(p: GLuint, l: GLint, n: usize) -> Vec<f32> {
    let mut v = vec![f32::from_bits(0x7fc0_1234); n + 1];
    // SAFETY: room for n + 1 values.
    unsafe { glGetUniformfv(p, l, v.as_mut_ptr()) };
    assert_eq!(err(), GL_NO_ERROR, "glGetUniformfv({l})");
    assert!(v[n].to_bits() == 0x7fc0_1234, "glGetUniformfv({l}) wrote past {n} values");
    v.truncate(n);
    v
}

fn geti(p: GLuint, l: GLint, n: usize) -> Vec<i32> {
    let mut v = vec![0x5a5a_5a5a; n + 1];
    // SAFETY: room for n + 1 values.
    unsafe { glGetUniformiv(p, l, v.as_mut_ptr()) };
    assert_eq!(err(), GL_NO_ERROR, "glGetUniformiv({l})");
    assert_eq!(v[n], 0x5a5a_5a5a, "glGetUniformiv({l}) wrote past {n} values");
    v.truncate(n);
    v
}

/// Distinct, exactly representable floats: `n` of them for `seed`.
fn floats(seed: u32, n: usize) -> Vec<f32> {
    (0..n).map(|i| (seed * 4096 + i as u32 + 1) as f32 * 0.125).collect()
}

fn bits(f: &[f32]) -> Vec<u32> {
    f.iter().map(|v| v.to_bits()).collect()
}

fn has_run(words: &[u32], run: &[u32]) -> bool {
    words.windows(run.len()).any(|w| w == run)
}

/* ---- the program of every kind ---- */

/* a vec4 array, a mat4 (VS); a struct, an array of structs, a struct holding
 * an array, a sampler, bool, bvec3, ivec2 (FS); `shared` is in both stages */
const VS_KINDS: &str = "attribute vec4 p;\nuniform vec4 a[3];\nuniform mat4 m;\nuniform mediump float shared;\n\
    varying vec2 t;\n\
    void main() { gl_Position = m * p + a[0] + a[1] + a[2] + vec4(shared); t = p.xy * 0.5 + 0.5; }\n";
const FS_KINDS: &str = "precision mediump float;\nvarying vec2 t;\n\
    struct S { float f; vec2 v; bool b; int i; };\n\
    struct T { vec2 x; float y; };\n\
    struct U { float m[3]; };\n\
    uniform S s;\nuniform T tt[2];\nuniform U u;\nuniform sampler2D smp;\n\
    uniform bool flag;\nuniform bvec3 bv;\nuniform ivec2 iv;\nuniform mediump float shared;\n\
    void main() {\n\
      float x = s.f + s.v.x + s.v.y + float(s.i) + tt[0].x.x + tt[0].x.y + tt[0].y + tt[1].x.x + tt[1].x.y + tt[1].y\n\
              + u.m[0] + u.m[1] + u.m[2] + float(iv.x + iv.y) + shared;\n\
      if (flag && s.b && bv.x && bv.y && bv.z) x += 1.0;\n\
      gl_FragColor = texture2D(smp, t) * x;\n\
    }\n";

use std::collections::BTreeSet;

fn kinds_expected() -> BTreeSet<(String, GLint, GLenum)> {
    [
        ("a[0]", 3, GL_FLOAT_VEC4),
        ("m", 1, GL_FLOAT_MAT4),
        ("shared", 1, GL_FLOAT),
        ("s.f", 1, GL_FLOAT),
        ("s.v", 1, GL_FLOAT_VEC2),
        ("s.b", 1, GL_BOOL),
        ("s.i", 1, GL_INT),
        ("tt[0].x", 1, GL_FLOAT_VEC2),
        ("tt[0].y", 1, GL_FLOAT),
        ("tt[1].x", 1, GL_FLOAT_VEC2),
        ("tt[1].y", 1, GL_FLOAT),
        ("u.m[0]", 3, GL_FLOAT),
        ("smp", 1, GL_SAMPLER_2D),
        ("flag", 1, GL_BOOL),
        ("bv", 1, GL_BOOL_VEC3),
        ("iv", 1, GL_INT_VEC2),
    ]
    .into_iter()
    .map(|(n, s, t)| (n.to_string(), s, t))
    .collect()
}

/* ---- the cases ---- */

/// Sanity: this is an ES 2.0 context on the V3D, and the back end's
/// uniform vectors are the ES 2.0 report's 384 a stage.
#[test]
fn the_context_is_es2_with_384_vectors() {
    let _g = rig();
    // SAFETY: plain GL calls.
    unsafe {
        let mut m = 0;
        glGetIntegerv(GL_MAJOR_VERSION, &mut m);
        assert_eq!(err(), GL_INVALID_ENUM, "GL_MAJOR_VERSION is an ES 3.0 query");
        let (mut v, mut f) = (0, 0);
        glGetIntegerv(GL_MAX_VERTEX_UNIFORM_VECTORS, &mut v);
        glGetIntegerv(GL_MAX_FRAGMENT_UNIFORM_VECTORS, &mut f);
        assert_eq!((err(), v, f), (GL_NO_ERROR, 384, 384));
    }
}

/// GL_ACTIVE_UNIFORMS, GL_ACTIVE_UNIFORM_MAX_LENGTH and glGetActiveUniform
/// for every index: struct, array, array of structs, sampler, bool kinds.
#[test]
fn active_uniforms_name_size_and_type() {
    let _g = rig();
    let p = link(VS_KINDS, FS_KINDS);
    let got = actives(p);
    let want = kinds_expected();
    assert_eq!(prog_i(p, GL_ACTIVE_UNIFORMS) as usize, want.len(), "{got:?}");
    let set: BTreeSet<_> = got.iter().cloned().collect();
    assert_eq!(set.len(), got.len(), "no uniform listed twice: {got:?}");
    assert_eq!(set, want);
    let longest = want.iter().map(|(n, _, _)| n.len()).max().unwrap() as GLint + 1;
    assert_eq!(prog_i(p, GL_ACTIVE_UNIFORM_MAX_LENGTH), longest);
}

/// glGetActiveUniform with a short buffer truncates with a NUL, and a bad
/// index is INVALID_VALUE.
#[test]
fn active_uniform_buffer_size_and_index() {
    let _g = rig();
    let p = link(VS_KINDS, FS_KINDS);
    let n = prog_i(p, GL_ACTIVE_UNIFORMS) as u32;
    for i in 0..n {
        let full = actives(p)[i as usize].0.clone();
        let mut buf = [0x7fu8; 4];
        let (mut len, mut size, mut ty) = (-1, -1, 0);
        // SAFETY: a 4-byte buffer, the size passed.
        unsafe { glGetActiveUniform(p, i, 4, &mut len, &mut size, &mut ty, buf.as_mut_ptr() as *mut c_char) };
        assert_eq!(err(), GL_NO_ERROR);
        let keep = full.len().min(3);
        assert_eq!(len as usize, keep, "{full}");
        assert_eq!(&buf[..keep], &full.as_bytes()[..keep]);
        assert_eq!(buf[keep], 0);
    }
    let (mut len, mut size, mut ty) = (-1, -1, 0);
    let mut buf = [0u8; 8];
    // SAFETY: as above.
    unsafe { glGetActiveUniform(p, n, 8, &mut len, &mut size, &mut ty, buf.as_mut_ptr() as *mut c_char) };
    assert_eq!(err(), GL_INVALID_VALUE, "index == GL_ACTIVE_UNIFORMS");
}

/// glGetUniformLocation: `a`, `a[0]`, the elements, `s.m[1]`-style names,
/// the inactive and the out-of-range answers.
#[test]
fn locations_of_arrays_structs_and_members() {
    let _g = rig();
    let p = link(VS_KINDS, FS_KINDS);
    let (a, a0, a1, a2) = (loc_ok(p, "a"), loc_ok(p, "a[0]"), loc_ok(p, "a[1]"), loc_ok(p, "a[2]"));
    assert_eq!(a, a0, "`a` is the location of `a[0]`");
    assert!(a0 != a1 && a1 != a2 && a0 != a2, "one location per element");
    assert_eq!(loc(p, "a[3]"), -1, "past the size");
    assert_eq!(loc(p, "a[-1]"), -1);
    assert_eq!(loc(p, "nonesuch"), -1);
    assert_eq!(loc(p, "gl_Position"), -1);
    assert_eq!(loc(p, "gl_FragColor"), -1);
    assert_eq!(loc(p, "s"), -1, "a struct is not a uniform location");
    assert_eq!(loc(p, "tt"), -1);
    assert_eq!(loc(p, "tt[0]"), -1);
    assert_eq!(loc(p, "u.m[3]"), -1);
    let (m0, m1, m2) = (loc_ok(p, "u.m[0]"), loc_ok(p, "u.m[1]"), loc_ok(p, "u.m[2]"));
    assert_eq!(loc_ok(p, "u.m"), m0);
    assert!(m0 != m1 && m1 != m2 && m0 != m2);
    let mut all: Vec<GLint> = [
        "a[0]", "a[1]", "a[2]", "m", "shared", "s.f", "s.v", "s.b", "s.i", "tt[0].x", "tt[0].y", "tt[1].x", "tt[1].y",
        "u.m[0]", "u.m[1]", "u.m[2]", "smp", "flag", "bv", "iv",
    ]
    .iter()
    .map(|n| loc_ok(p, n))
    .collect();
    let n = all.len();
    all.sort();
    all.dedup();
    assert_eq!(all.len(), n, "every active uniform and element has its own location");
    assert_eq!(err(), GL_NO_ERROR);
}

/// Set every uniform of the program, read each back with glGetUniformfv/iv,
/// draw, and find the values in the draw's blocks.
#[test]
fn set_read_back_and_draw() {
    let _g = rig();
    let p = link(VS_KINDS, FS_KINDS);
    // SAFETY: plain GL calls; the arrays hold the counts passed.
    unsafe {
        glUniform1i(loc_ok(p, "smp"), 0);
        glUniform4fv(loc_ok(p, "a"), 3, floats(1, 12).as_ptr());
        glUniformMatrix4fv(loc_ok(p, "m"), 1, 0, floats(2, 16).as_ptr());
        glUniform1f(loc_ok(p, "shared"), 77.5);
        glUniform1f(loc_ok(p, "s.f"), 11.25);
        glUniform2f(loc_ok(p, "s.v"), 12.25, 13.25);
        glUniform1i(loc_ok(p, "s.b"), 1);
        glUniform1i(loc_ok(p, "s.i"), -9001);
        glUniform2f(loc_ok(p, "tt[0].x"), 21.5, 22.5);
        glUniform1f(loc_ok(p, "tt[0].y"), 23.5);
        glUniform2f(loc_ok(p, "tt[1].x"), 31.5, 32.5);
        glUniform1f(loc_ok(p, "tt[1].y"), 33.5);
        glUniform1fv(loc_ok(p, "u.m"), 3, [41.75f32, 42.75, 43.75].as_ptr());
        glUniform1i(loc_ok(p, "flag"), 1);
        glUniform3i(loc_ok(p, "bv"), 1, 0, 5);
        glUniform2i(loc_ok(p, "iv"), -7, 4096);
    }
    assert_eq!(err(), GL_NO_ERROR);
    assert_eq!(getf(p, loc_ok(p, "a"), 4), floats(1, 4));
    assert_eq!(getf(p, loc_ok(p, "a[1]"), 4), floats(1, 12)[4..8]);
    assert_eq!(getf(p, loc_ok(p, "a[2]"), 4), floats(1, 12)[8..12]);
    assert_eq!(getf(p, loc_ok(p, "m"), 16), floats(2, 16));
    assert_eq!(getf(p, loc_ok(p, "shared"), 1), [77.5]);
    assert_eq!(getf(p, loc_ok(p, "s.f"), 1), [11.25]);
    assert_eq!(getf(p, loc_ok(p, "s.v"), 2), [12.25, 13.25]);
    assert_eq!(getf(p, loc_ok(p, "s.b"), 1), [1.0]);
    assert_eq!(geti(p, loc_ok(p, "s.b"), 1), [1]);
    assert_eq!(geti(p, loc_ok(p, "s.i"), 1), [-9001]);
    assert_eq!(getf(p, loc_ok(p, "tt[0].x"), 2), [21.5, 22.5]);
    assert_eq!(getf(p, loc_ok(p, "tt[1].x"), 2), [31.5, 32.5]);
    assert_eq!(getf(p, loc_ok(p, "tt[0].y"), 1), [23.5]);
    assert_eq!(getf(p, loc_ok(p, "tt[1].y"), 1), [33.5]);
    for (i, v) in [41.75f32, 42.75, 43.75].into_iter().enumerate() {
        assert_eq!(getf(p, loc_ok(p, &format!("u.m[{i}]")), 1), [v]);
    }
    assert_eq!(geti(p, loc_ok(p, "smp"), 1), [0]);
    assert_eq!(geti(p, loc_ok(p, "flag"), 1), [1]);
    assert_eq!(geti(p, loc_ok(p, "bv"), 3), [1, 0, 1], "a bvec3 reads back as 0 or 1");
    assert_eq!(geti(p, loc_ok(p, "iv"), 2), [-7, 4096]);

    let d = draw_accepted("the kinds program");
    assert!(has_run(&d.vs, &bits(&floats(1, 12))), "vec4 a[3] in the VS block {:x?}", d.vs);
    assert!(has_run(&d.vs, &bits(&floats(2, 16))), "mat4 m in the VS block");
    assert!(d.vs.contains(&77.5f32.to_bits()) && d.fs.contains(&77.5f32.to_bits()), "shared in both blocks");
    for f in [11.25f32, 12.25, 13.25, 21.5, 22.5, 23.5, 31.5, 32.5, 33.5, 41.75, 42.75, 43.75] {
        assert!(d.fs.contains(&f.to_bits()), "{f} in the FS block {:x?}", d.fs);
    }
    assert!(d.fs.contains(&(-9001i32 as u32)), "s.i in the FS block");
    assert!(d.fs.contains(&(-7i32 as u32)) && d.fs.contains(&4096), "iv in the FS block");
    assert!(has_run(&d.fs, &bits(&[21.5, 22.5])) && has_run(&d.fs, &bits(&[31.5, 32.5])), "the vec2s are runs");
}

/// A bool takes glUniform1i and glUniform1f; any non-zero is true.
#[test]
fn bool_uniforms_take_ints_and_floats() {
    let _g = rig();
    let p = link(VS_KINDS, FS_KINDS);
    let l = loc_ok(p, "flag");
    // SAFETY: plain GL calls.
    unsafe {
        glUniform1i(l, 5);
        assert_eq!((err(), geti(p, l, 1), getf(p, l, 1)), (GL_NO_ERROR, vec![1], vec![1.0]));
        glUniform1f(l, 0.5);
        assert_eq!((err(), geti(p, l, 1), getf(p, l, 1)), (GL_NO_ERROR, vec![1], vec![1.0]));
        glUniform1f(l, 0.0);
        assert_eq!((err(), geti(p, l, 1), getf(p, l, 1)), (GL_NO_ERROR, vec![0], vec![0.0]));
        glUniform1i(l, 0);
        assert_eq!((err(), geti(p, l, 1)), (GL_NO_ERROR, vec![0]));
    }
}

/// An element set alone leaves the others; a count past one element sets the
/// following elements; a count past the array is ignored (no error).
#[test]
fn array_elements_set_alone_and_by_count() {
    let _g = rig();
    let p = link(VS_KINDS, FS_KINDS);
    let (a0, a1, a2) = (loc_ok(p, "a[0]"), loc_ok(p, "a[1]"), loc_ok(p, "a[2]"));
    // SAFETY: plain GL calls; the arrays hold the counts passed.
    unsafe {
        glUniform4fv(a0, 3, floats(5, 12).as_ptr());
        glUniform4fv(a1, 1, floats(6, 4).as_ptr());
        assert_eq!(err(), GL_NO_ERROR);
        assert_eq!(getf(p, a0, 4), floats(5, 4));
        assert_eq!(getf(p, a1, 4), floats(6, 4));
        assert_eq!(getf(p, a2, 4), floats(5, 12)[8..]);
        glUniform4fv(a1, 2, floats(7, 8).as_ptr());
        assert_eq!(err(), GL_NO_ERROR);
        assert_eq!(getf(p, a1, 4), floats(7, 4));
        assert_eq!(getf(p, a2, 4), floats(7, 8)[4..]);
        /* ES 2.0.25 2.10.4: values for elements past the end are ignored, no error */
        glUniform4fv(a1, 3, floats(8, 12).as_ptr());
        assert_eq!(err(), GL_NO_ERROR, "count past the array is ignored");
        assert_eq!(getf(p, a1, 4), floats(8, 4));
        assert_eq!(getf(p, a2, 4), floats(8, 8)[4..]);
        assert_eq!(getf(p, a0, 4), floats(5, 4), "the elements before are as they were");
        glUniform4fv(a0, 4, floats(9, 16).as_ptr());
        assert_eq!(err(), GL_NO_ERROR, "count past the array is ignored");
        assert_eq!(getf(p, a2, 4), floats(9, 16)[8..12]);
        glUniform4fv(a0, 3, floats(5, 12).as_ptr());
        glUniform4fv(a1, 2, floats(7, 8).as_ptr());
    }
    let d = draw_accepted("element update");
    let mut want = floats(5, 4);
    want.extend(floats(7, 8));
    assert!(has_run(&d.vs, &bits(&want)), "the array in the VS block as set {:x?}", d.vs);
}

/// ES 2.0 error answers: type, count, transpose, location -1, no program.
#[test]
fn error_answers() {
    let _g = rig();
    let p = link(VS_KINDS, FS_KINDS);
    let f = loc_ok(p, "s.f");
    let v2 = loc_ok(p, "s.v");
    let iv = loc_ok(p, "iv");
    let smp = loc_ok(p, "smp");
    let m = loc_ok(p, "m");
    let a0 = loc_ok(p, "a[0]");
    let mut v = [0.0f32; 16];
    v[0] = 1.0;
    // SAFETY: plain GL calls; the arrays hold the counts passed.
    unsafe {
        glUniform1f(f, 1.0);
        glUniform2f(v2, 1.0, 2.0);
        assert_eq!(err(), GL_NO_ERROR);
        for (what, call) in [
            ("glUniform1i on a float", Box::new(move || glUniform1i(f, 1)) as Box<dyn Fn()>),
            ("glUniform2f on a float", Box::new(move || glUniform2f(f, 1.0, 2.0))),
            ("glUniform1f on a vec2", Box::new(move || glUniform1f(v2, 1.0))),
            ("glUniform3f on a vec2", Box::new(move || glUniform3f(v2, 1.0, 2.0, 3.0))),
            ("glUniform2f on an ivec2", Box::new(move || glUniform2f(iv, 1.0, 2.0))),
            ("glUniform1f on an ivec2", Box::new(move || glUniform1f(iv, 1.0))),
            ("glUniform1f on a sampler", Box::new(move || glUniform1f(smp, 1.0))),
            ("glUniform4f on a sampler", Box::new(move || glUniform4f(smp, 1.0, 2.0, 3.0, 4.0))),
            ("glUniform4f on a mat4", Box::new(move || glUniform4f(m, 1.0, 2.0, 3.0, 4.0))),
            ("glUniform1fv count 2 on a float", Box::new(move || glUniform1fv(f, 2, v.as_ptr()))),
            ("glUniform4fv on a float", Box::new(move || glUniform4fv(f, 1, v.as_ptr()))),
            ("glUniformMatrix3fv on a mat4", Box::new(move || glUniformMatrix3fv(m, 1, 0, v.as_ptr()))),
            ("glUniformMatrix4fv on a vec4", Box::new(move || glUniformMatrix4fv(a0, 1, 0, v.as_ptr()))),
        ] {
            call();
            assert_eq!(err(), GL_INVALID_OPERATION, "{what}");
        }
        glUniformMatrix4fv(m, 1, 1, v.as_ptr());
        assert_eq!(err(), GL_INVALID_VALUE, "transpose = GL_TRUE");
        glUniform1f(-1, 1.0);
        glUniform4fv(-1, 3, v.as_ptr());
        glUniform1i(-1, 1);
        glUniformMatrix4fv(-1, 1, 0, v.as_ptr());
        assert_eq!(err(), GL_NO_ERROR, "location -1 is silently ignored");
        /* getters */
        let mut out = [0.0f32; 4];
        glGetUniformfv(p, -1, out.as_mut_ptr());
        assert_eq!(err(), GL_INVALID_OPERATION, "glGetUniform of location -1");
        glGetUniformfv(p, 0x7fff_0000, out.as_mut_ptr());
        assert_eq!(err(), GL_INVALID_OPERATION, "glGetUniform of an unknown location");
        glGetUniformfv(0xdead, f, out.as_mut_ptr());
        assert_eq!(err(), GL_INVALID_VALUE, "glGetUniform of no program");
        /* the values set before the errors stand */
        assert_eq!(getf(p, f, 1), [1.0]);
        assert_eq!(getf(p, v2, 2), [1.0, 2.0]);
        /* no current program */
        glUseProgram(0);
        glUniform1f(f, 2.0);
        assert_eq!(err(), GL_INVALID_OPERATION, "glUniform with no current program");
        glUseProgram(p);
        assert_eq!(getf(p, f, 1), [1.0], "that call changed nothing");
    }
}

/// A location of one program used on another is INVALID_OPERATION; the
/// current program decides, and each program keeps its own values.
#[test]
fn two_programs_keep_their_own_values() {
    let _g = rig();
    let p1 = link(VS_KINDS, FS_KINDS);
    let (l1, m1) = (loc_ok(p1, "s.f"), loc_ok(p1, "a[0]"));
    // SAFETY: plain GL calls; the arrays hold the counts passed.
    unsafe {
        glUniform1f(l1, 5.0);
        glUniform4fv(m1, 3, floats(9, 12).as_ptr());
        let p2 = link(
            "attribute vec4 p;\nuniform vec4 q[2];\nvoid main() { gl_Position = p + q[0] + q[1]; }\n",
            "precision mediump float;\nuniform float w;\nvoid main() { gl_FragColor = vec4(w); }\n",
        );
        assert_eq!(prog_i(p2, GL_ACTIVE_UNIFORMS), 2);
        glUniform4fv(loc_ok(p2, "q"), 2, floats(10, 8).as_ptr());
        glUniform1f(loc_ok(p2, "w"), 6.0);
        assert_eq!(err(), GL_NO_ERROR);
        let d2 = draw_accepted("the second program");
        assert!(has_run(&d2.vs, &bits(&floats(10, 8))) && d2.fs.contains(&6.0f32.to_bits()));
        /* glGetUniform reads the named program, not the current one */
        assert_eq!(getf(p1, l1, 1), [5.0]);
        assert_eq!(getf(p1, m1, 4), floats(9, 4));
        glUseProgram(p1);
        let d1 = draw_accepted("the first program again");
        assert!(has_run(&d1.vs, &bits(&floats(9, 12))) && d1.fs.contains(&5.0f32.to_bits()));
        assert!(!d1.fs.contains(&6.0f32.to_bits()), "none of the other program's values");
    }
}

/// Relinking a program gives its locations back and takes new ones; a delete
/// of another program must not move this one's values.
#[test]
fn relink_and_delete_keep_other_programs() {
    let _g = rig();
    let keep = link(VS_KINDS, FS_KINDS);
    let (lf, la) = (loc_ok(keep, "s.f"), loc_ok(keep, "a[2]"));
    // SAFETY: plain GL calls; the arrays hold the counts passed.
    unsafe {
        glUniform1f(lf, 3.5);
        glUniform4fv(la, 1, floats(11, 4).as_ptr());
        let other = link(VS_KINDS, FS_KINDS);
        let lo = loc_ok(other, "s.f");
        glUniform1f(lo, 8.5);
        glLinkProgram(other);
        assert_eq!(prog_i(other, GL_LINK_STATUS), 1);
        glUseProgram(other);
        glUniform1f(loc_ok(other, "s.f"), 9.5);
        glDeleteProgram(other);
        glUseProgram(keep);
        assert_eq!(err(), GL_NO_ERROR);
        assert_eq!(getf(keep, lf, 1), [3.5]);
        assert_eq!(getf(keep, la, 4), floats(11, 4));
        assert_eq!(loc(keep, "s.f"), lf, "the location did not move");
        assert_eq!(loc(keep, "a[2]"), la);
        let d = draw_accepted("after the neighbours were relinked and deleted");
        assert!(d.fs.contains(&3.5f32.to_bits()) && has_run(&d.vs, &bits(&floats(11, 4))));
    }
}

/* ---- more than 255 active uniforms ---- */

fn big_vs() -> String {
    let mut s = String::from("attribute vec4 p;\nstruct V { vec4 a; vec4 b; vec4 c; vec4 d; };\nuniform V v[65];\nvoid main() {\n  vec4 x = p;\n");
    for i in 0..65 {
        s += &format!("  x += v[{i}].a + v[{i}].b + v[{i}].c + v[{i}].d;\n");
    }
    s + "  gl_Position = x;\n}\n"
}

fn big_fs() -> String {
    let mut s = String::from(
        "precision mediump float;\nstruct F { vec4 a; vec4 b; vec4 c; vec4 d; vec4 e; };\nuniform F f[64];\nvoid main() {\n  vec4 x = vec4(0.0);\n",
    );
    for i in 0..64 {
        s += &format!("  x += f[{i}].a + f[{i}].b + f[{i}].c + f[{i}].d + f[{i}].e;\n");
    }
    s + "  gl_FragColor = x;\n}\n"
}

/// 580 active uniforms: every one listed, located, set, read back and in the
/// draw's blocks at distinct values.
#[test]
fn more_than_255_active_uniforms() {
    let _g = rig();
    let p = link(&big_vs(), &big_fs());
    let got = actives(p);
    assert_eq!(prog_i(p, GL_ACTIVE_UNIFORMS), 65 * 4 + 64 * 5);
    let mut want = BTreeSet::new();
    for i in 0..65 {
        for m in ["a", "b", "c", "d"] {
            want.insert((format!("v[{i}].{m}"), 1, GL_FLOAT_VEC4));
        }
    }
    for i in 0..64 {
        for m in ["a", "b", "c", "d", "e"] {
            want.insert((format!("f[{i}].{m}"), 1, GL_FLOAT_VEC4));
        }
    }
    assert_eq!(got.iter().cloned().collect::<BTreeSet<_>>(), want);
    assert_eq!(got.len(), want.len(), "no uniform listed twice");
    assert_eq!(prog_i(p, GL_ACTIVE_UNIFORM_MAX_LENGTH), "f[63].e".len() as GLint + 1);
    let mut seen = BTreeSet::new();
    let mut vs_want = Vec::new();
    let mut fs_want = Vec::new();
    let mut seed = 100;
    for (name, _, _) in &got {
        let l = loc_ok(p, name);
        assert!(seen.insert(l), "{name}: a location of its own ({l})");
        seed += 1;
        let v = floats(seed, 4);
        // SAFETY: four values.
        unsafe { glUniform4fv(l, 1, v.as_ptr()) };
        assert_eq!(err(), GL_NO_ERROR, "glUniform4fv({name})");
        if name.starts_with('v') {
            vs_want.push(v);
        } else {
            fs_want.push(v);
        }
    }
    for (name, _, _) in &got {
        let l = loc_ok(p, name);
        let idx = got.iter().position(|g| &g.0 == name).unwrap();
        let v = floats(101 + idx as u32, 4);
        assert_eq!(getf(p, l, 4), v, "{name} reads back");
    }
    let d = draw_accepted("580 active uniforms");
    for v in vs_want {
        assert!(has_run(&d.vs, &bits(&v)), "a vec4 of the VS block is missing: {v:?}");
    }
    for v in fs_want {
        assert!(has_run(&d.fs, &bits(&v)), "a vec4 of the FS block is missing: {v:?}");
    }
    /* the last record and the first still answer after the draw */
    assert_eq!(getf(p, loc_ok(p, "v[0].a"), 4).len(), 4);
    assert!(loc(p, "f[64].a") < 0 && loc(p, "v[65].a") < 0);
}

// Not covered: one uniform array past 255 elements (`uniform vec4 big[300]`).
// `UniformRow::array_len` is a u8 (src/shaderc/online/reflect.rs), so the online
// front end refuses it; that is not 3cbecbb9's doing, and dEQP-GLES2 declares no
// uniform array past 255.
