// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The ES 2.0 context against the V3D clear quad of 7fa02269 (a scissored or
//! masked colour clear of several attachments is one quad) and e883be8f (an
//! integer clear program for a scissored or masked clear). The Pi 4 whole
//! dEQP-GLES2 tree (46f93767) predates both, and dEQP-GLES2 clears under a
//! scissor and a colour mask widely (fragment_ops, color_clear). Both
//! commits changed `draw_clear_quad` (src/v3d/backend.rs), which every clear
//! the TLB cannot do goes through. ES 2.0 has one colour attachment, so the
//! several-attachment program is never built and the integer program
//! (`CLEAR_FS_INT`) is never reached. This file replays the ES 2.0 clears
//! through the real GL core (a `report-es2` build) and the real V3D back end
//! and asserts, for each: GL_NO_ERROR, `v3d_refused_draws()` unchanged, no
//! "integer clear program" in the back end's log and, from the stand-in job
//! builder, which records every draw it is handed, that
//!
//! - a whole-target, unmasked clear is the TLB's (no quad),
//! - a scissored, masked or combined colour clear is exactly one quad (six
//!   indices, the one float fragment program every such clear shares) with
//!   the clear's scissor rectangle and colour mask in the draw's state,
//! - a depth or stencil clear under a scissor is one quad with the depth or
//!   stencil test on and no colour write.
//!
//! Surfaces: the default framebuffer, and FBOs with an RGBA8 texture, an RGB
//! texture, and RGBA4, RGB5_A1 and RGB565 renderbuffers, plus depth and
//! stencil renderbuffers. The texture layout code is the real
//! driver/src/v3d/v3d_tex.c.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table,report-es2 --test v3d_es2ctx_more_clears
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

fn texture_target(format: GLenum) {
    let texels = [0x7fu8; (W * H) as usize * 4];
    // SAFETY: plain GL calls; the buffer holds 4 bytes a texel.
    unsafe {
        let mut t = 0;
        glActiveTexture(GL_TEXTURE0);
        glGenTextures(1, &mut t);
        glBindTexture(GL_TEXTURE_2D, t);
        let data = texels.as_ptr() as *const c_void;
        glTexImage2D(GL_TEXTURE_2D, 0, format as GLint, W, H, 0, format, GL_UNSIGNED_BYTE, data);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, t, 0);
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

/// A colour target of ES 2.0 (with whether it stores alpha: the quad masks
/// alpha off on a format with none, `job_state`).
#[derive(Clone, Copy, Debug)]
enum Surface {
    Default,
    Texture(GLenum, bool),
    Renderbuffer(GLenum, bool),
}

const SURFACES: [Surface; 6] = [
    Surface::Default,
    Surface::Texture(GL_RGBA, true),
    Surface::Texture(GL_RGB, false),
    Surface::Renderbuffer(GL_RGBA4, true),
    Surface::Renderbuffer(GL_RGB5_A1, true),
    Surface::Renderbuffer(GL_RGB565, false),
];

/// Run `f(alpha)` on `s`; an incomplete FBO is skipped (the GL core's
/// answer). `depth_stencil` adds 16-bit depth and 8-bit stencil renderbuffers.
fn on_surface(s: Surface, depth_stencil: bool, f: impl FnOnce(bool)) {
    let (fb, alpha) = match s {
        Surface::Default => (0, true),
        Surface::Texture(format, alpha) => {
            let fb = bind_fbo();
            texture_target(format);
            (fb, alpha)
        }
        Surface::Renderbuffer(format, alpha) => {
            let fb = bind_fbo();
            renderbuffer(GL_COLOR_ATTACHMENT0, format);
            (fb, alpha)
        }
    };
    if depth_stencil && fb != 0 {
        renderbuffer(GL_DEPTH_ATTACHMENT, GL_DEPTH_COMPONENT16);
        renderbuffer(GL_STENCIL_ATTACHMENT, GL_STENCIL_INDEX8);
    }
    if fb == 0 || complete() {
        f(alpha);
    }
    if fb != 0 {
        unbind_fbo(fb);
    }
}

/// One clear, and what the job builder was handed for it.
fn clear(what: &str, mask: GLbitfield, scissor: Option<[GLint; 4]>, colormask: [u8; 4]) -> Vec<Quad> {
    DRAWS.lock().unwrap_or_else(|e| e.into_inner()).clear();
    accepted(what, || {
        // SAFETY: plain GL calls.
        unsafe {
            glClearColor(0.25, 0.5, 0.75, 1.0);
            glClearDepthf(0.5);
            glClearStencil(0x55);
            glColorMask(colormask[0], colormask[1], colormask[2], colormask[3]);
            match scissor {
                Some([x, y, w, h]) => {
                    glEnable(GL_SCISSOR_TEST);
                    glScissor(x, y, w, h);
                }
                None => glDisable(GL_SCISSOR_TEST),
            }
            glClear(mask);
            glDisable(GL_SCISSOR_TEST);
            glColorMask(1, 1, 1, 1);
        }
    });
    draws()
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

const RECT: [GLint; 4] = [5, 3, 17, 11];

/// The colour clears: whole-target and unmasked (the TLB's), scissored,
/// masked, and both, on every surface.
#[test]
fn colour_clears_are_one_quad() {
    let _g = rig();
    let mut programs = Vec::new();
    for s in SURFACES {
        on_surface(s, false, |alpha| {
            let full = if alpha { 0xf } else { 0x7 };
            let q = clear(&format!("{s:?} whole"), GL_COLOR_BUFFER_BIT, None, [1, 1, 1, 1]);
            assert!(q.is_empty(), "{s:?}: a whole-target, unmasked clear is the TLB's, not a quad: {q:?}");

            for (what, scissor, mask, want) in [
                ("scissored", Some(RECT), [1, 1, 1, 1], full),
                ("masked", None, [1, 0, 1, 1], 0b1101 & full),
                ("masked rgb", None, [1, 1, 1, 0], 0b0111),
                ("scissored and masked", Some(RECT), [0, 1, 0, 1], 0b1010 & full),
            ] {
                let q = clear(&format!("{s:?} {what}"), GL_COLOR_BUFFER_BIT, scissor, mask);
                assert_eq!(q.len(), 1, "{s:?} {what}: one quad: {q:?}");
                let q = &q[0];
                assert_eq!(q.count, 6, "{s:?} {what}: two triangles");
                assert_eq!(q.colormask, want, "{s:?} {what}: the colour mask");
                assert!(!q.depth && !q.stencil, "{s:?} {what}: colour only");
                assert_eq!(q.words, [0.25f32, 0.5, 0.75, 1.0].map(f32::to_bits), "{s:?} {what}: the colour");
                match scissor {
                    Some([x, y, w, h]) => {
                        let r = q.scissor.unwrap_or_else(|| panic!("{s:?} {what}: no scissor in the draw"));
                        assert_eq!((r[2] - r[0], r[3] - r[1]), (w as u32, h as u32), "{s:?} {what}: {r:?}");
                        if !matches!(s, Surface::Default) {
                            assert_eq!((r[0], r[1]), (x as u32, y as u32), "{s:?} {what}: {r:?}");
                        }
                    }
                    None => assert_eq!(q.scissor, None, "{s:?} {what}: no scissor"),
                }
                programs.push(q.fs);
            }
        });
    }
    assert!(programs.len() >= 4);
    assert!(programs.windows(2).all(|w| w[0] == w[1]), "one float clear program for every ES 2.0 clear: {programs:x?}");
}

/// Depth and stencil clears under a scissor, alone and with the colour.
#[test]
fn scissored_depth_and_stencil_clears_are_one_quad() {
    let _g = rig();
    for s in SURFACES {
        on_surface(s, true, |_| {
            for (what, mask, colour, depth, stencil) in [
                ("depth", GL_DEPTH_BUFFER_BIT, false, true, false),
                ("stencil", GL_STENCIL_BUFFER_BIT, false, false, true),
                ("depth and stencil", GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT, false, true, true),
                ("all three", GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT, true, true, true),
            ] {
                let q = clear(&format!("{s:?} {what}"), mask, Some(RECT), [1, 1, 1, 1]);
                assert_eq!(q.len(), 1, "{s:?} {what}: one quad: {q:?}");
                let q = &q[0];
                assert_eq!(q.count, 6, "{s:?} {what}: two triangles");
                assert_eq!((q.depth, q.stencil), (depth, stencil), "{s:?} {what}: the buffers");
                assert_eq!(q.colormask != 0, colour, "{s:?} {what}: colour written");
                let r = q.scissor.unwrap_or_else(|| panic!("{s:?} {what}: no scissor in the draw"));
                assert_eq!((r[2] - r[0], r[3] - r[1]), (RECT[2] as u32, RECT[3] as u32), "{s:?} {what}: {r:?}");
            }
        });
    }
}
