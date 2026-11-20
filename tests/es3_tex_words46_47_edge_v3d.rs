// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The draw-level edge cases of uniform words 46 and 47 on the V3D (issue
//! #1261; commit 298fe11c made the job builder write
//! `V3D_QUNIFORM_TEXTURE_DEPTH` and `V3D_QUNIFORM_TEXTURE_ARRAY_SIZE` from
//! `V3dJobTmu::depth` / `::array_size`). tests/v3d_tex_array.rs
//! (`the_job_tmu_carries_the_depth_and_the_layer_count`) covers the complete,
//! bound 3D texture and 2D array only. `texture_units` (src/v3d/backend.rs)
//! fills the two values from `b.layers`: 1 layer if the texture is
//! incomplete, else `max(layers, 1)`; depth = layers for a 3D texture, else 1;
//! array_size = layers for a 2D array, else 1; a stand-in (black) unit, for a
//! sampler the stream queries with nothing bound, is 1 x 1 x 1 x 1. Through
//! the real GL core and the real V3D back end (the job builder stood in for
//! by this test's `v3d_job_*`, v3d_tex.c built by the test), this file reads
//! the draw's `V3dJobTmu` for:
//!
//! * an incomplete 3D texture and an incomplete 2D array (depth / size 1);
//! * a complete 2D array of exactly one layer (size 1, depth 1);
//! * a sampler3D / sampler2DArray queried with no texture bound (stand-in);
//! * a sampler2D and a samplerCube (both 1 and 1);
//! * a sampler2DArrayShadow that both samples and is queried over a
//!   DEPTH_COMPONENT32F array.
//!
//! The base level is out of scope: the draw applies no GL_TEXTURE_BASE_LEVEL
//! for words 44/45 (src/gl/draw.rs), so 46/47 follow the same level-0 rule.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_tex_words46_47_edge_v3d
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
/// The FS and the VS sampler bindings of every draw handed to the job
/// builder.
type Bindings = (Vec<V3dJobTmu>, Vec<V3dJobTmu>);
static DRAWS: Mutex<Vec<Bindings>> = Mutex::new(Vec::new());

fn log_lines() -> Vec<String> {
    LOG.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

fn draws() -> Vec<Bindings> {
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
    let b = (records(d.tmu, d.num_tmu), records(d.vs_tmu, d.num_vs_tmu));
    DRAWS.lock().unwrap_or_else(|e| e.into_inner()).push(b);
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
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("es3_tex_words46_47_edge_v3d");
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

/// The one draw the job builder was handed since `rig`, once the job is
/// done: its FS and its VS sampler bindings.
fn the_draw() -> (Vec<V3dJobTmu>, Vec<V3dJobTmu>) {
    // SAFETY: plain GL call.
    unsafe { glFinish() };
    let d = draws();
    assert_eq!(d.len(), 1, "one draw reached the job builder: {:?}", log_lines());
    d.into_iter().next().unwrap()
}

/// The 8x4 `layers`-layer GL_DEPTH_COMPONENT32F array on unit 0 as dEQP's
/// texturesize cases make it: NEAREST, one level, GL_COMPARE_REF_TO_TEXTURE.
fn depth_array(w: usize, h: usize, layers: usize) -> GLuint {
    let texels: Vec<f32> = (0..w * h * layers).map(|i| i as f32 / (w * h * layers) as f32).collect();
    let mut t = 0;
    let target = GL_TEXTURE_2D_ARRAY;
    // SAFETY: plain GL calls; the texels outlive the upload.
    unsafe {
        glGenTextures(1, &mut t);
        glActiveTexture(GL_TEXTURE0);
        glBindTexture(target, t);
        glTexImage3D(
            target,
            0,
            GL_DEPTH_COMPONENT32F as GLint,
            w as GLsizei,
            h as GLsizei,
            layers as GLsizei,
            0,
            GL_DEPTH_COMPONENT,
            GL_FLOAT,
            texels.as_ptr() as *const c_void,
        );
        glTexParameteri(target, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
        glTexParameteri(target, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
        glTexParameteri(target, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE as GLint);
        glTexParameteri(target, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE as GLint);
        glTexParameteri(target, GL_TEXTURE_COMPARE_MODE, GL_COMPARE_REF_TO_TEXTURE as GLint);
        glTexParameteri(target, GL_TEXTURE_COMPARE_FUNC, GL_LEQUAL as GLint);
    }
    assert_eq!(err(), GL_NO_ERROR, "the depth array: {:?}", log_lines());
    t
}

/// What the draw's unit 0 carries: width, height, depth, array_size.
fn unit0(fs: &[V3dJobTmu]) -> (u32, u32, u32, u32) {
    let u = fs.iter().find(|u| u.unit == 0).unwrap_or_else(|| panic!("unit 0 bound: {fs:?}"));
    (u.width, u.height, u.depth, u.array_size)
}

/// Make `target` on unit 0 a 4x4 RGBA8 texture of `layers` layers (a cube's
/// six faces; a 2D texture's one level). `complete` false leaves the
/// default NEAREST_MIPMAP_LINEAR minification filter with level 0 only,
/// which is GL ES 3.0's incomplete texture (3.8.13); true sets NEAREST.
fn texture_on_unit0(target: GLenum, layers: i32, complete: bool) -> GLuint {
    let texels = vec![0x40u8; 4 * 4 * layers.max(1) as usize * 4];
    let mut t = 0;
    // SAFETY: plain GL calls; the texels outlive the uploads and hold 4x4xN RGBA8.
    unsafe {
        glGenTextures(1, &mut t);
        glActiveTexture(GL_TEXTURE0);
        glBindTexture(target, t);
        match target {
            GL_TEXTURE_3D | GL_TEXTURE_2D_ARRAY => glTexImage3D(
                target,
                0,
                GL_RGBA8 as GLint,
                4,
                4,
                layers,
                0,
                GL_RGBA,
                GL_UNSIGNED_BYTE,
                texels.as_ptr() as *const c_void,
            ),
            GL_TEXTURE_CUBE_MAP => {
                for face in 0..6 {
                    glTexImage2D(
                        GL_TEXTURE_CUBE_MAP_POSITIVE_X + face,
                        0,
                        GL_RGBA8 as GLint,
                        4,
                        4,
                        0,
                        GL_RGBA,
                        GL_UNSIGNED_BYTE,
                        texels.as_ptr() as *const c_void,
                    );
                }
            }
            _ => glTexImage2D(
                target,
                0,
                GL_RGBA8 as GLint,
                4,
                4,
                0,
                GL_RGBA,
                GL_UNSIGNED_BYTE,
                texels.as_ptr() as *const c_void,
            ),
        }
        if complete {
            glTexParameteri(target, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
            glTexParameteri(target, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
        }
    }
    assert_eq!(err(), GL_NO_ERROR, "the texture: {:?}", log_lines());
    t
}

/// A fragment shader whose only use of sampler `s` is `textureSize(s, 0)`;
/// `z` reads the third component too (a 3D or array sampler).
fn size_fs(sampler: &str, z: bool) -> String {
    let size = if z { "vec4(vec3(textureSize(s, 0)), 1.0)" } else { "vec4(vec2(textureSize(s, 0)), 0.0, 1.0)" };
    format!(
        "#version 300 es\nprecision highp float;\nuniform highp {sampler} s;\nin highp vec4 v;\nout vec4 o;\n\
         void main() {{ o = {size} + v * 0.0; }}\n"
    )
}

/// Draw `fs` over what is bound and give its unit 0 as the job builder got it.
fn draw_and_unit0(fs: &str) -> (u32, u32, u32, u32) {
    /* a test may draw more than once: each draw starts with nothing recorded */
    DRAWS.lock().unwrap_or_else(|e| e.into_inner()).clear();
    LOG.lock().unwrap_or_else(|e| e.into_inner()).clear();
    let p = program(VS, fs);
    assert_eq!(draw_triangle(), GL_NO_ERROR, "the draw is accepted: {:?}", log_lines());
    let (fs, _) = the_draw();
    assert!(!log_lines().iter().any(|l| l.contains("refused")), "{:?}", log_lines());
    // SAFETY: a name made above.
    unsafe { glDeleteProgram(p) };
    unit0(&fs)
}

/// #1261, word 46/47 rule "an incomplete texture has one layer": an incomplete
/// 3D texture (4x4x5, level 0 only, the default mipmapped minification
/// filter) queried by `textureSize` reaches the draw as the 1x1 stand-in
/// state with depth 1 and array_size 1, not its 5 slices.
#[test]
fn an_incomplete_3d_texture_gives_depth_1_and_array_size_1() {
    let _g = rig();
    let t = texture_on_unit0(GL_TEXTURE_3D, 5, false);
    let got = draw_and_unit0(&size_fs("sampler3D", true));
    assert_eq!(got, (1, 1, 1, 1), "width, height, depth, array_size");
    // SAFETY: a name made above.
    unsafe { glDeleteTextures(1, &t) };
}

/// #1261, the same rule for a 2D array: an incomplete 4x4x5 array gives
/// array_size 1 (and depth 1).
#[test]
fn an_incomplete_2d_array_gives_array_size_1() {
    let _g = rig();
    let t = texture_on_unit0(GL_TEXTURE_2D_ARRAY, 5, false);
    let got = draw_and_unit0(&size_fs("sampler2DArray", true));
    assert_eq!(got, (1, 1, 1, 1), "width, height, depth, array_size");
    // SAFETY: a name made above.
    unsafe { glDeleteTextures(1, &t) };
}

/// #1261, the layer count rule at its smallest: a complete 4x4 2D array of
/// exactly one layer has array_size 1 and depth 1 (its 4x4 size is kept, so
/// this is not the incomplete rule).
#[test]
fn a_complete_2d_array_of_one_layer_gives_array_size_1_and_depth_1() {
    let _g = rig();
    let t = texture_on_unit0(GL_TEXTURE_2D_ARRAY, 1, true);
    let got = draw_and_unit0(&size_fs("sampler2DArray", true));
    assert_eq!(got, (4, 4, 1, 1), "width, height, depth, array_size");
    // SAFETY: a name made above.
    unsafe { glDeleteTextures(1, &t) };
}

/// #1261, the stand-in rule: a sampler3D / sampler2DArray that `textureSize`
/// queries while its unit has no texture bound (texture object 0 of the
/// target) is not refused. The GL core leaves the unit out of the draw's
/// bindings and the back end adds the black stand-in for the unit its size
/// words name, 1 x 1 x 1 x 1.
#[test]
fn a_queried_3d_or_array_sampler_with_nothing_bound_draws_with_a_1_1_stand_in() {
    let _g = rig();
    for (sampler, target) in [("sampler3D", GL_TEXTURE_3D), ("sampler2DArray", GL_TEXTURE_2D_ARRAY)] {
        // SAFETY: plain GL calls.
        unsafe {
            glActiveTexture(GL_TEXTURE0);
            glBindTexture(target, 0);
        }
        let got = draw_and_unit0(&size_fs(sampler, true));
        assert_eq!(got, (1, 1, 1, 1), "{sampler}: width, height, depth, array_size");
    }
}

/// #1261, the "any other texture type is 1" rule: a complete sampler2D and a
/// complete samplerCube (4x4, size words only) give depth 1 and array_size 1
/// with their own width and height.
#[test]
fn a_2d_and_a_cube_unit_give_depth_1_and_array_size_1() {
    let _g = rig();
    for (sampler, target) in [("sampler2D", GL_TEXTURE_2D), ("samplerCube", GL_TEXTURE_CUBE_MAP)] {
        let t = texture_on_unit0(target, 1, true);
        let got = draw_and_unit0(&size_fs(sampler, false));
        assert_eq!(got, (4, 4, 1, 1), "{sampler}: width, height, depth, array_size");
        // SAFETY: a name made above.
        unsafe { glDeleteTextures(1, &t) };
    }
}

/// #1261 with the RET16_OF_32 rule: a `sampler2DArrayShadow` that both samples
/// (`texture`) and is queried (`textureSize`) over an 8x4x3
/// DEPTH_COMPONENT32F array. The unit is read by a look-up, so it is not the
/// size-only unit that 080240bc exempts: the GL core keys the look-up for
/// the 32-bit TMU output, the R32F array agrees with that and the draw is
/// accepted (not refused with RET16_OF_32), carrying width, height, depth 1
/// and array_size 3.
#[test]
fn a_sampling_and_queried_array_shadow_unit_over_a_32_bit_depth_array_draws() {
    let _g = rig();
    let t = depth_array(8, 4, 3);
    let fs = concat!(
        "#version 300 es\nprecision highp float;\nuniform highp sampler2DArrayShadow s;\nin highp vec4 v;\n",
        "out vec4 o;\nvoid main() { o = vec4(texture(s, vec4(v.xy, 1.0, 0.5))) + vec4(vec3(textureSize(s, 0)), 0.0); }\n"
    );
    let got = draw_and_unit0(fs);
    assert_eq!(got, (8, 4, 1, 3), "width, height, depth, array_size");
    // SAFETY: a name made above.
    unsafe { glDeleteTextures(1, &t) };
}
