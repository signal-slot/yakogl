// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! ES 2.0 context guard for 4a7b5174 (shaderc: gl_FragData[0] read back is
//! held in a slot, as gl_FragColor is), on the V3D 4.2 (Raspberry Pi 4). The whole dEQP-GLES2
//! tree of the Raspberry Pi 4 (46f93767) predates it, and its stage-1 promise is 0 Fail and only
//! the 34 NotSupported of tools/cts/es2-accepted.txt. 4a7b5174 changes the
//! lowering of every GLSL ES 1.00 fragment shader that writes gl_FragData[0]
//! (dEQP-GLES2 shaders.fragdata.*, shaders.builtin_variable.* and several
//! shaders.* cases use it); the answers asserted here come from GLSL ES 1.00
//! section 7.2 and dEQP-GLES2 (fragdata.test, es2fShaderFragDataTests.cpp),
//! not from the current output:
//!
//! - written with a constant, written then read and modified (`*=`, a
//!   partial write), written in a branch, written in a loop, and read back
//!   at a run-time index (`valid_dynamic_index`): compile and link, and the
//!   one colour output is stored once (the front end `compile_to_ir`, the
//!   same lowering both back ends take);
//! - `gl_FragColor` and `gl_FragData` in one shader, also in the arms of a
//!   static `if` or in a function main never calls: the compile fails;
//! - a constant index 1 fails (one draw buffer in an ES 2.0 report).
//! - a draw with a program of each valid shader is accepted: no GL error and
//!   `v3d_refused_draws()` unchanged (the rig of tests/v3d_es2ctx_refusals.rs:
//!   job builder stood in for, the real V3D back end and compiler).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table,report-es2 --test v3d_es2ctx_drift_fragdata
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
unsafe extern "C" fn v3d_job_draw(j: *mut V3dJob, _d: *const V3dJobDraw, _layout: *mut c_void) -> i32 {
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
    g
}

use yakogl::gl::buffer::glBindBuffer;
use yakogl::gl::draw::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::vertex::{glEnableVertexAttribArray, glVertexAttribPointer};
use yakogl::shaderc::online::lower::compile_to_ir;
use yakogl::shaderc::online::{arena::Arena, ir, sema::Stage};
use yakogl::v3d::backend::v3d_refused_draws;

const W: GLsizei = 32;
const H: GLsizei = 32;

/// The fragment shaders that must compile and link: (name, source).
const VALID: [(&str, &str); 6] = [
    (
        "constant",
        "precision mediump float;\nvarying vec4 v_color;\nvoid main (void)\n{\n    gl_FragData[0] = vec4(0.25, 0.5, 0.75, 1.0);\n}\n",
    ),
    (
        "read_modify_write",
        "precision mediump float;\nvarying vec4 v_color;\nvoid main (void)\n{\n    gl_FragData[0] = v_color;\n    gl_FragData[0].rgb *= 0.5;\n}\n",
    ),
    (
        "read_operand",
        "precision mediump float;\nvarying vec4 v_color;\nvoid main (void)\n{\n    gl_FragData[0] = vec4(1.0, 0.5, 0.25, 1.0);\n    gl_FragData[0] = gl_FragData[0] * v_color;\n}\n",
    ),
    (
        "branch",
        "precision mediump float;\nvarying vec4 v_color;\nvoid main (void)\n{\n    if (v_color.x > 0.5)\n        gl_FragData[0] = v_color;\n    else\n        gl_FragData[0] = vec4(0.0, 1.0, 0.0, 1.0);\n    gl_FragData[0].a *= 0.5;\n}\n",
    ),
    (
        "loop",
        "precision mediump float;\nvarying vec4 v_color;\nvoid main (void)\n{\n    gl_FragData[0] = vec4(0.0);\n    for (int i = 0; i < 4; i++)\n        gl_FragData[0] += v_color * 0.25;\n}\n",
    ),
    (
        "dynamic_index",
        "precision mediump float;\nvarying vec4 v_color;\nvarying float v_index;\nvoid main (void)\n{\n    gl_FragData[0] = v_color;\n    gl_FragData[int(v_index)] = gl_FragData[int(v_index)] * 0.5;\n}\n",
    ),
];

/// A shader that uses both outputs: GLSL ES 1.00 section 7.2 makes it a
/// compile error (fragdata.test write_fragcolor_and_fragdata_*).
const BOTH: [(&str, &str); 4] = [
    ("simple", "void main (void)\n{\n    gl_FragColor = vec4(1.0);\n    gl_FragData[0] = vec4(1.0);\n}\n"),
    (
        "static_if",
        "void main (void)\n{\n    if (false)\n        gl_FragColor = vec4(1.0);\n    else\n        gl_FragData[0] = vec4(1.0);\n}\n",
    ),
    (
        "unused_func",
        "void unused (void)\n{\n    gl_FragData[0] = vec4(1.0);\n}\nvoid main (void)\n{\n    gl_FragColor = vec4(1.0);\n}\n",
    ),
    (
        "read_back_beside_fragcolor",
        "precision mediump float;\nvoid main (void)\n{\n    gl_FragColor = vec4(0.5);\n    gl_FragData[0] *= 0.5;\n}\n",
    ),
];

/// Index 1 with one draw buffer (fragdata.test invalid_assign_to_1), also
/// beside a read-back of element 0.
const INDEX_1: [(&str, &str); 2] = [
    ("assign", "void main (void)\n{\n    gl_FragData[1] = vec4(1.0);\n}\n"),
    ("read_back_then_1", "void main (void)\n{\n    gl_FragData[0] *= 0.5;\n    gl_FragData[1] = vec4(1.0);\n}\n"),
];

const VS: &str = "attribute vec4 p;\nvarying vec4 v_color;\nvarying float v_index;\nvoid main (void)\n{\n    gl_Position = p;\n    v_color = p * 0.5 + 0.5;\n    v_index = 0.0;\n}\n";

/// Compile `src` as a fragment shader through the GL API; the compile status.
fn compiles(src: &str) -> bool {
    let text = CString::new(src).unwrap();
    // SAFETY: plain GL calls; one NUL-terminated source.
    unsafe {
        let s = glCreateShader(GL_FRAGMENT_SHADER);
        let p: [*const c_char; 1] = [text.as_ptr()];
        glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
        glCompileShader(s);
        let mut ok: GLint = 0;
        glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
        glDeleteShader(s);
        ok == 1
    }
}

/// Compile and link `fs` with [`VS`]; the link status, its log on a failure.
fn links(fs: &str) -> Result<(), String> {
    let (vs, fs) = (CString::new(VS).unwrap(), CString::new(fs).unwrap());
    // SAFETY: plain GL calls; NUL-terminated sources, a log buffer of the size announced.
    unsafe {
        let mk = |ty: GLenum, t: &CString| {
            let s = glCreateShader(ty);
            let p: [*const c_char; 1] = [t.as_ptr()];
            glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
            glCompileShader(s);
            let mut ok: GLint = 0;
            glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
            (s, ok == 1)
        };
        let ((v, vok), (f, fok)) = (mk(GL_VERTEX_SHADER, &vs), mk(GL_FRAGMENT_SHADER, &fs));
        assert!(vok, "the vertex shader compiles");
        if !fok {
            return Err("the fragment shader does not compile".into());
        }
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glBindAttribLocation(p, 0, c"p".as_ptr());
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        let mut buf = [0 as c_char; 1024];
        let mut len: GLsizei = 0;
        glGetProgramInfoLog(p, buf.len() as GLsizei, &mut len, buf.as_mut_ptr());
        let log = CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned();
        glDeleteShader(v);
        glDeleteShader(f);
        glDeleteProgram(p);
        if ok == 1 {
            Ok(())
        } else {
            Err(log)
        }
    }
}

/// The number of stores to colour output 0 the front end lowers `src` to
/// (Err: its compile message). The lowering is the one both back ends take.
fn stores(src: &str) -> Result<usize, String> {
    let mut buf = vec![0u8; 1 << 20];
    let a = Arena::new(&mut buf);
    match compile_to_ir(&a, Stage::Fragment, src) {
        Ok(l) => Ok(l
            .module
            .instrs
            .iter()
            .filter(|o| matches!(o, ir::Op::StoreOutput { slot: ir::IoSlot::FragColor(0), .. }))
            .count()),
        Err(e) => Err(e.msg.to_string()),
    }
}

#[test]
fn fragdata_shaders_compile_and_link() {
    let _g = rig();
    for (name, fs) in VALID {
        if let Err(log) = links(fs) {
            panic!("{name} does not link: {log}");
        }
        // SAFETY: a plain GL call.
        assert_eq!(unsafe { glGetError() }, GL_NO_ERROR, "{name}");
    }
}

/// Written then read and modified, in a branch, in a loop or at a run-time
/// index: the one colour output is stored once, at the end of main.
#[test]
fn fragdata_output_is_stored_once() {
    for (name, fs) in VALID {
        assert_eq!(stores(fs), Ok(1), "{name}");
    }
}

/// Masked and swizzled partial writes after a whole write compile, link, and
/// store once.
#[test]
fn partial_writes_compile() {
    let _g = rig();
    for body in
        ["gl_FragData[0].rgb *= 0.5;", "gl_FragData[0].a = gl_FragData[0].r;", "gl_FragData[0].xy = gl_FragData[0].yx;"]
    {
        let fs = format!(
            "precision mediump float;\nvarying vec4 v_color;\nvoid main (void)\n{{\n    gl_FragData[0] = v_color;\n    {body}\n}}\n"
        );
        assert_eq!(stores(&fs), Ok(1), "{body}");
        assert_eq!(links(&fs), Ok(()), "{body}");
    }
}

#[test]
fn fragcolor_and_fragdata_do_not_compile() {
    let _g = rig();
    for (name, fs) in BOTH {
        assert!(!compiles(fs), "{name} compiled");
        assert!(stores(fs).is_err(), "{name} lowered");
    }
}

#[test]
fn fragdata_index_1_does_not_compile() {
    let _g = rig();
    for (name, fs) in INDEX_1 {
        assert!(!compiles(fs), "{name} compiled");
        let m = stores(fs).expect_err(name);
        assert!(m.contains("one draw buffer"), "{name}: {m}");
    }
}

static TRI: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 3.0, -1.0, 0.0, 1.0, -1.0, 3.0, 0.0, 1.0];

/// Link each valid shader and draw a triangle with it: no GL error and no
/// draw refused by the back end.
#[test]
fn fragdata_shaders_draw() {
    let _g = rig();
    for (name, fs) in VALID {
        let (vs, fsc) = (CString::new(VS).unwrap(), CString::new(fs).unwrap());
        let base = v3d_refused_draws();
        // SAFETY: plain GL calls; TRI is static and outlives the draw.
        unsafe {
            let mk = |ty: GLenum, t: &CString| {
                let s = glCreateShader(ty);
                let p: [*const c_char; 1] = [t.as_ptr()];
                glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
                glCompileShader(s);
                s
            };
            let p = glCreateProgram();
            glAttachShader(p, mk(GL_VERTEX_SHADER, &vs));
            glAttachShader(p, mk(GL_FRAGMENT_SHADER, &fsc));
            glBindAttribLocation(p, 0, c"p".as_ptr());
            glLinkProgram(p);
            let mut ok: GLint = 0;
            glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
            assert_eq!(ok, 1, "{name} links: {:?}", log_lines());
            glUseProgram(p);
            glViewport(0, 0, W, H);
            glBindBuffer(GL_ARRAY_BUFFER, 0);
            glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, TRI.as_ptr() as *const c_void);
            glEnableVertexAttribArray(0);
            glDrawArrays(GL_TRIANGLES, 0, 3);
            glFinish();
        }
        let n = v3d_refused_draws().wrapping_sub(base);
        // SAFETY: a plain GL call.
        let e = unsafe { glGetError() };
        assert_eq!((e, n), (GL_NO_ERROR, 0), "{name}: error, refused draws; {:?}", log_lines());
    }
}
