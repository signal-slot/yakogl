// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Cube map completeness with `GL_TEXTURE_BASE_LEVEL` > 0 on the V3D (plan
//! #382 item cubecompl, issue #1261): the faces are looked at only from the
//! base level on.
//!
//! ES 3.0 3.8.13 ("Cube Completeness"): a cube map texture is cube complete
//! when the base level of each of its six faces is defined, all six have the
//! same positive square size and the same internal format; it is mipmap
//! cube complete when each face is also mipmap complete from the base level
//! to q. Levels below the base level are not looked at. An incomplete
//! texture samples (0, 0, 0, 1) (3.8.13's last paragraph) and its size
//! words are the stand-in's (1, 1). Mesa's state tracker
//! (`src/mesa/main/texobj.c` `_mesa_test_texobj_completeness`, its cube
//! loop over `baseLevel`) is not in third_party/ (only gallium, broadcom,
//! compiler, util and the like are vendored), so the spec is the reference.
//!
//! Today the GL core's check (src/gl/draw.rs `incomplete_reason`) looks at
//! level 0 alone: `faces_set` records the faces that hold a LEVEL 0 image,
//! and the per-level face records (`level_faces`) are looked at only for a
//! mipmap min filter, from level 1 up to the chain's last. The base level
//! plays no part. The item `basecompl` makes the check start at the base
//! level.
//!
//! Every texture is RGBA8, 16x16 at level 0 (8x8 at level 1), NEAREST
//! filters (only the base level counts for completeness), sampled and
//! queried in the fragment stage (`texture(s, ..)` + `textureSize(s, 0)`).
//! Each case pins: the GL error of the draw, whether the job builder got it,
//! whether the GL core counted it as a draw of an incomplete texture
//! (`incomplete_texture_draws`), whether unit 0 got the cube stand-in's
//! texture record (the (0, 0, 0, 1) image: the "sampled colour", as the
//! fake job builder of this harness reads no pixels), and words 44/45.
//!
//! - (a) a mutable cube with only levels 1..4 on all six faces, BASE_LEVEL 1;
//! - (b) as (a) with one face missing at level 1; (b2) a full level 0 under
//!   it;
//! - (c) as (a) with one face of another size at level 1; (c2) a full
//!   level 0 under it;
//! - (d) an immutable cube (`glTexStorage2D`, 5 levels), BASE_LEVEL 1;
//! - (e) BASE_LEVEL 0 controls, mutable and immutable.
//!
//! Asserts whose value disagrees with the spec today carry the suffix
//! `_flip_when_completeness_follows_the_base_level`; their doc comment gives
//! the spec value. The others hold after the fix. The size words follow the
//! base level (GlesTextureBind::base_level is filled).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_cube_base_level_compl_v3d
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
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("es3_cube_base_level_compl_v3d");
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

/* ---- the cube maps ---- */

use yakogl::gl::draw::incomplete_texture_draws;

/// What one draw of the module gave.
#[derive(Clone, Copy, PartialEq, Debug)]
struct Outcome {
    /// The GL error of the draw.
    error: GLenum,
    /// The job builder got the draw (no GL error, no refusal line).
    drew: bool,
    /// The GL core counted it as a draw of an incomplete texture.
    incomplete: bool,
    /// Unit 0 got the cube stand-in's texture record: it samples (0, 0, 0, 1).
    stand_in: bool,
    /// Words 44/45 (width, height) of unit 0; (0, 0) when the draw did not happen.
    size: (u32, u32),
}

/// A draw the job builder got: complete (its own record) or incomplete (the
/// stand-in), with words 44/45 `size`.
const fn drawn(incomplete: bool, size: (u32, u32)) -> Outcome {
    Outcome { error: GL_NO_ERROR, drew: true, incomplete, stand_in: incomplete, size }
}

/// The fragment program: samples and queries the cube on unit 0.
const FS: &str = "#version 300 es\nprecision highp float;\nuniform samplerCube s;\nin highp vec4 v;\nout vec4 o;\n\
                  void main() { o = texture(s, v.xyz) + vec4(vec2(textureSize(s, 0)), 0.0, 0.0); }\n";

/// Draw the program over whatever unit 0's cube target holds: the GL error,
/// whether the job builder got the draw, whether the GL core counted an
/// incomplete texture, unit 0's texture record and its words 44/45.
fn draw_unit0() -> (GLenum, bool, bool, u32, (u32, u32)) {
    LOG.lock().unwrap_or_else(|e| e.into_inner()).clear();
    DRAWS.lock().unwrap_or_else(|e| e.into_inner()).clear();
    let p = program(VS, FS);
    let before = incomplete_texture_draws();
    let e = draw_triangle();
    let incomplete = incomplete_texture_draws() != before;
    let refused = e != GL_NO_ERROR || log_lines().iter().any(|l| l.contains("refused"));
    let out = if refused {
        (e, false, incomplete, 0, (0, 0))
    } else {
        let d = the_draw();
        let u = d.tmu.iter().find(|u| u.unit == 0).unwrap_or_else(|| panic!("unit 0 bound: {:?}", d.tmu));
        assert_ne!(u.tex_state, 0, "the unit has its texture record");
        (e, true, incomplete, u.tex_state, (u.width, u.height))
    };
    // SAFETY: plain GL call.
    unsafe { glDeleteProgram(p) };
    out
}

/// The texture record the back end gives an incomplete cube (texture object
/// 0 on the cube target: no face at all), the (0, 0, 0, 1) stand-in.
fn stand_in_state() -> u32 {
    // SAFETY: plain GL calls.
    unsafe {
        glActiveTexture(GL_TEXTURE0);
        glBindTexture(GL_TEXTURE_CUBE_MAP, 0);
    }
    /* texture object 0 has no GL texture to resolve, so the GL core's
     * incomplete-draw counter does not count it */
    let (e, drew, _, state, size) = draw_unit0();
    assert_eq!((e, drew, size), (GL_NO_ERROR, true, (1, 1)), "{:?}", log_lines());
    state
}

/// Draw over the cube `t` (on unit 0, deleted after) and give the outcome.
fn outcome(t: GLuint) -> Outcome {
    let black = stand_in_state();
    // SAFETY: plain GL calls.
    unsafe {
        glActiveTexture(GL_TEXTURE0);
        glBindTexture(GL_TEXTURE_CUBE_MAP, t);
    }
    let (error, drew, incomplete, state, size) = draw_unit0();
    // SAFETY: plain GL call.
    unsafe { glDeleteTextures(1, &t) };
    Outcome { error, drew, incomplete, stand_in: drew && state == black, size }
}

/// One face of a cube level on the bound cube: `size` x `size` RGBA8, null
/// texels.
fn face(face: GLenum, level: GLint, size: GLsizei) {
    // SAFETY: a plain GL call; null texels.
    unsafe {
        glTexImage2D(
            GL_TEXTURE_CUBE_MAP_POSITIVE_X + face,
            level,
            GL_RGBA8 as GLint,
            size,
            size,
            0,
            GL_RGBA,
            GL_UNSIGNED_BYTE,
            core::ptr::null(),
        )
    }
}

/// NEAREST filters and `GL_TEXTURE_BASE_LEVEL` `base` on the bound cube.
fn params(base: GLint) {
    // SAFETY: plain GL calls.
    unsafe {
        glTexParameteri(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
        glTexParameteri(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
        glTexParameteri(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_BASE_LEVEL, base);
    }
}

/// A MUTABLE cube on unit 0 (`glTexImage2D` per face): the `levels` of a
/// 16x16 chain on every face, except that face 5 (-Z) gets no image at
/// level `missing` and face 4 (+Z) a 4x4 one at level `odd`; NEAREST
/// filters, BASE_LEVEL `base`. The uploads raise no error.
fn mutable_cube(levels: core::ops::Range<GLint>, missing: Option<GLint>, odd: Option<GLint>, base: GLint) -> GLuint {
    let mut t = 0;
    // SAFETY: plain GL calls.
    unsafe {
        glGenTextures(1, &mut t);
        glActiveTexture(GL_TEXTURE0);
        glBindTexture(GL_TEXTURE_CUBE_MAP, t);
    }
    for level in levels {
        for f in 0..6 {
            if f == 5 && missing == Some(level) {
                continue;
            }
            let size = if f == 4 && odd == Some(level) { 4 } else { 16 >> level };
            face(f, level, size);
        }
    }
    params(base);
    assert_eq!(err(), GL_NO_ERROR, "the uploads: {:?}", log_lines());
    t
}

/// An IMMUTABLE 16x16 cube on unit 0 (`glTexStorage2D`, 5 levels), NEAREST
/// filters, BASE_LEVEL `base`.
fn immutable_cube(base: GLint) -> GLuint {
    let mut t = 0;
    // SAFETY: plain GL calls.
    unsafe {
        glGenTextures(1, &mut t);
        glActiveTexture(GL_TEXTURE0);
        glBindTexture(GL_TEXTURE_CUBE_MAP, t);
        glTexStorage2D(GL_TEXTURE_CUBE_MAP, 5, GL_RGBA8, 16, 16);
    }
    params(base);
    assert_eq!(err(), GL_NO_ERROR, "the storage: {:?}", log_lines());
    t
}

/* ---- (e) controls: BASE_LEVEL 0 ---- */

/// Control (holds after the fix): a mutable cube with its full chain on all
/// six faces and BASE_LEVEL 0 is complete: it draws with its own record and
/// hands level 0's words (16, 16).
#[test]
fn e_a_mutable_cube_with_base_level_0_is_complete() {
    let _g = rig();
    assert_eq!(outcome(mutable_cube(0..5, None, None, 0)), drawn(false, (16, 16)));
}

/// Control (holds after the fix): an immutable 16x16 cube of 5 levels with
/// BASE_LEVEL 0 is complete and hands (16, 16).
#[test]
fn e_an_immutable_cube_with_base_level_0_is_complete() {
    let _g = rig();
    assert_eq!(outcome(immutable_cube(0)), drawn(false, (16, 16)));
}

/// Control (holds after the fix): a mutable cube with -Z missing at level 0
/// and BASE_LEVEL 0 is not cube complete (ES 3.0 3.8.13): the stand-in and
/// words (1, 1).
#[test]
fn e_a_mutable_cube_missing_a_face_at_base_level_0_is_incomplete() {
    let _g = rig();
    assert_eq!(outcome(mutable_cube(0..5, Some(0), None, 0)), drawn(true, (1, 1)));
}

/* ---- (a) only levels 1.. on all six faces, BASE_LEVEL 1 ---- */

/// Case (a), pins today's behaviour: a mutable cube with ONLY levels 1..4
/// (8x8 down to 1x1) on all six faces and BASE_LEVEL 1 is drawn as
/// incomplete (`faces_set` records level-0 faces only: "cube map without
/// all six faces"), with the stand-in record and words (1, 1).
///
/// Spec value: ES 3.0 3.8.13 looks at the six faces' BASE level only, here
/// six 8x8 RGBA8 images: the cube is cube complete, samples its own texels,
/// and `textureSize(s, 0)` is level 1's size: **`drawn(false, (8, 8))`**
/// (the words also need item basefix's base-level size words). Flip to that
/// when completeness follows the base level.
#[test]
fn a_a_mutable_cube_with_only_levels_1_up_and_base_level_1_is_incomplete_flip_when_completeness_follows_the_base_level()
{
    let _g = rig();
    assert_eq!(outcome(mutable_cube(1..5, None, None, 1)), drawn(true, (1, 1)));
}

/* ---- (b) a face missing at the base level ---- */

/// Case (b), holds after the fix: as (a) with -Z missing at level 1, the
/// base level. Not cube complete by ES 3.0 3.8.13 (a face's base level is
/// not defined); today incomplete as well (for the level-0 reason), so the
/// stand-in and (1, 1) are the spec value.
#[test]
fn b_a_mutable_cube_with_only_levels_1_up_and_a_face_missing_at_base_level_1_is_incomplete() {
    let _g = rig();
    assert_eq!(outcome(mutable_cube(1..5, Some(1), None, 1)), drawn(true, (1, 1)));
}

/// Case (b2), pins today's behaviour: all six faces at levels 0..4 except
/// -Z missing at level 1, BASE_LEVEL 1. Today the check looks at level 0
/// (the per-level face records only under a mipmap filter), so the cube is
/// complete; it hands the base level's words (8, 8) (GlesTextureBind::
/// base_level is filled).
///
/// Spec value: ES 3.0 3.8.13: -Z's base level (1) is not defined, so the
/// cube is not cube complete: the stand-in, **`drawn(true, (1, 1))`**. Flip
/// to that when completeness follows the base level.
#[test]
fn b2_a_full_level_0_under_a_face_missing_at_base_level_1_is_complete_flip_when_completeness_follows_the_base_level() {
    let _g = rig();
    assert_eq!(outcome(mutable_cube(0..5, Some(1), None, 1)), drawn(false, (8, 8)));
}

/* ---- (c) faces of different sizes at the base level ---- */

/// Case (c), holds after the fix: as (a) with +Z 4x4 instead of 8x8 at
/// level 1, the base level. Not cube complete by ES 3.0 3.8.13 (the six
/// base images are not of one size); today incomplete as well (for the
/// level-0 reason), so the stand-in and (1, 1) are the spec value.
#[test]
fn c_a_mutable_cube_with_faces_of_different_sizes_at_base_level_1_is_incomplete() {
    let _g = rig();
    assert_eq!(outcome(mutable_cube(1..5, None, Some(1), 1)), drawn(true, (1, 1)));
}

/// Case (c2), pins today's behaviour: all six faces at levels 0..4 except
/// +Z 4x4 at level 1, BASE_LEVEL 1. Today the check looks at level 0, so
/// the cube is complete; it hands the base level's words (8, 8).
///
/// Spec value: ES 3.0 3.8.13: the six base (level 1) images are not of one
/// size, so the cube is not cube complete: the stand-in,
/// **`drawn(true, (1, 1))`**. Flip to that when completeness follows the
/// base level.
#[test]
fn c2_a_full_level_0_under_faces_of_different_sizes_at_base_level_1_is_complete_flip_when_completeness_follows_the_base_level(
) {
    let _g = rig();
    assert_eq!(outcome(mutable_cube(0..5, None, Some(1), 1)), drawn(false, (8, 8)));
}

/* ---- (d) an immutable cube, BASE_LEVEL 1 ---- */

/// Case (d), completeness holds after the fix: an immutable 16x16 cube of 5
/// levels with BASE_LEVEL 1 is cube complete (ES 3.0 3.8.13; glTexStorage2D
/// defines every level of every face, 3.8.4) and draws with its own record.
#[test]
fn d_an_immutable_cube_with_base_level_1_is_complete() {
    let _g = rig();
    let o = outcome(immutable_cube(1));
    assert_eq!((o.error, o.drew, o.incomplete, o.stand_in), (GL_NO_ERROR, true, false, false));
}

/// Case (d): its words 44/45 are the base level's (8, 8), `textureSize(s, 0)`
/// at BASE_LEVEL 1 (GLSL ES 3.00 8.9.1; tests/es3_texsize_base_level_v3d.rs
/// pins the same for an immutable cube).
#[test]
fn d_an_immutable_cube_with_base_level_1_hands_the_base_levels_words() {
    let _g = rig();
    assert_eq!(outcome(immutable_cube(1)).size, (8, 8));
}
