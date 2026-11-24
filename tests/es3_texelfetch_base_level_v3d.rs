// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! V3D `texelFetch()` with `GL_TEXTURE_BASE_LEVEL` > 0 (plan #382 item
//! fetchbase, issue #1261). A `texelFetch(s, .., 1)` of a complete
//! mipmapped 2D 8x8, 3D 8x8x4, 2D-array 8x8x5 and RGBA8I (`isampler2D`)
//! texture, BASE_LEVEL 0 (controls) and 1 (pins).
//!
//! GLES 3.0 §3.8.10 / GLSL ES 3.00 §8.8: the lod of texelFetch is relative
//! to the base level, so the level read is base + lod (here 1 + 1 = 2).
//! Today the back end ignores the base level: the TMU texture state record
//! has Base Level 0 (src/v3d/texture.rs `state`) and the unit's size words
//! (`V3dJobTmu` width, height, depth, array_size = words 44-47) are
//! level 0's. The test reads both, as tests/es3_sample_base_level_v3d.rs
//! (record bytes in the fake GPU memory, byte 15: base_level bits 7:4,
//! max_level 3:0) and tests/es3_texsize_base_level_v3d.rs (size words) do.
//!
//! What can and cannot be observed host-side:
//! - Observed and pinned: the state's Base Level / Max Level and the four
//!   size words of the unit. The job builder is handed the same
//!   `V3dJobTmu` for a texelFetch as for `texture()`; the draw carries no
//!   texel-fetch parameter word of its own that this test can see (the lod
//!   is an operand of the shader's own TMU write, not a draw uniform).
//! - NOT observable host-side: whether the V3D TMU applies a txf lod
//!   relative to the state's Base Level, and the fetched texel. The fake
//!   job builder never runs the TMU, and the Mesa files that would answer
//!   it (v3dx_state.c, v3d_uniforms.c, v3d40_vir_emit_tex in the broadcom
//!   compiler) are not in third_party/mesa-26.2.1 (only the format tables,
//!   tiling and CLE are vendored). The post-fix state values below follow
//!   Mesa's v3dx_state.c (Base Level = first_level, Max Level = last level)
//!   as the sibling tests state them; the TMU's txf lod behaviour is left
//!   to a board run, and nothing here pins a guess about the fetch result.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_texelfetch_base_level_v3d
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
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("es3_texelfetch_base_level_v3d");
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

/* ---- the textures: 8 texels wide, four levels, a colour per level ---- */

/// Levels 0..=3: 8, 4, 2, 1 texels.
const LEVELS: usize = 4;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Kind {
    Tex2d,
    Tex3d,
    Array,
    /// A 2D RGBA8I texture read through an `isampler2D`.
    Int2d,
}

impl Kind {
    fn target(self) -> GLenum {
        match self {
            Kind::Tex2d | Kind::Int2d => GL_TEXTURE_2D,
            Kind::Tex3d => GL_TEXTURE_3D,
            Kind::Array => GL_TEXTURE_2D_ARRAY,
        }
    }
    /// The sampler type and the `texelFetch()` call of the shader, lod 1.
    fn sampler(self) -> (&'static str, &'static str) {
        match self {
            Kind::Tex2d => ("sampler2D", "texelFetch(s, ivec2(1, 1), 1)"),
            Kind::Tex3d => ("sampler3D", "texelFetch(s, ivec3(1, 1, 1), 1)"),
            Kind::Array => ("sampler2DArray", "texelFetch(s, ivec3(1, 1, 1), 1)"),
            Kind::Int2d => ("isampler2D", "vec4(texelFetch(s, ivec2(1, 1), 1))"),
        }
    }
}

/// A complete mipmapped `kind` texture on unit 0 with GL_TEXTURE_BASE_LEVEL
/// `base`: 8x8 (3D 8x8x4 halved per level, array 8x8x5 at every level),
/// level `n` filled with the value (n*30 + 20, 0, 0, 100).
fn texture(kind: Kind, base: GLint) -> GLuint {
    let target = kind.target();
    let mut t = 0;
    // SAFETY: plain GL calls; the texel arrays outlive each upload.
    unsafe {
        glGenTextures(1, &mut t);
        glActiveTexture(GL_TEXTURE0);
        glBindTexture(target, t);
        for level in 0..LEVELS {
            let w = (8usize >> level).max(1);
            let d = match kind {
                Kind::Tex2d | Kind::Int2d => 1,
                Kind::Tex3d => (4usize >> level).max(1),
                Kind::Array => 5,
            };
            let texel = [(level * 30 + 20) as u8, 0, 0, 100];
            let texels: Vec<u8> = (0..w * w * d).flat_map(|_| texel).collect();
            let p = texels.as_ptr() as *const c_void;
            let (l, w, d) = (level as GLint, w as GLsizei, d as GLsizei);
            match kind {
                Kind::Tex2d => glTexImage2D(target, l, GL_RGBA8 as GLint, w, w, 0, GL_RGBA, GL_UNSIGNED_BYTE, p),
                Kind::Int2d => glTexImage2D(target, l, GL_RGBA8I as GLint, w, w, 0, GL_RGBA_INTEGER, GL_BYTE, p),
                _ => glTexImage3D(target, l, GL_RGBA8 as GLint, w, w, d, 0, GL_RGBA, GL_UNSIGNED_BYTE, p),
            }
        }
        glTexParameteri(target, GL_TEXTURE_MIN_FILTER, GL_NEAREST_MIPMAP_NEAREST as GLint);
        glTexParameteri(target, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
        glTexParameteri(target, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE as GLint);
        glTexParameteri(target, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE as GLint);
        glTexParameteri(target, GL_TEXTURE_BASE_LEVEL, base);
    }
    assert_eq!(err(), GL_NO_ERROR, "the upload of {kind:?}: {:?}", log_lines());
    t
}

/// What a `texelFetch(s, .., 1)` draw of a `kind` texture with
/// GL_TEXTURE_BASE_LEVEL `base` hands the job builder for unit 0:
/// the TMU state's `(base_level, max_level)` and the unit's
/// `(width, height, depth, array_size)` words (44-47).
fn unit0(kind: Kind, base: GLint) -> ((u32, u32), (u32, u32, u32, u32)) {
    let t = texture(kind, base);
    let (ty, call) = kind.sampler();
    let fs = format!(
        "#version 300 es\nprecision highp float;\nprecision highp {ty};\nuniform {ty} s;\nin highp vec4 v;\n\
         out vec4 o;\nvoid main() {{ o = {call}; }}\n"
    );
    let p = program(VS, &fs);
    let e = draw_triangle();
    let lines = log_lines();
    assert_eq!(e, GL_NO_ERROR, "{kind:?} base {base}: the draw is accepted: {lines:?}");
    assert!(!lines.iter().any(|l| l.contains("refused")), "{kind:?} base {base}: {lines:?}");
    let d = the_draw();
    // SAFETY: plain GL calls.
    unsafe {
        glDeleteProgram(p);
        glDeleteTextures(1, &t);
    }
    let u = d.tmu.iter().find(|u| u.unit == 0).unwrap_or_else(|| panic!("{kind:?}: unit 0 bound: {:?}", d.tmu));
    assert_ne!(u.tex_state, 0, "{kind:?} base {base}: the unit has its texture record");
    let b15 = with_platform(|p| {
        let gpu = u64::from(u.tex_state);
        let (base_addr, a) = p.allocs.range(..=gpu).next_back().expect("an allocation below the record");
        let off = (gpu - base_addr) as usize;
        assert!(off + V3D_TEX_STATE_BYTES <= a.size, "the record lies in its allocation");
        // SAFETY: inside the allocation (checked above).
        unsafe { *(a.mem.as_ptr() as *const u8).add(off + 15) }
    });
    ((u32::from(b15 >> 4), u32::from(b15 & 15)), (u.width, u.height, u.depth, u.array_size))
}

/* ---- BASE_LEVEL 0: the controls, correct today and after the fix ---- */

/// Control: texelFetch lod 1 of a 2D 8x8 texture at BASE_LEVEL 0: TMU levels
/// 0 to 3, level-0 size words 8 x 8.
#[test]
fn a_2d_texel_fetch_at_base_level_0_reads_the_level_0_state() {
    let _g = rig();
    assert_eq!(unit0(Kind::Tex2d, 0), ((0, 3), (8, 8, 1, 1)));
}

/// Control: the 3D 8x8x4 texture at BASE_LEVEL 0: (0, 3), words 8, 8, 4.
#[test]
fn a_3d_texel_fetch_at_base_level_0_reads_the_level_0_state() {
    let _g = rig();
    assert_eq!(unit0(Kind::Tex3d, 0), ((0, 3), (8, 8, 4, 1)));
}

/// Control: the 2D array 8x8x5 at BASE_LEVEL 0: (0, 3), words 8, 8, 1, 5.
#[test]
fn a_2d_array_texel_fetch_at_base_level_0_reads_the_level_0_state() {
    let _g = rig();
    assert_eq!(unit0(Kind::Array, 0), ((0, 3), (8, 8, 1, 5)));
}

/// Control: an isampler2D over RGBA8I at BASE_LEVEL 0: (0, 3), words 8 x 8.
#[test]
fn an_integer_2d_texel_fetch_at_base_level_0_reads_the_level_0_state() {
    let _g = rig();
    assert_eq!(unit0(Kind::Int2d, 0), ((0, 3), (8, 8, 1, 1)));
}

/* ---- BASE_LEVEL 1: the size words follow the base level (the draw applies it); the TMU state's base level is still 0, pinned ---- */

/// The size words are the base level's (the draw applies `draw::effective_base_level`); the TMU state's Base Level is still 0, pinned here. Before the draw applied it: with GL_TEXTURE_BASE_LEVEL 1 the 2D texture's TMU
/// state has Base Level 0 (Max Level 3) and the size words are level 0's
/// (8, 8). Mesa's v3dx_state.c programs Base Level = the GL first level and
/// v3d_uniforms.c writes the size of that level. After the fix: state
/// (1, 3), words (4, 4, 1, 1).
#[test]
fn a_2d_texel_fetch_at_base_level_1_hands_the_base_level_words_flip_when_the_tmu_base_level_is_applied() {
    let _g = rig();
    assert_eq!(unit0(Kind::Tex2d, 1), ((0, 3), (4, 4, 1, 1)));
}

/// The size words are the base level's (the draw applies `draw::effective_base_level`); the TMU state's Base Level is still 0, pinned here. Before the draw applied it: the 3D texture at BASE_LEVEL 1 has state (0, 3)
/// and words (8, 8, 4, 1). After the fix: (1, 3) and (4, 4, 2, 1) (the
/// depth word 46 is the base level's depth, v3d_uniforms.c).
#[test]
fn a_3d_texel_fetch_at_base_level_1_hands_the_base_level_words_flip_when_the_tmu_base_level_is_applied() {
    let _g = rig();
    assert_eq!(unit0(Kind::Tex3d, 1), ((0, 3), (4, 4, 2, 1)));
}

/// The size words are the base level's (the draw applies `draw::effective_base_level`); the TMU state's Base Level is still 0, pinned here. Before the draw applied it: the 2D array at BASE_LEVEL 1 has state (0, 3)
/// and words (8, 8, 1, 5). After the fix: (1, 3) and (4, 4, 1, 5) (the layer
/// count is not minified).
#[test]
fn a_2d_array_texel_fetch_at_base_level_1_hands_the_base_level_words_flip_when_the_tmu_base_level_is_applied() {
    let _g = rig();
    assert_eq!(unit0(Kind::Array, 1), ((0, 3), (4, 4, 1, 5)));
}

/// The size words are the base level's (the draw applies `draw::effective_base_level`); the TMU state's Base Level is still 0, pinned here. Before the draw applied it: the isampler2D at BASE_LEVEL 1 has state (0, 3)
/// and words (8, 8, 1, 1). After the fix: (1, 3) and (4, 4, 1, 1).
#[test]
fn an_integer_2d_texel_fetch_at_base_level_1_hands_the_base_level_words_flip_when_the_tmu_base_level_is_applied() {
    let _g = rig();
    assert_eq!(unit0(Kind::Int2d, 1), ((0, 3), (4, 4, 1, 1)));
}
