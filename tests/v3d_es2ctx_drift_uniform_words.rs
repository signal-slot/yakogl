// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The ES 2.0 context against three V3D commits that came after the Pi 4's
//! last whole dEQP-GLES2 tree (46f93767), whose stage-1 promise is 0 Fail
//! and only the 34 NotSupported of tools/cts/es2-accepted.txt:
//!
//! - 298fe11c: the job builder (driver/src/v3d/v3d_job.c `write_uniforms`)
//!   writes the uniform words 46 and 47 (`QUNIFORM_TEXTURE_DEPTH`,
//!   `_ARRAY_SIZE`) from `struct v3d_job_tmu`'s new `depth` and
//!   `array_size`, which `texture_units` (src/v3d/backend.rs) fills; the
//!   record grew from 20 to 28 bytes.
//! - 080240bc: a unit that only textureSize reads (its size words, no TMU
//!   P0/P1 word) takes a 32-bit texture without the return-width refusal.
//! - 35ea14d6: a shadow look-up of a fixed-point depth texture clamps its
//!   reference to [0, 1] (`LodKey::fixed_depth`).
//!
//! What an ES 2.0 program reaches of them, read from the code:
//!
//! - Words 46 and 47 are ABSENT from every ES 2.0 stream. The compiler emits
//!   a size word only through `SOp::TexSizeWord` (src/shaderc/online/v3d/
//!   prepare.rs): textureSize (`Op::TexSize`, lane 2 of a 3D sampler is 46,
//!   of a 2D array 47), a textureGrad's level (`grad_lod`, lane 2 of a 3D
//!   sampler is 46) and the non-seamless cube level (`cube_lod`, lane 0 =
//!   44 only). GLSL ES 1.00 has no textureSize, no sampler3D (no
//!   GL_OES_texture_3D) and no 2D array, and the ES 2.0 context lists no
//!   GL_EXT_shader_texture_lod (no texture*GradEXT). So the only size word an
//!   ES 2.0 program carries is 44, the face size of a LINEAR-filtered cube
//!   (`LodKey::cube_noseam`, an ES 2.0-only variant), and 46/47 never reach
//!   `write_uniforms`: the new `depth` / `array_size` fields are written but
//!   not read in an ES 2.0 draw.
//! - The size word 44 (and 45, should it appear) has Mesa's value:
//!   v3d_uniforms.c `get_texture_size`, QUNIFORM_TEXTURE_WIDTH =
//!   `u_minify(width0, first_level)`, _HEIGHT = `u_minify(height0,
//!   first_level)`; ES 2.0 has no GL_TEXTURE_BASE_LEVEL, so first_level is 0
//!   and the value is the base level's size, a cube's one face; an
//!   incomplete texture is Mesa's 1x1 fallback. The stand-in job builder
//!   here resolves each word the way `write_uniforms` does (`find_tmu` by
//!   unit = the word's data) and compares with that.
//! - Were 46/47 read, Mesa's values for an ES 2.0 texture are `u_minify(depth0
//!   = 1, 0)` = 1 for both a 2D texture and a cube, and `array_size` = 1 for a
//!   2D texture (6 for a gallium cube, PIPE_TEXTURE_CUBE's array_size, while
//!   `texture_units` gives a cube 1: unread, since no stream asks for word
//!   47 of a cube in any GLSL version; textureSize of a samplerCube is two
//!   lanes). The 2D records are pinned at depth 1 / 1 layer, the cube
//!   records at depth 1.
//! - 080240bc's branch needs a unit no P0/P1 word names; every ES 2.0 sampler
//!   read is a TMU look-up, so every unit a size word names is also looked
//!   up (pinned on every draw here), and a depth texture
//!   (GL_OES_depth_texture, ES 2.0's one 32-bit class) read by texture2D
//!   keeps its 32-bit look-up and draws.
//! - 35ea14d6's key bit is set only for a shadow look-up; GLSL ES 1.00 has no
//!   shadow sampler (sampler2DShadow is a reserved word, and the context
//!   lists no GL_EXT_shadow_samplers). Pinned: those shaders do not compile,
//!   and an ES 1.00 program compiled with every `fixed_depth` bit set is word
//!   for word the program without them.
//!
//! The draws go through the real GL core (a `report-es2` build, GLSL ES 1.00
//! shaders) and the real V3D back end; the job builder is stood in for (as in
//! tests/v3d_es2ctx_refusals.rs) and records each draw's uniform streams and
//! sampler bindings; the texture layout and tiling code is the real
//! driver/src/v3d/v3d_tex.c, built with cc by this test. Formats: every ES
//! 2.0 colour format (RGBA, RGB, LUMINANCE, LUMINANCE_ALPHA, ALPHA of
//! UNSIGNED_BYTE, RGB 565, RGBA 4444 and 5551), and those of the listed
//! extensions (BGRA_EXT, RED_EXT, RG_EXT, HALF_FLOAT_OES; the context lists
//! no GL_OES_texture_float, so no FLOAT texture), on 2D and cube textures,
//! NEAREST and LINEAR, and the depth textures of GL_OES_depth_texture (2D).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table,report-es2 --test v3d_es2ctx_drift_uniform_words
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
use yakogl::gl::state::*;
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps, OsalV3dCpuOf};
use yakogl::v3d::backend::yakogl_backend_v3d;
use yakogl::v3d::ffi::*;
use yakogl::v3d::kick::V3dKick;

/* ---- the job builder: records each draw's streams, draws nothing ---- */

static LOG: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn log_lines() -> Vec<String> {
    LOG.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// One draw as the job builder was handed it: the uniform streams
/// `(contents, data)` of the fragment, vertex and coordinate shaders, and
/// the sampler bindings of the fragment stream and of the vertex/coordinate
/// streams.
#[derive(Clone, Debug, Default)]
struct Draw {
    fs: Vec<(u32, u32)>,
    vs: Vec<(u32, u32)>,
    cs: Vec<(u32, u32)>,
    tmu: Vec<V3dJobTmu>,
    vs_tmu: Vec<V3dJobTmu>,
}

static DRAWS: Mutex<Vec<Draw>> = Mutex::new(Vec::new());

fn take_draws() -> Vec<Draw> {
    std::mem::take(&mut *DRAWS.lock().unwrap_or_else(|e| e.into_inner()))
}

fn records(p: *const V3dJobTmu, n: u32) -> Vec<V3dJobTmu> {
    if p.is_null() || n == 0 {
        Vec::new()
    } else {
        // SAFETY: the draw's `n` records, live for the call.
        unsafe { std::slice::from_raw_parts(p, n as usize) }.to_vec()
    }
}

fn words(p: *const V3dUniform, n: u32) -> Vec<(u32, u32)> {
    if p.is_null() || n == 0 {
        Vec::new()
    } else {
        // SAFETY: a program's stream of `n` words, live for the call.
        unsafe { std::slice::from_raw_parts(p, n as usize) }.iter().map(|w| (w.contents, w.data)).collect()
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
    // SAFETY: the programs, when given, live for the call.
    let (fs, vs, cs) = unsafe {
        (
            d.fs.as_ref().map(|p| words(p.uniforms, p.num_uniforms)).unwrap_or_default(),
            d.vs.as_ref().map(|p| words(p.uniforms, p.num_uniforms)).unwrap_or_default(),
            d.cs.as_ref().map(|p| words(p.uniforms, p.num_uniforms)).unwrap_or_default(),
        )
    };
    let draw = Draw { fs, vs, cs, tmu: records(d.tmu, d.num_tmu), vs_tmu: records(d.vs_tmu, d.num_vs_tmu) };
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
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("v3d_es2ctx_drift_uniform_words");
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
    /* the tiled-copy budget of the Pi 4 images other than T5 (16 MiB) */
    if what == OSAL_QUERY_V3D_TILED_BUDGET && out_sz == 4 {
        // SAFETY: a u32 out-parameter.
        unsafe { *(out as *mut u32) = 16 << 20 };
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
        glViewport(0, 0, 32, 32);
        glActiveTexture(GL_TEXTURE0);
    }
    LOG.lock().unwrap_or_else(|e| e.into_inner()).clear();
    take_draws();
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

/// The GL texture unit the sampler reads: not 0, so that the streams' index
/// (the program's sampler binding, `BINDING`) and the GL unit differ.
const UNIT: u32 = 3;

/// The program's one sampler binding, which the TMU P0/P1 words (data >> 24)
/// and the size words (data) name, and which `struct v3d_job_tmu`'s `unit`
/// is (`find_tmu` in v3d_job.c): Mesa's `texstate->textures[data]`, the
/// shader's sampler index, which Mesa's state tracker maps to the GL unit
/// the sampler uniform holds.
const BINDING: u32 = 0;

const VS: &str = "attribute vec4 p;\nvarying vec2 t;\nvoid main() { gl_Position = p; t = p.xy * 0.5 + 0.5; }\n";
const FS_2D: &str = "precision mediump float;\nvarying vec2 t;\nuniform sampler2D s;\n\
                     void main() { gl_FragColor = texture2D(s, t); }\n";
const FS_CUBE: &str = "precision mediump float;\nvarying vec2 t;\nuniform samplerCube s;\n\
                       void main() { gl_FragColor = textureCube(s, vec3(t * 2.0 - 1.0, 0.5)); }\n";
const VS_TEX_2D: &str = "attribute vec4 p;\nuniform sampler2D s;\nvarying vec4 c;\n\
                         void main() { gl_Position = p; c = texture2DLod(s, p.xy * 0.5 + 0.5, 0.0); }\n";
const VS_TEX_CUBE: &str = "attribute vec4 p;\nuniform samplerCube s;\nvarying vec4 c;\n\
                           void main() { gl_Position = p; c = textureCubeLod(s, vec3(p.xy, 0.5), 0.0); }\n";
const FS_PASS: &str = "precision mediump float;\nvarying vec4 c;\nvoid main() { gl_FragColor = c; }\n";

/// The error flag, drained: the first error since the last call.
fn err() -> GLenum {
    // SAFETY: plain GL calls.
    unsafe {
        let e = glGetError();
        while glGetError() != GL_NO_ERROR {}
        e
    }
}

/// Does `text` compile as a shader of `ty`?
fn compiles(ty: GLenum, text: &str) -> bool {
    let text = CString::new(text).unwrap();
    // SAFETY: plain GL calls; one NUL-terminated source.
    unsafe {
        let s = glCreateShader(ty);
        let p: [*const c_char; 1] = [text.as_ptr()];
        glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
        glCompileShader(s);
        let mut ok: GLint = 0;
        glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
        glDeleteShader(s);
        ok == 1
    }
}

fn shader(ty: GLenum, text: &str) -> GLuint {
    let src = CString::new(text).unwrap();
    // SAFETY: plain GL calls; one NUL-terminated source.
    unsafe {
        let s = glCreateShader(ty);
        let p: [*const c_char; 1] = [src.as_ptr()];
        glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
        glCompileShader(s);
        let mut ok: GLint = 0;
        glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
        assert_eq!(ok, 1, "{text:?} did not compile");
        s
    }
}

/// Make `vs` + `fs` the current program; its sampler `s` reads `UNIT`.
fn program(vs: &str, fs: &str) -> GLuint {
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
        assert_eq!(ok, 1, "{vs:?} + {fs:?} links");
        glUseProgram(p);
        let s = glGetUniformLocation(p, c"s".as_ptr());
        assert!(s >= 0, "the sampler is active");
        glUniform1i(s, UNIT as GLint);
        glDeleteShader(v);
        glDeleteShader(f);
        p
    }
}

fn drop_program(p: GLuint) {
    // SAFETY: plain GL calls.
    unsafe {
        glUseProgram(0);
        glDeleteProgram(p);
    }
}

static TRI: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 3.0, -1.0, 0.0, 1.0, -1.0, 3.0, 0.0, 1.0];

/// One triangle over the surface, accepted (no GL error, no draw the back
/// end refused); the draws the job builder was handed.
fn draw(what: &str) -> Vec<Draw> {
    take_draws();
    let base = v3d_refused_draws();
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
    let d = take_draws();
    assert_eq!(d.len(), 1, "{what}: one draw reaches the job builder");
    d
}

const FACES: [GLenum; 6] = [
    GL_TEXTURE_CUBE_MAP_POSITIVE_X,
    GL_TEXTURE_CUBE_MAP_NEGATIVE_X,
    GL_TEXTURE_CUBE_MAP_POSITIVE_Y,
    GL_TEXTURE_CUBE_MAP_NEGATIVE_Y,
    GL_TEXTURE_CUBE_MAP_POSITIVE_Z,
    GL_TEXTURE_CUBE_MAP_NEGATIVE_Z,
];

/// A texture of `target` bound to `UNIT`: `w` x `h` of `format`/`type_`
/// (every cube face, or only `faces` of them), the filters given.
fn texture(
    target: GLenum,
    format: GLenum,
    type_: GLenum,
    (w, h): (GLsizei, GLsizei),
    faces: usize,
    min: GLenum,
    mag: GLenum,
) -> GLuint {
    let mut t = 0;
    let texels = vec![0x3cu8; (w * h) as usize * 16];
    // SAFETY: plain GL calls; the buffer holds 16 bytes a texel.
    unsafe {
        glActiveTexture(GL_TEXTURE0 + UNIT);
        glGenTextures(1, &mut t);
        glBindTexture(target, t);
        let images: &[GLenum] = if target == GL_TEXTURE_CUBE_MAP { &FACES[..faces] } else { &[GL_TEXTURE_2D] };
        for f in images {
            glTexImage2D(*f, 0, format as GLint, w, h, 0, format, type_, texels.as_ptr() as *const c_void);
        }
        glTexParameteri(target, GL_TEXTURE_MIN_FILTER, min as GLint);
        glTexParameteri(target, GL_TEXTURE_MAG_FILTER, mag as GLint);
        glTexParameteri(target, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE as GLint);
        glTexParameteri(target, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE as GLint);
        glActiveTexture(GL_TEXTURE0);
    }
    assert_eq!(err(), GL_NO_ERROR, "{format:#x}/{type_:#x} {w}x{h} is an ES 2.0 texture");
    t
}

fn drop_texture(t: GLuint) {
    // SAFETY: plain GL calls.
    unsafe {
        glActiveTexture(GL_TEXTURE0 + UNIT);
        glBindTexture(GL_TEXTURE_2D, 0);
        glBindTexture(GL_TEXTURE_CUBE_MAP, 0);
        glActiveTexture(GL_TEXTURE0);
        glDeleteTextures(1, &t);
    }
}

fn extension_listed(name: &str) -> bool {
    // SAFETY: GL_EXTENSIONS is a NUL-terminated string.
    let s = unsafe { CStr::from_ptr(glGetString(GL_EXTENSIONS) as *const c_char) };
    s.to_string_lossy().split(' ').any(|e| e == name)
}

/// The texture size words (`QUNIFORM_TEXTURE_WIDTH` 44 .. `_ARRAY_SIZE` 47).
const WIDTH: u32 = 44;
const HEIGHT: u32 = 45;
const DEPTH: u32 = 46;
const ARRAY_SIZE: u32 = 47;

/// The texture a test bound to `UNIT`, as Mesa's v3d_uniforms.c
/// `get_texture_size` reads it with first_level 0: width0, height0 (a cube's
/// face), depth0 = 1.
#[derive(Clone, Copy, Debug)]
struct Bound {
    width: u32,
    height: u32,
    cube: bool,
}

impl Bound {
    /// Mesa's value of size word `contents` (44 / 45 / 46 / 47); a gallium
    /// cube's array_size is 6, a 2D texture's 1.
    fn mesa(self, contents: u32) -> u32 {
        match contents {
            WIDTH => self.width,
            HEIGHT => self.height,
            DEPTH => 1,
            ARRAY_SIZE if self.cube => 6,
            ARRAY_SIZE => 1,
            c => panic!("{c} is not a size word"),
        }
    }
}

/// One stage's stream against its sampler bindings: no word 46/47 (ES 2.0
/// never asks for them), every size word resolves, as `write_uniforms`
/// resolves it (the record whose `unit` is the word's data), to Mesa's value
/// for the texture bound there, every binding a size word names is also
/// looked up (a P0/P1 word names it, data >> 24: 080240bc's size-only branch
/// is not taken), and the record of `BINDING` carries depth 1 (and 1 layer
/// for a 2D texture). The number of size words.
fn check_stream(what: &str, stream: &[(u32, u32)], tmus: &[V3dJobTmu], bound: Bound) -> usize {
    for &(c, d) in stream {
        assert!(c != DEPTH && c != ARRAY_SIZE, "{what}: word {c} (data {d}) in an ES 2.0 stream: {stream:?}");
    }
    let looked_up: Vec<u32> = stream
        .iter()
        .filter(|&&(c, _)| c == V3D_QUNIFORM_TMU_CONFIG_P0 || c == V3D_QUNIFORM_TMU_CONFIG_P1)
        .map(|&(_, d)| d >> 24)
        .collect();
    let mut sizes = 0;
    for &(c, d) in stream.iter().filter(|&&(c, _)| c == WIDTH || c == HEIGHT) {
        sizes += 1;
        assert_eq!(d, BINDING, "{what}: size word {c} names the sampler binding");
        assert!(looked_up.contains(&d), "{what}: binding {d} is read by a TMU look-up as well: {stream:?}");
        let t = tmus.iter().find(|t| t.unit == d);
        let t = t.unwrap_or_else(|| panic!("{what}: size word {c} of binding {d} has a record: {tmus:?}"));
        let v = if c == WIDTH { t.width } else { t.height };
        assert_eq!(v, bound.mesa(c), "{what}: size word {c} as v3d_uniforms.c writes it");
    }
    if !looked_up.is_empty() {
        let t = tmus.iter().find(|t| t.unit == BINDING);
        let t = t.unwrap_or_else(|| panic!("{what}: binding {BINDING} has a record: {tmus:?}"));
        assert_eq!(t.depth, bound.mesa(DEPTH), "{what}: the binding's depth (word 46 would read it)");
        if !bound.cube {
            assert_eq!(t.array_size, bound.mesa(ARRAY_SIZE), "{what}: the binding's layers (word 47 would read it)");
        }
    }
    sizes
}

/// The fragment-stage draws: the FS stream checked against `tmu`, and the
/// vertex/coordinate streams carry no texture word at all.
fn check_fs_draw(what: &str, d: &Draw, bound: Bound) -> usize {
    assert!(
        d.fs.iter().any(|&(c, x)| c == V3D_QUNIFORM_TMU_CONFIG_P0 && x >> 24 == BINDING),
        "{what}: the FS looks up binding {BINDING}: {:?}",
        d.fs
    );
    for s in [&d.vs, &d.cs] {
        assert!(!s.iter().any(|&(c, _)| (WIDTH..=ARRAY_SIZE).contains(&c)), "{what}: VS/CS size words: {s:?}");
    }
    check_stream(what, &d.fs, &d.tmu, bound)
}

/// The ES 2.0 colour textures: every core format/type pair, and those of
/// the extensions the context lists (asserted by `the_context_is_es2`).
const COLOUR: [(GLenum, GLenum, &str); 12] = [
    (GL_RGBA, GL_UNSIGNED_BYTE, "RGBA/UNSIGNED_BYTE"),
    (GL_RGB, GL_UNSIGNED_BYTE, "RGB/UNSIGNED_BYTE"),
    (GL_RGB, GL_UNSIGNED_SHORT_5_6_5, "RGB565"),
    (GL_RGBA, GL_UNSIGNED_SHORT_4_4_4_4, "RGBA4"),
    (GL_RGBA, GL_UNSIGNED_SHORT_5_5_5_1, "RGB5_A1"),
    (GL_LUMINANCE, GL_UNSIGNED_BYTE, "LUMINANCE"),
    (GL_ALPHA, GL_UNSIGNED_BYTE, "ALPHA"),
    (GL_LUMINANCE_ALPHA, GL_UNSIGNED_BYTE, "LUMINANCE_ALPHA"),
    (GL_BGRA_EXT, GL_UNSIGNED_BYTE, "BGRA_EXT"),
    (GL_RED_EXT, GL_UNSIGNED_BYTE, "RED_EXT"),
    (GL_RG_EXT, GL_UNSIGNED_BYTE, "RG_EXT"),
    (GL_RGBA, GL_HALF_FLOAT_OES, "RGBA/HALF_FLOAT_OES"),
];

/* ---- the cases ---- */

/// Sanity: an ES 2.0 context, which lists the extensions whose formats are
/// drawn here and none of those that would put a size or shadow look-up
/// into GLSL ES 1.00 (GL_OES_texture_3D, GL_EXT_shader_texture_lod,
/// GL_EXT_shadow_samplers) or a FLOAT texture (GL_OES_texture_float). Should
/// one of the latter become listed, this file must grow its cases (words
/// 46 for texture3D, 44/45 for texture*GradEXT, the shadow clamp, FLOAT
/// formats) before the assertion changes.
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
    for name in ["GL_OES_texture_half_float", "GL_EXT_texture_format_BGRA8888", "GL_EXT_texture_rg"] {
        assert!(extension_listed(name), "{name} is listed: its formats are drawn here");
    }
    for name in ["GL_OES_texture_3D", "GL_EXT_shader_texture_lod", "GL_EXT_shadow_samplers", "GL_OES_texture_float"] {
        assert!(!extension_listed(name), "{name} is not listed (else this file's cases must grow)");
    }
}

/// GLSL ES 1.00 reaches neither textureSize (GLSL ES 3.00 8.8), nor a
/// shadow sampler (sampler2DShadow and sampler3D are reserved words, GLSL
/// ES 1.00 3.7; their extensions are not listed), nor a gradient look-up:
/// the paths 298fe11c's word 46/47, 080240bc's size-only unit and
/// 35ea14d6's clamp stand on.
#[test]
fn glsl_es_100_has_no_size_shadow_or_gradient_look_up() {
    let _g = rig();
    let fs = |decl: &str, body: &str| {
        format!("{decl}precision mediump float;\nvarying vec2 t;\n{body}\nvoid main() {{ gl_FragColor = f(); }}\n")
    };
    let cases = [
        ("textureSize", fs("", "uniform sampler2D s;\nvec4 f() { return vec4(vec2(textureSize(s, 0)), 0.0, 1.0); }")),
        ("sampler2DShadow", fs("", "uniform sampler2DShadow s;\nvec4 f() { return vec4(1.0); }")),
        (
            "shadow2DEXT",
            fs(
                "#extension GL_EXT_shadow_samplers : require\n",
                "uniform sampler2DShadow s;\nvec4 f() { return vec4(shadow2DEXT(s, vec3(t, 0.5))); }",
            ),
        ),
        ("sampler3D", fs("", "uniform sampler3D s;\nvec4 f() { return texture3D(s, vec3(t, 0.5)); }")),
        (
            "texture2DGradEXT",
            fs(
                "#extension GL_EXT_shader_texture_lod : require\n",
                "uniform sampler2D s;\nvec4 f() { return texture2DGradEXT(s, t, vec2(0.1), vec2(0.1)); }",
            ),
        ),
    ];
    for (what, text) in &cases {
        assert!(!compiles(GL_FRAGMENT_SHADER, text), "{what} compiles in GLSL ES 1.00: {text:?}");
        assert_eq!(err(), GL_NO_ERROR, "{what}: a failed compile is no GL error");
    }
    assert!(compiles(GL_FRAGMENT_SHADER, FS_2D), "the control compiles");
}

/// 2D textures of every ES 2.0 colour format, NEAREST and LINEAR: the draw
/// is accepted, the FS stream carries no size word (Mesa emits none for a
/// texture2D look-up), and the binding is the 8x4 texture at depth 1, one
/// layer.
#[test]
fn two_d_textures_draw_with_no_size_word() {
    let _g = rig();
    let p = program(VS, FS_2D);
    for (format, type_, name) in COLOUR {
        for filter in [GL_NEAREST, GL_LINEAR] {
            let t = texture(GL_TEXTURE_2D, format, type_, (8, 4), 1, filter, filter);
            let what = format!("2D {name} filter {filter:#x}");
            let d = draw(&what);
            let n = check_fs_draw(&what, &d[0], Bound { width: 8, height: 4, cube: false });
            assert_eq!(n, 0, "{what}: no size word");
            drop_texture(t);
        }
    }
    drop_program(p);
}

/// Cube textures of every ES 2.0 colour format: NEAREST carries no size
/// word; LINEAR (the ES 2.0 per-face variant, `LodKey::cube_noseam`) carries
/// the face-size word 44, which resolves to Mesa's 16 (QUNIFORM_TEXTURE_WIDTH
/// of a 16x16 cube), and no 46/47.
#[test]
fn cube_textures_draw_and_the_size_word_is_the_face_size() {
    let _g = rig();
    let p = program(VS, FS_CUBE);
    for (format, type_, name) in COLOUR {
        for filter in [GL_NEAREST, GL_LINEAR] {
            let t = texture(GL_TEXTURE_CUBE_MAP, format, type_, (16, 16), 6, filter, filter);
            let what = format!("cube {name} filter {filter:#x}");
            let d = draw(&what);
            let n = check_fs_draw(&what, &d[0], Bound { width: 16, height: 16, cube: true });
            if filter == GL_LINEAR {
                assert!(n > 0, "{what}: the per-face level reads the face size");
            } else {
                assert_eq!(n, 0, "{what}: a NEAREST cube has no size word");
            }
            drop_texture(t);
        }
    }
    drop_program(p);
}

/// A full mip chain (glGenerateMipmap, LINEAR_MIPMAP_LINEAR): the size word
/// is still level 0's (ES 2.0 has no base level: Mesa's first_level 0).
#[test]
fn mipmapped_textures_read_the_base_level_size() {
    let _g = rig();
    for (target, fs, w, h) in [(GL_TEXTURE_2D, FS_2D, 8, 4), (GL_TEXTURE_CUBE_MAP, FS_CUBE, 16, 16)] {
        let p = program(VS, fs);
        let t = texture(target, GL_RGBA, GL_UNSIGNED_BYTE, (w, h), 6, GL_LINEAR_MIPMAP_LINEAR, GL_LINEAR);
        // SAFETY: plain GL calls.
        unsafe {
            glActiveTexture(GL_TEXTURE0 + UNIT);
            glGenerateMipmap(target);
            glActiveTexture(GL_TEXTURE0);
        }
        assert_eq!(err(), GL_NO_ERROR);
        let what = format!("mipmapped {target:#x}");
        let d = draw(&what);
        let cube = target == GL_TEXTURE_CUBE_MAP;
        let n = check_fs_draw(&what, &d[0], Bound { width: w as u32, height: h as u32, cube });
        assert_eq!(n > 0, cube, "{what}: only the LINEAR cube reads its face size");
        drop_texture(t);
        drop_program(p);
    }
}

/// An incomplete cube (one face of six) read LINEAR: the draw is accepted
/// and the face-size word reads 1, Mesa's 1x1 fallback texture.
#[test]
fn an_incomplete_cube_reads_size_one() {
    let _g = rig();
    let p = program(VS, FS_CUBE);
    let t = texture(GL_TEXTURE_CUBE_MAP, GL_RGBA, GL_UNSIGNED_BYTE, (16, 16), 1, GL_LINEAR, GL_LINEAR);
    let d = draw("incomplete cube");
    let n = check_fs_draw("incomplete cube", &d[0], Bound { width: 1, height: 1, cube: true });
    assert!(n > 0, "the incomplete LINEAR cube keeps its per-face level");
    drop_texture(t);
    drop_program(p);
}

/// GL_OES_depth_texture: ES 2.0's one class of 32-bit channels, read by a
/// plain texture2D (not a shadow look-up). The draw is accepted, the unit is
/// looked up (so 080240bc's size-only branch is not what takes it) and
/// carries no size word.
#[test]
fn depth_textures_draw_through_sampler2d() {
    let _g = rig();
    assert!(extension_listed("GL_OES_depth_texture"), "GL_OES_depth_texture is listed");
    let p = program(VS, FS_2D);
    for (format, type_, name) in [
        (GL_DEPTH_COMPONENT, GL_UNSIGNED_SHORT, "DEPTH_COMPONENT/UNSIGNED_SHORT"),
        (GL_DEPTH_COMPONENT, GL_UNSIGNED_INT, "DEPTH_COMPONENT/UNSIGNED_INT"),
        (GL_DEPTH_STENCIL_OES, GL_UNSIGNED_INT_24_8_OES, "DEPTH_STENCIL/UNSIGNED_INT_24_8"),
    ] {
        if format == GL_DEPTH_STENCIL_OES && !extension_listed("GL_OES_packed_depth_stencil") {
            continue;
        }
        let t = texture(GL_TEXTURE_2D, format, type_, (8, 4), 1, GL_NEAREST, GL_NEAREST);
        let what = format!("2D {name}");
        let d = draw(&what);
        assert_eq!(check_fs_draw(&what, &d[0], Bound { width: 8, height: 4, cube: false }), 0, "{what}");
        drop_texture(t);
    }
    drop_program(p);
}

/// Vertex texture fetch (texture2DLod, textureCubeLod), when the context
/// has vertex texture units: accepted, and no stream asks for word 46/47;
/// any size word resolves to Mesa's value.
#[test]
fn vertex_texture_fetch_draws() {
    let _g = rig();
    // SAFETY: a plain GL query.
    let units = unsafe {
        let mut n = 0;
        glGetIntegerv(GL_MAX_VERTEX_TEXTURE_IMAGE_UNITS, &mut n);
        n
    };
    if units <= UNIT as GLint {
        eprintln!("skipped: {units} vertex texture units");
        return;
    }
    for (target, vs, w, h) in [(GL_TEXTURE_2D, VS_TEX_2D, 8, 4), (GL_TEXTURE_CUBE_MAP, VS_TEX_CUBE, 16, 16)] {
        let p = program(vs, FS_PASS);
        for filter in [GL_NEAREST, GL_LINEAR] {
            let t = texture(target, GL_RGBA, GL_UNSIGNED_BYTE, (w, h), 6, filter, filter);
            let what = format!("vertex {target:#x} filter {filter:#x}");
            let d = &draw(&what)[0];
            let bound = Bound { width: w as u32, height: h as u32, cube: target == GL_TEXTURE_CUBE_MAP };
            assert!(
                d.vs.iter().any(|&(c, x)| c == V3D_QUNIFORM_TMU_CONFIG_P0 && x >> 24 == BINDING),
                "{what}: the VS looks up binding {BINDING}: {:?}",
                d.vs
            );
            check_stream(&format!("{what} VS"), &d.vs, &d.vs_tmu, bound);
            check_stream(&format!("{what} CS"), &d.cs, &d.vs_tmu, bound);
            check_stream(&format!("{what} FS"), &d.fs, &d.tmu, bound);
            drop_texture(t);
        }
        drop_program(p);
    }
}

/// 35ea14d6 at the compiler: an ES 1.00 program's texture2D / textureCube
/// look-ups (vertex and fragment) compiled with every `fixed_depth` bit set
/// are word for word, and uniform for uniform, the program without them:
/// the bit acts on a shadow look-up only, which GLSL ES 1.00 has not.
#[test]
fn fixed_depth_leaves_es100_programs_unchanged() {
    use yakogl::shaderc::online::arena::Arena;
    use yakogl::shaderc::online::v3d::chain::{compile_program_keyed, Prim, Target};
    use yakogl::shaderc::online::{lower, sema};
    use yakogl::shaderc::LodKey;
    /* the compile key is process-wide: the rig's lock keeps the GL core's
     * compiles out of the way */
    let _g = rig();
    let compile = |vs: &str, fs: &str, key: LodKey| {
        let mut mem = vec![0u8; 16 << 20];
        let a = Arena::new(&mut mem);
        let vs = a.alloc_str(vs).expect("arena");
        let fs = a.alloc_str(fs).expect("arena");
        let v = lower::compile_to_ir(&a, sema::Stage::Vertex, vs).map_err(|e| e.msg.to_string()).expect("VS");
        let f = lower::compile_to_ir(&a, sema::Stage::Fragment, fs).map_err(|e| e.msg.to_string()).expect("FS");
        let p = compile_program_keyed(&v.module, &f.module, Prim::Triangles, Target { vpm_size: 65536 }, key, key, &a)
            .unwrap_or_else(|e| panic!("the program compiles: {e:?}"));
        let uni = |u: &[yakogl::shaderc::online::v3d::select::Uniform]| {
            u.iter().map(|w| (w.contents, w.data)).collect::<Vec<_>>()
        };
        (
            (p.fs.code.words.to_vec(), uni(p.fs.code.uniforms)),
            (p.vs.code.words.to_vec(), uni(p.vs.code.uniforms)),
            (p.cs.code.words.to_vec(), uni(p.cs.code.uniforms)),
        )
    };
    for (vs, fs, name) in [
        (VS, FS_2D, "FS texture2D"),
        (VS, FS_CUBE, "FS textureCube"),
        (VS_TEX_2D, FS_PASS, "VS texture2DLod"),
        (VS_TEX_CUBE, FS_PASS, "VS textureCubeLod"),
    ] {
        for base in [LodKey::NONE, LodKey { ret32: 0xffff, ..LodKey::NONE }] {
            let plain = compile(vs, fs, base);
            let keyed = compile(vs, fs, LodKey { fixed_depth: 0xffff, ..base });
            assert_eq!(keyed, plain, "{name}, ret32 {:#x}: fixed_depth changes nothing", base.ret32);
        }
    }
}
