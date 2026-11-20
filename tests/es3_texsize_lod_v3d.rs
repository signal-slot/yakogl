// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! `textureSize` with an explicit level of detail 1 and 2 on the V3D (plan
//! #382 item texsizelod, issue #1261; the dEQP-GLES3
//! `texture_functions.texturesize.sampler3d_*` / `sampler2darray_*` cases of
//! image S32b and the 3D `textureGrad` cases of S32b in
//! tools/cts/pi4-gles3-tree.txt).
//!
//! GLSL ES 3.00 8.8: `textureSize(s, lod)` answers the size of the texture's
//! level `lod`: width and height minified, a 3D texture's depth minified, a
//! 2D array's layer count NOT minified (every level has the same number of
//! layers). The four size uniform words (44 width, 45 height, 46 depth, 47
//! array size) carry the LEVEL-0 numbers whatever the lod is: `texture_units`
//! (src/v3d/backend.rs) copies the texture object's base image and the job
//! builder writes `v3d_job_tmu`'s `width`, `height`, `depth`, `array_size`
//! as is (298fe11c, docs/notes/v3d-texture-size-words.md). The minify is the
//! shader's: `Op::TexSize` (src/shaderc/online/v3d/prepare.rs, Mesa's
//! `ntq_emit_txs` / `ntq_minify`) lowers each size lane to
//! `max(word >> lod, 1)` — a logical shift, so a level past the chain lands
//! on 1 — and a 2D array's third lane loads word 47 with no shift at all
//! ("Don't minify the array size"). The same level-0 words scale a 3D
//! `textureGrad`'s gradient lanes (`grad_lod`, prepare.rs), which is why they
//! must stay the base level's.
//!
//! This file pins both halves at lod 1 and 2, over a 16x8x4 volume and a
//! 16x8 array of 5 layers, both with the whole mip chain given level by
//! level, at the draw level and on the QPU interpreter:
//!
//! * the draw (the rig of tests/es3_tex_words46_47_edge_v3d.rs, the job
//!   builder stood in for by this test's `v3d_job_*`, v3d_tex.c built by the
//!   test): in the fragment stage and in the vertex stage, with the lod
//!   written as a constant in the shader or passed as a `uniform int`, the
//!   draw is accepted and the draw's unit-0 `V3dJobTmu` carries (16, 8, 4, 1)
//!   for the `sampler3D` and (16, 8, 1, 5) for the `sampler2DArray` and for
//!   the size-only `sampler2DArrayShadow` — at lod 1 and at lod 2 the same
//!   values, so the words never minify, and the layer count least of all.
//! * the interpreter (`v3d::sim`, the run chain.rs's
//!   `texture_size_of_a_volume_and_an_array` makes for 64x16x8 at levels 0,
//!   1 and 5): the program compiled for the level-0 words (16, 8, 4 / 16, 8,
//!   5), run at lod 1 and 2, returns 8x4x2 and 4x2x1 for the volume — the
//!   depth minified down to 1 — and 8x4x5 and 4x2x5 for the array and for
//!   the shadow array: the layer count never minifies. Revert check: minified
//!   like the other two lanes it would come out 2 at lod 1 and 1 at lod 2,
//!   and a shader that walks the layers would drop half of them.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_texsize_lod_v3d
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
use yakogl::shaderc::online::arena::Arena;
use yakogl::shaderc::online::v3d::chain::{compile_program, Prim, Target};
use yakogl::shaderc::online::v3d::select::{
    QUNIFORM_TEXTURE_ARRAY_SIZE, QUNIFORM_TEXTURE_DEPTH, QUNIFORM_TEXTURE_HEIGHT, QUNIFORM_TEXTURE_WIDTH,
    QUNIFORM_UNIFORM,
};
use yakogl::shaderc::online::v3d::sim::{self, Env, FsInputs, Inputs, VsInputs, LANES};
use yakogl::shaderc::online::{lower, sema};
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
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("es3_texsize_lod_v3d");
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

/// The plain fragment shader of the vertex cases: it reads the varying, so
/// the varying and the vertex stage's look-up stay live.
const PASS_FS: &str =
    "#version 300 es\nprecision highp float;\nin highp vec4 v;\nout vec4 o;\nvoid main() { o = v; }\n";

/// Make `vs` + `fs` the current program, its sampler `s` on unit 0; if
/// `lod` is given, the program's `uniform int lod` takes that value (a level
/// the draw hands the shader rather than one baked into it).
fn program(vs: &str, fs: &str, lod: Option<GLint>) -> GLuint {
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
        if let Some(l) = lod {
            let loc = glGetUniformLocation(p, c"lod".as_ptr());
            assert!(loc >= 0, "the program has a uniform int lod: {vs}{fs}");
            glUniform1i(loc, l);
        }
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

/// The one draw the job builder was handed since the last clear, once the
/// job is done: its FS and its VS sampler bindings.
fn the_draw() -> (Vec<V3dJobTmu>, Vec<V3dJobTmu>) {
    // SAFETY: plain GL call.
    unsafe { glFinish() };
    let d = draws();
    assert_eq!(d.len(), 1, "one draw reached the job builder: {:?}", log_lines());
    d.into_iter().next().unwrap()
}

/* ---- the textures: 16x8, the volume 4 deep, the arrays 5 layers ---- */

const W: usize = 16;
const H: usize = 8;
/// The 3D texture's level-0 depth: 4, 2, 1, 1, 1 down the chain.
const DEPTH: usize = 4;
/// The arrays' layer count, the same at every level.
const LAYERS: usize = 5;
/// Levels 0..=4: 16x8, 8x4, 4x2, 2x1, 1x1 — the whole mip chain.
const LEVELS: usize = 5;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Kind {
    Tex3d,
    Array,
    ArrayShadow,
}

impl Kind {
    fn target(self) -> GLenum {
        match self {
            Kind::Tex3d => GL_TEXTURE_3D,
            Kind::Array | Kind::ArrayShadow => GL_TEXTURE_2D_ARRAY,
        }
    }
    /// The sampler type of the shader querying it.
    fn sampler(self) -> &'static str {
        match self {
            Kind::Tex3d => "sampler3D",
            Kind::Array => "sampler2DArray",
            Kind::ArrayShadow => "sampler2DArrayShadow",
        }
    }
    /// The `(width, height, depth, array_size)` the words 44-47 must carry:
    /// the level-0 numbers, at any lod.
    fn level0(self) -> (u32, u32, u32, u32) {
        match self {
            Kind::Tex3d => (16, 8, 4, 1),
            Kind::Array | Kind::ArrayShadow => (16, 8, 1, 5),
        }
    }
}

/// A complete `kind` texture on unit 0: 16x8, the whole mip chain given
/// level by level (the volume 4 deep halving to 1, the arrays 5 layers at
/// every level), NEAREST filters; the shadow array DEPTH_COMPONENT32F with
/// compare mode, as dEQP's texturesize cases make it.
fn texture(kind: Kind) -> GLuint {
    let target = kind.target();
    let mut t = 0;
    // SAFETY: plain GL calls; null texels (undefined contents) need no storage on our side.
    unsafe {
        glGenTextures(1, &mut t);
        glActiveTexture(GL_TEXTURE0);
        glBindTexture(target, t);
        for level in 0..LEVELS {
            let w = (W >> level).max(1) as GLsizei;
            let h = (H >> level).max(1) as GLsizei;
            let none = core::ptr::null();
            match kind {
                Kind::Tex3d => {
                    let d = (DEPTH >> level).max(1) as GLsizei;
                    glTexImage3D(target, level as GLint, GL_RGBA8 as GLint, w, h, d, 0, GL_RGBA, GL_UNSIGNED_BYTE, none)
                }
                Kind::Array => glTexImage3D(
                    target,
                    level as GLint,
                    GL_RGBA8 as GLint,
                    w,
                    h,
                    LAYERS as GLsizei,
                    0,
                    GL_RGBA,
                    GL_UNSIGNED_BYTE,
                    none,
                ),
                Kind::ArrayShadow => glTexImage3D(
                    target,
                    level as GLint,
                    GL_DEPTH_COMPONENT32F as GLint,
                    w,
                    h,
                    LAYERS as GLsizei,
                    0,
                    GL_DEPTH_COMPONENT,
                    GL_FLOAT,
                    none,
                ),
            }
        }
        glTexParameteri(target, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
        glTexParameteri(target, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
        glTexParameteri(target, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE as GLint);
        glTexParameteri(target, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE as GLint);
        if kind == Kind::ArrayShadow {
            glTexParameteri(target, GL_TEXTURE_COMPARE_MODE, GL_COMPARE_REF_TO_TEXTURE as GLint);
            glTexParameteri(target, GL_TEXTURE_COMPARE_FUNC, GL_LEQUAL as GLint);
        }
    }
    assert_eq!(err(), GL_NO_ERROR, "the upload of {kind:?}: {:?}", log_lines());
    t
}

/// The lod of the shader's `textureSize(s, lod)`: written as a constant, or
/// as the uniform the program declares and the draw sets.
fn lod_arg(constant: bool, lod: GLint) -> String {
    if constant {
        lod.to_string()
    } else {
        "lod".to_string()
    }
}
fn lod_decl(constant: bool) -> &'static str {
    if constant {
        ""
    } else {
        "uniform int lod;\n"
    }
}

/// The fragment-stage query `o = vec4(vec3(textureSize(s, lod)), 1.0)` drawn
/// at lod 1 and at lod 2: the `(width, height, depth, array_size)` each
/// draw's FS unit-0 record carries. Each draw is accepted first (no GL
/// error, no refusal).
fn fs_unit0(kind: Kind, constant_lod: bool) -> [(u32, u32, u32, u32); 2] {
    let t = texture(kind);
    let ty = kind.sampler();
    let mut got = [(0u32, 0u32, 0u32, 0u32); 2];
    for (i, lod) in (1..=2).enumerate() {
        let fs = format!(
            "#version 300 es\nprecision highp float;\nprecision highp {ty};\nuniform {ty} s;\n{}\
             in highp vec4 v;\nout vec4 o;\nvoid main() {{ o = vec4(vec3(textureSize(s, {})), 1.0); }}\n",
            lod_decl(constant_lod),
            lod_arg(constant_lod, lod),
        );
        DRAWS.lock().unwrap_or_else(|e| e.into_inner()).clear();
        LOG.lock().unwrap_or_else(|e| e.into_inner()).clear();
        let p = program(VS, &fs, if constant_lod { None } else { Some(lod) });
        assert_eq!(draw_triangle(), GL_NO_ERROR, "{kind:?} lod {lod}: the draw is accepted: {:?}", log_lines());
        let (fs_tmu, _) = the_draw();
        assert!(!log_lines().iter().any(|l| l.contains("refused")), "{kind:?} lod {lod}: {:?}", log_lines());
        let u = fs_tmu.iter().find(|u| u.unit == 0).unwrap_or_else(|| panic!("{kind:?}: unit 0 bound: {fs_tmu:?}"));
        assert_ne!(u.tex_state, 0, "{kind:?} lod {lod}: the unit has its texture record");
        got[i] = (u.width, u.height, u.depth, u.array_size);
        // SAFETY: a name made above.
        unsafe { glDeleteProgram(p) };
    }
    // SAFETY: a name made above.
    unsafe { glDeleteTextures(1, &t) };
    got
}

/// The same with the query in the vertex stage (`v = vec4(vec3(textureSize(s,
/// lod)), 1.0)`, read by the fragment shader): the VS unit-0 records.
fn vs_unit0(kind: Kind, constant_lod: bool) -> [(u32, u32, u32, u32); 2] {
    let t = texture(kind);
    let ty = kind.sampler();
    let mut got = [(0u32, 0u32, 0u32, 0u32); 2];
    for (i, lod) in (1..=2).enumerate() {
        let vs = format!(
            "#version 300 es\nlayout(location = 0) in highp vec4 a_pos;\nuniform highp {ty} s;\n{}\
             out highp vec4 v;\nvoid main() {{ v = vec4(vec3(textureSize(s, {})), 1.0); gl_Position = a_pos; }}\n",
            lod_decl(constant_lod),
            lod_arg(constant_lod, lod),
        );
        DRAWS.lock().unwrap_or_else(|e| e.into_inner()).clear();
        LOG.lock().unwrap_or_else(|e| e.into_inner()).clear();
        let p = program(&vs, PASS_FS, if constant_lod { None } else { Some(lod) });
        assert_eq!(draw_triangle(), GL_NO_ERROR, "{kind:?} lod {lod}: the draw is accepted: {:?}", log_lines());
        let (_, vs_tmu) = the_draw();
        assert!(!log_lines().iter().any(|l| l.contains("refused")), "{kind:?} lod {lod}: {:?}", log_lines());
        let u = vs_tmu.iter().find(|u| u.unit == 0).unwrap_or_else(|| panic!("{kind:?}: VS unit 0 bound: {vs_tmu:?}"));
        assert_ne!(u.tex_state, 0, "{kind:?} lod {lod}: the unit has its texture record");
        got[i] = (u.width, u.height, u.depth, u.array_size);
        // SAFETY: a name made above.
        unsafe { glDeleteProgram(p) };
    }
    // SAFETY: a name made above.
    unsafe { glDeleteTextures(1, &t) };
    got
}

/* ---- the draws: the words carry level 0 at lod 1 and at lod 2 ---- */

/// A 16x8x4 `sampler3D` queried at a constant lod 1 and 2: the FS unit 0 of
/// both draws carries the LEVEL-0 (16, 8, 4, 1). The words are the texture's
/// base level whatever the level asked for; the shader side minifies them
/// (the interpreter section below runs it).
#[test]
fn a_3d_texture_size_at_a_constant_lod_hands_the_level_0_words_fragment() {
    let _g = rig();
    assert_eq!(fs_unit0(Kind::Tex3d, true), [Kind::Tex3d.level0(); 2]);
}

/// The same with the lod as a `uniform int` the draw sets: the level asked
/// for travels as shader data, not into the texture table.
#[test]
fn a_3d_texture_size_at_a_uniform_lod_hands_the_level_0_words_fragment() {
    let _g = rig();
    assert_eq!(fs_unit0(Kind::Tex3d, false), [Kind::Tex3d.level0(); 2]);
}

/// A 16x8 5-layer `sampler2DArray`: (16, 8, 1, 5) at both lods. The layer
/// count 5 is the SAME at lod 1 and at lod 2 — the words do not minify it,
/// and the shader does not either (prepare.rs, "Don't minify the array
/// size"): every level of an array has the same layers.
#[test]
fn a_2d_array_texture_size_at_a_constant_lod_hands_the_level_0_layer_count_fragment() {
    let _g = rig();
    assert_eq!(fs_unit0(Kind::Array, true), [Kind::Array.level0(); 2], "array_size 5 at lod 1 and at lod 2");
}

/// The uniform-lod form of the same.
#[test]
fn a_2d_array_texture_size_at_a_uniform_lod_hands_the_level_0_layer_count_fragment() {
    let _g = rig();
    assert_eq!(fs_unit0(Kind::Array, false), [Kind::Array.level0(); 2], "array_size 5 at lod 1 and at lod 2");
}

/// The size-only `sampler2DArrayShadow` over a DEPTH_COMPONENT32F array (the
/// RET16_OF_32 exemption of 080240bc, tests/es3_shadow_array_texsize_v3d.rs)
/// queried at a constant lod: the draw is accepted and carries (16, 8, 1, 5)
/// at both lods.
#[test]
fn a_shadow_array_texture_size_at_a_constant_lod_hands_the_level_0_layer_count_fragment() {
    let _g = rig();
    assert_eq!(
        fs_unit0(Kind::ArrayShadow, true),
        [Kind::ArrayShadow.level0(); 2],
        "array_size 5 at lod 1 and at lod 2"
    );
}

/// The uniform-lod form of the same.
#[test]
fn a_shadow_array_texture_size_at_a_uniform_lod_hands_the_level_0_layer_count_fragment() {
    let _g = rig();
    assert_eq!(
        fs_unit0(Kind::ArrayShadow, false),
        [Kind::ArrayShadow.level0(); 2],
        "array_size 5 at lod 1 and at lod 2"
    );
}

/// The vertex stage carries the same rule: the VS unit 0 of a
/// `textureSize(s, lod)` drawn at lod 1 and 2 holds the level-0 words 16, 8,
/// 4 and 1 for a 3D texture (`vs_textures` reach the draw through the same
/// `texture_units` as the fragment stage's).
#[test]
fn a_3d_texture_size_at_a_constant_lod_hands_the_level_0_words_vertex() {
    let _g = rig();
    assert_eq!(vs_unit0(Kind::Tex3d, true), [Kind::Tex3d.level0(); 2]);
}

/// The uniform-lod form of the vertex 3D case.
#[test]
fn a_3d_texture_size_at_a_uniform_lod_hands_the_level_0_words_vertex() {
    let _g = rig();
    assert_eq!(vs_unit0(Kind::Tex3d, false), [Kind::Tex3d.level0(); 2]);
}

/// The vertex `sampler2DArray`: (16, 8, 1, 5) at both lods, the layer count
/// not minified by the level.
#[test]
fn a_2d_array_texture_size_at_a_constant_lod_hands_the_level_0_layer_count_vertex() {
    let _g = rig();
    assert_eq!(vs_unit0(Kind::Array, true), [Kind::Array.level0(); 2], "array_size 5 at lod 1 and at lod 2");
}

/// The uniform-lod form of the vertex array case.
#[test]
fn a_2d_array_texture_size_at_a_uniform_lod_hands_the_level_0_layer_count_vertex() {
    let _g = rig();
    assert_eq!(vs_unit0(Kind::Array, false), [Kind::Array.level0(); 2], "array_size 5 at lod 1 and at lod 2");
}

/// The vertex size-only shadow array: accepted (tests/es3_shadow_array_texsize_v3d.rs
/// drew lod 0; here lod 1 and 2) and carrying (16, 8, 1, 5).
#[test]
fn a_shadow_array_texture_size_at_a_constant_lod_hands_the_level_0_layer_count_vertex() {
    let _g = rig();
    assert_eq!(
        vs_unit0(Kind::ArrayShadow, true),
        [Kind::ArrayShadow.level0(); 2],
        "array_size 5 at lod 1 and at lod 2"
    );
}

/// The uniform-lod form of the vertex shadow array case.
#[test]
fn a_shadow_array_texture_size_at_a_uniform_lod_hands_the_level_0_layer_count_vertex() {
    let _g = rig();
    assert_eq!(
        vs_unit0(Kind::ArrayShadow, false),
        [Kind::ArrayShadow.level0(); 2],
        "array_size 5 at lod 1 and at lod 2"
    );
}

/* ---- the interpreter: the shader minifies the depth, never the layers ---- */

/// One output component as the TLB writes it: the f16 the QPU ends with.
fn hf(x: f32) -> u32 {
    u32::from(sim::f32_to_half(x))
}

/// The uniform stream `sim::run` wants for a program querying `textureSize`
/// over a 16x8 texture whose third word (46 the depth, 47 the layer count)
/// holds `third`: every slot by its contents — a constant's own value, the
/// stage uniform block's dword at the lod uniform's value, a size word's
/// level-0 number. The same mapping chain.rs's
/// `texture_size_of_a_volume_and_an_array` builds.
fn size_stream(uniforms: &[(u32, u32)], third: u32, lod: u32) -> Vec<u32> {
    uniforms
        .iter()
        .map(|&(c, d)| match c {
            0 => d,
            QUNIFORM_UNIFORM => lod,
            QUNIFORM_TEXTURE_WIDTH => 16,
            QUNIFORM_TEXTURE_HEIGHT => 8,
            QUNIFORM_TEXTURE_DEPTH | QUNIFORM_TEXTURE_ARRAY_SIZE => third,
            k => panic!("uniform contents {k} not modelled here"),
        })
        .collect()
}

/// The size words a size query carries: the entries of the uniform list in
/// the texture-size range, as `(contents, data)`.
fn size_words(uniforms: &[(u32, u32)]) -> Vec<(u32, u32)> {
    uniforms
        .iter()
        .copied()
        .filter(|(c, _)| (QUNIFORM_TEXTURE_WIDTH..=QUNIFORM_TEXTURE_ARRAY_SIZE).contains(c))
        .collect()
}

/// Compile `VS` + an FS reading `ivec3 s = textureSize(t, lod_src)` and
/// writing `vec4(vec3(s) / 64.0, 1.0)`, over the level-0 words 16, 8 and
/// `third`, and run the fragment program on the QPU interpreter at the level
/// `lod` (`lod_src` is `lod` for a uniform one, its number for a constant).
/// The lod uniform travels through the stream (contents 1), so one program
/// answers every level; a constant lod folds into the shift. Returns the
/// program's uniform list and the TLB's two half2 registers for every lane.
fn fragment_size_run(
    sampler: &str,
    lod_src: &str,
    third: u32,
    lod: u32,
) -> (Vec<(u32, u32)>, (sim::Lanes, sim::Lanes)) {
    let fs = format!(
        "#version 300 es\nprecision highp float;\nuniform highp {sampler} t;\n{}\
         layout(location = 0) out vec4 o;\nvoid main() {{ ivec3 s = textureSize(t, {lod_src}); o = vec4(vec3(s) / 64.0, 1.0); }}\n",
        if lod_src == "lod" { "uniform int lod;\n" } else { "" },
    );
    let mut mem = vec![0u8; 16 << 20];
    let a = Arena::new(&mut mem);
    let vs = a.alloc_str(VS_INTERP).unwrap();
    let fs = a.alloc_str(&fs).unwrap();
    let v = lower::compile_to_ir(&a, sema::Stage::Vertex, vs).unwrap_or_else(|e| panic!("vertex: {}", e.msg));
    let f = lower::compile_to_ir(&a, sema::Stage::Fragment, fs).unwrap_or_else(|e| panic!("fragment: {}", e.msg));
    let p = compile_program(&v.module, &f.module, Prim::Triangles, Target { vpm_size: 65536 }, &a)
        .unwrap_or_else(|e| panic!("{sampler} {lod_src}: {e:?}"));
    let uniforms: Vec<(u32, u32)> = p.fs.code.uniforms.iter().map(|u| (u.contents, u.data)).collect();
    let stream = size_stream(&uniforms, third, lod);
    let inputs = FsInputs {
        w: [1.0f32.to_bits(); LANES],
        w_centroid: [1.0f32.to_bits(); LANES],
        z: [0; LANES],
        x: [0; LANES],
        y: [0; LANES],
        back_facing: [false; LANES],
        varyings: Vec::new(),
    };
    let env = Env { threads: p.fs.threads, ..Env::default() };
    let o = sim::run(p.fs.code.words, &stream, &Inputs::Fragment(std::boxed::Box::new(inputs)), &env)
        .unwrap_or_else(|e| panic!("{sampler} lod {lod}: the run: {e}"));
    (uniforms, (o.tlb[0].value, o.tlb[1].value))
}

const VS_INTERP: &str = "#version 300 es\nlayout(location = 0) in highp vec4 a_pos;\n\
                         void main() { gl_Position = a_pos; }\n";

/// Assert the two half2 TLB registers carry `w/64, h/64` and `d/64, 1` on
/// every lane: the `textureSize` result the shader hands the output.
fn assert_frag_sizes(tag: &str, lanes: &(sim::Lanes, sim::Lanes), want: (f32, f32, f32)) {
    let want = [hf(want.0 / 64.0) | hf(want.1 / 64.0) << 16, hf(want.2 / 64.0) | hf(1.0) << 16];
    for l in 0..LANES {
        assert_eq!([lanes.0[l], lanes.1[l]], want.map(Some), "{tag}, lane {l}");
    }
}

/// The 16x8x4 volume run at lod 1 and 2 with a `uniform int lod`: the depth
/// word 4 minified like width and height — 4 >> 1 = 2, 4 >> 2 = 1, and the
/// max(word >> lod, 1) clamp is what keeps a level past the chain at 1.
#[test]
fn a_3d_texture_size_at_a_uniform_lod_minifies_the_depth_on_the_interpreter() {
    let _g = rig();
    for (lod, want) in [(1u32, (8.0, 4.0, 2.0)), (2, (4.0, 2.0, 1.0))] {
        let (uniforms, lanes) = fragment_size_run("sampler3D", "lod", 4, lod);
        assert!(uniforms.iter().any(|&(c, d)| c == QUNIFORM_TEXTURE_DEPTH && d == 0), "sampler3D: {uniforms:?}");
        assert!(uniforms.iter().any(|&(c, _)| c == QUNIFORM_UNIFORM), "the lod travels as a uniform: {uniforms:?}");
        assert_frag_sizes(&format!("sampler3D lod {lod}"), &lanes, want);
    }
}

/// The 16x8 array of 5 layers run at lod 1 and 2: width and height minify,
/// the layer count word 47 is loaded with NO shift ("Don't minify the array
/// size", prepare.rs): 5 at both levels. Revert check: minified like the
/// other two lanes it would come out 2 at lod 1 and 1 at lod 2 — a shader
/// looping over the layers would then drop half of them.
#[test]
fn a_2d_array_texture_size_at_a_uniform_lod_never_minifies_the_layer_count_on_the_interpreter() {
    let _g = rig();
    for (lod, want) in [(1u32, (8.0, 4.0, 5.0)), (2, (4.0, 2.0, 5.0))] {
        let (uniforms, lanes) = fragment_size_run("sampler2DArray", "lod", 5, lod);
        assert!(
            uniforms.iter().any(|&(c, d)| c == QUNIFORM_TEXTURE_ARRAY_SIZE && d == 0),
            "sampler2DArray: {uniforms:?}"
        );
        assert_frag_sizes(&format!("sampler2DArray lod {lod}"), &lanes, want);
    }
}

/// The same for the `sampler2DArrayShadow` (GLSL ES 3.00 8.8: its size query
/// answers like the array's; the compare mode plays no part in it).
#[test]
fn a_shadow_array_texture_size_at_a_uniform_lod_never_minifies_the_layer_count_on_the_interpreter() {
    let _g = rig();
    for (lod, want) in [(1u32, (8.0, 4.0, 5.0)), (2, (4.0, 2.0, 5.0))] {
        let (uniforms, lanes) = fragment_size_run("sampler2DArrayShadow", "lod", 5, lod);
        assert!(
            uniforms.iter().any(|&(c, d)| c == QUNIFORM_TEXTURE_ARRAY_SIZE && d == 0),
            "sampler2DArrayShadow: {uniforms:?}"
        );
        assert_frag_sizes(&format!("sampler2DArrayShadow lod {lod}"), &lanes, want);
    }
}

/// A constant lod folds into the shift instead of travelling as a uniform:
/// the uniform list still carries exactly the three level-0 size words
/// (width, height, and the depth or the layer count) at lod 1 and at lod 2,
/// for every sampler — the words the job builder's cases write (298fe11c)
/// are the same no matter which level the shader will divide them by — and
/// the runs give the same minified results.
#[test]
fn constant_lod_programs_carry_the_level_0_words_and_minify_like_the_uniform_ones() {
    let _g = rig();
    for (sampler, third, third_word, want) in [
        ("sampler3D", 4u32, QUNIFORM_TEXTURE_DEPTH, [(8.0, 4.0, 2.0), (4.0, 2.0, 1.0)]),
        ("sampler2DArray", 5u32, QUNIFORM_TEXTURE_ARRAY_SIZE, [(8.0, 4.0, 5.0), (4.0, 2.0, 5.0)]),
        ("sampler2DArrayShadow", 5u32, QUNIFORM_TEXTURE_ARRAY_SIZE, [(8.0, 4.0, 5.0), (4.0, 2.0, 5.0)]),
    ] {
        for (i, want) in want.into_iter().enumerate() {
            let lod = 1 + i as u32;
            let (uniforms, lanes) = fragment_size_run(sampler, &lod.to_string(), third, lod);
            assert_eq!(
                size_words(&uniforms),
                [(QUNIFORM_TEXTURE_WIDTH, 0), (QUNIFORM_TEXTURE_HEIGHT, 0), (third_word, 0)],
                "{sampler} lod {lod}: the level-0 words, data the binding",
            );
            assert!(
                !uniforms.iter().any(|&(c, _)| c == QUNIFORM_UNIFORM),
                "{sampler} lod {lod}: a constant lod is not a uniform: {uniforms:?}"
            );
            assert_frag_sizes(&format!("{sampler} constant lod {lod}"), &lanes, want);
        }
    }
}

/// The vertex stage on the interpreter: `v = vec4(vec3(textureSize(s, lod))
/// / 64.0, 1.0)` run at lod 1 and 2. The volume minifies its depth, the
/// array and the shadow array keep 5 layers — the vertex stage lowers the
/// query through the same `Op::TexSize` as the fragment stage (vspath), and
/// its size words reach the draw's `vs_tmu` the same way.
#[test]
fn a_vertex_texture_size_minifies_the_depth_but_never_the_layer_count_on_the_interpreter() {
    let _g = rig();
    for (sampler, third, want) in [
        ("sampler3D", 4u32, [(8.0, 4.0, 2.0), (4.0, 2.0, 1.0)]),
        ("sampler2DArray", 5u32, [(8.0, 4.0, 5.0), (4.0, 2.0, 5.0)]),
        ("sampler2DArrayShadow", 5u32, [(8.0, 4.0, 5.0), (4.0, 2.0, 5.0)]),
    ] {
        for (i, want) in want.into_iter().enumerate() {
            let lod = 1 + i as u32;
            let got = vertex_size_run(sampler, third, lod);
            assert!(
                size_words(&got.0)
                    .iter()
                    .any(|&(c, d)| (c == QUNIFORM_TEXTURE_DEPTH || c == QUNIFORM_TEXTURE_ARRAY_SIZE) && d == 0),
                "{sampler} lod {lod}: the third word: {:?}",
                got.0
            );
            let want = [want.0 / 64.0, want.1 / 64.0, want.2 / 64.0, 1.0];
            assert_eq!(got.1, want, "{sampler} lod {lod}: the varying v");
        }
    }
}

const FS_VEC4: &str = "#version 300 es\nprecision mediump float;\nin vec4 v;\nout vec4 o;\nvoid main() { o = v; }\n";

/// `vs` sampling `s` run on the interpreter over `attrs` (per attribute
/// location, its components); returns the program's VS uniform list and the
/// varying `v` of lane 0, the last four VPM output rows (es3_shaderc_v3d.rs's
/// `run_vs`).
fn vertex_size_run(sampler: &str, third: u32, lod: u32) -> (Vec<(u32, u32)>, [f32; 4]) {
    let vs = format!(
        "#version 300 es\nlayout(location = 0) in highp vec4 a_pos;\nuniform highp {sampler} s;\nuniform int lod;\n\
         out highp vec4 v;\nvoid main() {{ v = vec4(vec3(textureSize(s, lod)) / 64.0, 1.0); gl_Position = a_pos; }}\n"
    );
    let mut mem = vec![0u8; 16 << 20];
    let a = Arena::new(&mut mem);
    let vs = a.alloc_str(&vs).unwrap();
    let fs = a.alloc_str(FS_VEC4).unwrap();
    let v = lower::compile_to_ir(&a, sema::Stage::Vertex, vs).unwrap_or_else(|e| panic!("vertex: {}", e.msg));
    let f = lower::compile_to_ir(&a, sema::Stage::Fragment, fs).unwrap_or_else(|e| panic!("fragment: {}", e.msg));
    let p = compile_program(&v.module, &f.module, Prim::Triangles, Target { vpm_size: 65536 }, &a)
        .unwrap_or_else(|e| panic!("{sampler} vertex: {e:?}"));
    let uniforms: Vec<(u32, u32)> = p.vs.code.uniforms.iter().map(|u| (u.contents, u.data)).collect();
    let stream: Vec<u32> = uniforms
        .iter()
        .map(|&(c, d)| match c {
            0 => d,
            QUNIFORM_UNIFORM => lod,
            QUNIFORM_TEXTURE_WIDTH => 16,
            QUNIFORM_TEXTURE_HEIGHT => 8,
            QUNIFORM_TEXTURE_DEPTH | QUNIFORM_TEXTURE_ARRAY_SIZE => third,
            2..=5 => 1.0f32.to_bits(),
            k => panic!("uniform contents {k} not modelled here"),
        })
        .collect();
    // the VPM input rows: the a_pos record's components.
    let comps = [1.0f32, 2.0, 3.0, 4.0];
    let mut rows: Vec<[u32; LANES]> = Vec::new();
    for rec in 0..p.attributes.iter().filter(|e| e.1.is_some()).count() {
        assert_eq!(p.attributes.iter().filter(|e| e.1 == Some(rec as u8)).count(), 1, "one attribute at record {rec}");
        for c in comps.iter().take(p.vs.vattr_sizes[rec] as usize) {
            rows.push([c.to_bits(); LANES]);
        }
    }
    let env = Env { threads: p.vs.threads, ..Env::default() };
    let o = sim::run(p.vs.code.words, &stream, &Inputs::Vertex(VsInputs { vpm: rows }), &env)
        .unwrap_or_else(|e| panic!("{sampler} vertex lod {lod}: the run: {e}"));
    let out: Vec<[u32; LANES]> = o.vpm_out.iter().map(|r| r.expect("every output row written")).collect();
    let v = &out[out.len() - 4..];
    (uniforms, core::array::from_fn(|c| f32::from_bits(v[c][0])))
}
