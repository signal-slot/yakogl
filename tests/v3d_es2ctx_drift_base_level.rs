// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The ES 2.0 context against 337a9453 (`texture::state_at`: the TMU state's
//! Base Level from a first level, clamped to the last level). ES 2.0 has no
//! GL_TEXTURE_BASE_LEVEL, so in a `report-es2` context every texture shader
//! state record must keep Base Level 0 and the Max Level it had at
//! 337a9453^: the records dEQP-GLES2 texture.mipmap.*, texture.size.*,
//! the incomplete-texture cases and the cube maps sample. The Pi 4 (V3D 4.2)
//! whole dEQP-GLES2 tree (46f93767) predates the commit. This file draws
//! through the real GL core (a `report-es2` build, GLSL ES 1.00 shaders) and
//! the real V3D back end, and reads the two fields from the 24-byte record
//! in the fake GPU memory (byte 15: Base Level bits 7:4, Max Level 3:0), as
//! tests/es3_sample_base_level_v3d.rs reads them.
//!
//! The pinned values, as the code computed them at 337a9453^
//! (src/v3d/texture.rs and src/v3d/backend.rs there):
//!
//! - `texture::state` builds every record with `base_level: 0` and
//!   `max_level: layout.levels - 1`; `backend::tex_state_for` then sets
//!   `max_level` to `layout.levels - 1` of the copy it tiles (or the render
//!   image's levels - 1). Nothing else writes Base Level.
//! - `texture::layout` lays out `levels_sampled(b)` levels: the binding's
//!   `mip_levels` when `mip_filter != 0`, else 1. The GL core
//!   (`gl::draw::sampling_fields`) sets `mip_filter` only for a `*_MIPMAP_*`
//!   min filter on a texture with more than one level, and `mip_levels` to
//!   the levels the application filled.
//! - An incomplete texture (`GlesTextureBind::incomplete`) samples the 1x1
//!   stand-in (`backend::black_state`, `black_binding`: `mip_levels` 1), a
//!   record with Max Level 0.
//!
//! So: a full 8x8 chain (4 levels) sampled with a mipmap filter is (0, 3),
//! sampled with LINEAR (0, 0); a texture with only level 0 is (0, 0); an
//! incomplete chain is the stand-in's (0, 0); a full 8x8 cube chain with a
//! mipmap filter is (0, 3), with NEAREST (0, 0); an NPOT texture (ES 2.0
//! 3.8.2: CLAMP_TO_EDGE, no mipmap filter) is (0, 0) with NEAREST and with
//! LINEAR. A GL_TEXTURE_BASE_LEVEL call is an INVALID_ENUM in ES 2.0 and
//! leaves the record as it was.
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
struct Draw {
    tmu: Vec<V3dJobTmu>,
}
static DRAWS: Mutex<Vec<Draw>> = Mutex::new(Vec::new());

fn log_lines() -> Vec<String> {
    LOG.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

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
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("v3d_es2ctx_drift_base_level");
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

/* ---- helpers ---- */

use yakogl::gl::buffer::glBindBuffer;
use yakogl::gl::draw::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::texture::*;
use yakogl::gl::vertex::{glEnableVertexAttribArray, glVertexAttribPointer};

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
    let c = CString::new(text).unwrap();
    // SAFETY: plain GL calls; one NUL-terminated source.
    unsafe {
        let s = glCreateShader(ty);
        let p: [*const c_char; 1] = [c.as_ptr()];
        glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
        glCompileShader(s);
        let mut ok: GLint = 0;
        glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
        assert_eq!(ok, 1, "{text} did not compile");
        s
    }
}

/// The vertex shader of every case (GLSL ES 1.00): `v` in 0..1 over the
/// triangle's screen part.
const VS: &str = "attribute highp vec4 a_pos;\nvarying highp vec2 v;\n\
                  void main() { v = a_pos.xy * 0.5 + 0.5; gl_Position = a_pos; }\n";

/// The 2D fragment shader: `texture2D` of unit 0.
const FS_2D: &str = "precision mediump float;\nuniform sampler2D s;\nvarying highp vec2 v;\n\
                     void main() { gl_FragColor = texture2D(s, v); }\n";

/// The cube fragment shader: `textureCube` of unit 0 (the +X face's square).
const FS_CUBE: &str = "precision mediump float;\nuniform samplerCube s;\nvarying highp vec2 v;\n\
                       void main() { gl_FragColor = textureCube(s, vec3(1.0, v * 2.0 - 1.0)); }\n";

/// Make `VS` + `fs` the current program, `a_pos` at 0, its sampler `s` on
/// unit 0.
fn program(fs: &str) -> GLuint {
    let (v, f) = (shader(GL_VERTEX_SHADER, VS), shader(GL_FRAGMENT_SHADER, fs));
    // SAFETY: plain GL calls; NUL-terminated names.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glBindAttribLocation(p, 0, c"a_pos".as_ptr());
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "{VS}{fs} did not link");
        glUseProgram(p);
        glUniform1i(glGetUniformLocation(p, c"s".as_ptr()), 0);
        glDeleteShader(v);
        glDeleteShader(f);
        p
    }
}

/// A full-screen triangle into the default framebuffer: the GL error it
/// raises.
fn draw_triangle() -> GLenum {
    static TRI: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 3.0, -1.0, 0.0, 1.0, -1.0, 3.0, 0.0, 1.0];
    // SAFETY: TRI is static and outlives the draw.
    unsafe {
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, TRI.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glDrawArrays(GL_TRIANGLES, 0, 3);
    }
    err()
}

/// The one draw since `rig`, once the job is done.
fn the_draw() -> Draw {
    // SAFETY: plain GL call.
    unsafe { glFinish() };
    let d = draws();
    assert_eq!(d.len(), 1, "one draw reached the job builder: {:?}", log_lines());
    d.into_iter().next().unwrap()
}

/* ---- the textures ---- */

/// How a case's texture is made.
#[derive(Clone, Copy, Debug)]
struct Tex {
    cube: bool,
    width: usize,
    height: usize,
    /// The levels uploaded, from 0 (a full chain of an 8x8 image is 4).
    levels: usize,
    min: GLenum,
    mag: GLenum,
    wrap: GLenum,
}

impl Tex {
    const fn d2(width: usize, height: usize, levels: usize, min: GLenum, mag: GLenum) -> Tex {
        Tex { cube: false, width, height, levels, min, mag, wrap: GL_REPEAT }
    }
    const fn cube(levels: usize, min: GLenum, mag: GLenum) -> Tex {
        Tex { cube: true, width: 8, height: 8, levels, min, mag, wrap: GL_CLAMP_TO_EDGE }
    }
    const fn clamp(self) -> Tex {
        Tex { wrap: GL_CLAMP_TO_EDGE, ..self }
    }
    fn target(self) -> GLenum {
        if self.cube {
            GL_TEXTURE_CUBE_MAP
        } else {
            GL_TEXTURE_2D
        }
    }
}

/// The six faces in GL's order.
const FACES: [GLenum; 6] = [
    GL_TEXTURE_CUBE_MAP_POSITIVE_X,
    GL_TEXTURE_CUBE_MAP_NEGATIVE_X,
    GL_TEXTURE_CUBE_MAP_POSITIVE_Y,
    GL_TEXTURE_CUBE_MAP_NEGATIVE_Y,
    GL_TEXTURE_CUBE_MAP_POSITIVE_Z,
    GL_TEXTURE_CUBE_MAP_NEGATIVE_Z,
];

/// `t`'s texture on unit 0, every level `n` filled with (n*60 + 40, 0, 0,
/// 255) as GL_RGBA / GL_UNSIGNED_BYTE (each cube face the same); then, when
/// `base_level` is given, glTexParameteri(GL_TEXTURE_BASE_LEVEL), whose GL
/// error is returned (GL_NO_ERROR otherwise).
fn texture(t: Tex, base_level: Option<GLint>) -> (GLuint, GLenum) {
    let target = t.target();
    let faces: &[GLenum] = if t.cube { &FACES } else { &[GL_TEXTURE_2D] };
    let mut name = 0;
    let mut e = GL_NO_ERROR;
    // SAFETY: plain GL calls; the texel arrays outlive each upload.
    unsafe {
        glGenTextures(1, &mut name);
        glActiveTexture(GL_TEXTURE0);
        glBindTexture(target, name);
        glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
        for level in 0..t.levels {
            let (w, h) = ((t.width >> level).max(1), (t.height >> level).max(1));
            let texel = [(level * 60 + 40) as u8, 0, 0, 255];
            let texels: Vec<u8> = (0..w * h).flat_map(|_| texel).collect();
            let p = texels.as_ptr() as *const c_void;
            let (l, w, h) = (level as GLint, w as GLsizei, h as GLsizei);
            for &f in faces {
                glTexImage2D(f, l, GL_RGBA as GLint, w, h, 0, GL_RGBA, GL_UNSIGNED_BYTE, p);
            }
        }
        glPixelStorei(GL_UNPACK_ALIGNMENT, 4);
        glTexParameteri(target, GL_TEXTURE_MIN_FILTER, t.min as GLint);
        glTexParameteri(target, GL_TEXTURE_MAG_FILTER, t.mag as GLint);
        glTexParameteri(target, GL_TEXTURE_WRAP_S, t.wrap as GLint);
        glTexParameteri(target, GL_TEXTURE_WRAP_T, t.wrap as GLint);
    }
    assert_eq!(err(), GL_NO_ERROR, "the upload of {t:?}: {:?}", log_lines());
    if let Some(b) = base_level {
        // SAFETY: plain GL call.
        unsafe { glTexParameteri(target, GL_TEXTURE_BASE_LEVEL, b) };
        e = err();
    }
    (name, e)
}

/// Unit 0's record of one draw sampling `t`: `(base_level, max_level)` from
/// byte 15 of its texture shader state record, and the GL error of the
/// GL_TEXTURE_BASE_LEVEL call when `base_level` is given. The draw is
/// accepted and refuses nothing.
fn unit0_levels_with(t: Tex, base_level: Option<GLint>) -> ((u32, u32), GLenum) {
    let (name, e) = texture(t, base_level);
    let p = program(if t.cube { FS_CUBE } else { FS_2D });
    let de = draw_triangle();
    let lines = log_lines();
    assert_eq!(de, GL_NO_ERROR, "{t:?}: the draw is accepted: {lines:?}");
    assert!(!lines.iter().any(|l| l.contains("refused")), "{t:?}: {lines:?}");
    let d = the_draw();
    // SAFETY: plain GL calls.
    unsafe {
        glDeleteProgram(p);
        glDeleteTextures(1, &name);
    }
    DRAWS.lock().unwrap_or_else(|e| e.into_inner()).clear();
    let u = d.tmu.iter().find(|u| u.unit == 0).unwrap_or_else(|| panic!("{t:?}: unit 0 bound: {:?}", d.tmu));
    assert_ne!(u.tex_state, 0, "{t:?}: the unit has its texture record");
    let b15 = with_platform(|p| {
        let gpu = u64::from(u.tex_state);
        let (base_addr, a) = p.allocs.range(..=gpu).next_back().expect("an allocation below the record");
        let off = (gpu - base_addr) as usize;
        assert!(off + V3D_TEX_STATE_BYTES <= a.size, "the record lies in its allocation");
        // SAFETY: inside the allocation (checked above).
        unsafe { *(a.mem.as_ptr() as *const u8).add(off + 15) }
    });
    ((u32::from(b15 >> 4), u32::from(b15 & 15)), e)
}

fn unit0_levels(t: Tex) -> (u32, u32) {
    unit0_levels_with(t, None).0
}

/* ---- the cases ---- */

/// Sanity: this is an ES 2.0 context (GL_MAJOR_VERSION is an ES 3.0 query),
/// the Pi 4 tree's build, `report-es2`.
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

/// A full 8x8 mip chain (levels 0..=3) with a mipmap min filter
/// (texture.mipmap.2d.*): `mip_filter != 0`, `mip_levels` 4, a 4-level
/// layout: (0, 3), for each of the four mipmap filters.
#[test]
fn a_full_2d_chain_with_a_mipmap_filter_is_base_0_max_3() {
    let _g = rig();
    let mipmap =
        [GL_NEAREST_MIPMAP_NEAREST, GL_LINEAR_MIPMAP_NEAREST, GL_NEAREST_MIPMAP_LINEAR, GL_LINEAR_MIPMAP_LINEAR];
    for min in mipmap {
        assert_eq!(unit0_levels(Tex::d2(8, 8, 4, min, GL_NEAREST)), (0, 3), "min filter {min:#x}");
    }
}

/// The same full chain with a min filter without mipmaps: `mip_filter` 0,
/// a 1-level layout: (0, 0).
#[test]
fn a_full_2d_chain_without_a_mipmap_filter_is_base_0_max_0() {
    let _g = rig();
    for min in [GL_NEAREST, GL_LINEAR] {
        assert_eq!(unit0_levels(Tex::d2(8, 8, 4, min, GL_LINEAR)), (0, 0), "min filter {min:#x}");
    }
}

/// A texture with only level 0 (texture.size.2d.*, NEAREST and LINEAR):
/// `levels` 1, no mipmap filter: (0, 0).
#[test]
fn a_2d_texture_with_level_0_only_is_base_0_max_0() {
    let _g = rig();
    for (w, h) in [(64, 64), (16, 128), (1, 1)] {
        for f in [GL_NEAREST, GL_LINEAR] {
            assert_eq!(unit0_levels(Tex::d2(w, h, 1, f, f)), (0, 0), "{w}x{h} filter {f:#x}");
        }
    }
}

/// An incomplete chain (levels 0 and 1 of an 8x8 texture with a mipmap
/// filter, the incomplete-texture cases): the GL core marks the binding
/// incomplete and the back end samples the 1x1 stand-in: (0, 0).
#[test]
fn an_incomplete_chain_samples_the_stand_in_at_base_0_max_0() {
    let _g = rig();
    for min in [GL_NEAREST_MIPMAP_NEAREST, GL_LINEAR_MIPMAP_LINEAR] {
        assert_eq!(unit0_levels(Tex::d2(8, 8, 2, min, GL_NEAREST)), (0, 0), "2D, min filter {min:#x}");
        assert_eq!(unit0_levels(Tex::cube(2, min, GL_NEAREST)), (0, 0), "cube, min filter {min:#x}");
    }
}

/// A full 8x8 cube chain (texture.mipmap.cube.*): (0, 3) with a mipmap
/// filter; level 0 only, or the chain without a mipmap filter
/// (texture.filtering.cube.*): (0, 0).
#[test]
fn a_cube_map_is_base_0_max_its_last_sampled_level() {
    let _g = rig();
    assert_eq!(unit0_levels(Tex::cube(4, GL_NEAREST_MIPMAP_NEAREST, GL_NEAREST)), (0, 3));
    assert_eq!(unit0_levels(Tex::cube(4, GL_LINEAR_MIPMAP_LINEAR, GL_LINEAR)), (0, 3));
    assert_eq!(unit0_levels(Tex::cube(4, GL_LINEAR, GL_LINEAR)), (0, 0));
    assert_eq!(unit0_levels(Tex::cube(1, GL_NEAREST, GL_NEAREST)), (0, 0));
}

/// An NPOT texture with CLAMP_TO_EDGE and NEAREST or LINEAR (the ES 2.0
/// NPOT rules, texture.size.2d.*npot*): one level: (0, 0).
#[test]
fn an_npot_texture_with_clamp_is_base_0_max_0() {
    let _g = rig();
    for (w, h) in [(7, 5), (63, 63), (100, 30)] {
        for f in [GL_NEAREST, GL_LINEAR] {
            assert_eq!(unit0_levels(Tex::d2(w, h, 1, f, f).clamp()), (0, 0), "{w}x{h} filter {f:#x}");
        }
    }
}

/// GL_TEXTURE_BASE_LEVEL is not an ES 2.0 texture parameter: the call is an
/// INVALID_ENUM, and the full chain's record stays (0, 3), not the (1, 3)
/// or (2, 3) `state_at` would give for a first level of 1 or 2.
#[test]
fn texture_base_level_is_refused_and_the_record_keeps_base_0() {
    let _g = rig();
    for b in [1, 2] {
        let (levels, e) = unit0_levels_with(Tex::d2(8, 8, 4, GL_NEAREST_MIPMAP_NEAREST, GL_NEAREST), Some(b));
        assert_eq!(e, GL_INVALID_ENUM, "GL_TEXTURE_BASE_LEVEL {b} in ES 2.0");
        assert_eq!(levels, (0, 3), "GL_TEXTURE_BASE_LEVEL {b}");
    }
}
