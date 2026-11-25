// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The ES 2.0 context against the V3D clear-plan commits 949f0bef (clears of
//! glDrawBuffers lists whose draw buffer 0 is GL_NONE: the `state::unsupported`
//! refusal now applies to draws only), ab0222cf (r and rg integer clears keep
//! the wide store's absent channels at 0, 0, 1: `int_clear_channels` /
//! `apply_clear_plan` in the GL core) and 9fe54bc6 (`clear_color_words`, the
//! clear-colour words of 16F, 32F and 128-bpp float targets, now called from
//! `state::clear_words`). The Pi 4 whole dEQP-GLES2 tree (46f93767) predates
//! them, and nearly every dEQP-GLES2 case begins with a whole-target glClear.
//! tests/v3d_es2ctx_more_clears.rs covers the scissored and masked clears;
//! this file covers the whole, unmasked glClear of colour, depth and stencil,
//! alone and in every combination (seven masks), on the default framebuffer,
//! on RGBA8, RGB565, RGBA4 and RGB5_A1 renderbuffers and RGBA/UNSIGNED_BYTE,
//! RGB/UNSIGNED_SHORT_5_6_5, RGBA/UNSIGNED_SHORT_4_4_4_4 and
//! RGBA/UNSIGNED_SHORT_5_5_5_1 textures, with and without DEPTH_COMPONENT16
//! and STENCIL_INDEX8 renderbuffers, with clear values 0, 1, mid and out of
//! range (clamped). For each it asserts
//!
//! - GL_NO_ERROR and `v3d_refused_draws()` unchanged;
//! - the clear path, as before 949f0bef: the Clear Render Targets words (the
//!   job's clear values at job end), no clear draw (a quad) and no "integer
//!   clear program" in the log;
//! - the packed colour words of an 8-bit-tile-buffer target (every ES 2.0
//!   colour format, RGB565 and RGBA4 included, clears through the one
//!   R8G8B8A8_UNORM word) are the rule written in `rgba8_word`: each channel
//!   clamped to [0, 1], times 255, rounded half to even, red in the low byte.
//!   `clear_rt[0]` is that word and its words 1..3 are 0 (the 16F and 32F
//!   rows of 9fe54bc6 are not reached);
//! - the draw-buffer handling: ES 2.0 has draw buffer 0 alone, so a colour
//!   clear (no clear_target_mask) sets every RT slot of the job to the same
//!   word and loads none, the loop of `v3d_clear` as it was before 949f0bef
//!   (slots 1..3 are inert); a clear without colour leaves them all loaded.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table,report-es2 --test v3d_es2ctx_drift_clear_plan
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

/// One draw the job builder was handed.
#[derive(Clone, Debug)]
#[allow(dead_code)] // only printed in a failure message: no clear quad is expected here
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

/// What a job's end was handed: the job's clear values.
#[derive(Clone, Debug)]
struct End {
    load_rt0: bool,
    rgba8: u32,
    z: f32,
    stencil: u32,
    load_rt: [bool; 3],
    rgba8_rt: [u32; 3],
    clear_rt: [[u32; 4]; 4],
}

static ENDS: Mutex<Vec<End>> = Mutex::new(Vec::new());

fn ends() -> Vec<End> {
    ENDS.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

#[no_mangle]
unsafe extern "C" fn v3d_job_end(
    j: *mut V3dJob,
    c: *const V3dJobClear,
    _tile_list_start: *mut u32,
    _tile_list_end: *mut u32,
) -> bool {
    // SAFETY: the backend passes its live job, clear values and lists.
    unsafe {
        let c = &*c;
        ENDS.lock().unwrap_or_else(|e| e.into_inner()).push(End {
            load_rt0: c.load_rt0,
            rgba8: c.rgba8,
            z: c.z,
            stencil: c.stencil,
            load_rt: c.load_rt,
            rgba8_rt: c.rgba8_rt,
            clear_rt: c.clear_rt,
        });
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
    ENDS.lock().unwrap_or_else(|e| e.into_inner()).clear();
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

fn renderbuffer(attachment: GLenum, format: GLenum) {
    // SAFETY: plain GL calls with room for one name.
    unsafe {
        let mut rb = 0;
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorage(GL_RENDERBUFFER, format, W, H);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, attachment, GL_RENDERBUFFER, rb);
    }
}

fn texture_target(format: GLenum, ty: GLenum) {
    let texels = [0x7fu8; (W * H) as usize * 4];
    // SAFETY: plain GL calls; the buffer holds 4 bytes a texel, enough for any type here.
    unsafe {
        let mut t = 0;
        glActiveTexture(GL_TEXTURE0);
        glGenTextures(1, &mut t);
        glBindTexture(GL_TEXTURE_2D, t);
        let data = texels.as_ptr() as *const c_void;
        glTexImage2D(GL_TEXTURE_2D, 0, format as GLint, W, H, 0, format, ty, data);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, t, 0);
    }
}

fn complete() -> bool {
    // SAFETY: a plain GL call.
    unsafe { glCheckFramebufferStatus(GL_FRAMEBUFFER) == GL_FRAMEBUFFER_COMPLETE }
}

fn unbind_fbo(fb: GLuint) {
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fb);
    }
}

/// A colour target of ES 2.0.
#[derive(Clone, Copy, Debug)]
enum Surface {
    Default,
    /// An FBO with a texture colour attachment of (format, type).
    Texture(GLenum, GLenum),
    Renderbuffer(GLenum),
}

const SURFACES: [Surface; 9] = [
    Surface::Default,
    Surface::Texture(GL_RGBA, GL_UNSIGNED_BYTE),
    Surface::Texture(GL_RGB, GL_UNSIGNED_SHORT_5_6_5),
    Surface::Texture(GL_RGBA, GL_UNSIGNED_SHORT_4_4_4_4),
    Surface::Texture(GL_RGBA, GL_UNSIGNED_SHORT_5_5_5_1),
    Surface::Renderbuffer(GL_RGBA8),
    Surface::Renderbuffer(GL_RGB565),
    Surface::Renderbuffer(GL_RGBA4),
    Surface::Renderbuffer(GL_RGB5_A1),
];

/// Run `f(has_depth, has_stencil)` on `s`. `zs` adds the 16-bit depth and
/// 8-bit stencil renderbuffers. An incomplete FBO is skipped with a note (the
/// GL core's answer, not a back-end refusal); returns whether `f` ran.
fn on_surface(s: Surface, zs: bool, f: impl FnOnce(bool, bool)) -> bool {
    let fb = match s {
        Surface::Default => 0,
        Surface::Texture(format, ty) => {
            let fb = bind_fbo();
            texture_target(format, ty);
            fb
        }
        Surface::Renderbuffer(format) => {
            let fb = bind_fbo();
            renderbuffer(GL_COLOR_ATTACHMENT0, format);
            fb
        }
    };
    if zs && fb != 0 {
        renderbuffer(GL_DEPTH_ATTACHMENT, GL_DEPTH_COMPONENT16);
        renderbuffer(GL_STENCIL_ATTACHMENT, GL_STENCIL_INDEX8);
    }
    let ran = fb == 0 || complete();
    if ran {
        f(zs || fb == 0, zs || fb == 0);
    } else {
        eprintln!("skipped {s:?} zs={zs}: the framebuffer is not complete (the GL core's answer)");
    }
    if fb != 0 {
        unbind_fbo(fb);
    }
    ran
}

/// The alpha bits of a surface: GL says the alpha of a buffer without any
/// reads 1, and a clear rounds alpha to the bits the buffer has (the GL
/// core's rule since 0.1.0, `quantise_unorm`; the colour channels are not
/// rounded, the 8-bit tile buffer holds them).
fn alpha_bits(s: Surface) -> u32 {
    match s {
        Surface::Default => 8,
        /* the GL core stores every RGBA texture as RGBA8, whatever its type */
        Surface::Texture(GL_RGBA, _) => 8,
        Surface::Texture(..) => 0,
        Surface::Renderbuffer(GL_RGBA8) => 8,
        Surface::Renderbuffer(GL_RGBA4) => 4,
        Surface::Renderbuffer(GL_RGB5_A1) => 1,
        Surface::Renderbuffer(_) => 0,
    }
}

/// The clear colour after the target's alpha rule: bits 0 reads 1; fewer
/// than 8 round clamp(a) * (2^bits - 1) half up, in steps of 1 / (2^bits - 1).
fn with_alpha_rule(mut c: [f32; 4], bits: u32) -> [f32; 4] {
    c[3] = match bits {
        0 => 1.0,
        8 => c[3],
        b => {
            let steps = ((1u32 << b) - 1) as f32;
            ((c[3].clamp(0.0, 1.0) * steps + 0.5).floor()) / steps
        }
    };
    c
}

/// The rule for the clear colour word of an ES 2.0 colour target, written
/// here and not taken from the back end: clamp each channel to [0, 1],
/// scale by 255, round half to even; red is the low byte.
fn rgba8_word(c: [f32; 4]) -> u32 {
    let mut w = 0u32;
    for (i, v) in c.iter().enumerate() {
        let x = f64::from(v.clamp(0.0, 1.0)) * 255.0;
        let (fl, frac) = (x.floor(), x - x.floor());
        let r = if frac > 0.5 || (frac == 0.5 && (fl as u32) % 2 == 1) { fl + 1.0 } else { fl };
        w |= (r as u32) << (8 * i);
    }
    w
}

/// One whole, unmasked glClear; the job ends and quads it produced.
fn clear(what: &str, mask: GLbitfield, colour: [f32; 4], depth: f32, stencil: GLint) -> (Vec<End>, Vec<Quad>) {
    DRAWS.lock().unwrap_or_else(|e| e.into_inner()).clear();
    ENDS.lock().unwrap_or_else(|e| e.into_inner()).clear();
    accepted(what, || {
        // SAFETY: plain GL calls.
        unsafe {
            glClearColor(colour[0], colour[1], colour[2], colour[3]);
            glClearDepthf(depth);
            glClearStencil(stencil);
            glDisable(GL_SCISSOR_TEST);
            glColorMask(1, 1, 1, 1);
            glClear(mask);
        }
    });
    (ends(), draws())
}

/* ---- the cases ---- */

/// Sanity: this is an ES 2.0 context (the Pi 4 tree's build, `report-es2`).
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

const COLOURS: [[f32; 4]; 5] = [
    [0.0, 0.0, 0.0, 0.0],
    [1.0, 1.0, 1.0, 1.0],
    [0.25, 0.5, 0.75, 0.125],
    /* out of range: clamped by the colour clear */
    [-3.0, 7.0, 2.5, -0.5],
    /* near the rounding points: 0.5 * 255 = 127.5 (to 128) */
    [0.5, 1.5 / 255.0, 254.5 / 255.0, 0.999],
];

/// (depth, stencil) values: 0, 1, mid, out of range (the depth clamps to
/// [0, 1]; the stencil is compared on its low 8 bits).
const ZS_VALUES: [(f32, GLint); 4] = [(0.0, 0), (1.0, 0xff), (0.5, 0x55), (-2.0, 0x1ff)];

const MASKS: [(&str, GLbitfield); 7] = [
    ("colour", GL_COLOR_BUFFER_BIT),
    ("depth", GL_DEPTH_BUFFER_BIT),
    ("stencil", GL_STENCIL_BUFFER_BIT),
    ("colour+depth", GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT),
    ("colour+stencil", GL_COLOR_BUFFER_BIT | GL_STENCIL_BUFFER_BIT),
    ("depth+stencil", GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT),
    ("all three", GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT),
];

/// Whole clears of every mask, value and surface: the Clear Render Targets
/// words, no quad, the one packing, render target 0 alone.
#[test]
fn whole_clears_keep_the_tlb_path_and_the_packing() {
    let _g = rig();
    let mut ran = 0;
    for s in SURFACES {
        for zs in [false, true] {
            ran += usize::from(on_surface(s, zs, |has_d, has_s| {
                for (name, mask) in MASKS {
                    for colour in COLOURS {
                        for (depth, stencil) in ZS_VALUES {
                            let what = format!("{s:?} zs={zs} {name} {colour:?} z={depth} s={stencil:#x}");
                            let (e, q) = clear(&what, mask, colour, depth, stencil);
                            assert!(
                                q.is_empty(),
                                "{what}: a whole unmasked clear is the TLB's, not a clear draw: {q:?}"
                            );
                            let real = mask & GL_COLOR_BUFFER_BIT != 0
                                || (mask & GL_DEPTH_BUFFER_BIT != 0 && has_d)
                                || (mask & GL_STENCIL_BUFFER_BIT != 0 && has_s);
                            if !real {
                                /* clearing buffers the target lacks does nothing: no job */
                                assert!(e.is_empty(), "{what}: a clear of absent buffers kicks nothing: {e:?}");
                                continue;
                            }
                            assert_eq!(e.len(), 1, "{what}: one job: {e:?}");
                            let e = &e[0];
                            if mask & GL_COLOR_BUFFER_BIT != 0 {
                                let w = rgba8_word(with_alpha_rule(colour, alpha_bits(s)));
                                assert!(!e.load_rt0, "{what}: a cleared colour is not loaded");
                                assert_eq!(e.rgba8, w, "{what}: the RGBA8 word");
                                /* no draw-buffer list in ES 2.0: the colour clear gives every RT
                                 * slot of the job the one word (slots 1..3 are inert), as before
                                 * 949f0bef */
                                for n in 0..4 {
                                    assert_eq!(e.clear_rt[n], [w, 0, 0, 0], "{what}: RT {n}'s words");
                                }
                                assert_eq!(e.load_rt, [false; 3], "{what}: RT 1..3 not loaded");
                                assert_eq!(e.rgba8_rt, [w; 3], "{what}: RT 1..3 words");
                            } else {
                                assert!(e.load_rt0, "{what}: an uncleared colour is loaded");
                                assert_eq!((e.rgba8, e.clear_rt), (0, [[0; 4]; 4]), "{what}: no colour word");
                                assert_eq!((e.load_rt, e.rgba8_rt), ([true; 3], [0; 3]), "{what}: RT 1..3 untouched");
                            }
                            if mask & GL_DEPTH_BUFFER_BIT != 0 && has_d {
                                assert_eq!(e.z, depth.clamp(0.0, 1.0), "{what}: the clear depth");
                            }
                            if mask & GL_STENCIL_BUFFER_BIT != 0 && has_s {
                                assert_eq!(e.stencil & 0xff, stencil as u32 & 0xff, "{what}: the clear stencil");
                            }
                        }
                    }
                }
            }));
        }
    }
    assert!(ran >= 6, "the surfaces ran: {ran}");
}
