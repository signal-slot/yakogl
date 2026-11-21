// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The ES 2.0 context against three V3D commits the Pi 4's last whole
//! dEQP-GLES2 tree (46f93767) predates:
//!
//! * 298fe11c: the job builder writes uniform words 46/47
//!   (`V3D_QUNIFORM_TEXTURE_DEPTH`, `_ARRAY_SIZE`), looked up by sampler
//!   binding, and `struct v3d_job_tmu` grew `depth` / `array_size`.
//! * 080240bc: a unit that only textureSize reads takes a 32-bit texture
//!   without the return-width refusal.
//! * 35ea14d6: a shadow look-up of a fixed-point depth texture clamps its
//!   reference to [0, 1] (`LodKey::fixed_depth`).
//!
//! GLSL ES 1.00 has no textureSize (ES 3.00 8.8), so the compiler never emits
//! words 46/47 for an ES 2.0 program: `SOp::TexSizeWord` is made only by
//! textureSize (src/shaderc/online/v3d/prepare.rs); words 44/45 appear only
//! for the non-seamless cube look-up. The expected values come from Mesa's
//! v3d_uniforms.c as 298fe11c cites it (QUNIFORM_TEXTURE_DEPTH is a 3D
//! texture's base-level depth, 1 for any other texture;
//! QUNIFORM_TEXTURE_ARRAY_SIZE is a 2D array's layer count, 1 for 2D and cube)
//! and the spec, not from the current output. So words 46/47 are ABSENT from
//! every ES 2.0 fragment stream, and the sampler binding the job carries for a
//! 2D or cube sampler has depth 1 and array_size 1 (never 0, which the job
//! builder refuses when a word reads it) and the base level's size as
//! width/height. The job builder is stood in for (as in
//! v3d_es2ctx_refusals.rs), so the words are read where the back end hands
//! them over: each draw's uniform list and `V3dJobTmu` array.
//!
//! The 080240bc textureSize path is not reachable from GLSL ES 1.00. The
//! shadow path (EXT_shadow_samplers) is tested if the core lists it.
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
use yakogl::gl::framebuffer::*;
use yakogl::gl::state::*;
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps, OsalV3dCpuOf};
use yakogl::v3d::backend::yakogl_backend_v3d;
use yakogl::v3d::ffi::*;
use yakogl::v3d::kick::V3dKick;

/* ---- the job builder: records, draws nothing ---- */

/// What one draw carried to the job builder: the fragment stream's uniform
/// words as (contents, data), its sampler bindings, and the number of vertex
/// stage bindings.
struct Draw {
    uniforms: Vec<(u32, u32)>,
    tmu: Vec<V3dJobTmu>,
    vs_tmu: u32,
}
static DRAWS: Mutex<Vec<Draw>> = Mutex::new(Vec::new());

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
    // SAFETY: the backend passes its live job and draw; `fs` and `tmu` point at
    // `num_uniforms` / `num_tmu` live entries for the call.
    unsafe {
        let d = &*d;
        let fs = &*d.fs;
        let uniforms = if fs.uniforms.is_null() {
            Vec::new()
        } else {
            core::slice::from_raw_parts(fs.uniforms, fs.num_uniforms as usize)
                .iter()
                .map(|u| (u.contents, u.data))
                .collect()
        };
        let tmu =
            if d.tmu.is_null() { Vec::new() } else { core::slice::from_raw_parts(d.tmu, d.num_tmu as usize).to_vec() };
        DRAWS.lock().unwrap_or_else(|e| e.into_inner()).push(Draw { uniforms, tmu, vs_tmu: d.num_vs_tmu });
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
use yakogl::v3d::backend::v3d_refused_draws;

const W: GLsizei = 32;
const H: GLsizei = 32;

const VS: &str = "attribute vec4 p;\nvarying vec2 t;\nvoid main() { gl_Position = p; t = p.xy * 0.5 + 0.5; }\n";
const FS_TEX: &str = "precision mediump float;\nvarying vec2 t;\nuniform sampler2D s;\n\
                      void main() { gl_FragColor = texture2D(s, t); }\n";
const FS_CUBE: &str = "precision mediump float;\nvarying vec2 t;\nuniform samplerCube s;\n\
                       void main() { gl_FragColor = textureCube(s, vec3(t, 0.5)); }\n";

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

/// Make `fs` (with `VS`) the current program; sampler `s`, if any, reads unit 0.
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
        let s = glGetUniformLocation(p, c"s".as_ptr());
        if s >= 0 {
            glUniform1i(s, 0);
        }
    }
}

/// The one rule of the file: what `f` does is accepted. No GL error and the
/// back end refused no draw (`v3d_refused_draws` is read as a difference).
fn accepted(what: &str, f: impl FnOnce()) {
    let base = v3d_refused_draws();
    f();
    // SAFETY: a plain GL call.
    unsafe { glFinish() };
    let (e, n) = (err(), v3d_refused_draws().wrapping_sub(base));
    assert_eq!((e, n), (GL_NO_ERROR, 0), "{what}: error, refused draws; {:?}", log_lines());
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

/// A 2D texture bound to unit 0: `w` x `h` of `format`/`type_` filled with
/// 0x7f, the filter and wrap given.
fn texture(
    target: GLenum,
    internal: GLenum,
    format: GLenum,
    type_: GLenum,
    w: GLsizei,
    h: GLsizei,
    min: GLenum,
) -> GLuint {
    let mut t = 0;
    let texels = vec![0x7fu8; (w * h) as usize * 16];
    // SAFETY: plain GL calls; the buffer holds 16 bytes a texel.
    unsafe {
        glActiveTexture(GL_TEXTURE0);
        glGenTextures(1, &mut t);
        glBindTexture(target, t);
        let faces: &[GLenum] = if target == GL_TEXTURE_CUBE_MAP {
            &[
                GL_TEXTURE_CUBE_MAP_POSITIVE_X,
                GL_TEXTURE_CUBE_MAP_POSITIVE_X + 1,
                GL_TEXTURE_CUBE_MAP_POSITIVE_X + 2,
                GL_TEXTURE_CUBE_MAP_POSITIVE_X + 3,
                GL_TEXTURE_CUBE_MAP_POSITIVE_X + 4,
                GL_TEXTURE_CUBE_MAP_POSITIVE_X + 5,
            ]
        } else {
            &[GL_TEXTURE_2D]
        };
        for f in faces {
            glTexImage2D(*f, 0, internal as GLint, w, h, 0, format, type_, texels.as_ptr() as *const c_void);
        }
        glTexParameteri(target, GL_TEXTURE_MIN_FILTER, min as GLint);
        glTexParameteri(target, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
    }
    t
}

fn extension(name: &str) -> bool {
    let has = extension_listed(name);
    if !has {
        eprintln!("skipped: {name} is not advertised");
    }
    has
}

fn extension_listed(name: &str) -> bool {
    // SAFETY: GL_EXTENSIONS is a NUL-terminated string.
    let s = unsafe { CStr::from_ptr(glGetString(GL_EXTENSIONS) as *const c_char) };
    s.to_string_lossy().split(' ').any(|e| e == name)
}

/* ---- the cases ---- */

/// The ES 2.0 texel formats as (name, internal, format, type, needs
/// OES_texture_float).
const FORMATS: [(&str, GLenum, GLenum, GLenum, bool); 6] = [
    ("RGBA/UNSIGNED_BYTE", GL_RGBA, GL_RGBA, GL_UNSIGNED_BYTE, false),
    ("RGB565", GL_RGB, GL_RGB, GL_UNSIGNED_SHORT_5_6_5, false),
    ("RGBA4", GL_RGBA, GL_RGBA, GL_UNSIGNED_SHORT_4_4_4_4, false),
    ("LUMINANCE", GL_LUMINANCE, GL_LUMINANCE, GL_UNSIGNED_BYTE, false),
    ("ALPHA", GL_ALPHA, GL_ALPHA, GL_UNSIGNED_BYTE, false),
    ("RGBA/FLOAT", GL_RGBA, GL_RGBA, GL_FLOAT, true),
];

const SIZE: GLsizei = 8;

/// No draw asked for word 46/47, and the sampler binding of each has depth 1
/// and array size 1 (and, with `expect_size`, the base level's size).
fn check_draws(what: &str, expect_size: bool) {
    let draws = DRAWS.lock().unwrap_or_else(|e| e.into_inner());
    assert!(!draws.is_empty(), "{what}: the draw reached the job builder; {:?}", log_lines());
    for d in draws.iter() {
        assert!(
            !d.uniforms.iter().any(|&(c, _)| c == 46 || c == 47),
            "{what}: ES 2.0 programs never request TEXTURE_DEPTH / _ARRAY_SIZE: {:?}",
            d.uniforms
        );
        assert_eq!(d.vs_tmu, 0, "{what}: the vertex stage samples nothing");
        assert_eq!(d.tmu.len(), 1, "{what}: one sampler binding, unit 0: {:?}", d.tmu);
        let t = d.tmu[0];
        assert_eq!(t.unit, 0, "{what}");
        assert_ne!(t.tex_state, 0, "{what}: a texture record");
        assert_ne!(t.sampler, 0, "{what}: a sampler record");
        if expect_size {
            assert_eq!((t.width, t.height), (SIZE as u32, SIZE as u32), "{what}: the base level's size");
        }
        /* Mesa: depth is a 3D texture's, array size a 2D array's layer count:
         * both 1 for a 2D texture and for a cube (not its six faces) */
        assert_eq!((t.depth, t.array_size), (1, 1), "{what}: the binding's depth and layer count");
    }
}

/// Sanity: an ES 2.0 context.
#[test]
fn the_context_is_es2() {
    let _g = rig();
    let mut m = 0;
    // SAFETY: a plain GL call.
    unsafe { glGetIntegerv(GL_MAJOR_VERSION, &mut m) };
    assert_eq!(err(), GL_INVALID_ENUM, "GL_MAJOR_VERSION is an ES 3.0 query");
}

/// Every ES 2.0 texel format sampled as 2D and cube: accepted, no word 46/47,
/// depth 1 and array size 1.
#[test]
fn textured_draws_carry_no_depth_or_layer_words() {
    let _g = rig();
    let float = extension_listed("GL_OES_texture_float");
    if !float {
        eprintln!("skipped: RGBA/FLOAT, GL_OES_texture_float is not advertised");
    }
    for (target, fs) in [(GL_TEXTURE_2D, FS_TEX), (GL_TEXTURE_CUBE_MAP, FS_CUBE)] {
        for (name, internal, format, ty, needs_float) in FORMATS {
            if needs_float && !float {
                continue;
            }
            let what = format!("{name} {}", if target == GL_TEXTURE_2D { "2D" } else { "cube" });
            DRAWS.lock().unwrap_or_else(|e| e.into_inner()).clear();
            accepted(&what, || {
                program(fs);
                texture(target, internal, format, ty, SIZE, SIZE, GL_NEAREST);
                draw_triangle();
            });
            check_draws(&what, true);
        }
    }
}

/// A mipmapped 2D and cube texture (the filter that reads the level of
/// detail): still no 46/47, and the binding's size is the base level's.
#[test]
fn mipmapped_samplers_carry_no_depth_or_layer_words() {
    let _g = rig();
    for (target, fs) in [(GL_TEXTURE_2D, FS_TEX), (GL_TEXTURE_CUBE_MAP, FS_CUBE)] {
        let what = format!("mipmapped target {target:#x}");
        accepted(&what, || {
            program(fs);
            texture(target, GL_RGBA, GL_RGBA, GL_UNSIGNED_BYTE, SIZE, SIZE, GL_LINEAR_MIPMAP_LINEAR);
            // SAFETY: a plain GL call.
            unsafe { glGenerateMipmap(target) };
            draw_triangle();
        });
        check_draws(&what, true);
    }
}

/// An incomplete texture (a mipmapped filter on a one-level texture) reads
/// black through a 1x1 stand-in: depth and array size are 1 as well.
#[test]
fn incomplete_texture_binding_has_depth_one() {
    let _g = rig();
    for (target, fs) in [(GL_TEXTURE_2D, FS_TEX), (GL_TEXTURE_CUBE_MAP, FS_CUBE)] {
        let what = format!("incomplete target {target:#x}");
        accepted(&what, || {
            program(fs);
            texture(target, GL_RGBA, GL_RGBA, GL_UNSIGNED_BYTE, SIZE, SIZE, GL_LINEAR_MIPMAP_LINEAR);
            draw_triangle();
        });
        check_draws(&what, false);
    }
}

/// EXT_shadow_samplers, if the core lists it (35ea14d6's clamp of a
/// fixed-point depth texture's reference): the draw is accepted.
#[test]
fn shadow_samplers_if_listed_are_accepted() {
    let _g = rig();
    if !extension("GL_EXT_shadow_samplers") || !extension("GL_OES_depth_texture") {
        return;
    }
    let fs = "#extension GL_EXT_shadow_samplers : require\nprecision mediump float;\nvarying vec2 t;\n\
              uniform sampler2DShadowEXT s;\n\
              void main() { gl_FragColor = vec4(shadow2DEXT(s, vec3(t, 1.5))); }\n";
    for ty in [GL_UNSIGNED_SHORT, GL_UNSIGNED_INT] {
        accepted(&format!("shadow look-up of depth type {ty:#x}"), || {
            program(fs);
            texture(GL_TEXTURE_2D, GL_DEPTH_COMPONENT, GL_DEPTH_COMPONENT, ty, SIZE, SIZE, GL_NEAREST);
            // SAFETY: plain GL calls.
            unsafe {
                glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_COMPARE_MODE_EXT, GL_COMPARE_REF_TO_TEXTURE_EXT as GLint);
                glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_COMPARE_FUNC_EXT, GL_LEQUAL as GLint);
            }
            draw_triangle();
        });
    }
}
