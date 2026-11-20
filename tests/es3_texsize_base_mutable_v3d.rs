// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! `textureSize` on MUTABLE textures (`glTexImage2D/3D`) with
//! `GL_TEXTURE_BASE_LEVEL` > 0 on the V3D (plan #382 item basemut, issue
//! #1261).
//!
//! The V3D size uniform words 44-47 (width, height, 3D depth, array layer
//! count; docs/notes/v3d-texture-size-words.md) are taken from level 0 today
//! and ignore the base level, and the GL core's completeness check in
//! src/gl/draw.rs does not look at it either. The item `basefix` makes the
//! words base-relative. ES 3.0 3.8.13: a mutable texture is complete when
//! levels base..=q are defined consistently (levels below the base level are
//! not looked at, and nothing is clamped), where q = min(last level of the
//! full chain, `GL_TEXTURE_MAX_LEVEL`); with a base level that is not
//! defined it is incomplete and samples as (0, 0, 0, 1). `textureSize(s, lod)`
//! answers the size of level base+lod; array layers never minify.
//!
//! Siblings (immutable textures and explicit lod):
//! tests/es3_texsize_base_level_v3d.rs, tests/es3_texsize_base_level_vs_v3d.rs,
//! tests/es3_texsize_base_clamp_v3d.rs, tests/es3_texsize_lod_v3d.rs. This
//! file covers the `glTexImage*` path: 16x16 2D, 16x16x8 3D, 16x16 with 5
//! layers (2D array), in four cases:
//!
//! - (a) a full chain, BASE_LEVEL 1;
//! - (b) only levels 1.. defined, BASE_LEVEL 1 (complete by the spec);
//! - (c) BASE_LEVEL past the last defined level (incomplete);
//! - (d) BASE_LEVEL 1 with MAX_LEVEL 1.
//!
//! Tests whose value changes with the fix carry the suffix
//! `_flip_when_the_base_level_is_applied`; the others are controls that hold
//! after it. Fragment stage.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_texsize_base_mutable_v3d
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
struct Draw {
    tmu: Vec<V3dJobTmu>,
    vs_tmu: Vec<V3dJobTmu>,
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
    let draw = Draw { tmu: records(d.tmu, d.num_tmu), vs_tmu: records(d.vs_tmu, d.num_vs_tmu) };
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
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("es3_texsize_base_mutable_v3d");
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

/// The plain vertex shader of the fragment cases.
const VS: &str = "#version 300 es\nlayout(location = 0) in highp vec4 a_pos;\nout highp vec4 v;\n\
                  void main() { v = a_pos; gl_Position = a_pos; }\n";

/// Make `vs` + `fs` the current program, its sampler `s` on unit 0.
fn program(vs: &str, fs: &str) -> GLuint {
    let (v, f) = (shader(GL_VERTEX_SHADER, vs), shader(GL_FRAGMENT_SHADER, fs));
    // SAFETY: plain GL calls; NUL-terminated names.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "{vs}{fs} did not link");
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

#[derive(Clone, Copy, PartialEq, Debug)]
enum Kind {
    Tex2d,
    Tex3d,
    Array,
}

impl Kind {
    fn target(self) -> GLenum {
        match self {
            Kind::Tex2d => GL_TEXTURE_2D,
            Kind::Tex3d => GL_TEXTURE_3D,
            Kind::Array => GL_TEXTURE_2D_ARRAY,
        }
    }
    fn sampler(self) -> &'static str {
        match self {
            Kind::Tex2d => "sampler2D",
            Kind::Tex3d => "sampler3D",
            Kind::Array => "sampler2DArray",
        }
    }
    /// The `textureSize(s, 0)` expression as a vec3.
    fn query(self) -> &'static str {
        match self {
            Kind::Tex2d => "vec3(vec2(textureSize(s, 0)), 0.0)",
            _ => "vec3(textureSize(s, 0))",
        }
    }
}

/// Which stage asks `textureSize(s, 0)`.
#[derive(Clone, Copy, PartialEq, Debug)]
#[allow(dead_code)] // the vertex stage is the sibling files' case; kept for the shared helper
enum Stage {
    Fragment,
    Vertex,
}

/// Draw `kind`'s `textureSize(s, 0)` in `stage` over the texture `t` (on
/// unit 0, deleted after) and give the unit 0 record the job builder got as
/// `(width, height, depth, array_size)`; `None` if the draw was refused (a
/// GL error or a refusal line).
fn unit0_sizes(kind: Kind, stage: Stage, t: GLuint) -> Option<(u32, u32, u32, u32)> {
    let (ty, query) = (kind.sampler(), kind.query());
    let (vs, fs) = match stage {
        Stage::Fragment => (
            VS.to_string(),
            format!(
                "#version 300 es\nprecision highp float;\nprecision highp {ty};\nuniform {ty} s;\nin highp vec4 v;\n\
                 out vec4 o;\nvoid main() {{ o = vec4({query}, 1.0); }}\n"
            ),
        ),
        Stage::Vertex => (
            format!(
                "#version 300 es\nprecision highp {ty};\nuniform {ty} s;\nlayout(location = 0) in highp vec4 a_pos;\n\
                 out highp vec4 v;\nvoid main() {{ v = vec4({query}, 1.0); gl_Position = a_pos; }}\n"
            ),
            "#version 300 es\nprecision highp float;\nin highp vec4 v;\nout vec4 o;\nvoid main() { o = v; }\n"
                .to_string(),
        ),
    };
    let p = program(&vs, &fs);
    let e = draw_triangle();
    let lines = log_lines();
    let refused = e != GL_NO_ERROR || lines.iter().any(|l| l.contains("refused"));
    let out = if refused {
        None
    } else {
        let d = the_draw();
        let recs = if stage == Stage::Fragment { &d.tmu } else { &d.vs_tmu };
        let u = recs.iter().find(|u| u.unit == 0).unwrap_or_else(|| panic!("{kind:?}: unit 0 bound: {recs:?}"));
        assert_ne!(u.tex_state, 0, "{kind:?}: the unit has its texture record");
        Some((u.width, u.height, u.depth, u.array_size))
    };
    // SAFETY: plain GL calls.
    unsafe {
        glDeleteProgram(p);
        glDeleteTextures(1, &t);
    }
    out
}

/// A MUTABLE `kind` texture on unit 0 (`glTexImage2D/3D`; 16x16, 3D 8 deep,
/// array 5 layers at level 0, halved per level, layers kept) with exactly
/// the `levels` defined, NEAREST filters, `GL_TEXTURE_BASE_LEVEL` `base` and
/// `GL_TEXTURE_MAX_LEVEL` `max` if given. Also the first GL error of the
/// uploads.
fn mutable(kind: Kind, levels: &[GLint], base: GLint, max: Option<GLint>) -> (GLuint, GLenum) {
    let target = kind.target();
    let (mut t, up);
    t = 0;
    // SAFETY: plain GL calls; null texels.
    unsafe {
        glGenTextures(1, &mut t);
        glActiveTexture(GL_TEXTURE0);
        glBindTexture(target, t);
        for &level in levels {
            let w = (16 >> level) as GLsizei;
            match kind {
                Kind::Tex2d => glTexImage2D(
                    target,
                    level,
                    GL_RGBA8 as GLint,
                    w,
                    w,
                    0,
                    GL_RGBA,
                    GL_UNSIGNED_BYTE,
                    core::ptr::null(),
                ),
                Kind::Tex3d | Kind::Array => {
                    let d = if kind == Kind::Tex3d { (8 >> level) as GLsizei } else { 5 };
                    glTexImage3D(
                        target,
                        level,
                        GL_RGBA8 as GLint,
                        w,
                        w,
                        d,
                        0,
                        GL_RGBA,
                        GL_UNSIGNED_BYTE,
                        core::ptr::null(),
                    )
                }
            }
        }
        up = err();
        glTexParameteri(target, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
        glTexParameteri(target, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
        glTexParameteri(target, GL_TEXTURE_BASE_LEVEL, base);
        if let Some(m) = max {
            glTexParameteri(target, GL_TEXTURE_MAX_LEVEL, m);
        }
    }
    (t, up)
}

/// Draw `kind` over a fresh mutable texture and give unit 0's words.
fn words(kind: Kind, levels: &[GLint], base: GLint, max: Option<GLint>) -> Option<(u32, u32, u32, u32)> {
    LOG.lock().unwrap_or_else(|e| e.into_inner()).clear();
    DRAWS.lock().unwrap_or_else(|e| e.into_inner()).clear();
    let (t, up) = mutable(kind, levels, base, max);
    assert_eq!(up, GL_NO_ERROR, "the upload of {kind:?}: {:?}", log_lines());
    unit0_sizes(kind, Stage::Fragment, t)
}

/// The levels `r` of a chain, as a vector.
fn lv(r: core::ops::Range<GLint>) -> Vec<GLint> {
    r.collect()
}

/* ---- control: base level 0 ---- */

/// Control (holds after the fix): a 16x16 `GL_TEXTURE_2D` with its full chain and BASE_LEVEL 0
/// hands the level-0 words (16, 16, 1, 1).
#[test]
fn a_mutable_2d_with_a_full_chain_and_base_level_0_hands_level_0() {
    let _g = rig();
    assert_eq!(words(Kind::Tex2d, &lv(0..5), 0, None), Some((16, 16, 1, 1)));
}

/// Control (holds after the fix): a 16x16x8 `GL_TEXTURE_3D` with its full chain and BASE_LEVEL 0
/// hands the level-0 words (16, 16, 8, 1).
#[test]
fn a_mutable_3d_with_a_full_chain_and_base_level_0_hands_level_0() {
    let _g = rig();
    assert_eq!(words(Kind::Tex3d, &lv(0..4), 0, None), Some((16, 16, 8, 1)));
}

/// Control (holds after the fix): a 16x16 `GL_TEXTURE_2D_ARRAY` of 5 layers with its full chain and BASE_LEVEL 0
/// hands the level-0 words (16, 16, 1, 5).
#[test]
fn a_mutable_array_with_a_full_chain_and_base_level_0_hands_level_0() {
    let _g = rig();
    assert_eq!(words(Kind::Array, &lv(0..5), 0, None), Some((16, 16, 1, 5)));
}

/* ---- (a) a full chain, BASE_LEVEL 1 ---- */

/// Case (a), pins today's behaviour: a 16x16 `GL_TEXTURE_2D` with levels 0..5 defined and
/// BASE_LEVEL 1 draws and hands the LEVEL-0 words (16, 16, 1, 1). `textureSize(s, 0)` is
/// level 1's size, so the correct words are **(8, 8, 1, 1)** (width, height, depth, layers;
/// layers never minify). Flip the assert to (8, 8, 1, 1) when the base level is
/// applied.
#[test]
fn a_mutable_2d_with_a_full_chain_and_base_level_1_hands_level_0_flip_when_the_base_level_is_applied() {
    let _g = rig();
    assert_eq!(words(Kind::Tex2d, &lv(0..5), 1, None), Some((16, 16, 1, 1)));
}

/// Case (a), pins today's behaviour: a 16x16x8 `GL_TEXTURE_3D` with levels 0..4 defined and
/// BASE_LEVEL 1 draws and hands the LEVEL-0 words (16, 16, 8, 1). `textureSize(s, 0)` is
/// level 1's size, so the correct words are **(8, 8, 4, 1)** (width, height, depth, layers;
/// layers never minify). Flip the assert to (8, 8, 4, 1) when the base level is
/// applied.
#[test]
fn a_mutable_3d_with_a_full_chain_and_base_level_1_hands_level_0_flip_when_the_base_level_is_applied() {
    let _g = rig();
    assert_eq!(words(Kind::Tex3d, &lv(0..4), 1, None), Some((16, 16, 8, 1)));
}

/// Case (a), pins today's behaviour: a 16x16 `GL_TEXTURE_2D_ARRAY` of 5 layers with levels 0..5 defined and
/// BASE_LEVEL 1 draws and hands the LEVEL-0 words (16, 16, 1, 5). `textureSize(s, 0)` is
/// level 1's size, so the correct words are **(8, 8, 1, 5)** (width, height, depth, layers;
/// layers never minify). Flip the assert to (8, 8, 1, 5) when the base level is
/// applied.
#[test]
fn a_mutable_array_with_a_full_chain_and_base_level_1_hands_level_0_flip_when_the_base_level_is_applied() {
    let _g = rig();
    assert_eq!(words(Kind::Array, &lv(0..5), 1, None), Some((16, 16, 1, 5)));
}

/* ---- (b) only levels 1.. defined, BASE_LEVEL 1 ---- */

/// Case (b), pins today's behaviour: a 16x16 `GL_TEXTURE_2D` with ONLY levels 1.. defined and
/// BASE_LEVEL 1 is complete by ES 3.0 3.8.13 (levels below the base are not
/// looked at), but the GL core's completeness check demands level 0 ("level 0
/// has no image") and the draw hands the incomplete stand-in (1, 1, 1, 1). The
/// correct words are **(8, 8, 1, 1)**. Flip the assert to (8, 8, 1, 1) when the base level
/// is applied by the GL core's completeness check too (basefix must change
/// src/gl/draw.rs completeness, not only the bind fill).
#[test]
fn a_mutable_2d_with_only_levels_1_up_and_base_level_1_is_taken_as_incomplete_flip_when_the_base_level_is_applied() {
    let _g = rig();
    assert_eq!(words(Kind::Tex2d, &lv(1..5), 1, None), Some((1, 1, 1, 1)));
}

/// Case (b), pins today's behaviour: a 16x16x8 `GL_TEXTURE_3D` with ONLY levels 1.. defined and
/// BASE_LEVEL 1 is complete by ES 3.0 3.8.13 (levels below the base are not
/// looked at), but the GL core's completeness check demands level 0 ("level 0
/// has no image") and the draw hands the incomplete stand-in (1, 1, 1, 1). The
/// correct words are **(8, 8, 4, 1)**. Flip the assert to (8, 8, 4, 1) when the base level
/// is applied by the GL core's completeness check too (basefix must change
/// src/gl/draw.rs completeness, not only the bind fill).
#[test]
fn a_mutable_3d_with_only_levels_1_up_and_base_level_1_is_taken_as_incomplete_flip_when_the_base_level_is_applied() {
    let _g = rig();
    assert_eq!(words(Kind::Tex3d, &lv(1..4), 1, None), Some((1, 1, 1, 1)));
}

/// Case (b), a defect for a reason other than the base level, pinned as is: a
/// 2D array cannot be specified from level 1 up without level 0 at all:
/// `glTexImage3D(GL_TEXTURE_2D_ARRAY, 1, ...)` on an array with no level 0
/// raises GL_INVALID_VALUE (src/gl/texture.rs `tex_image_3d`: the array's
/// storage is sized by level 0 and a level > 0 is compared to it), where ES 3.0
/// 3.8.3 accepts it (levels may be specified in any order). The levels stay
/// undefined and the draw hands the incomplete stand-in (1, 1, 1, 1). The
/// base-level fix alone changes neither assert; once the upload works the draw
/// should be (8, 8, 1, 5).
#[test]
fn a_mutable_array_with_only_levels_1_up_cannot_be_specified_without_level_0() {
    let _g = rig();
    let (t, up) = mutable(Kind::Array, &lv(1..5), 1, None);
    assert_eq!(up, GL_INVALID_VALUE, "{:?}", log_lines());
    assert_eq!(unit0_sizes(Kind::Array, Stage::Fragment, t), Some((1, 1, 1, 1)));
}

/* ---- (c) BASE_LEVEL past the last defined level ---- */

/// Case (c), pins today's behaviour: a 16x16 `GL_TEXTURE_2D` with levels 0 and 1 defined and
/// BASE_LEVEL 5 draws and hands the LEVEL-0 words (16, 16, 1, 1). A mutable texture is
/// not clamped (unlike an immutable one) and level 5 is not defined, so it is
/// incomplete and the correct words are the stand-in **(1, 1, 1, 1)** (as
/// tests/es3_tex_words46_47_edge_v3d.rs pins for an incomplete texture). Flip
/// the assert to (1, 1, 1, 1) when the GL core's completeness check uses the
/// base level; not a clamp to level 1.
#[test]
fn a_mutable_2d_with_base_level_past_its_defined_levels_hands_level_0_flip_when_the_base_level_is_applied() {
    let _g = rig();
    assert_eq!(words(Kind::Tex2d, &lv(0..2), 5, None), Some((16, 16, 1, 1)));
}

/// Case (c), pins today's behaviour: a 16x16x8 `GL_TEXTURE_3D` with levels 0 and 1 defined and
/// BASE_LEVEL 5 draws and hands the LEVEL-0 words (16, 16, 8, 1). A mutable texture is
/// not clamped (unlike an immutable one) and level 5 is not defined, so it is
/// incomplete and the correct words are the stand-in **(1, 1, 1, 1)** (as
/// tests/es3_tex_words46_47_edge_v3d.rs pins for an incomplete texture). Flip
/// the assert to (1, 1, 1, 1) when the GL core's completeness check uses the
/// base level; not a clamp to level 1.
#[test]
fn a_mutable_3d_with_base_level_past_its_defined_levels_hands_level_0_flip_when_the_base_level_is_applied() {
    let _g = rig();
    assert_eq!(words(Kind::Tex3d, &lv(0..2), 5, None), Some((16, 16, 8, 1)));
}

/// Case (c), pins today's behaviour: a 16x16 `GL_TEXTURE_2D_ARRAY` of 5 layers with levels 0 and 1 defined and
/// BASE_LEVEL 5 draws and hands the LEVEL-0 words (16, 16, 1, 5). A mutable texture is
/// not clamped (unlike an immutable one) and level 5 is not defined, so it is
/// incomplete and the correct words are the stand-in **(1, 1, 1, 1)** (as
/// tests/es3_tex_words46_47_edge_v3d.rs pins for an incomplete texture). Flip
/// the assert to (1, 1, 1, 1) when the GL core's completeness check uses the
/// base level; not a clamp to level 1.
#[test]
fn a_mutable_array_with_base_level_past_its_defined_levels_hands_level_0_flip_when_the_base_level_is_applied() {
    let _g = rig();
    assert_eq!(words(Kind::Array, &lv(0..2), 5, None), Some((16, 16, 1, 5)));
}

/* ---- (d) BASE_LEVEL 1 with MAX_LEVEL 1 ---- */

/// Case (d), pins today's behaviour: a 16x16 `GL_TEXTURE_2D` with its full chain, BASE_LEVEL 1
/// and MAX_LEVEL 1 draws and hands the LEVEL-0 words (16, 16, 1, 1). The texture is
/// complete with the single level 1, so the correct words are **(8, 8, 1, 1)**. Flip
/// the assert to (8, 8, 1, 1) when the base level is applied.
#[test]
fn a_mutable_2d_with_base_level_1_and_max_level_1_hands_level_0_flip_when_the_base_level_is_applied() {
    let _g = rig();
    assert_eq!(words(Kind::Tex2d, &lv(0..5), 1, Some(1)), Some((16, 16, 1, 1)));
}

/// Case (d), pins today's behaviour: a 16x16x8 `GL_TEXTURE_3D` with its full chain, BASE_LEVEL 1
/// and MAX_LEVEL 1 draws and hands the LEVEL-0 words (16, 16, 8, 1). The texture is
/// complete with the single level 1, so the correct words are **(8, 8, 4, 1)**. Flip
/// the assert to (8, 8, 4, 1) when the base level is applied.
#[test]
fn a_mutable_3d_with_base_level_1_and_max_level_1_hands_level_0_flip_when_the_base_level_is_applied() {
    let _g = rig();
    assert_eq!(words(Kind::Tex3d, &lv(0..4), 1, Some(1)), Some((16, 16, 8, 1)));
}

/// Case (d), pins today's behaviour: a 16x16 `GL_TEXTURE_2D_ARRAY` of 5 layers with its full chain, BASE_LEVEL 1
/// and MAX_LEVEL 1 draws and hands the LEVEL-0 words (16, 16, 1, 5). The texture is
/// complete with the single level 1, so the correct words are **(8, 8, 1, 5)**. Flip
/// the assert to (8, 8, 1, 5) when the base level is applied.
#[test]
fn a_mutable_array_with_base_level_1_and_max_level_1_hands_level_0_flip_when_the_base_level_is_applied() {
    let _g = rig();
    assert_eq!(words(Kind::Array, &lv(0..5), 1, Some(1)), Some((16, 16, 1, 5)));
}
