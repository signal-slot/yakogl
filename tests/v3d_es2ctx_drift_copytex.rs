// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The ES 2.0 context against 0658692a (glCopyTex* copies an RGB10_A2 surface
//! into an RGB10A2 texture as the packed word). The change sits in
//! `texture_copy_from_fb`, which every glCopyTexImage2D / glCopyTexSubImage2D
//! reaches, and both calls are ES 2.0 core (dEQP-GLES2
//! texture.specification.basic_copyteximage2d.*, basic_copytexsubimage2d.*,
//! negative_api.texture.copyteximage2d_*). The Pi 4 whole tree (46f93767)
//! predates it. Through the real GL core (a `report-es2` build) and the real
//! V3D back end (rig of tests/v3d_es2ctx_refusals.rs: the job builder is stood
//! in for, the texture layout is the real driver/src/v3d/v3d_tex.c) this file
//! pins, for each source (RGBA8 texture attachment, RGB565 and RGBA4
//! renderbuffer) and each destination (RGBA, RGB, LUMINANCE, LUMINANCE_ALPHA,
//! ALPHA): the copy answers GL_NO_ERROR (also where ES 2.0 3.7.2 would say
//! GL_INVALID_OPERATION: see `expect`), for the
//! image and the sub-image call, on a 2D texture and on a cube face, at level
//! 0 and level 1, with offsets, and with a source rectangle partly outside
//! the framebuffer (accepted: the outside texels are undefined, not an error).
//! The ES 2.0 errors of the neighbouring arguments are pinned too.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table,report-es2 --test v3d_es2ctx_drift_copytex
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

use yakogl::gl::texture::*;

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

#[derive(Clone, Copy, Debug)]
enum Src {
    Rgba8,
    Rgb565,
    Rgba4,
}
const SRCS: [Src; 3] = [Src::Rgba8, Src::Rgb565, Src::Rgba4];

/// The answer the GL core gives every pair. ES 2.0 3.7.2 makes a destination
/// with a component the source lacks (RGB565 into RGBA, LUMINANCE_ALPHA or
/// ALPHA) GL_INVALID_OPERATION; the core does not check that rule (no
/// dEQP-GLES2 case asks: its copy cases read RGBA framebuffers), and
/// 0658692a did not touch it. This file pins the answer the Pi 4 tree was
/// measured with, GL_NO_ERROR for all fifteen pairs, so a change in either
/// direction shows here; when the core learns the rule, `expect` takes it.
fn expect(_src: Src, _dst: GLenum) -> GLenum {
    GL_NO_ERROR
}

const DSTS: [GLenum; 5] = [GL_RGBA, GL_RGB, GL_LUMINANCE, GL_LUMINANCE_ALPHA, GL_ALPHA];

const FACES: [GLenum; 7] = [
    GL_TEXTURE_2D,
    GL_TEXTURE_CUBE_MAP_POSITIVE_X,
    GL_TEXTURE_CUBE_MAP_NEGATIVE_X,
    GL_TEXTURE_CUBE_MAP_POSITIVE_Y,
    GL_TEXTURE_CUBE_MAP_NEGATIVE_Y,
    GL_TEXTURE_CUBE_MAP_POSITIVE_Z,
    GL_TEXTURE_CUBE_MAP_NEGATIVE_Z,
];

/// The texture of the last case, deleted when the next one is made: the
/// stand-in platform keeps every texture's storage until then.
static LAST: Mutex<GLuint> = Mutex::new(0);

fn bind_target(face: GLenum) {
    let bind = if face == GL_TEXTURE_2D { GL_TEXTURE_2D } else { GL_TEXTURE_CUBE_MAP };
    let mut last = LAST.lock().unwrap_or_else(|e| e.into_inner());
    let mut t = 0;
    // SAFETY: plain GL calls with room for one name.
    unsafe {
        if *last != 0 {
            glDeleteTextures(1, &*last);
        }
        glActiveTexture(GL_TEXTURE0);
        glGenTextures(1, &mut t);
        glBindTexture(bind, t);
    }
    *last = t;
}

/// An FBO of `src` (W x H), cleared, left bound; returns its name.
fn source(src: Src) -> GLuint {
    let mut fb = 0;
    // SAFETY: plain GL calls with room for one name each.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glViewport(0, 0, W, H);
        match src {
            Src::Rgba8 => {
                let mut t = 0;
                glGenTextures(1, &mut t);
                glBindTexture(GL_TEXTURE_2D, t);
                glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA as GLint, W, H, 0, GL_RGBA, GL_UNSIGNED_BYTE, core::ptr::null());
                glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, t, 0);
            }
            Src::Rgb565 | Src::Rgba4 => {
                let mut rb = 0;
                glGenRenderbuffers(1, &mut rb);
                glBindRenderbuffer(GL_RENDERBUFFER, rb);
                let f = if matches!(src, Src::Rgb565) { GL_RGB565 } else { GL_RGBA4 };
                glRenderbufferStorage(GL_RENDERBUFFER, f, W, H);
                glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb);
            }
        }
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "{src:?}");
        glClearColor(0.25, 0.5, 0.75, 1.0);
        glClear(GL_COLOR_BUFFER_BIT);
        glFinish();
    }
    assert_eq!(err(), GL_NO_ERROR, "the {src:?} source");
    fb
}

fn unbind(fb: GLuint) {
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fb);
    }
}

/// The texture of the next case: a fresh one bound for `face`'s binding point.
/// For the sub-image call it is defined first (two levels, `dst`, 16 x 16).
fn define(face: GLenum, dst: GLenum) {
    bind_target(face);
    let texels = vec![0x40u8; 16 * 16 * 4];
    for f in faces_of(face) {
        for (level, size) in [(0, 16), (1, 8)] {
            // SAFETY: plain GL call; the buffer holds 4 bytes a texel at most.
            unsafe {
                glTexImage2D(
                    f,
                    level,
                    dst as GLint,
                    size,
                    size,
                    0,
                    dst,
                    GL_UNSIGNED_BYTE,
                    texels.as_ptr() as *const c_void,
                );
            }
        }
    }
    assert_eq!(err(), GL_NO_ERROR, "defining {dst:#x}");
}

fn faces_of(face: GLenum) -> Vec<GLenum> {
    if face == GL_TEXTURE_2D {
        vec![GL_TEXTURE_2D]
    } else {
        FACES[1..].to_vec()
    }
}

/// The source rectangles: inside, offset, partly outside on each side, wholly outside.
const RECTS: [(GLint, GLint, GLsizei, GLsizei); 7] = [
    (0, 0, 16, 16),
    (5, 7, 8, 4),
    (-4, -4, 16, 16),
    (24, 24, 16, 16),
    (-8, 20, 16, 16),
    (W + 4, H + 4, 8, 8),
    (0, 0, 0, 0),
];

/* ---- the cases ---- */

/// Sanity: this is an ES 2.0 context (GL_MAJOR_VERSION is an ES 3.0 query).
#[test]
fn the_context_is_es2() {
    let _g = rig();
    let mut m = 0;
    // SAFETY: a plain GL call.
    unsafe { glGetIntegerv(GL_MAJOR_VERSION, &mut m) };
    assert_eq!(err(), GL_INVALID_ENUM);
}

/// glCopyTexImage2D, every source x destination, on the 2D target and every
/// cube face, levels 0 and 1, every rectangle (offset, partly outside).
#[test]
fn copyteximage2d_pairs() {
    let _g = rig();
    for src in SRCS {
        let fb = source(src);
        for dst in DSTS {
            for face in FACES {
                for level in [0, 1] {
                    for (x, y, w, h) in RECTS {
                        bind_target(face);
                        /* a cube face is square */
                        let h = if face == GL_TEXTURE_2D { h } else { w };
                        let (w, h) = (w >> level, h >> level);
                        // SAFETY: a plain GL call.
                        unsafe { glCopyTexImage2D(face, level, dst, x, y, w, h, 0) };
                        assert_eq!(
                            err(),
                            expect(src, dst),
                            "CopyTexImage2D {src:?} -> {dst:#x} target {face:#x} level {level} rect {x},{y} {w}x{h}"
                        );
                    }
                }
            }
        }
        unbind(fb);
    }
}

/// glCopyTexSubImage2D into a defined texture: the same grid plus offsets
/// into the texture. The sub-image call has the same format rule.
#[test]
fn copytexsubimage2d_pairs() {
    let _g = rig();
    for src in SRCS {
        let fb = source(src);
        for dst in DSTS {
            for face in [GL_TEXTURE_2D, GL_TEXTURE_CUBE_MAP_POSITIVE_X, GL_TEXTURE_CUBE_MAP_NEGATIVE_Z] {
                define(face, dst);
                for level in [0, 1] {
                    let size = 16 >> level;
                    for (x, y, w, h) in RECTS {
                        for (xo, yo) in [(0, 0), (2 >> level, 3 >> level)] {
                            let h = if face == GL_TEXTURE_2D { h } else { w };
                            let (w, h) = ((w >> level).min(size - xo), (h >> level).min(size - yo));
                            // SAFETY: a plain GL call.
                            unsafe { glCopyTexSubImage2D(face, level, xo, yo, x, y, w, h) };
                            assert_eq!(
                                err(),
                                expect(src, dst),
                                "CopyTexSubImage2D {src:?} -> {dst:#x} target {face:#x} level {level} \
                                 offset {xo},{yo} rect {x},{y} {w}x{h}"
                            );
                        }
                    }
                }
            }
        }
        unbind(fb);
    }
}

/// The sub-image of a copy-defined texture: a glCopyTexImage2D defines the
/// level, a glCopyTexSubImage2D replaces a part of it (dEQP
/// basic_copytexsubimage2d reads the level it just copied).
#[test]
fn copytexsubimage2d_into_a_copied_level() {
    let _g = rig();
    for src in [Src::Rgba8, Src::Rgba4] {
        let fb = source(src);
        for dst in DSTS {
            bind_target(GL_TEXTURE_2D);
            // SAFETY: plain GL calls.
            unsafe {
                glCopyTexImage2D(GL_TEXTURE_2D, 0, dst, 0, 0, 16, 16, 0);
                assert_eq!(err(), GL_NO_ERROR, "{src:?} -> {dst:#x}");
                glCopyTexSubImage2D(GL_TEXTURE_2D, 0, 4, 4, 8, 8, 8, 8);
            }
            assert_eq!(err(), GL_NO_ERROR, "sub-image {src:?} -> {dst:#x}");
        }
        unbind(fb);
    }
}

/// The ES 2.0 errors of the other arguments, which the format conversion must
/// not move (dEQP negative_api.texture.copyteximage2d_*).
#[test]
fn copytex_argument_errors() {
    let _g = rig();
    let fb = source(Src::Rgba8);
    bind_target(GL_TEXTURE_2D);
    // SAFETY: plain GL calls.
    unsafe {
        /* the target */
        glCopyTexImage2D(GL_TEXTURE_3D, 0, GL_RGBA, 0, 0, 4, 4, 0);
        assert_eq!(err(), GL_INVALID_ENUM, "target");
        glCopyTexImage2D(GL_TEXTURE_CUBE_MAP, 0, GL_RGBA, 0, 0, 4, 4, 0);
        assert_eq!(err(), GL_INVALID_ENUM, "the cube map itself");
        /* the internal format: ES 2.0 has the unsized names only */
        glCopyTexImage2D(GL_TEXTURE_2D, 0, 0x1234, 0, 0, 4, 4, 0);
        assert_eq!(err(), GL_INVALID_ENUM, "unknown format");
        glCopyTexImage2D(GL_TEXTURE_2D, 0, GL_DEPTH_COMPONENT, 0, 0, 4, 4, 0);
        assert_eq!(err(), GL_INVALID_ENUM, "depth format");
        /* values */
        glCopyTexImage2D(GL_TEXTURE_2D, -1, GL_RGBA, 0, 0, 4, 4, 0);
        assert_eq!(err(), GL_INVALID_VALUE, "level -1");
        glCopyTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, 0, 0, -1, 4, 0);
        assert_eq!(err(), GL_INVALID_VALUE, "width -1");
        glCopyTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, 0, 0, 4, -1, 0);
        assert_eq!(err(), GL_INVALID_VALUE, "height -1");
        glCopyTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, 0, 0, 4, 4, 1);
        assert_eq!(err(), GL_INVALID_VALUE, "border 1");
        glCopyTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, 0, 0, 1 << 20, 4, 0);
        assert_eq!(err(), GL_INVALID_VALUE, "width past the maximum");
        glCopyTexImage2D(GL_TEXTURE_2D, 100, GL_RGBA, 0, 0, 4, 4, 0);
        assert_eq!(err(), GL_INVALID_VALUE, "level past the maximum");
        /* sub-image */
        glCopyTexSubImage2D(GL_TEXTURE_3D, 0, 0, 0, 0, 0, 4, 4);
        assert_eq!(err(), GL_INVALID_ENUM, "sub target");
        glCopyTexSubImage2D(GL_TEXTURE_2D, -1, 0, 0, 0, 0, 4, 4);
        assert_eq!(err(), GL_INVALID_VALUE, "sub level -1");
        glCopyTexSubImage2D(GL_TEXTURE_2D, 0, -1, 0, 0, 0, 4, 4);
        assert_eq!(err(), GL_INVALID_VALUE, "sub xoffset -1");
        glCopyTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 0, 0, -1, 4);
        assert_eq!(err(), GL_INVALID_VALUE, "sub width -1");
    }
    /* sub-image of a texture with no level 0, and of a level past the chain, and outside the level */
    define(GL_TEXTURE_2D, GL_RGBA);
    // SAFETY: plain GL calls.
    unsafe {
        glCopyTexSubImage2D(GL_TEXTURE_2D, 0, 8, 8, 0, 0, 16, 16);
        assert_eq!(err(), GL_INVALID_VALUE, "sub rectangle past the level");
        glCopyTexSubImage2D(GL_TEXTURE_2D, 5, 0, 0, 0, 0, 1, 1);
        assert_eq!(err(), GL_INVALID_OPERATION, "sub level not defined");
        glBindTexture(GL_TEXTURE_2D, 0);
        glCopyTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 0, 0, 1, 1);
        assert_eq!(err(), GL_INVALID_OPERATION, "sub with texture 0 bound");
    }
    /* an incomplete framebuffer: no attachment */
    // SAFETY: plain GL calls.
    unsafe {
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, 0, 0);
        bind_target(GL_TEXTURE_2D);
        glCopyTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, 0, 0, 4, 4, 0);
        assert_eq!(err(), GL_INVALID_FRAMEBUFFER_OPERATION, "incomplete framebuffer");
        glCopyTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 0, 0, 1, 1);
        assert_eq!(err(), GL_INVALID_FRAMEBUFFER_OPERATION, "sub, incomplete framebuffer");
    }
    unbind(fb);
}

/// The default framebuffer of the context as the source.
#[test]
fn copytex_from_the_default_framebuffer() {
    let _g = rig();
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glClear(GL_COLOR_BUFFER_BIT);
    }
    for dst in [GL_RGBA, GL_RGB, GL_LUMINANCE, GL_LUMINANCE_ALPHA, GL_ALPHA] {
        bind_target(GL_TEXTURE_2D);
        // SAFETY: plain GL calls.
        unsafe { glCopyTexImage2D(GL_TEXTURE_2D, 0, dst, 0, 0, 16, 16, 0) };
        let e = err();
        assert!(e == GL_NO_ERROR || e == GL_INVALID_OPERATION, "{dst:#x}: {e:#x}");
    }
}
