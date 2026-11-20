// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! `textureSize` of a `sampler2DArrayShadow` on the V3D (plan #382 item
//! ret16array, issue #1261; image S32b of tools/cts/pi4-gles3-tree.txt: the
//! last two of its 18 textureSize cases, `texturesize.sampler2darrayshadow_
//! fragment` and `_vertex`). 9c876db3 pinned that a DEPTH_COMPONENT32F 2D
//! array (`GLES_TEXFMT_R32F`) of a size-only shadow-array sampler was refused,
//! verbatim,
//!
//! ```text
//! a texture of 32-bit channels (R32F, RGBA32F, RGBA32UI, RGBA32I) read by a look-up compiled for the 16-bit TMU output
//! ```
//!
//! (`RET16_OF_32` in src/v3d/texture.rs). 080240bc then changed
//! `texture_units` (src/v3d/backend.rs): a bound unit no TMU P0/P1 word names
//! (a `textureSize` lowers to no `Op::Tex`; its words are `TEXTURE_WIDTH`,
//! `TEXTURE_HEIGHT` and `TEXTURE_ARRAY_SIZE`) is checked against its own
//! format's return size, so the return-width refusal no longer applies to it.
//! What this file shows, as tests/es3_shadow_texsize_v3d.rs does for the 2D
//! and cube cases, through the real GL core and the real V3D back end (the
//! job builder stood in for by this test's `v3d_job_*`, v3d_tex.c built by
//! the test):
//!
//! * the draws: a size-only `sampler2DArrayShadow` of an 8x4x3
//!   DEPTH_COMPONENT32F array is accepted in the fragment and in the vertex
//!   stage, and the unit's `V3dJobTmu` carries width and height. The layer
//!   count (word 47) is not asserted here: it is the job builder's, pinned by
//!   tests/es3_tex_words_v3d.rs;
//! * the rule 080240bc left: a 16-bit look-up of the R32F array is still
//!   `RET16_OF_32` (`refusal_for(bind, false)`), pinned at that level, and a
//!   look-up draw (compiled for the 32-bit output by the core) still draws;
//! * the compiler's key and the word-47 stream, as before.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_shadow_array_texsize_v3d
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-v3d", feature = "host-test", not(feature = "report-es2")))]

use core::ffi::{c_char, c_int, c_void, CStr};
use std::collections::BTreeMap;
use std::ffi::CString;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Mutex, MutexGuard, Once, OnceLock};
use yakogl::backend::{GlesTextureBind, GLES_TEXFMT_R32F, GLES_TEXFMT_RGBA8, GLES_TEXTYPE_2D_ARRAY};
use yakogl::gl::consts::*;
use yakogl::gl::context;
use yakogl::gl::state::*;
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps, OsalV3dCpuOf};
use yakogl::shaderc::online::arena::Arena;
use yakogl::shaderc::online::ir::Op;
use yakogl::shaderc::online::v3d::chain::{compile_program, Prim, Target};
use yakogl::shaderc::online::v3d::select::{
    QUNIFORM_TEXTURE_ARRAY_SIZE, QUNIFORM_TMU_CONFIG_P0, QUNIFORM_TMU_CONFIG_P1,
};
use yakogl::shaderc::online::{lower, sema};
use yakogl::v3d::backend::yakogl_backend_v3d;
use yakogl::v3d::ffi::*;
use yakogl::v3d::kick::V3dKick;
use yakogl::v3d::texture;

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
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("es3_shadow_array_texsize_v3d");
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

/// RET16_OF_32, verbatim (src/v3d/texture.rs).
const RET16_OF_32: &str = "a texture of 32-bit channels (R32F, RGBA32F, RGBA32UI, RGBA32I) read by a look-up \
                           compiled for the 16-bit TMU output";

/// The number of `Op::Tex` instructions of a fragment shader reading
/// `sampler` `s` in `body`.
fn tex_ops(sampler: &str, body: &str) -> usize {
    let fs = format!(
        "#version 300 es\nprecision highp float;\nprecision highp {sampler};\nuniform {sampler} s;\n\
         in highp vec3 v_texCoord;\nlayout(location = 0) out highp vec4 o;\nvoid main() {{ {body} }}\n"
    );
    let mut mem = vec![0u8; 16 << 20];
    let a = Arena::new(&mut mem);
    let fs = a.alloc_str(&fs).unwrap();
    let f = lower::compile_to_ir(&a, sema::Stage::Fragment, fs).unwrap_or_else(|e| panic!("fragment: {}", e.msg));
    f.module.instrs.iter().filter(|op| matches!(op, Op::Tex { .. })).count()
}

/// The depth array the dEQP case binds (DEPTH_COMPONENT32F, `layers` layers):
/// R32F in the GL core's storage.
fn depth32(type_: u8, layers: u16) -> GlesTextureBind {
    GlesTextureBind {
        format: GLES_TEXFMT_R32F,
        type_,
        layers,
        base_gpu: 0x1_0000_0000,
        width: 4,
        height: 4,
        stride_texels: 4,
        bytes: 4 * 4 * 4 * layers as u32,
        mip_levels: 1,
        ..GlesTextureBind::ZERO
    }
}

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

const PASS_FS: &str =
    "#version 300 es\nprecision highp float;\nin highp vec4 v;\nout vec4 o;\nvoid main() { o = v; }\n";

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

const SIZE_FS: &str =
    "#version 300 es\nprecision highp float;\nuniform highp sampler2DArrayShadow s;\nin highp vec4 v;\n\
                       out vec4 o;\nvoid main() { o = vec4(vec3(textureSize(s, 0)), 1.0); }\n";
const SIZE_VS: &str = "#version 300 es\nlayout(location = 0) in highp vec4 a_pos;\nuniform highp sampler2DArrayShadow s;\n\
                       out highp vec4 v;\nvoid main() { v = vec4(vec3(textureSize(s, 0)), 1.0); gl_Position = a_pos; }\n";

#[test]
fn the_refusal_text_is_pinned() {
    let why = texture::refusal_for(&depth32(GLES_TEXTYPE_2D_ARRAY, 4), false).expect("refused");
    assert_eq!(why, RET16_OF_32);
    assert!(texture::GAPS.contains(&why), "a backend gap: -ENOTSUP, not -EINVAL");
}

#[test]
fn an_array_shadow_texture_size_has_no_look_up_to_key_the_program_by() {
    let size = "o = vec4(vec3(textureSize(s, 0)), 1.0);";
    assert_eq!(tex_ops("sampler2DArrayShadow", size), 0, "textureSize of a 2D array shadow sampler");
    assert_eq!(
        tex_ops("sampler2DArrayShadow", "o = vec4(texture(s, vec4(v_texCoord, 0.5)));"),
        1,
        "the control: a look-up is a Tex"
    );
}

/// The look-up rule, which stays: a look-up compiled for the 16-bit TMU
/// output (`ret32` false) of the 32-bit depth array is `RET16_OF_32`; the
/// 32-bit key accepts it. Replaces 9c876db3's `..._is_refused_flip_when_
/// ret16_size_is_fixed`; the size-only draws below are what flipped. Kept at
/// this level: the GL core keys a real look-up draw for 32 bits, so no draw
/// reaches the 16-bit key over R32F.
#[test]
fn a_16_bit_look_up_of_a_32_bit_depth_array_is_still_refused_ret16_of_32() {
    let b = depth32(GLES_TEXTYPE_2D_ARRAY, 4);
    assert_eq!(texture::refusal_for(&b, false), Some(RET16_OF_32));
    assert_eq!(texture::refusal_for(&b, true), None, "the 32-bit key accepts the depth array");
    assert!(texture::return_32(&b), "an R32F depth returns 32 bits");
}

/// The flip: a draw whose FS only asks the 8x4x3 DEPTH_COMPONENT32F array's
/// size is accepted, and its unit 0 binding carries width and height.
#[test]
fn a_size_only_array_shadow_sampler_of_a_32_bit_depth_array_draws_its_size_fragment() {
    let _g = rig();
    let t = depth_array(8, 4, 3);
    let p = program(VS, SIZE_FS);
    assert_eq!(draw_triangle(), GL_NO_ERROR, "the draw is accepted: {:?}", log_lines());
    let (fs, _) = the_draw();
    assert!(!log_lines().iter().any(|l| l.contains("refused")), "{:?}", log_lines());
    let u = fs.iter().find(|u| u.unit == 0).unwrap_or_else(|| panic!("unit 0 bound: {fs:?}"));
    assert_eq!((u.width, u.height), (8, 4));
    assert_ne!(u.tex_state, 0, "the unit has its texture record");
    // SAFETY: plain GL calls.
    unsafe {
        glDeleteProgram(p);
        glDeleteTextures(1, &t);
    }
}

/// The same with the size asked in the vertex stage.
#[test]
fn a_size_only_array_shadow_sampler_of_a_32_bit_depth_array_draws_its_size_vertex() {
    let _g = rig();
    let t = depth_array(8, 4, 3);
    let p = program(SIZE_VS, PASS_FS);
    assert_eq!(draw_triangle(), GL_NO_ERROR, "the draw is accepted: {:?}", log_lines());
    let (_, vs) = the_draw();
    assert!(!log_lines().iter().any(|l| l.contains("refused")), "{:?}", log_lines());
    let u = vs.iter().find(|u| u.unit == 0).unwrap_or_else(|| panic!("VS unit 0 bound: {vs:?}"));
    assert_eq!((u.width, u.height), (8, 4));
    assert_ne!(u.tex_state, 0, "the unit has its texture record");
    // SAFETY: plain GL calls.
    unsafe {
        glDeleteProgram(p);
        glDeleteTextures(1, &t);
    }
}

/// The control: a shadow look-up of the same array (the core keys the
/// program for the 32-bit output) still draws.
#[test]
fn an_array_shadow_look_up_of_the_same_array_still_draws() {
    let _g = rig();
    let t = depth_array(8, 4, 3);
    let fs = "#version 300 es\nprecision highp float;\nuniform highp sampler2DArrayShadow s;\nin highp vec4 v;\n\
              out vec4 o;\nvoid main() { o = vec4(texture(s, vec4(v.xy, 1.0, 0.5))); }\n";
    let p = program(VS, fs);
    assert_eq!(draw_triangle(), GL_NO_ERROR, "{:?}", log_lines());
    let (fs, _) = the_draw();
    let u = fs.iter().find(|u| u.unit == 0).expect("unit 0 bound");
    assert_eq!((u.width, u.height), (8, 4));
    // SAFETY: plain GL calls.
    unsafe {
        glDeleteProgram(p);
        glDeleteTextures(1, &t);
    }
}

/// The size program also carries word 47 (the job builder's, tests/es3_tex_words_v3d.rs).
#[test]
fn the_array_size_program_also_carries_word_47() {
    let fs = "#version 300 es\nprecision highp float;\nprecision highp sampler2DArrayShadow;\n\
              uniform sampler2DArrayShadow s;\nlayout(location = 0) out highp vec4 o;\n\
              void main() { o = vec4(vec3(textureSize(s, 0)), 1.0); }\n";
    let vs = "#version 300 es\nin highp vec4 a_position;\nvoid main() { gl_Position = a_position; }\n";
    let mut mem = vec![0u8; 16 << 20];
    let a = Arena::new(&mut mem);
    let vs = a.alloc_str(vs).unwrap();
    let fs = a.alloc_str(fs).unwrap();
    let v = lower::compile_to_ir(&a, sema::Stage::Vertex, vs).unwrap_or_else(|e| panic!("vertex: {}", e.msg));
    let f = lower::compile_to_ir(&a, sema::Stage::Fragment, fs).unwrap_or_else(|e| panic!("fragment: {}", e.msg));
    let p = compile_program(&v.module, &f.module, Prim::Triangles, Target { vpm_size: 65536 }, &a)
        .unwrap_or_else(|e| panic!("program: {e:?}"));
    let c: Vec<u32> = p.fs.code.uniforms.iter().map(|u| u.contents).collect();
    assert!(c.contains(&QUNIFORM_TEXTURE_ARRAY_SIZE), "{c:?}");
}

/// The fragment stream (contents, data) of `body` over a `sampler2DArrayShadow s`.
fn fs_stream(body: &str) -> Vec<(u32, u32)> {
    let fs = format!(
        "#version 300 es\nprecision highp float;\nprecision highp sampler2DArrayShadow;\n\
         uniform sampler2DArrayShadow s;\nin highp vec3 v_texCoord;\n\
         layout(location = 0) out highp vec4 o;\nvoid main() {{ {body} }}\n"
    );
    let vs = "#version 300 es\nin highp vec4 a_position;\nout highp vec3 v_texCoord;\n\
              void main() { v_texCoord = a_position.xyz; gl_Position = a_position; }\n";
    let mut mem = vec![0u8; 16 << 20];
    let a = Arena::new(&mut mem);
    let vs = a.alloc_str(vs).unwrap();
    let fs = a.alloc_str(&fs).unwrap();
    let v = lower::compile_to_ir(&a, sema::Stage::Vertex, vs).unwrap_or_else(|e| panic!("vertex: {}", e.msg));
    let f = lower::compile_to_ir(&a, sema::Stage::Fragment, fs).unwrap_or_else(|e| panic!("fragment: {}", e.msg));
    let p = compile_program(&v.module, &f.module, Prim::Triangles, Target { vpm_size: 65536 }, &a)
        .unwrap_or_else(|e| panic!("program: {e:?}"));
    p.fs.code.uniforms.iter().map(|u| (u.contents, u.data)).collect()
}

/// Whether the stream has a TMU P0/P1 word for unit 0 (what `texture_units`
/// calls "looked up": the unit is not size-only, so 080240bc's exemption does
/// not reach it).
fn looks_up_unit0(w: &[(u32, u32)]) -> bool {
    w.iter().any(|&(c, d)| (c == QUNIFORM_TMU_CONFIG_P0 || c == QUNIFORM_TMU_CONFIG_P1) && d >> 24 == 0)
}

/// Criterion c3 of issue #1261: the exemption 080240bc made, and 7b6a4f0c
/// pinned for the size-only unit, is for a unit that only queries. A unit
/// that is also sampled has a P0/P1 word, so `texture_units` does not exempt
/// it: it is checked against the key its look-up was compiled for, and a
/// 16-bit key over the 32-bit depth array is `RET16_OF_32` (`refusal_for`,
/// decided in `tex_state_for` before any uniform word is written, so the
/// missing word 47 does not mix in). A draw cannot reach the 16-bit key here
/// (the core keys a real look-up for 32 bits), so this is pinned at the level
/// that tells the reasons apart: the stream's classification and the rule.
#[test]
fn a_sampled_and_queried_array_shadow_unit_of_a_32_bit_depth_array_keeps_ret16_of_32() {
    let both = fs_stream("o = vec4(texture(s, vec4(v_texCoord, 0.5))) + vec4(vec3(textureSize(s, 0)), 1.0);");
    assert!(both.iter().any(|&(c, _)| c == QUNIFORM_TEXTURE_ARRAY_SIZE), "it is queried: {both:?}");
    assert!(looks_up_unit0(&both), "it is sampled, so not size-only: {both:?}");
    let size_only = fs_stream("o = vec4(vec3(textureSize(s, 0)), 1.0);");
    assert!(!looks_up_unit0(&size_only), "the size-only unit has no P0/P1 word: {size_only:?}");

    let b = depth32(GLES_TEXTYPE_2D_ARRAY, 4);
    assert_eq!(texture::refusal_for(&b, false), Some(RET16_OF_32));
    assert_eq!(texture::refusal_for(&b, true), None);
}

/// The control: a sampling-only unit of the same array has a look-up word and
/// no size word, and the 16-bit key is `RET16_OF_32`.
#[test]
fn a_sampling_only_array_shadow_unit_of_a_32_bit_depth_array_keeps_ret16_of_32() {
    let s = fs_stream("o = vec4(texture(s, vec4(v_texCoord, 0.5)));");
    assert!(looks_up_unit0(&s), "{s:?}");
    assert!(!s.iter().any(|&(c, _)| c == QUNIFORM_TEXTURE_ARRAY_SIZE), "{s:?}");
    assert_eq!(texture::refusal_for(&depth32(GLES_TEXTYPE_2D_ARRAY, 4), false), Some(RET16_OF_32));
}

#[test]
fn a_16_bit_format_is_not_refused_that_way() {
    let b = GlesTextureBind { format: GLES_TEXFMT_RGBA8, ..depth32(GLES_TEXTYPE_2D_ARRAY, 4) };
    assert_eq!(texture::refusal_for(&b, false), None);
}
