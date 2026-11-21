// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The ES 2.0 context against bf80c82a (`submit_job` asserts that a job's tile
//! state fits the pool instead of capping it; 6d2e7d0d made MAX_TARGET 4096).
//! The Pi 4 whole dEQP-GLES2 tree (46f93767) predates both. A `debug_assert!`
//! (`submit_job`, src/v3d/backend.rs) that trips is a panic in a debug build,
//! and where a former cap kept a binner inside its buffer it would be a
//! crash. This file drives the real GL core (a `report-es2` build) and the
//! real V3D back end through draws and clears on the sizes dEQP-GLES2
//! reaches (fbo.render.*, fbo.completeness.size.*, texture.size.*,
//! shaders.* at 4096 x 2..6) and asserts, for each: GL_NO_ERROR,
//! `v3d_refused_draws()` unchanged, no panic, and, from the stand-in job
//! builder, which records every job's target, that each job
//!
//! - is untyped (ES 2.0 colour formats), one render target, no MSAA,
//! - has a side no larger than GL_MAX_RENDERBUFFER_SIZE (4096), and
//! - has a tile state (tiles x 256 bytes, `job_tile_state_bytes`) within the
//!   pool, the size `init` gives for 4096 x 4096 at 4 render targets
//!   (16384 tiles, 4 MiB).
//!
//! Sizes: GL_MAX_RENDERBUFFER_SIZE and GL_MAX_TEXTURE_SIZE as the context
//! reports them; 1x1; max x 1 and 1 x max; max x 2..6 (the shader executor's
//! targets); NPOT and power-of-two sides of fbo.completeness.size (3, 5, 15,
//! 17, 31, 33, 63, 65, 127, 129, 255, 257, 511, 513, 1023, 1025, 2047, 2049,
//! 4095); 2048 (texture.size.2d); with a texture, an RGBA4, RGB5_A1 and
//! RGB565 renderbuffer, and with depth and stencil renderbuffers; and many
//! jobs in one frame (the target switched between draws). The default
//! framebuffer is the context's 32 x 32. The frame tiling is the stand-in
//! `v3d_frame_tiling_bpp` below, a copy of driver/src/v3d/v3d_frame.c's rule.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table,report-es2 --test v3d_es2ctx_drift_tile_pool
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
    JOBS.lock().unwrap_or_else(|e| e.into_inner()).push(JobTarget {
        width: t.width,
        height: t.height,
        rts: t.rts,
        msaa: t.msaa,
        typed: t.typed,
    });
    // SAFETY: as above.
    unsafe {
        (*bcl).used = 16;
        (*ind).used = 256;
    }
    t.width != 0 && t.height != 0
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

/// One draw the job builder was handed (kept for the shared rig).
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

/// One job's target, as the job builder was handed it.
#[derive(Clone, Copy, Debug)]
struct JobTarget {
    width: u32,
    height: u32,
    rts: u32,
    msaa: bool,
    typed: bool,
}

static JOBS: Mutex<Vec<JobTarget>> = Mutex::new(Vec::new());

fn jobs() -> Vec<JobTarget> {
    JOBS.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

static DRAWS: Mutex<Vec<Quad>> = Mutex::new(Vec::new());

#[allow(dead_code)]
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
    g
}

/* ---- helpers ---- */

use yakogl::gl::texture::*;
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

/// The one rule of the file: what `f` does is accepted (no GL error, no
/// refused draw), and no clear reaches the integer clear program.
fn accepted(what: &str, f: impl FnOnce()) {
    let base = v3d_refused_draws();
    f();
    // SAFETY: a plain GL call.
    unsafe { glFinish() };
    let (e, n) = (err(), v3d_refused_draws().wrapping_sub(base));
    assert_eq!((e, n), (GL_NO_ERROR, 0), "{what}: error, refused draws; {:?}", log_lines());
    assert!(
        log_lines().iter().all(|l| !l.contains("integer clear program")),
        "{what}: an ES 2.0 clear reached the integer clear program; {:?}",
        log_lines()
    );
}

use yakogl::gl::buffer::glBindBuffer;
use yakogl::gl::draw::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::vertex::{glEnableVertexAttribArray, glVertexAttribPointer};

const VS: &str = "attribute vec4 p;\nvarying vec2 t;\nvoid main() { gl_Position = p; t = p.xy * 0.5 + 0.5; }\n";
const FS: &str = "precision mediump float;\nvarying vec2 t;\nvoid main() { gl_FragColor = vec4(t, 0.5, 1.0); }\n";

static TRI: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 3.0, -1.0, 0.0, 1.0, -1.0, 3.0, 0.0, 1.0];

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

/// The program every draw of the file uses, made once per test.
fn use_program() {
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
    // SAFETY: TRI is static and outlives the draw.
    unsafe {
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, TRI.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glDrawArrays(GL_TRIANGLES, 0, 3);
    }
}

fn geti(pname: GLenum) -> GLint {
    let mut v = 0;
    // SAFETY: a plain GL query with room for one value.
    unsafe { glGetIntegerv(pname, &mut v) };
    v
}

/// The pool `init` sizes: the tile state of 4096 x 4096 at 4 render targets.
const POOL_TILE_STATE: u32 = (4096 / 32) * (4096 / 32) * 256;

/// Every job the builder was handed since the last call: untyped, one render
/// target, no MSAA, sides in 1..=max, tile state within the pool (the
/// arithmetic of `job_tile_state_bytes`, with the tile sizes of
/// `v3d_frame_tiling_bpp` for an untyped job, max_bpp 0).
fn check_jobs(what: &str, max: u32) -> Vec<JobTarget> {
    let js = jobs();
    JOBS.lock().unwrap_or_else(|e| e.into_inner()).clear();
    for j in &js {
        assert!(!j.typed && !j.msaa && j.rts <= 1, "{what}: an ES 2.0 job is untyped, single-target, no MSAA: {j:?}");
        assert!((1..=max).contains(&j.width) && (1..=max).contains(&j.height), "{what}: side out of range: {j:?}");
        let (tw, th) = match j.rts {
            0 | 1 => (64, 64),
            2 => (64, 32),
            _ => (32, 32),
        };
        let state = j.width.div_ceil(tw) * j.height.div_ceil(th) * 256;
        assert!(state <= POOL_TILE_STATE, "{what}: tile state {state} over the pool {POOL_TILE_STATE}: {j:?}");
    }
    js
}

#[derive(Clone, Copy, Debug)]
enum Colour {
    Texture(GLenum),
    Renderbuffer(GLenum),
}

/// The names one FBO made, so they can be freed (a 4096 x 4096 target is
/// 64 MiB of the stand-in platform's memory).
struct Fbo {
    fb: GLuint,
    tex: Vec<GLuint>,
    rbs: Vec<GLuint>,
}

fn make_fbo(w: GLsizei, h: GLsizei, c: Colour, ds: bool) -> Fbo {
    let mut f = Fbo { fb: 0, tex: Vec::new(), rbs: Vec::new() };
    // SAFETY: plain GL calls with room for one name each.
    unsafe {
        glGenFramebuffers(1, &mut f.fb);
        glBindFramebuffer(GL_FRAMEBUFFER, f.fb);
        glViewport(0, 0, w, h);
        match c {
            Colour::Texture(format) => {
                let mut t = 0;
                glActiveTexture(GL_TEXTURE0);
                glGenTextures(1, &mut t);
                glBindTexture(GL_TEXTURE_2D, t);
                glTexImage2D(GL_TEXTURE_2D, 0, format as GLint, w, h, 0, format, GL_UNSIGNED_BYTE, core::ptr::null());
                glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
                glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
                glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, t, 0);
                f.tex.push(t);
            }
            Colour::Renderbuffer(format) => f.rbs.push(rb(GL_COLOR_ATTACHMENT0, format, w, h)),
        }
        if ds {
            f.rbs.push(rb(GL_DEPTH_ATTACHMENT, GL_DEPTH_COMPONENT16, w, h));
            f.rbs.push(rb(GL_STENCIL_ATTACHMENT, GL_STENCIL_INDEX8, w, h));
        }
    }
    f
}

/// # Safety
/// Plain GL calls.
unsafe fn rb(attachment: GLenum, format: GLenum, w: GLsizei, h: GLsizei) -> GLuint {
    let mut r = 0;
    // SAFETY: the caller's contract; room for one name.
    unsafe {
        glGenRenderbuffers(1, &mut r);
        glBindRenderbuffer(GL_RENDERBUFFER, r);
        glRenderbufferStorage(GL_RENDERBUFFER, format, w, h);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, attachment, GL_RENDERBUFFER, r);
    }
    r
}

impl Fbo {
    fn complete(&self) -> bool {
        // SAFETY: plain GL calls.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, self.fb);
            glCheckFramebufferStatus(GL_FRAMEBUFFER) == GL_FRAMEBUFFER_COMPLETE
        }
    }
    fn free(self) {
        // SAFETY: plain GL calls on names this FBO made.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteFramebuffers(1, &self.fb);
            for t in &self.tex {
                glDeleteTextures(1, t);
            }
            for r in &self.rbs {
                glDeleteRenderbuffers(1, r);
            }
        }
    }
}

/// Draws and clears on a `w` x `h` FBO, each accepted, and the jobs within
/// the pool. Returns whether the FBO was complete (an incomplete one is the
/// GL core's answer and is not drawn on).
fn on_fbo(w: GLsizei, h: GLsizei, c: Colour, ds: bool) -> bool {
    let what = format!("{w}x{h} {c:?} ds={ds}");
    let max = geti(GL_MAX_RENDERBUFFER_SIZE) as u32;
    let f = make_fbo(w, h, c, ds);
    let ok = f.complete();
    if ok {
        accepted(&what, || {
            // SAFETY: plain GL calls.
            unsafe {
                glViewport(0, 0, w, h);
                glClearColor(0.25, 0.5, 0.75, 1.0);
                glClearDepthf(1.0);
                glClearStencil(0);
                glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT);
                if ds {
                    glEnable(GL_DEPTH_TEST);
                    glEnable(GL_STENCIL_TEST);
                }
                draw_triangle();
                glDisable(GL_DEPTH_TEST);
                glDisable(GL_STENCIL_TEST);
                /* a scissored clear is a quad draw in the same job's target */
                glEnable(GL_SCISSOR_TEST);
                glScissor(0, 0, (w / 2).max(1), (h / 2).max(1));
                glClear(GL_COLOR_BUFFER_BIT);
                glDisable(GL_SCISSOR_TEST);
                draw_triangle();
            }
        });
        let js = check_jobs(&what, max);
        assert!(!js.is_empty(), "{what}: no job reached the builder");
        assert!(js.iter().all(|j| (j.width, j.height) == (w as u32, h as u32)), "{what}: target size: {js:?}");
    } else {
        eprintln!("{what}: skipped, the framebuffer is not complete (the GL core's answer)");
        let _ = err();
    }
    f.free();
    ok
}

const COLOURS: [Colour; 4] = [
    Colour::Texture(GL_RGBA),
    Colour::Texture(GL_RGB),
    Colour::Renderbuffer(GL_RGBA4),
    Colour::Renderbuffer(GL_RGB565),
];

/* ---- the cases ---- */

/// This is an ES 2.0 context and its limits are the ones the pool and the
/// sizes below are written for.
#[test]
fn the_context_is_es2_and_reports_4096() {
    let _g = rig();
    assert_eq!(geti(GL_MAX_RENDERBUFFER_SIZE), 4096);
    assert_eq!(geti(GL_MAX_TEXTURE_SIZE), 4096);
    let _ = err();
    let _ = geti(GL_MAJOR_VERSION);
    assert_eq!(err(), GL_INVALID_ENUM, "GL_MAJOR_VERSION is an ES 3.0 query");
    assert_eq!(POOL_TILE_STATE, 4 << 20);
}

/// The default framebuffer: clears and draws, jobs inside the pool.
#[test]
fn default_framebuffer() {
    let _g = rig();
    use_program();
    accepted("default framebuffer", || {
        // SAFETY: plain GL calls.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glViewport(0, 0, W, H);
            glClearColor(0.0, 0.0, 0.0, 1.0);
            glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT);
            draw_triangle();
            glEnable(GL_SCISSOR_TEST);
            glScissor(3, 3, 9, 9);
            glClear(GL_COLOR_BUFFER_BIT);
            glDisable(GL_SCISSOR_TEST);
            draw_triangle();
        }
    });
    let js = check_jobs("default framebuffer", 4096);
    assert!(!js.is_empty());
}

/// fbo.completeness.size.*, fbo.render.* and texture.size.* sides: square and
/// NPOT, every colour format, with and without depth and stencil.
#[test]
fn square_and_npot_sizes() {
    let _g = rig();
    use_program();
    let mut drawn = 0;
    for s in [1, 2, 3, 4, 5, 7, 8, 15, 16, 17, 31, 32, 33, 63, 64, 65, 127, 128, 129, 255, 256, 257, 511, 512, 513] {
        for c in COLOURS {
            for ds in [false, true] {
                drawn += on_fbo(s, s, c, ds) as u32;
            }
        }
    }
    assert!(drawn > 100, "the FBOs were complete: {drawn}");
}

/// The large sides of texture.size.2d / fbo.render: 1023..2049 and the
/// context's limits.
#[test]
fn large_square_sizes() {
    let _g = rig();
    use_program();
    for s in [1023, 1024, 1025, 2047, 2048, 2049, 4095, geti(GL_MAX_TEXTURE_SIZE), geti(GL_MAX_RENDERBUFFER_SIZE)] {
        for (c, ds) in [
            (Colour::Texture(GL_RGBA), false),
            (Colour::Renderbuffer(GL_RGB565), true),
            (Colour::Renderbuffer(GL_RGBA4), false),
        ] {
            assert!(on_fbo(s, s, c, ds), "{s}x{s} {c:?} ds={ds} is complete at the limits the context reports");
        }
    }
}

/// Non-square: max x 1, 1 x max, and the pairs of fbo.completeness.size.
#[test]
fn non_square_sizes() {
    let _g = rig();
    use_program();
    let m = geti(GL_MAX_RENDERBUFFER_SIZE);
    for (w, h) in [
        (m, 1),
        (1, m),
        (m, 2),
        (2, m),
        (m, 3),
        (m, 4),
        (m, 5),
        (m, 6),
        (m, 17),
        (17, m),
        (m - 1, 1),
        (1, m - 1),
        (1, 255),
        (255, 1),
        (127, 129),
        (129, 127),
        (1, 1025),
        (2049, 3),
        (3, 2049),
        (2048, 1),
        (1, 2048),
        (33, 513),
        (513, 33),
    ] {
        for (c, ds) in [
            (Colour::Texture(GL_RGBA), false),
            (Colour::Texture(GL_RGB), true),
            (Colour::Renderbuffer(GL_RGB5_A1), true),
        ] {
            assert!(on_fbo(w, h, c, ds), "{w}x{h} {c:?} ds={ds} is complete");
        }
    }
}

/// 1 x 1 targets of every format, with depth and stencil.
#[test]
fn one_by_one() {
    let _g = rig();
    use_program();
    for c in COLOURS {
        for ds in [false, true] {
            assert!(on_fbo(1, 1, c, ds), "1x1 {c:?} ds={ds}");
        }
    }
}

/// Many jobs in one frame: the draw target switched between draws, with no
/// finish in between, over targets from 1 x 1 to the limit, and back to the
/// default framebuffer.
#[test]
fn many_jobs_in_one_frame() {
    let _g = rig();
    use_program();
    let m = geti(GL_MAX_RENDERBUFFER_SIZE);
    let sizes = [(m, 2), (1, m), (64, 64), (1, 1), (m, 6), (33, 129), (2048, 2048), (m, m)];
    let fbos: Vec<(GLsizei, GLsizei, Fbo)> = sizes
        .iter()
        .enumerate()
        .map(|(i, &(w, h))| {
            let c = if i % 2 == 0 { Colour::Texture(GL_RGBA) } else { Colour::Renderbuffer(GL_RGB565) };
            (w, h, make_fbo(w, h, c, i % 3 == 0))
        })
        .collect();
    assert!(fbos.iter().all(|(_, _, f)| f.complete()));
    accepted("many jobs", || {
        // SAFETY: plain GL calls.
        unsafe {
            for round in 0..6 {
                for (w, h, f) in &fbos {
                    glBindFramebuffer(GL_FRAMEBUFFER, f.fb);
                    glViewport(0, 0, *w, *h);
                    glClearColor(round as f32 / 8.0, 0.5, 0.25, 1.0);
                    glClear(GL_COLOR_BUFFER_BIT);
                    draw_triangle();
                }
                glBindFramebuffer(GL_FRAMEBUFFER, 0);
                glViewport(0, 0, W, H);
                draw_triangle();
            }
        }
    });
    let js = check_jobs("many jobs", m as u32);
    assert!(js.len() >= 6 * (sizes.len() + 1), "one job per target switch at least: {}", js.len());
    for (w, h, f) in fbos {
        assert!(js.iter().any(|j| (j.width, j.height) == (w as u32, h as u32)), "a job on {w}x{h}");
        f.free();
    }
}

/// A target past the limit is an error or incomplete, never a job: one more
/// than MAX_TARGET does not reach `submit_job`.
#[test]
fn past_the_limit_is_no_job() {
    let _g = rig();
    use_program();
    let m = geti(GL_MAX_RENDERBUFFER_SIZE);
    let f = make_fbo(m + 1, 1, Colour::Renderbuffer(GL_RGBA4), false);
    let e = err();
    let complete = f.complete();
    assert!(e == GL_INVALID_VALUE || !complete, "a renderbuffer past GL_MAX_RENDERBUFFER_SIZE: {e:#x}");
    if complete {
        draw_triangle();
        // SAFETY: a plain GL call.
        unsafe { glFinish() };
    }
    check_jobs("past the limit", m as u32);
    let _ = err();
    f.free();
}
