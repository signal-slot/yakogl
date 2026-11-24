// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The ES 2.0 context against 337a9453 (v3d `texture::state_at` sets the TMU
//! state's base level for a first level, clamped to the last level). ES 2.0
//! has no GL_TEXTURE_BASE_LEVEL, so in a `report-es2` context the texture
//! shader state the back end packs must keep `base_level` 0 and its
//! `max_level` as at 337a9453^, whatever a later commit wires `state_at`
//! into. The Pi 4 (V3D 4.2) whole dEQP-GLES2 tree (46f93767) predates the
//! commit; dEQP-GLES2 texture.mipmap.*, texture.size.*, the incomplete
//! texture cases and the cube map cases read these words.
//!
//! The values pinned here are what the code computed at 337a9453^ (and at
//! 337a9453, where nothing calls `state_at`): `tex_state_for`
//! (src/v3d/backend.rs) takes `texture::state`, whose `base_level` is the
//! literal 0 and whose `max_level` is `layout.levels - 1`, the layout of
//! `texture::levels_sampled(b)` levels: `b.mip_levels` (at least 1) when the
//! binding's min filter uses mipmaps (`b.mip_filter != 0`), else 1. The back
//! end then sets `max_level` to `levels - 1` again (the copy-image path). So,
//! per case:
//!
//! - a 64x64 texture with its whole chain (7 levels) and a mipmap min
//!   filter: base level 0, max level 6;
//! - the same chain with a non-mipmap min filter: 0, 0;
//! - a texture with only level 0 and a non-mipmap filter: 0, 0;
//! - an incomplete chain (levels 0 to 2 of a 64x64 under a mipmap filter; the
//!   GL core answers (0,0,0,1) and no state is packed; were one, it has base
//!   level 0 and a max level below the full chain's 6).
//! - a cube map, 16x16 faces, the chain of 5 levels, mipmap filter: 0, 4;
//! - an NPOT 37x19 texture, CLAMP_TO_EDGE, NEAREST and LINEAR (ES 2.0 allows
//!   no mipmaps for NPOT): 0, 0, width 37, height 19.
//!
//! Every state is read from the `V3dTexState` the real packer
//! `v3d_tex_pack_state` is handed (recorded in `PACKED`), through the real GL
//! entry points and V3D back end, the job builder faked. Each case asserts
//! GL_NO_ERROR and `v3d_refused_draws()` unchanged. The texture layout code
//! is the real driver/src/v3d/v3d_tex.c.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table,report-es2 --test v3d_es2ctx_drift_base_level
//! cargo clippy --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table,report-es2 --test v3d_es2ctx_drift_base_level
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
use std::sync::atomic::{AtomicU64, Ordering};
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

/// One job the back end opened.
#[allow(dead_code)]
#[derive(Clone, Debug)]
struct Job {
    begin: u64,
    /// 0 while the job is open.
    end: u64,
    address: u32,
    draws: u32,
    /// The TLB loads the colour (false: the job clears it).
    load_rt0: bool,
    rgba8: u32,
    /// A kick went to the platform after the job ended.
    submitted: bool,
}

static JOBS: Mutex<Vec<Job>> = Mutex::new(Vec::new());
static SEQ: AtomicU64 = AtomicU64::new(1);

#[allow(dead_code)]
fn jobs() -> Vec<Job> {
    JOBS.lock().unwrap_or_else(|e| e.into_inner()).clone()
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
    JOBS.lock().unwrap_or_else(|e| e.into_inner()).push(Job {
        begin: SEQ.fetch_add(1, Ordering::SeqCst),
        end: 0,
        address: t.address,
        draws: 0,
        load_rt0: true,
        rgba8: 0,
        submitted: false,
    });
    t.width != 0 && t.height != 0
}

#[no_mangle]
unsafe extern "C" fn v3d_job_draw(j: *mut V3dJob, _d: *const V3dJobDraw, _layout: *mut c_void) -> i32 {
    // SAFETY: the backend passes its live job.
    unsafe {
        (*j).draws += 1;
        (*(*j).bcl).used += 120;
    }
    if let Some(last) = JOBS.lock().unwrap_or_else(|e| e.into_inner()).last_mut() {
        last.draws += 1;
    }
    V3D_JOB_OK
}

#[no_mangle]
unsafe extern "C" fn v3d_job_end(
    j: *mut V3dJob,
    c: *const V3dJobClear,
    _tile_list_start: *mut u32,
    _tile_list_end: *mut u32,
) -> bool {
    // SAFETY: the backend passes its live job and clear; its lists are live.
    let c = unsafe {
        (*(*j).bcl).used += 1;
        (*(*j).rcl).used = 200;
        &*c
    };
    if let Some(last) = JOBS.lock().unwrap_or_else(|e| e.into_inner()).last_mut() {
        last.end = SEQ.fetch_add(1, Ordering::SeqCst);
        (last.load_rt0, last.rgba8) = (c.load_rt0, c.rgba8);
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
    // SAFETY: `s` is the back end's live state.
    PACKED.lock().unwrap_or_else(|e| e.into_inner()).push(unsafe { *s });
    // SAFETY: the library's call, passed on; `s` is its live state.
    unsafe { f(out, s) }
}

/// Every texture shader state the back end handed the packer since the
/// rig's last reset, in order.
static PACKED: Mutex<Vec<V3dTexState>> = Mutex::new(Vec::new());

fn packed() -> Vec<V3dTexState> {
    PACKED.lock().unwrap_or_else(|e| e.into_inner()).clone()
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
    for j in JOBS.lock().unwrap_or_else(|e| e.into_inner()).iter_mut() {
        if j.end != 0 {
            j.submitted = true;
        }
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
    JOBS.lock().unwrap_or_else(|e| e.into_inner()).clear();
    PACKED.lock().unwrap_or_else(|e| e.into_inner()).clear();
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
const FS_2D: &str = "precision mediump float;\nvarying vec2 t;\nuniform sampler2D s;\nvoid main() { gl_FragColor = texture2D(s, t); }\n";
const FS_CUBE: &str =
    "precision mediump float;\nvarying vec2 t;\nuniform samplerCube s;\nvoid main() { gl_FragColor = textureCube(s, vec3(t, 1.0)); }\n";

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

/// Make `fs` (with `VS`) the current program; sampler `s` reads unit 0.
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
        glUniform1i(glGetUniformLocation(p, c"s".as_ptr()), 0);
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

fn param(target: GLenum, name: GLenum, v: GLenum) {
    // SAFETY: a plain GL call.
    unsafe { glTexParameteri(target, name, v as GLint) };
}

/// Upload `levels` levels of a `w` x `h` RGBA image to `face`, each level
/// half the size of the one before (at least 1 texel).
fn fill(face: GLenum, w: GLsizei, h: GLsizei, levels: u32) {
    for l in 0..levels {
        let (lw, lh) = ((w >> l).max(1), (h >> l).max(1));
        let px = vec![0x80u8; (lw * lh * 4) as usize];
        // SAFETY: `px` holds the level's texels.
        unsafe {
            let data = px.as_ptr() as *const c_void;
            glTexImage2D(face, l as GLint, GL_RGBA as GLint, lw, lh, 0, GL_RGBA, GL_UNSIGNED_BYTE, data);
        }
    }
}

/// Draw with the bound texture sampled and return the one texture state the
/// back end packed for it. GL_NO_ERROR and no refused draw are part of it.
fn sampled_state(what: &str) -> V3dTexState {
    let all = sampled_states(what);
    assert_eq!(all.len(), 1, "{what}: one texture state packed, got {all:?}; {:?}", log_lines());
    all[0]
}

/// As [`sampled_state`], every state packed for the draw (an incomplete
/// texture's draw packs none: the GL core answers (0,0,0,1) for it).
fn sampled_states(what: &str) -> Vec<V3dTexState> {
    let base = v3d_refused_draws();
    draw_triangle();
    // SAFETY: a plain GL call.
    unsafe { glFinish() };
    let (e, n) = (err(), v3d_refused_draws().wrapping_sub(base));
    assert_eq!((e, n), (GL_NO_ERROR, 0), "{what}: error, refused draws; {:?}", log_lines());
    packed()
}

fn new_texture(target: GLenum) -> GLuint {
    let mut t = 0;
    // SAFETY: plain GL calls with room for one name.
    unsafe {
        glActiveTexture(GL_TEXTURE0);
        glGenTextures(1, &mut t);
        glBindTexture(target, t);
    }
    t
}

fn delete_texture(t: GLuint) {
    // SAFETY: a name this test made.
    unsafe {
        glBindTexture(GL_TEXTURE_2D, 0);
        glBindTexture(GL_TEXTURE_CUBE_MAP, 0);
        glDeleteTextures(1, &t);
    }
}

/// A texture of `w` x `h` with `levels` levels uploaded and the given
/// filters, CLAMP_TO_EDGE, sampled by one draw; its packed state.
fn case_2d(what: &str, w: GLsizei, h: GLsizei, levels: u32, min: GLenum, mag: GLenum) -> V3dTexState {
    let all = case_2d_states(what, w, h, levels, min, mag);
    assert_eq!(all.len(), 1, "{what}: one texture state packed, got {all:?}; {:?}", log_lines());
    all[0]
}

fn case_2d_states(what: &str, w: GLsizei, h: GLsizei, levels: u32, min: GLenum, mag: GLenum) -> Vec<V3dTexState> {
    let _g = rig();
    program(FS_2D);
    let t = new_texture(GL_TEXTURE_2D);
    fill(GL_TEXTURE_2D, w, h, levels);
    param(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, min);
    param(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, mag);
    param(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
    param(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
    assert_eq!(err(), GL_NO_ERROR, "{what}: setup");
    let all = sampled_states(what);
    delete_texture(t);
    all
}

/// A cube map of 16x16 faces with `levels` levels and the given min filter.
fn case_cube(what: &str, levels: u32, min: GLenum) -> V3dTexState {
    let _g = rig();
    program(FS_CUBE);
    let t = new_texture(GL_TEXTURE_CUBE_MAP);
    for face in GL_TEXTURE_CUBE_MAP_POSITIVE_X..GL_TEXTURE_CUBE_MAP_POSITIVE_X + 6 {
        fill(face, 16, 16, levels);
    }
    param(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_MIN_FILTER, min);
    param(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_MAG_FILTER, GL_LINEAR);
    param(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
    param(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
    assert_eq!(err(), GL_NO_ERROR, "{what}: setup");
    let s = sampled_state(what);
    delete_texture(t);
    s
}

/* ---- the cases ---- */

#[test]
fn full_chain_mipmap_filters() {
    for min in [GL_LINEAR_MIPMAP_LINEAR, GL_NEAREST_MIPMAP_NEAREST, GL_LINEAR_MIPMAP_NEAREST, GL_NEAREST_MIPMAP_LINEAR]
    {
        let s = case_2d("full chain", 64, 64, 7, min, GL_LINEAR);
        assert_eq!((s.width, s.height), (64, 64));
        assert_eq!((s.base_level, s.max_level), (0, 6), "min filter {min:#x}: {s:?}");
    }
}

#[test]
fn full_chain_without_mipmap_filter() {
    /* the chain is stored, the draw samples level 0 only: levels_sampled is 1 */
    let s = case_2d("full chain, linear", 64, 64, 7, GL_LINEAR, GL_LINEAR);
    assert_eq!((s.width, s.height), (64, 64));
    assert_eq!((s.base_level, s.max_level), (0, 0), "{s:?}");
}

#[test]
fn level_zero_only() {
    for (min, mag) in [(GL_NEAREST, GL_NEAREST), (GL_LINEAR, GL_LINEAR)] {
        let s = case_2d("level 0 only", 64, 64, 1, min, mag);
        assert_eq!((s.width, s.height), (64, 64));
        assert_eq!((s.base_level, s.max_level), (0, 0), "{s:?}");
    }
}

#[test]
fn incomplete_chain() {
    /* levels 0 to 2 of a 64x64 under a mipmap filter: incomplete in ES 2.0
     * (it samples as (0, 0, 0, 1), and at 337a9453^ the draw packs no
     * texture state for it); were a state packed, it keeps base level 0 and
     * does not claim the missing levels */
    for s in case_2d_states("incomplete chain", 64, 64, 3, GL_LINEAR_MIPMAP_LINEAR, GL_LINEAR) {
        assert_eq!(s.base_level, 0, "{s:?}");
        assert!(s.max_level <= 2, "{s:?}");
    }
}

#[test]
fn cube_map() {
    let s = case_cube("cube map", 5, GL_LINEAR_MIPMAP_LINEAR);
    assert_eq!((s.width, s.height, s.depth), (16, 16, 6));
    assert_eq!((s.base_level, s.max_level), (0, 4), "{s:?}");
}

#[test]
fn cube_map_level_zero_only() {
    let s = case_cube("cube map, level 0", 1, GL_NEAREST);
    assert_eq!((s.width, s.height, s.depth), (16, 16, 6));
    assert_eq!((s.base_level, s.max_level), (0, 0), "{s:?}");
}

#[test]
fn npot_clamp_nearest_and_linear() {
    for (min, mag) in
        [(GL_NEAREST, GL_NEAREST), (GL_LINEAR, GL_LINEAR), (GL_NEAREST, GL_LINEAR), (GL_LINEAR, GL_NEAREST)]
    {
        let s = case_2d("npot 37x19", 37, 19, 1, min, mag);
        assert_eq!((s.width, s.height), (37, 19), "{s:?}");
        assert_eq!((s.base_level, s.max_level), (0, 0), "{s:?}");
    }
}
