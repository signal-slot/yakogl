// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Size-only `sampler3D` and `sampler2DArray` units over 32-bit (R32F)
//! textures on the V3D (plan #382 item r16plain, issue #1261; image S32b of
//! tools/cts/pi4-gles3-tree.txt: the `texturesize.sampler3d_*` and
//! `texturesize.sampler2darray_*` cases). 080240bc exempted a bound unit no
//! TMU P0/P1 word names — a `textureSize` lowers to no `Op::Tex`, and its
//! words are size words alone — from the return-size rule RET16_OF_32
//! (src/v3d/texture.rs, `texture_units` in src/v3d/backend.rs); 298fe11c
//! (words4647) made `write_uniforms` in driver/src/v3d/v3d_job.c write the
//! 3D depth and the layer count those units' `textureSize` streams ask for
//! (`V3D_QUNIFORM_TEXTURE_DEPTH` 46, `_ARRAY_SIZE` 47, from `v3d_job_tmu`'s
//! `depth` and `array_size`, filled in `texture_units` from the binding's
//! `layers`). Before that, every size-only 3D or 2D-array draw ended in
//! `the job builder refused the draw (a uniform, program or primitive it
//! does not write)` (`Attempt::Unsupported` after `v3d_job_draw`,
//! GL_INVALID_OPERATION); the pinning tests flipped in the same commit
//! (tests/es3_tex_words_v3d.rs, tests/es3_texsize3d_v3d.rs).
//!
//! What this file shows, through the real GL core and the real V3D back end
//! (the job builder stood in for by this test's `v3d_job_*`, v3d_tex.c built
//! by the test, as in tests/es3_shadow_array_texsize_v3d.rs):
//!
//! * the four draws: a `textureSize`-only `sampler3D` of a 4x4x3 R32F
//!   `GL_TEXTURE_3D` and a `textureSize`-only `sampler2DArray` of a 4x4x3
//!   R32F `GL_TEXTURE_2D_ARRAY`, each in the fragment and in the vertex
//!   stage, are accepted — no GL error, no refusal line, and the unit 0
//!   record reaches the job builder with its texture record, its 4x4 base
//!   level, and the value the size word reads: `depth` 3 for the 3D texture
//!   (word 46) and `array_size` 3 for the array (word 47), 1 the other way
//!   around (v3d_job.c `find_tmu` copies those fields into the uniform;
//!   tools/v3d/job_check.c checks the words it writes);
//! * the streams: the stage that asks carries the size word with the sampler
//!   binding as its `data` and no other depth/array word, and carries no TMU
//!   P0/P1 word for unit 0 — the size-only classification 080240bc exempts.
//!
//! The sampling side is not here: units that sample an R32F 3D or 2D-array
//! texture (the key those draws get and RET16_OF_32 at the predicate level)
//! are pinned by tests/es3_sampling_r32f_ret16_v3d.rs, and the shadow-array
//! size case by tests/es3_shadow_array_texsize_v3d.rs; the stream contents
//! and the C switch by tests/es3_texsize3d_v3d.rs and
//! tests/es3_tex_words_v3d.rs. This file carries the accepted draw across
//! to plain 3D and 2D-array samplers, both stages, with the values carried.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_size_only_r32f_v3d
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
use yakogl::shaderc::online::v3d::select::{QUNIFORM_TEXTURE_ARRAY_SIZE, QUNIFORM_TEXTURE_DEPTH};
use yakogl::v3d::backend::yakogl_backend_v3d;
use yakogl::v3d::ffi::*;
use yakogl::v3d::kick::V3dKick;

/* ---- the job builder: records each draw's sampler bindings and streams ---- */

static LOG: Mutex<Vec<String>> = Mutex::new(Vec::new());
/// One draw handed to the job builder: its FS and VS sampler bindings, and
/// the uniform streams of its fragment, vertex and coordinate programs (the
/// words the C job builder would write from these records, kept as the
/// `V3dUniform`s of the draw).
#[derive(Clone)]
struct Draw {
    tmu: Vec<V3dJobTmu>,
    vs_tmu: Vec<V3dJobTmu>,
    fs: Vec<V3dUniform>,
    vs: Vec<V3dUniform>,
    cs: Vec<V3dUniform>,
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

/// `count` uniform words at `p`, or none (the C reads the program streams
/// through `v3d_job_draw`'s `fs`/`vs`/`cs`; the fake copies them).
fn words(p: *const V3dUniform, n: u32) -> Vec<V3dUniform> {
    if p.is_null() || n == 0 {
        Vec::new()
    } else {
        // SAFETY: a program's stream of `n` words, live for the call.
        unsafe { std::slice::from_raw_parts(p, n as usize) }.to_vec()
    }
}

/// A fragment program's stream, or none for a null record.
fn fs_words(p: *const V3dFsProg) -> Vec<V3dUniform> {
    if p.is_null() {
        Vec::new()
    } else {
        // SAFETY: the draw's live program record.
        let p = unsafe { &*p };
        words(p.uniforms, p.num_uniforms)
    }
}

/// A vertex or coordinate program's stream, or none for a null record.
fn vs_prog_words(p: *const V3dVsProg) -> Vec<V3dUniform> {
    if p.is_null() {
        Vec::new()
    } else {
        // SAFETY: the draw's live program record.
        let p = unsafe { &*p };
        words(p.uniforms, p.num_uniforms)
    }
}

#[no_mangle]
unsafe extern "C" fn v3d_job_draw(j: *mut V3dJob, d: *const V3dJobDraw, _layout: *mut c_void) -> i32 {
    // SAFETY: the backend passes its live job and a draw whose pointers live for the call.
    let d = unsafe { &*d };
    let draw = Draw {
        tmu: records(d.tmu, d.num_tmu),
        vs_tmu: records(d.vs_tmu, d.num_vs_tmu),
        fs: fs_words(d.fs),
        vs: vs_prog_words(d.vs),
        cs: vs_prog_words(d.cs),
    };
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
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("es3_size_only_r32f_v3d");
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

/// The one draw since `rig`, once the job is done.
fn the_draw() -> Draw {
    // SAFETY: plain GL call.
    unsafe { glFinish() };
    let d = draws();
    assert_eq!(d.len(), 1, "one draw reached the job builder: {:?}", log_lines());
    d.into_iter().next().unwrap()
}

/// A `w x h x d` R32F texture (`GL_TEXTURE_3D` or `GL_TEXTURE_2D_ARRAY`) on
/// unit 0: the image dEQP's texturesize cases make, NEAREST, one level.
fn r32f_volume(target: GLenum, w: usize, h: usize, d: usize) -> GLuint {
    let texels: Vec<f32> = (0..w * h * d).map(|i| i as f32 / (w * h * d) as f32).collect();
    let mut t = 0;
    // SAFETY: plain GL calls; the texels outlive the upload.
    unsafe {
        glGenTextures(1, &mut t);
        glActiveTexture(GL_TEXTURE0);
        glBindTexture(target, t);
        glTexImage3D(
            target,
            0,
            GL_R32F as GLint,
            w as GLsizei,
            h as GLsizei,
            d as GLsizei,
            0,
            GL_RED,
            GL_FLOAT,
            texels.as_ptr() as *const c_void,
        );
        glTexParameteri(target, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
        glTexParameteri(target, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
        glTexParameteri(target, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE as GLint);
        glTexParameteri(target, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE as GLint);
    }
    assert_eq!(err(), GL_NO_ERROR, "the upload: {:?}", log_lines());
    t
}

/// The GL error and log of a `w x h x d` R32F `target` texture asked its
/// size by `vs` + `fs`, and the draw the job builder was handed.
fn size_draw(target: GLenum, w: usize, h: usize, d: usize, vs: &str, fs: &str) -> (GLenum, Vec<String>, Draw) {
    let t = r32f_volume(target, w, h, d);
    let p = program(vs, fs);
    let e = draw_triangle();
    let lines = log_lines();
    let draw = the_draw();
    // SAFETY: plain GL calls.
    unsafe {
        glDeleteProgram(p);
        glDeleteTextures(1, &t);
    }
    (e, lines, draw)
}

/// The `(contents, data)` pairs of `streams`.
fn stream_words(streams: &[&[V3dUniform]]) -> Vec<(u32, u32)> {
    streams.iter().flat_map(|s| s.iter()).map(|w| (w.contents, w.data)).collect()
}

/// What the asking stage's streams carry: `word` once, its `data` the
/// sampler binding 0; no other depth/array word; and no TMU P0/P1 word for
/// unit 0 — a unit only `textureSize` asks of, which 080240bc exempts from
/// the return-size rule (the `looked_up` set in `texture_units`).
fn assert_size_only_stream(streams: &[&[V3dUniform]], word: u32, other: u32, what: &str) {
    let w = stream_words(streams);
    let held: Vec<(u32, u32)> = w.iter().copied().filter(|&(c, _)| c == word).collect();
    assert_eq!(held.len(), 1, "{what}: the stream carries word `{word}` once: {w:?}");
    assert_eq!(held[0].1, 0, "{what}: the size word's data is the sampler binding: {w:?}");
    assert!(
        !w.iter().any(|&(c, _)| c == other),
        "{what}: a 3D size asks no layer count, an array's none a depth: {w:?}"
    );
    assert!(
        !w.iter().any(|&(c, d)| (c == V3D_QUNIFORM_TMU_CONFIG_P0 || c == V3D_QUNIFORM_TMU_CONFIG_P1) && d >> 24 == 0),
        "{what}: a look-up would name unit 0 with a P0/P1 word: {w:?}"
    );
}

/// The unit 0 record the job builder was handed for `what`: its texture
/// record, its 4x4 base level, and the value the size word reads from it —
/// `depth` for word 46, `array_size` for word 47 (driver/src/v3d/v3d_job.c
/// `find_tmu`); the width and height words read `width` and `height`.
fn assert_unit0(tmu: &[V3dJobTmu], depth: u32, array_size: u32, what: &str) {
    let u = tmu.iter().find(|u| u.unit == 0).unwrap_or_else(|| panic!("{what}: unit 0 bound: {tmu:?}"));
    assert_ne!(u.tex_state, 0, "{what}: the unit has its texture record");
    assert_eq!((u.width, u.height), (4, 4), "{what}: the base level the width and height words read");
    assert_eq!(u.depth, depth, "{what}: word 46 reads the record's `depth`");
    assert_eq!(u.array_size, array_size, "{what}: word 47 reads the record's `array_size`");
}

const SIZE3D_FS: &str = "#version 300 es\nprecision highp float;\nuniform highp sampler3D s;\nin highp vec4 v;\n\
                          out vec4 o;\nvoid main() { o = vec4(vec3(textureSize(s, 0)), 1.0); }\n";
const SIZE3D_VS: &str = "#version 300 es\nlayout(location = 0) in highp vec4 a_pos;\nuniform highp sampler3D s;\n\
                          out highp vec4 v;\nvoid main() { v = vec4(vec3(textureSize(s, 0)), 1.0); gl_Position = a_pos; }\n";
const SIZEARRAY_FS: &str =
    "#version 300 es\nprecision highp float;\nuniform highp sampler2DArray s;\nin highp vec4 v;\n\
                             out vec4 o;\nvoid main() { o = vec4(vec3(textureSize(s, 0)), 1.0); }\n";
const SIZEARRAY_VS: &str = "#version 300 es\nlayout(location = 0) in highp vec4 a_pos;\n\
                            uniform highp sampler2DArray s;\nout highp vec4 v;\n\
                            void main() { v = vec4(vec3(textureSize(s, 0)), 1.0); gl_Position = a_pos; }\n";

/// The flip words4647 made: a `textureSize`-only `sampler3D` of a 4x4x3 R32F
/// 3D texture is accepted in the fragment stage — the exemption of 080240bc
/// kept the RET16_OF_32 rule off the unit, and the depth word 46 the stream
/// asks for is one the job builder now writes, from the record's `depth`.
#[test]
fn a_size_only_sampler3d_of_an_r32f_3d_texture_draws_its_depth_fragment() {
    let _g = rig();
    let (e, lines, d) = size_draw(GL_TEXTURE_3D, 4, 4, 3, VS, SIZE3D_FS);
    assert_eq!(e, GL_NO_ERROR, "the draw is accepted: {lines:?}");
    assert!(!lines.iter().any(|l| l.contains("refused")), "{lines:?}");
    assert_size_only_stream(&[&d.fs], QUNIFORM_TEXTURE_DEPTH, QUNIFORM_TEXTURE_ARRAY_SIZE, "FS 3D");
    assert_unit0(&d.tmu, 3, 1, "FS 3D");
}

/// The same with the size asked in the vertex stage.
#[test]
fn a_size_only_sampler3d_of_an_r32f_3d_texture_draws_its_depth_vertex() {
    let _g = rig();
    let (e, lines, d) = size_draw(GL_TEXTURE_3D, 4, 4, 3, SIZE3D_VS, PASS_FS);
    assert_eq!(e, GL_NO_ERROR, "the draw is accepted: {lines:?}");
    assert!(!lines.iter().any(|l| l.contains("refused")), "{lines:?}");
    assert_size_only_stream(&[&d.vs, &d.cs], QUNIFORM_TEXTURE_DEPTH, QUNIFORM_TEXTURE_ARRAY_SIZE, "VS 3D");
    assert_unit0(&d.vs_tmu, 3, 1, "VS 3D");
}

/// The layer-count half of the flip: a `textureSize`-only `sampler2DArray`
/// of a 4x4x3 R32F array, fragment stage; word 47 from the record's
/// `array_size`.
#[test]
fn a_size_only_sampler2darray_of_an_r32f_array_texture_draws_its_layer_count_fragment() {
    let _g = rig();
    let (e, lines, d) = size_draw(GL_TEXTURE_2D_ARRAY, 4, 4, 3, VS, SIZEARRAY_FS);
    assert_eq!(e, GL_NO_ERROR, "the draw is accepted: {lines:?}");
    assert!(!lines.iter().any(|l| l.contains("refused")), "{lines:?}");
    assert_size_only_stream(&[&d.fs], QUNIFORM_TEXTURE_ARRAY_SIZE, QUNIFORM_TEXTURE_DEPTH, "FS array");
    assert_unit0(&d.tmu, 1, 3, "FS array");
}

/// The same in the vertex stage.
#[test]
fn a_size_only_sampler2darray_of_an_r32f_array_texture_draws_its_layer_count_vertex() {
    let _g = rig();
    let (e, lines, d) = size_draw(GL_TEXTURE_2D_ARRAY, 4, 4, 3, SIZEARRAY_VS, PASS_FS);
    assert_eq!(e, GL_NO_ERROR, "the draw is accepted: {lines:?}");
    assert!(!lines.iter().any(|l| l.contains("refused")), "{lines:?}");
    assert_size_only_stream(&[&d.vs, &d.cs], QUNIFORM_TEXTURE_ARRAY_SIZE, QUNIFORM_TEXTURE_DEPTH, "VS array");
    assert_unit0(&d.vs_tmu, 1, 3, "VS array");
}
