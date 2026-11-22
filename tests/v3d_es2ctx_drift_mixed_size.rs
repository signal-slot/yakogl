// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The ES 2.0 context against e6d72a23 (v3d: colour attachments of different
//! sizes draw and clear at their intersection). The commit changed how the
//! back end sizes a job: `extra_targets` returns a frame, the minimum of RT
//! 0's size and every written attachment past it, and `draw_call` builds the
//! `Target` (width, height, tiles, clip window) from that frame instead of
//! from RT 0. ES 2.0 has one colour attachment, so the frame must stay RT 0's
//! own size and `Target.cut` must stay false: the Pi 4 (V3D 4.2) whole
//! dEQP-GLES2 tree (46f93767) predates the commit, and fbo.completeness and
//! fbo.render build framebuffers whose colour and depth attachments differ in
//! size. This file replays them through the real GL core (a `report-es2`
//! build) and the real V3D back end and asserts, from the stand-in job
//! builder, which records every job's target and every tiling it is asked for:
//!
//! - (a) a colour texture and a depth (or stencil) renderbuffer of different
//!   sizes, either one larger: GL_FRAMEBUFFER_INCOMPLETE_DIMENSIONS;
//!   glClear and glDrawArrays on it: GL_INVALID_FRAMEBUFFER_OPERATION, no
//!   job begun, nothing submitted, no draw refused by the back end;
//! - (b) colour and depth of the same size, 1x1, 64x64, 2048x2048 and 37x23:
//!   the job is the full attachment (width, height) and its tiling covers
//!   exactly that area; a whole-target clear is the TLB's (no quad), a draw
//!   after it is one draw in a job of the same extent;
//! - (c) a scissored clear on the default-sized target (the window, and a
//!   32x32 texture with a depth renderbuffer) is one quad with the
//!   scissor box in its state, in a job of the target's own size.
//!
//! Every accepted sequence asserts GL_NO_ERROR and `v3d_refused_draws()`
//! unchanged. The answers are the same as before e6d72a23 (read from its
//! parent: with `num_targets <= 1`, `extra_targets` returned
//! `(0, rt, formats)` and the target was `d.target_w` by `d.target_h`; the
//! commit returns `frame: (d.target_w, d.target_h)` and `cut: false`). The
//! texture layout code is the real driver/src/v3d/v3d_tex.c.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table,report-es2 --test v3d_es2ctx_drift_mixed_size
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
    JOBS.lock().unwrap_or_else(|e| e.into_inner()).push((t.width, t.height));
    t.width != 0 && t.height != 0
}

/// Every job begun: its width and height.
static JOBS: Mutex<Vec<(u32, u32)>> = Mutex::new(Vec::new());

fn jobs() -> Vec<(u32, u32)> {
    JOBS.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Every tiling asked for: the frame it was asked for and the tile it chose.
static TILINGS: Mutex<Vec<[u32; 4]>> = Mutex::new(Vec::new());

fn tilings() -> Vec<[u32; 4]> {
    TILINGS.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

#[no_mangle]
unsafe extern "C" fn v3d_job_draw(j: *mut V3dJob, d: *const V3dJobDraw, _layout: *mut c_void) -> i32 {
    // SAFETY: the backend passes its live job, draw and state.
    unsafe {
        let (d, st) = (&*d, &*(*d).st);
        let words = if d.fs_storage.words.is_null() {
            Vec::new()
        } else {
            std::slice::from_raw_parts(d.fs_storage.words, d.fs_storage.count as usize).to_vec()
        };
        DRAWS.lock().unwrap_or_else(|e| e.into_inner()).push(Quad {
            count: d.count,
            fs: d.fs as usize,
            scissor: st.scissor.then_some([st.scissor_minx, st.scissor_miny, st.scissor_maxx, st.scissor_maxy]),
            colormask: st.colormask,
            depth: st.depth_enabled,
            stencil: st.stencil_enabled,
            words,
        });
        (*j).draws += 1;
        (*(*j).bcl).used += 120;
    }
    V3D_JOB_OK
}

/// One draw the job builder was handed (fields not asserted on are kept for the failure message).
#[allow(dead_code)]
#[derive(Clone, Debug)]
struct Quad {
    count: u32,
    /// The fragment program's address: one value for every float clear.
    fs: usize,
    scissor: Option<[u32; 4]>,
    colormask: u32,
    depth: bool,
    stencil: bool,
    /// The fragment uniform block: the clear colour's four words.
    words: Vec<u32>,
}

static DRAWS: Mutex<Vec<Quad>> = Mutex::new(Vec::new());

fn draws() -> Vec<Quad> {
    DRAWS.lock().unwrap_or_else(|e| e.into_inner()).clone()
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
    TILINGS.lock().unwrap_or_else(|e| e.into_inner()).push([width, height, tw, th]);
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
    DRAWS.lock().unwrap_or_else(|e| e.into_inner()).clear();
    JOBS.lock().unwrap_or_else(|e| e.into_inner()).clear();
    TILINGS.lock().unwrap_or_else(|e| e.into_inner()).clear();
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

const VS: &str = "attribute vec4 p;\nvoid main() { gl_Position = p; }\n";
const FS: &str = "precision mediump float;\nvoid main() { gl_FragColor = vec4(0.25, 0.5, 0.75, 1.0); }\n";

/// The error flag, drained: the first error since the last call.
fn err() -> GLenum {
    // SAFETY: plain GL calls.
    unsafe {
        let e = glGetError();
        while glGetError() != GL_NO_ERROR {}
        e
    }
}

fn submitted() -> usize {
    with_platform(|p| p.submitted)
}

/// Forget the jobs, tilings and draws recorded so far.
fn forget() {
    JOBS.lock().unwrap_or_else(|e| e.into_inner()).clear();
    TILINGS.lock().unwrap_or_else(|e| e.into_inner()).clear();
    DRAWS.lock().unwrap_or_else(|e| e.into_inner()).clear();
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

/// A depth or stencil renderbuffer: its format, attachment point and size.
type Ds = (GLenum, GLenum, GLsizei, GLsizei);

/// An FBO with a `cw` x `ch` RGBA texture colour and, if given, a
/// renderbuffer; its name and its completeness status.
struct Fbo(GLuint, GLenum);

fn fbo(cw: GLsizei, ch: GLsizei, ds: Option<Ds>) -> Fbo {
    // SAFETY: plain GL calls with room for one name each.
    unsafe {
        let (mut fb, mut t) = (0, 0);
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glActiveTexture(GL_TEXTURE0);
        glGenTextures(1, &mut t);
        glBindTexture(GL_TEXTURE_2D, t);
        glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA as GLint, cw, ch, 0, GL_RGBA, GL_UNSIGNED_BYTE, core::ptr::null());
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, t, 0);
        if let Some((format, attachment, dw, dh)) = ds {
            let mut rb = 0;
            glGenRenderbuffers(1, &mut rb);
            glBindRenderbuffer(GL_RENDERBUFFER, rb);
            glRenderbufferStorage(GL_RENDERBUFFER, format, dw, dh);
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, attachment, GL_RENDERBUFFER, rb);
        }
        glViewport(0, 0, cw, ch);
        let status = glCheckFramebufferStatus(GL_FRAMEBUFFER);
        assert_eq!(err(), GL_NO_ERROR, "framebuffer setup {cw}x{ch}");
        Fbo(fb, status)
    }
}

fn unbind(f: Fbo) {
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &f.0);
    }
}

fn finish() {
    // SAFETY: a plain GL call.
    unsafe { glFinish() };
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

/// (a) Colour and depth/stencil of different sizes: ES 2.0 4.4.5 says
/// INCOMPLETE_DIMENSIONS, glClear and glDraw* say
/// INVALID_FRAMEBUFFER_OPERATION and the back end sees nothing.
#[test]
fn colour_and_depth_of_different_sizes_are_incomplete_and_refuse_work() {
    let _g = rig();
    program();
    assert_eq!(err(), GL_NO_ERROR);
    let cases = [
        ("depth smaller", GL_DEPTH_COMPONENT16, GL_DEPTH_ATTACHMENT, 16, 16),
        ("depth larger", GL_DEPTH_COMPONENT16, GL_DEPTH_ATTACHMENT, 64, 64),
        ("depth wider", GL_DEPTH_COMPONENT16, GL_DEPTH_ATTACHMENT, 64, 32),
        ("depth taller", GL_DEPTH_COMPONENT16, GL_DEPTH_ATTACHMENT, 32, 64),
        ("stencil smaller", GL_STENCIL_INDEX8, GL_STENCIL_ATTACHMENT, 16, 16),
        ("stencil larger", GL_STENCIL_INDEX8, GL_STENCIL_ATTACHMENT, 64, 64),
    ];
    for (what, format, attachment, dw, dh) in cases {
        let f = fbo(W, H, Some((format, attachment, dw, dh)));
        assert_eq!(f.1, GL_FRAMEBUFFER_INCOMPLETE_DIMENSIONS, "{what}: completeness");
        forget();
        let (kicks, refused) = (submitted(), v3d_refused_draws());
        // SAFETY: plain GL calls.
        unsafe {
            glClearColor(0.25, 0.5, 0.75, 1.0);
            glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT);
        }
        assert_eq!(err(), GL_INVALID_FRAMEBUFFER_OPERATION, "{what}: glClear");
        draw_triangle();
        assert_eq!(err(), GL_INVALID_FRAMEBUFFER_OPERATION, "{what}: glDrawArrays");
        let idx = [0u16, 1, 2];
        // SAFETY: an index array that outlives the call.
        unsafe { glDrawElements(GL_TRIANGLES, 3, GL_UNSIGNED_SHORT, idx.as_ptr() as *const c_void) };
        assert_eq!(err(), GL_INVALID_FRAMEBUFFER_OPERATION, "{what}: glDrawElements");
        finish();
        assert_eq!(err(), GL_NO_ERROR, "{what}: glFinish");
        assert!(jobs().is_empty(), "{what}: a job was begun: {:?}", jobs());
        assert!(draws().is_empty(), "{what}: a draw reached the job builder: {:?}", draws());
        assert_eq!(submitted(), kicks, "{what}: a kick was submitted; {:?}", log_lines());
        assert_eq!(v3d_refused_draws().wrapping_sub(refused), 0, "{what}: the back end refused a draw");
        unbind(f);
    }
}

/// (b) Colour and depth of one size: the job is the whole attachment, the
/// tiling covers exactly that area (the tile grid reaches the edge and not
/// a tile past it), and a whole-target clear is the TLB's.
#[test]
fn same_size_colour_and_depth_are_the_full_attachment() {
    let _g = rig();
    program();
    for (w, h) in [(1, 1), (64, 64), (2048, 2048), (37, 23)] {
        let f = fbo(w, h, Some((GL_DEPTH_COMPONENT16, GL_DEPTH_ATTACHMENT, w, h)));
        assert_eq!(f.1, GL_FRAMEBUFFER_COMPLETE, "{w}x{h}: completeness");
        for pass in ["clear", "draw", "clear then draw"] {
            let what = format!("{w}x{h} {pass}");
            forget();
            let refused = v3d_refused_draws();
            // SAFETY: plain GL calls.
            unsafe {
                glDisable(GL_SCISSOR_TEST);
                glViewport(0, 0, w, h);
                if pass != "draw" {
                    glClearColor(0.25, 0.5, 0.75, 1.0);
                    glClearDepthf(0.5);
                    glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
                }
            }
            if pass != "clear" {
                draw_triangle();
            }
            finish();
            let (e, n) = (err(), v3d_refused_draws().wrapping_sub(refused));
            assert_eq!((e, n), (GL_NO_ERROR, 0), "{what}: error, refused draws; {:?}", log_lines());
            let (j, t, d) = (jobs(), tilings(), draws());
            assert!(!j.is_empty(), "{what}: no job");
            assert!(j.iter().all(|&e| e == (w as u32, h as u32)), "{what}: every job is the full attachment: {j:?}");
            assert!(!t.is_empty(), "{what}: no tiling asked for");
            for [fw, fh, tile_w, tile_h] in t {
                assert_eq!((fw, fh), (w as u32, h as u32), "{what}: the tiling's frame");
                let (nx, ny) = (fw.div_ceil(tile_w), fh.div_ceil(tile_h));
                assert!(nx * tile_w >= fw && (nx - 1) * tile_w < fw, "{what}: tiles in x");
                assert!(ny * tile_h >= fh && (ny - 1) * tile_h < fh, "{what}: tiles in y");
            }
            match pass {
                "clear" => assert!(d.is_empty(), "{what}: a whole-target clear is the TLB's, not a quad: {d:?}"),
                _ => assert_eq!(d.len(), 1, "{what}: one draw: {d:?}"),
            }
        }
        unbind(f);
    }
}

/// (c) A scissored clear still clears the scissor box, on the window and on
/// a 32x32 texture with a depth renderbuffer: one quad with the box in its
/// state, in a job of the target's own size.
#[test]
fn a_scissored_clear_clears_the_scissor_box() {
    let _g = rig();
    const RECT: [GLint; 4] = [5, 3, 17, 11];
    for fbo_case in [false, true] {
        let f = fbo_case.then(|| fbo(W, H, Some((GL_DEPTH_COMPONENT16, GL_DEPTH_ATTACHMENT, W, H))));
        let what = if fbo_case { "texture and depth" } else { "window" };
        forget();
        let refused = v3d_refused_draws();
        // SAFETY: plain GL calls.
        unsafe {
            glViewport(0, 0, W, H);
            glClearColor(0.25, 0.5, 0.75, 1.0);
            glEnable(GL_SCISSOR_TEST);
            glScissor(RECT[0], RECT[1], RECT[2], RECT[3]);
            glClear(GL_COLOR_BUFFER_BIT);
            glDisable(GL_SCISSOR_TEST);
        }
        finish();
        let (e, n) = (err(), v3d_refused_draws().wrapping_sub(refused));
        assert_eq!((e, n), (GL_NO_ERROR, 0), "{what}: {:?}", log_lines());
        let d = draws();
        assert_eq!(d.len(), 1, "{what}: one quad: {d:?}");
        assert_eq!(d[0].count, 6, "{what}: two triangles");
        let r = d[0].scissor.unwrap_or_else(|| panic!("{what}: no scissor in the draw"));
        assert_eq!((r[2] - r[0], r[3] - r[1]), (RECT[2] as u32, RECT[3] as u32), "{what}: {r:?}");
        if fbo_case {
            assert_eq!((r[0], r[1]), (RECT[0] as u32, RECT[1] as u32), "{what}: {r:?}");
        }
        let j = jobs();
        assert!(!j.is_empty() && j.iter().all(|&e| e == (W as u32, H as u32)), "{what}: jobs {j:?}");
        if let Some(f) = f {
            unbind(f);
        }
    }
}
