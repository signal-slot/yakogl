// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! glGenerateMipmap with `GL_TEXTURE_BASE_LEVEL` 1 on V3D (plan #382 item
//! genmipbase, issue #1261). GLES 3.0 section 3.8.9: the levels base+1..q are
//! derived from the image of level `base`; levels below the base, and the
//! base level itself, are left as they are.
//!
//! What the harness observes: the GL core's glGenerateMipmap on a texture
//! that no job rendered into (the CPU box filter, `generate_mipmap_3d` for a
//! 3D texture), read back through a framebuffer attachment of each level
//! (glFramebufferTexture2D / glFramebufferTextureLayer + glReadPixels). The
//! TFU path of `v3d_generate_mipmap` (a render target's chain, see
//! tests/v3d_tfu_mipmap_nowait.rs) is NOT observed here: it needs the OSAL's
//! TFU queue and a stored render image. By reading, it is the same today: the
//! GL core hands the back end `texture_bind_2d` (no base level in it) and
//! `chain_levels`, and the TFU job `v3d_tex_tfu_pack_mipmap` makes levels
//! 1.. from level 0.
//!
//! Texture: 8x8 (3D 8x8x4), four levels, level n filled with
//! (n*60+40, 0, 0, 255): 40, 100, 160, 220 in red, so a generated level's
//! red tells which source level it came from (box filters of a flat
//! colour keep it). Today the GL core ignores GL_TEXTURE_BASE_LEVEL in
//! glGenerateMipmap: it reads level 0 and rewrites levels 1.. (the base level
//! 1 included), raising no error. The spec's result at BASE_LEVEL 1 is level 1
//! kept (100) and levels 2, 3 made from it (100, 100): the tests that differ
//! are named `_flip_when_the_base_level_is_applied`. BASE_LEVEL 0 and level 0
//! staying untouched match the spec and are plain regression tests. Today's
//! behaviour does NOT match section 3.8.9 for a base level above 0.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_genmip_base_level_v3d
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-v3d", feature = "host-test", not(feature = "report-es2")))]

use core::ffi::{c_char, c_int, c_void, CStr};
use std::collections::BTreeMap;
use std::ffi::CString;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Mutex, MutexGuard, Once, OnceLock};
use yakogl::gl::consts::*;
use yakogl::gl::context;
use yakogl::gl::state::*;
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps, OsalV3dCpuOf};
use yakogl::v3d::backend::yakogl_backend_v3d;
use yakogl::v3d::ffi::*;
use yakogl::v3d::kick::V3dKick;

/* ---- the job builder: records each draw's sampler bindings ---- */

static LOG: Mutex<Vec<String>> = Mutex::new(Vec::new());
/// One draw handed to the job builder: its fragment-stage sampler bindings.
#[derive(Clone)]
#[allow(dead_code)]
struct Draw {
    tmu: Vec<V3dJobTmu>,
}
static DRAWS: Mutex<Vec<Draw>> = Mutex::new(Vec::new());

fn log_lines() -> Vec<String> {
    LOG.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

#[allow(dead_code)]
fn draws() -> Vec<Draw> {
    DRAWS.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// `n` records at `p`, or none.
fn records(p: *const V3dJobTmu, n: u32) -> Vec<V3dJobTmu> {
    if p.is_null() || n == 0 {
        Vec::new()
    } else {
        // SAFETY: the draw's `n` records, live for the call.
        unsafe { std::slice::from_raw_parts(p, n as usize) }.to_vec()
    }
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
    // SAFETY: the backend passes its live job and a draw whose pointers live for the call.
    let d = unsafe { &*d };
    let draw = Draw { tmu: records(d.tmu, d.num_tmu) };
    DRAWS.lock().unwrap_or_else(|e| e.into_inner()).push(draw);
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
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("es3_genmip_base_level_v3d");
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
    trampoline!(v3d_tex_pack_state(out: *mut u8, s: *const V3dTexState) -> bool);
    trampoline!(v3d_tex_pack_sampler(out: *mut u8, s: *const V3dSampler) -> bool);
    trampoline!(v3d_tex_pack_sampler_compare(out: *mut u8, s: *const V3dSampler, compare_op: u32) -> bool);
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
    use yakogl::osal::{OSAL_QUERY_V3D_BIN_POOL, OSAL_QUERY_V3D_CPU_OF_GPU, OSAL_QUERY_V3D_VPM_BYTES};
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
/// each starts with nothing recorded and nothing pending. The lock also
/// serialises the interpreter's own compiles with the GL core's.
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
    }
    LOG.lock().unwrap_or_else(|e| e.into_inner()).clear();
    DRAWS.lock().unwrap_or_else(|e| e.into_inner()).clear();
    g
}

use yakogl::gl::framebuffer::*;
use yakogl::gl::texture::*;

fn err() -> GLenum {
    // SAFETY: plain GL calls.
    unsafe {
        let e = glGetError();
        while glGetError() != GL_NO_ERROR {}
        e
    }
}

const RED: [u8; 4] = [0, 0, 0, 255];

/// Level `n`'s red: 40, 100, 160, 220.
fn red(n: usize) -> u8 {
    (n * 60 + 40) as u8
}

/// A mipmapped RGBA8 texture, 8x8 (3D: 8x8x4 halved per level), four levels,
/// GL_TEXTURE_BASE_LEVEL `base`, bound to unit 0.
fn make(target: GLenum, base: GLint) -> GLuint {
    let mut t = 0;
    // SAFETY: plain GL calls; the arrays outlive each upload.
    unsafe {
        glGenTextures(1, &mut t);
        glActiveTexture(GL_TEXTURE0);
        glBindTexture(target, t);
        for level in 0..4usize {
            let w = (8usize >> level).max(1);
            let d = if target == GL_TEXTURE_3D { (4usize >> level).max(1) } else { 1 };
            let texel = [red(level), RED[1], RED[2], RED[3]];
            let texels: Vec<u8> = (0..w * w * d).flat_map(|_| texel).collect();
            let (l, w, d) = (level as GLint, w as GLsizei, d as GLsizei);
            let p = texels.as_ptr() as *const c_void;
            if target == GL_TEXTURE_3D {
                glTexImage3D(target, l, GL_RGBA8 as GLint, w, w, d, 0, GL_RGBA, GL_UNSIGNED_BYTE, p);
            } else {
                glTexImage2D(target, l, GL_RGBA8 as GLint, w, w, 0, GL_RGBA, GL_UNSIGNED_BYTE, p);
            }
        }
        glTexParameteri(target, GL_TEXTURE_BASE_LEVEL, base);
    }
    assert_eq!(err(), GL_NO_ERROR, "{:?}", log_lines());
    t
}

/// The red of texel (0, 0) (layer 0) of `level`, read through a framebuffer.
fn read_level(target: GLenum, t: GLuint, level: GLint) -> u8 {
    let mut fbo = 0;
    let mut px = [0u8; 4];
    // SAFETY: plain GL calls.
    unsafe {
        glGenFramebuffers(1, &mut fbo);
        glBindFramebuffer(GL_FRAMEBUFFER, fbo);
        if target == GL_TEXTURE_3D {
            glFramebufferTextureLayer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, t, level, 0);
        } else {
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, target, t, level);
        }
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
        glReadPixels(0, 0, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr() as *mut c_void);
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fbo);
    }
    assert_eq!(err(), GL_NO_ERROR, "reading level {level}");
    px[0]
}

/// glGenerateMipmap on `target` with BASE_LEVEL `base`: the GL error and the
/// red of levels 0..=3 afterwards.
fn generate(target: GLenum, base: GLint) -> (GLenum, [u8; 4]) {
    let t = make(target, base);
    // SAFETY: plain GL call.
    unsafe { glGenerateMipmap(target) };
    let e = err();
    let reds = [0, 1, 2, 3].map(|l| read_level(target, t, l));
    // SAFETY: plain GL call.
    unsafe { glDeleteTextures(1, &t) };
    (e, reds)
}

/* ---- BASE_LEVEL 0: the controls, correct today and after the fix ---- */

/// Control: a 2D texture at BASE_LEVEL 0: no error; levels 1..3 are made from
/// level 0 (red 40) and level 0 is kept.
#[test]
fn a_2d_texture_at_base_level_0_generates_levels_1_to_3_from_level_0() {
    let _g = rig();
    assert_eq!(generate(GL_TEXTURE_2D, 0), (GL_NO_ERROR, [40, 40, 40, 40]));
}

/// Control: as above for a 3D texture.
#[test]
fn a_3d_texture_at_base_level_0_generates_levels_1_to_3_from_level_0() {
    let _g = rig();
    assert_eq!(generate(GL_TEXTURE_3D, 0), (GL_NO_ERROR, [40, 40, 40, 40]));
}

/* ---- BASE_LEVEL 1 ---- */

/// Matches the spec: with BASE_LEVEL 1 glGenerateMipmap raises no error, and
/// level 0 (below the base) is left as it was (red 40).
#[test]
fn a_2d_texture_at_base_level_1_raises_no_error_and_keeps_level_0() {
    let _g = rig();
    let (e, reds) = generate(GL_TEXTURE_2D, 1);
    assert_eq!(e, GL_NO_ERROR);
    assert_eq!(reds[0], 40);
}

/// Pins today's behaviour: at BASE_LEVEL 1 the 2D chain is made from level 0:
/// levels 2 and 3 have level 0's red (40). Spec (3.8.9): they derive from the
/// base level, level 1 (red 100). After the fix: 100, 100.
#[test]
fn a_2d_texture_at_base_level_1_makes_levels_2_and_3_from_level_0_flip_when_the_base_level_is_applied() {
    let _g = rig();
    let (_, reds) = generate(GL_TEXTURE_2D, 1);
    assert_eq!(&reds[2..], &[40, 40]);
}

/// Pins today's behaviour: at BASE_LEVEL 1 the base level itself is
/// overwritten (red 100 becomes level 0's 40). Spec: the base level is
/// untouched, red 100. After the fix: 100.
#[test]
fn a_2d_texture_at_base_level_1_overwrites_the_base_level_flip_when_the_base_level_is_applied() {
    let _g = rig();
    let (_, reds) = generate(GL_TEXTURE_2D, 1);
    assert_eq!(reds[1], 40);
}

/// Matches the spec: the 3D texture at BASE_LEVEL 1 raises no error and keeps
/// level 0.
#[test]
fn a_3d_texture_at_base_level_1_raises_no_error_and_keeps_level_0() {
    let _g = rig();
    let (e, reds) = generate(GL_TEXTURE_3D, 1);
    assert_eq!(e, GL_NO_ERROR);
    assert_eq!(reds[0], 40);
}

/// Pins today's behaviour: the 3D chain at BASE_LEVEL 1 is made from level 0
/// (levels 1, 2, 3 all red 40). Spec: level 1 untouched (100), levels 2 and 3
/// from it (100, 100). After the fix: [40, 100, 100, 100].
#[test]
fn a_3d_texture_at_base_level_1_generates_from_level_0_flip_when_the_base_level_is_applied() {
    let _g = rig();
    let (_, reds) = generate(GL_TEXTURE_3D, 1);
    assert_eq!(reds, [40, 40, 40, 40]);
}
