// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Host guards for the causes behind the eleven refusals that stopped the
//! Pi 4's dEQP-GLES3 images again at b1b5b845 (plan #398, issue #2;
//! docs/notes/v3d-refusals.md section F), through the GL core, the V3D
//! back end and the REAL job builder, frame and layout code of
//! `driver/src/v3d` (built with `cc` and loaded with `dlopen`, as
//! tests/es3_tf_v3d.rs does). Only the platform (memory that answers, jobs
//! that finish at once) is stood in for. Each test is the minimal GL shape
//! of a cause, not dEQP's whole case.
//!
//! | image | refusing case | cause | test | expected |
//! | --- | --- | --- | --- | --- |
//! | S03a | shaders.builtin_functions.precision.abs.highp_fragment.vec2 | (a) target past the tile memory | `a_target_past_2048_draws` | draws |
//! | T05ca | texture.format.compressed.etc2_eac_rgba8_cube_pot | (b) ETC2 not sampled | `etc2_cube_maps_draw` | draws |
//! | T05cb | texture.format.compressed.etc2_rgb8_cube_npot | (b) | `etc2_cube_maps_draw` | draws |
//! | T05cc | texture.format.compressed.etc2_rgb8_cube_pot | (b) | `etc2_cube_maps_draw` | draws |
//! | T17a | texture.wrap.etc2_eac_rgba8.clamp_clamp_linear_pot | (b) | `etc2_wrap_clamp_linear_draws` | draws |
//! | T14a | texture.units.8_units.only_cube.2 | (c) no GPU memory | `eight_cube_units_without_memory_still_refuse` | refuses (expected) |
//! | TF1a | transform_feedback.array.interleaved.lines.highp_mat3 | (d) job builder | `tf_mat3_mat4_lines_draw` | draws |
//! | TF1b | transform_feedback.array.separate.lines.highp_mat3 | (d) | `tf_mat3_mat4_lines_draw` | draws |
//! | TF2a | transform_feedback.basic_types.interleaved.lines.lowp_mat4 | (d) | `tf_mat3_mat4_lines_draw` | draws |
//! | TF2b | transform_feedback.basic_types.separate.lines.highp_mat4 | (d) | `tf_mat3_mat4_lines_draw` | draws |
//! | TF2c | transform_feedback.random_full_array_capture.interleaved.lines.1 | (d) | none | left out |
//!
//! TF2c is left out: dEQP's random_full_array_capture draws a program that
//! a random generator (seed 1) makes, which is not reproduced here, and the
//! note says its cause is not the input cap and is unknown.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d_pi4_aborts2_refusals
//! ```
#![cfg(all(
    feature = "host-test",
    feature = "shaderc-online",
    feature = "gpu-v3d",
    not(feature = "report-es2"),
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
/* the trampolines take the C functions' parameter lists as they are */
#![allow(clippy::too_many_arguments)]
#![allow(dead_code)]

use core::ffi::{c_char, c_int, c_void, CStr};
use std::collections::BTreeMap;
use std::ffi::CString;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Mutex, MutexGuard, Once, OnceLock};
use yakogl::gl::buffer::*;
use yakogl::gl::consts::*;
use yakogl::gl::context;
use yakogl::gl::draw::*;
use yakogl::gl::es3::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps, OsalV3dCpuOf};
use yakogl::v3d::backend::yakogl_backend_v3d;
use yakogl::v3d::ffi::*;
use yakogl::v3d::kick::V3dKick;

/// Every log line the library wrote since the test began.
static LOG: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Every line since the process began: the back end logs a refusal once per
/// process, so a test that needs the line looks here.
static ALL_LOG: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn log_take() -> Vec<String> {
    core::mem::take(&mut *LOG.lock().unwrap_or_else(|e| e.into_inner()))
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
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("v3d_pi4_aborts2_refusals");
        std::fs::create_dir_all(&dir).expect("the scratch directory");
        let so = dir.join(format!("v3d_job-{}.so", std::process::id()));
        /* the compiler's own temporary files next to the object */
        let st = Command::new("cc")
            .current_dir(&root)
            .env("TMPDIR", &dir)
            .args(["-std=c11", "-O1", "-shared", "-fPIC", "-Wall", "-Wextra", "-Werror"])
            .args(["-Idriver/src/v3d", "-Idriver/src/v3d/generated"])
            .args(["-Ithird_party/mesa-26.2.1/src/broadcom", "-Ithird_party/mesa-26.2.1/src"])
            .arg("-Ithird_party/mesa-26.2.1/include")
            .args(["driver/src/v3d/v3d_job.c", "driver/src/v3d/v3d_frame.c", "driver/src/v3d/v3d_tex.c"])
            .args(["driver/src/v3d/v3d_tiling_mesa.c", "-lm", "-o"])
            .arg(&so)
            .status()
            .expect("cc runs");
        assert!(st.success(), "the V3D job, frame and layout code builds");
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
    assert!(!p.is_null(), "{name} in the V3D C code");
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

    trampoline!(v3d_job_begin(j: *mut V3dJob, t: *const V3dJobTarget, bcl: *mut V3dCl, rcl: *mut V3dCl,
                              ind: *mut V3dCl) -> bool);
    trampoline!(v3d_job_draw(j: *mut V3dJob, d: *const V3dJobDraw, layout: *mut c_void) -> i32);
    trampoline!(v3d_job_end(j: *mut V3dJob, c: *const V3dJobClear, tile_list_start: *mut u32,
                            tile_list_end: *mut u32) -> bool);
    trampoline!(v3d_frame_tiling(width: u32, height: u32, t: *mut V3dTiling));
    trampoline!(v3d_frame_tiling_rts(width: u32, height: u32, rts: u32, t: *mut V3dTiling));
    trampoline!(v3d_frame_tiling_ms(width: u32, height: u32, rts: u32, msaa: bool, t: *mut V3dTiling));
    trampoline!(v3d_frame_tiling_bpp(width: u32, height: u32, rts: u32, msaa: bool, max_bpp: u32,
                                     t: *mut V3dTiling));
    trampoline!(v3d_frame_alloc_sizes(t: *const V3dTiling, draws: u32, page_size: u32, tile_alloc: *mut u32,
                                      tile_state: *mut u32));
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
    /// The most bytes live at once: a carve-out of this size.
    cap: usize,
}
static PLATFORM: Mutex<Option<Platform>> = Mutex::new(None);

fn with_platform<R>(f: impl FnOnce(&mut Platform) -> R) -> R {
    let mut g = PLATFORM.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(|| Platform { next: 0, allocs: BTreeMap::new(), submitted: 0, cap: usize::MAX }))
}

unsafe extern "C" fn p_gpu_alloc(_heap: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
    with_platform(|p| {
        let live: usize = p.allocs.values().map(|a| a.size).sum();
        if size == 0 || size > 256 << 20 || live + size > p.cap {
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
    ALL_LOG.lock().unwrap_or_else(|e| e.into_inner()).push(s.clone());
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

/* ---- the context --------------------------------------------------------- */

static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

/// The one context and back end of the file; the tests take turns and each
/// starts with no error pending, no capture and the log empty.
fn rig() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    ONCE.call_once(|| {
        // SAFETY: statics; the lock serialises every GL call of the file.
        unsafe {
            osal_set_ops(osal());
            yakogl::backend::g_backend = &yakogl_backend_v3d;
            yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            assert_eq!(yakogl::backend::init(), 0, "{:?}", log_take());
            assert!(!context::gl_context_create(32, 32).is_null(), "{:?}", log_take());
        }
    });
    // SAFETY: plain GL calls on the one context.
    unsafe {
        if geti(GL_TRANSFORM_FEEDBACK_ACTIVE) == 1 {
            glEndTransformFeedback();
        }
        glBindTransformFeedback(GL_TRANSFORM_FEEDBACK, 0);
        glUseProgram(0);
        glFinish();
        while glGetError() != GL_NO_ERROR {}
    }
    log_take();
    g
}

fn geti(pname: GLenum) -> GLint {
    let mut v = i32::MIN;
    // SAFETY: room for one value.
    unsafe { glGetIntegerv(pname, &mut v) };
    v
}

/* ---- dEQP's programs and draws ------------------------------------------ */

#[derive(Clone, Copy, PartialEq, Debug)]
enum Class {
    Float,
    Int,
    Uint,
}

/// A basic type of `TransformFeedbackTests::init`'s `basicTypes`, in its
/// order: (name, class, columns, rows).
const TYPES: [(&str, Class, usize, usize); 21] = [
    ("float", Class::Float, 1, 1),
    ("vec2", Class::Float, 1, 2),
    ("vec3", Class::Float, 1, 3),
    ("vec4", Class::Float, 1, 4),
    ("mat2", Class::Float, 2, 2),
    ("mat2x3", Class::Float, 2, 3),
    ("mat2x4", Class::Float, 2, 4),
    ("mat3x2", Class::Float, 3, 2),
    ("mat3", Class::Float, 3, 3),
    ("mat3x4", Class::Float, 3, 4),
    ("mat4x2", Class::Float, 4, 2),
    ("mat4x3", Class::Float, 4, 3),
    ("mat4", Class::Float, 4, 4),
    ("int", Class::Int, 1, 1),
    ("ivec2", Class::Int, 1, 2),
    ("ivec3", Class::Int, 1, 3),
    ("ivec4", Class::Int, 1, 4),
    ("uint", Class::Uint, 1, 1),
    ("uvec2", Class::Uint, 1, 2),
    ("uvec3", Class::Uint, 1, 3),
    ("uvec4", Class::Uint, 1, 4),
];

type Ty = (&'static str, Class, usize, usize);

fn ty(name: &str) -> Ty {
    *TYPES.iter().find(|t| t.0 == name).expect("a type of the table")
}

/// One varying of dEQP's program (`v_varA`, `v_varB`): `array` 0 is a plain
/// varying, n an array of n elements.
#[derive(Clone, Copy, Debug)]
struct Varying {
    name: &'static str,
    array: usize,
}

/// A transform feedback case: two varyings of one type at one precision, as the
/// `basic_types` (neither an array) and `array` groups declare them.
#[derive(Clone, Copy, Debug)]
struct Case {
    ty: &'static str,
    a: usize,
    b: usize,
    prim: GLenum,
    mode: GLenum,
    /// An interpolation qualifier in place of `smooth` / `flat`.
    interp: Option<&'static str>,
    /// The varyings' precision qualifier: `lowp`, `mediump` or `highp`.
    prec: &'static str,
}

impl Case {
    fn varyings(&self) -> [Varying; 2] {
        [Varying { name: "v_varA", array: self.a }, Varying { name: "v_varB", array: self.b }]
    }

    /// The scalar inputs the fragment shader reads: every component of
    /// every element of both varyings (it adds them all into the colour).
    fn fragment_inputs(&self) -> usize {
        let (_, _, cols, rows) = ty(self.ty);
        self.varyings().iter().map(|v| v.array.max(1) * cols * rows).sum()
    }

    fn mode_prim(&self) -> String {
        let mode = if self.mode == GL_INTERLEAVED_ATTRIBS { "interleaved" } else { "separate" };
        let prim = match self.prim {
            GL_POINTS => "points",
            GL_LINES => "lines",
            _ => "triangles",
        };
        format!("{prim}_{mode}")
    }

    /// dEQP's name below `functional.transform_feedback.`.
    fn name(&self) -> String {
        let group = match (self.interp, self.a) {
            (Some(i), _) => return format!("interpolation.{i}.{}_vec4_{}", self.prec, self.mode_prim()),
            (_, 0) => "basic_types",
            _ => "array",
        };
        let mode = if self.mode == GL_INTERLEAVED_ATTRIBS { "interleaved" } else { "separate" };
        let prim = match self.prim {
            GL_POINTS => "points",
            GL_LINES => "lines",
            _ => "triangles",
        };
        format!("{group}.{mode}.{prim}.{}_{}", self.prec, self.ty)
    }

    /// The `array` group's lengths for a type (`TransformFeedbackTests::init`):
    /// (1, 2) for a matrix or a separate capture, (3, 4) otherwise.
    fn array_of(ty_name: &'static str, prim: GLenum, mode: GLenum) -> Case {
        let (_, _, cols, _) = ty(ty_name);
        let (a, b) = if cols > 1 || mode == GL_SEPARATE_ATTRIBS { (1, 2) } else { (3, 4) };
        Case { ty: ty_name, a, b, prim, mode, interp: None, prec: "highp" }
    }

    fn at(mut self, prec: &'static str) -> Case {
        self.prec = prec;
        self
    }

    fn basic(ty_name: &'static str, prim: GLenum, mode: GLenum) -> Case {
        Case { ty: ty_name, a: 0, b: 0, prim, mode, interp: None, prec: "highp" }
    }
}

/// What a case did: every GL error after the draw and the finish, the
/// library's log lines, and the words the capture wrote and should have.
#[derive(Debug)]
struct Outcome {
    errors: Vec<GLenum>,
    log: Vec<String>,
    captured: Vec<Vec<u32>>,
    expected: Vec<Vec<u32>>,
}

fn shader(type_: GLenum, src: &str) -> GLuint {
    let c = CString::new(src).unwrap();
    // SAFETY: plain GL calls; the string lives for the call.
    unsafe {
        let s = glCreateShader(type_);
        let srcs = [c.as_ptr()];
        glShaderSource(s, 1, srcs.as_ptr(), core::ptr::null());
        glCompileShader(s);
        let mut ok: GLint = 0;
        glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
        assert_eq!(ok, 1, "compile 0x{type_:x}: {src}\n{:?}", log_take());
        s
    }
}

/// The attributes of a case, in location order from 1: (name, GLSL type,
/// rows) for each column of each element of each varying.
fn attributes(c: &Case) -> Vec<(String, String, usize)> {
    let (name, _, cols, rows) = ty(c.ty);
    let mut out = Vec::new();
    for v in c.varyings() {
        let base = v.name.strip_prefix("v_").unwrap();
        for e in 0..v.array.max(1) {
            for col in 0..cols {
                let mut a = format!("a_{base}");
                if v.array != 0 {
                    a += &format!("_e{e}");
                }
                let t = if cols > 1 {
                    a += &format!("_c{col}");
                    format!("vec{rows}")
                } else {
                    name.to_string()
                };
                out.push((a, t, rows));
            }
        }
    }
    out
}

/// `genShaderSources` of es3fTransformFeedbackTests.cpp for `c` (the text
/// tests/es3_shaderc_infolog.rs ports in full): one attribute per column of
/// each element, a vertex shader that copies them to the varyings, and a
/// fragment shader that adds every varying vector into `res`.
fn sources(c: &Case) -> (String, String) {
    let (name, class, cols, rows) = ty(c.ty);
    let flat = c.interp.unwrap_or(if class == Class::Float { "smooth" } else { "flat" });
    let mut vs = String::from("#version 300 es\nin highp vec4 a_position;\n");
    let mut fs = String::from(
        "#version 300 es\nlayout(location = 0) out mediump vec4 o_color;\n\
         uniform highp vec4 u_scale;\nuniform highp vec4 u_bias;\n",
    );
    for (a, t, _) in attributes(c) {
        vs += &format!("in highp {t} {a};\n");
    }
    for (dir, s) in [("out", &mut vs), ("in", &mut fs)] {
        for v in c.varyings() {
            let arr = if v.array != 0 { format!("[{}]", v.array) } else { String::new() };
            s.push_str(&format!("{flat} {dir} {} {name} {}{arr};\n", c.prec, v.name));
        }
    }
    vs += "\nvoid main (void)\n{\n\tgl_Position = a_position;\n";
    fs += "\nvoid main (void)\n{\n\thighp vec4 res = vec4(0.0);\n";
    if c.prim == GL_POINTS {
        vs += "\tgl_PointSize = 1.0;\n";
    }
    let mut attrs = attributes(c).into_iter();
    for v in c.varyings() {
        for e in 0..v.array.max(1) {
            for col in 0..cols {
                let path = format!(
                    "{}{}{}",
                    v.name,
                    if v.array != 0 { format!("[{e}]") } else { String::new() },
                    if cols > 1 { format!("[{col}]") } else { String::new() }
                );
                vs += &format!("\t{path} = {};\n", attrs.next().unwrap().0);
                fs += &match rows {
                    2 => format!("\tres += vec2({path}).xxyy;\n"),
                    3 => format!("\tres += vec3({path}).xyzx;\n"),
                    _ => format!("\tres += vec4({path});\n"),
                };
            }
        }
    }
    fs += "\to_color = res * u_scale + u_bias;\n}\n";
    vs += "}\n";
    (vs, fs)
}

/// Draw `c` as dEQP does (rasterizer discard off, the default framebuffer,
/// four vertices of a client array per attribute) and read the capture back.
fn run(c: &Case) -> Outcome {
    let (_, class, _, _) = ty(c.ty);
    let (vs, fs) = sources(c);
    const VERTS: usize = 4;
    let attrs = attributes(c);
    /* attribute k, vertex v, row r: a distinct number (a float's bits for the float class) */
    let data: Vec<Vec<u32>> = attrs
        .iter()
        .enumerate()
        .map(|(k, (_, _, rows))| {
            (0..VERTS * rows)
                .map(|i| match class {
                    Class::Float => ((k * 64 + i) as f32 * 0.25 + 0.5).to_bits(),
                    _ => (k * 64 + i) as u32 + 3,
                })
                .collect()
        })
        .collect();
    let count = match c.prim {
        GL_TRIANGLES => 3,
        _ => VERTS,
    };
    /* the capture, per buffer: one record a vertex of the draw's primitives, A then B (interleaved),
     * or A in buffer 0 and B in buffer 1 (separate) */
    let (_, _, cols, rows) = ty(c.ty);
    let per_varying = |vi: usize, v: usize| -> Vec<u32> {
        let first: usize = c.varyings()[..vi].iter().map(|x| x.array.max(1) * cols).sum();
        let mut w = Vec::new();
        for e in 0..c.varyings()[vi].array.max(1) {
            for col in 0..cols {
                w.extend_from_slice(&data[first + e * cols + col][v * rows..(v + 1) * rows]);
            }
        }
        w
    };
    let mut expected: Vec<Vec<u32>> = vec![Vec::new(); if c.mode == GL_SEPARATE_ATTRIBS { 2 } else { 1 }];
    for v in 0..count {
        for vi in 0..2 {
            let buf = if c.mode == GL_SEPARATE_ATTRIBS { vi } else { 0 };
            expected[buf].extend(per_varying(vi, v));
        }
    }
    let positions: [f32; 16] = [-0.5, -0.5, 0.0, 1.0, 0.5, -0.5, 0.0, 1.0, 0.5, 0.5, 0.0, 1.0, -0.5, 0.5, 0.0, 1.0];
    let mut captured: Vec<Vec<u32>> = expected.iter().map(|e| vec![0u32; e.len()]).collect();
    let mut errors = Vec::new();
    // SAFETY: plain GL calls; every pointer outlives its call (the client arrays the draw reads, the names).
    unsafe {
        let p = glCreateProgram();
        let (v, f) = (shader(GL_VERTEX_SHADER, &vs), shader(GL_FRAGMENT_SHADER, &fs));
        glAttachShader(p, v);
        glAttachShader(p, f);
        glDeleteShader(v);
        glDeleteShader(f);
        glBindAttribLocation(p, 0, c"a_position".as_ptr());
        for (k, (a, _, _)) in attrs.iter().enumerate() {
            let n = CString::new(a.as_str()).unwrap();
            glBindAttribLocation(p, 1 + k as GLuint, n.as_ptr());
        }
        let names = [CString::new("v_varA").unwrap(), CString::new("v_varB").unwrap()];
        let ptrs: Vec<*const c_char> = names.iter().map(|n| n.as_ptr()).collect();
        glTransformFeedbackVaryings(p, 2, ptrs.as_ptr(), c.mode);
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "link failed: {:?}", log_take());
        glUseProgram(p);
        glUniform4f(glGetUniformLocation(p, c"u_scale".as_ptr()), 0.25, 0.25, 0.25, 0.25);
        glUniform4f(glGetUniformLocation(p, c"u_bias".as_ptr()), 0.0, 0.0, 0.0, 0.0);

        let mut bufs = vec![0; expected.len()];
        glGenBuffers(bufs.len() as GLsizei, bufs.as_mut_ptr());
        for (i, b) in bufs.iter().enumerate() {
            glBindBuffer(GL_TRANSFORM_FEEDBACK_BUFFER, *b);
            glBufferData(
                GL_TRANSFORM_FEEDBACK_BUFFER,
                (expected[i].len() * 4 + 64) as GLsizeiptr,
                core::ptr::null(),
                GL_DYNAMIC_READ,
            );
            glBindBufferBase(GL_TRANSFORM_FEEDBACK_BUFFER, i as GLuint, *b);
        }

        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glEnableVertexAttribArray(0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, positions.as_ptr().cast());
        for (k, (_, _, rows)) in attrs.iter().enumerate() {
            let loc = 1 + k as GLuint;
            glEnableVertexAttribArray(loc);
            let at = data[k].as_ptr().cast::<c_void>();
            if class == Class::Float {
                glVertexAttribPointer(loc, *rows as GLint, GL_FLOAT, 0, 0, at);
            } else {
                let t = if class == Class::Int { GL_INT } else { GL_UNSIGNED_INT };
                glVertexAttribIPointer(loc, *rows as GLint, t, 0, at);
            }
        }
        while glGetError() != GL_NO_ERROR {}
        log_take();
        glViewport(0, 0, 32, 32);
        glBeginTransformFeedback(c.prim);
        glDrawArrays(c.prim, 0, count as GLsizei);
        glEndTransformFeedback();
        glFinish();
        loop {
            let e = glGetError();
            if e == GL_NO_ERROR {
                break;
            }
            errors.push(e);
        }
        let log = log_take();
        for (i, b) in bufs.iter().enumerate() {
            glBindBuffer(GL_TRANSFORM_FEEDBACK_BUFFER, *b);
            let n = captured[i].len();
            let m = glMapBufferRange(GL_TRANSFORM_FEEDBACK_BUFFER, 0, (n * 4) as GLsizeiptr, GL_MAP_READ_BIT);
            assert!(!m.is_null(), "the capture maps");
            core::ptr::copy_nonoverlapping(m as *const u32, captured[i].as_mut_ptr(), n);
            glUnmapBuffer(GL_TRANSFORM_FEEDBACK_BUFFER);
            glBindBufferBase(GL_TRANSFORM_FEEDBACK_BUFFER, i as GLuint, 0);
        }
        glDeleteBuffers(bufs.len() as GLsizei, bufs.as_ptr());
        glUseProgram(0);
        glDeleteProgram(p);
        for k in 0..=attrs.len() {
            glDisableVertexAttribArray(k as GLuint);
        }
        while glGetError() != GL_NO_ERROR {}
        Outcome { errors, log, captured, expected }
    }
}

/* ---- the guards ---- */

use yakogl::gl::framebuffer::*;
use yakogl::gl::texture::*;
use yakogl::v3d::backend::v3d_refused_draws;

const VS: &str = "#version 300 es\nin vec4 p;\nout vec2 uv;\nvoid main() { uv = p.xy * 0.5 + 0.5; gl_Position = p; }\n";

static TRI: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 3.0, -1.0, 0.0, 1.0, -1.0, 3.0, 0.0, 1.0];

/// Link `fs` with `VS` and make it current; returns the program.
fn program(fs: &str) -> GLuint {
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        let (v, f) = (shader(GL_VERTEX_SHADER, VS), shader(GL_FRAGMENT_SHADER, fs));
        glAttachShader(p, v);
        glAttachShader(p, f);
        glBindAttribLocation(p, 0, c"p".as_ptr());
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "link: {:?}", log_take());
        glUseProgram(p);
        p
    }
}

/// Draw the big triangle, finish, and return every GL error raised.
fn draw_all() -> Vec<GLenum> {
    // SAFETY: TRI is static and outlives the draw.
    unsafe {
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, TRI.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glDrawArrays(GL_TRIANGLES, 0, 3);
        glFinish();
    }
    let mut errors = Vec::new();
    loop {
        // SAFETY: plain GL call.
        let e = unsafe { glGetError() };
        if e == GL_NO_ERROR {
            return errors;
        }
        errors.push(e);
    }
}

fn refusal_lines(log: &[String]) -> Vec<&String> {
    log.iter().filter(|l| l.contains("draw refused") || l.contains("out of GPU memory")).collect()
}

/// (a) S03a: the shader executor's `precision.abs.highp_fragment.vec2` draws
/// into a target wider than the 2048x2048 tile memory the image was sized
/// for. b1b5b845 refused it (-EINVAL, GL_OUT_OF_MEMORY, the image ended);
/// since #1261 the tile memory is sized for 4096 and the draw is taken.
#[test]
fn a_target_past_2048_draws() {
    let _g = rig();
    for (w, h) in [(4096, 2), (4096, 6), (2049, 16)] {
        let mut fb = 0;
        let mut rb = 0;
        // SAFETY: plain GL calls with room for one name each.
        unsafe {
            glGenFramebuffers(1, &mut fb);
            glBindFramebuffer(GL_FRAMEBUFFER, fb);
            glGenRenderbuffers(1, &mut rb);
            glBindRenderbuffer(GL_RENDERBUFFER, rb);
            glRenderbufferStorage(GL_RENDERBUFFER, GL_RGBA8, w, h);
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb);
            assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
            glViewport(0, 0, w, h);
        }
        let p = program(
            "#version 300 es\nprecision highp float;\nin vec2 uv;\nlayout(location = 0) out vec4 c;\n\
             void main() { c = vec4(abs(uv - 0.5), 0.0, 1.0); }\n",
        );
        log_take();
        let refused = v3d_refused_draws();
        let errors = draw_all();
        assert_eq!(errors, [], "{w}x{h}: {:?}", log_take());
        assert_eq!(v3d_refused_draws(), refused, "{w}x{h}: a refused draw");
        // SAFETY: plain GL calls.
        unsafe {
            glUseProgram(0);
            glDeleteProgram(p);
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteFramebuffers(1, &fb);
            glDeleteRenderbuffers(1, &rb);
        }
    }
}

/// A texture of `target` sampled by `fs` on unit 0 (uniform `s`), after
/// `upload` has filled it; returns the errors of the draw, the refused
/// draws it added and the log.
fn sampled_draw(target: GLenum, fs: &str, upload: impl FnOnce()) -> (Vec<GLenum>, u32, Vec<String>) {
    let mut t = 0;
    // SAFETY: plain GL calls.
    unsafe {
        glViewport(0, 0, 32, 32);
        glActiveTexture(GL_TEXTURE0);
        glGenTextures(1, &mut t);
        glBindTexture(target, t);
    }
    upload();
    let p = program(fs);
    // SAFETY: plain GL calls.
    unsafe {
        glUniform1i(glGetUniformLocation(p, c"s".as_ptr()), 0);
        while glGetError() != GL_NO_ERROR {}
    }
    log_take();
    let refused = v3d_refused_draws();
    let errors = draw_all();
    let added = v3d_refused_draws() - refused;
    let log = log_take();
    // SAFETY: plain GL calls.
    unsafe {
        glUseProgram(0);
        glDeleteProgram(p);
        glBindTexture(target, 0);
        glDeleteTextures(1, &t);
        while glGetError() != GL_NO_ERROR {}
    }
    (errors, added, log)
}

fn fill(n: usize, seed: u32) -> Vec<u8> {
    (0..n).map(|i| (i as u32).wrapping_mul(31).wrapping_add(seed) as u8).collect()
}

/// (b) T05ca/T05cb/T05cc: ETC2 RGBA8+EAC and RGB8 cube maps, 8x8 (POT) and
/// 12x12 (NPOT), sampled with a samplerCube. b1b5b845 refused "a texel
/// format the V3D does not sample"; 455c3bb5 put the four ETC2 classes in
/// `V3D_SAMPLED_TEXFMTS`, so the draw is taken.
#[test]
fn etc2_cube_maps_draw() {
    let _g = rig();
    const FS: &str = "#version 300 es\nprecision mediump float;\nin vec2 uv;\nuniform samplerCube s;\n\
                      layout(location = 0) out vec4 c;\nvoid main() { c = texture(s, vec3(uv - 0.5, 1.0)); }\n";
    for (name, fmt, bytes_per_block) in
        [("etc2_eac_rgba8", GL_COMPRESSED_RGBA8_ETC2_EAC, 16usize), ("etc2_rgb8", GL_COMPRESSED_RGB8_ETC2, 8)]
    {
        for side in [8, 12] {
            let n = (side as usize / 4) * (side as usize / 4) * bytes_per_block;
            let (errors, refused, log) = sampled_draw(GL_TEXTURE_CUBE_MAP, FS, || {
                // SAFETY: plain GL calls; each face's data is its imageSize bytes.
                unsafe {
                    for p in [GL_TEXTURE_MIN_FILTER, GL_TEXTURE_MAG_FILTER] {
                        glTexParameteri(GL_TEXTURE_CUBE_MAP, p, GL_LINEAR as GLint);
                    }
                    for face in 0..6u32 {
                        let d = fill(n, 70 + face);
                        glCompressedTexImage2D(
                            GL_TEXTURE_CUBE_MAP_POSITIVE_X + face,
                            0,
                            fmt,
                            side,
                            side,
                            0,
                            n as GLsizei,
                            d.as_ptr() as *const c_void,
                        );
                    }
                }
            });
            assert_eq!(errors, [], "{name} cube {side}x{side}: {log:?}");
            assert_eq!(refused, 0, "{name} cube {side}x{side}: {log:?}");
            assert!(refusal_lines(&log).is_empty(), "{name} cube {side}x{side}: {log:?}");
        }
    }
}

/// (b) T17a: `wrap.etc2_eac_rgba8.clamp_clamp_linear_pot`, a 2D
/// RGBA8_ETC2_EAC texture with CLAMP_TO_EDGE and LINEAR, drawn over
/// coordinates that leave [0, 1] (and the RGB8 class beside it).
#[test]
fn etc2_wrap_clamp_linear_draws() {
    let _g = rig();
    const FS: &str = "#version 300 es\nprecision mediump float;\nin vec2 uv;\nuniform sampler2D s;\n\
                      layout(location = 0) out vec4 c;\nvoid main() { c = texture(s, uv * 3.0 - 1.0); }\n";
    for (fmt, bytes_per_block) in [(GL_COMPRESSED_RGBA8_ETC2_EAC, 16usize), (GL_COMPRESSED_RGB8_ETC2, 8)] {
        let n = (16 / 4) * (16 / 4) * bytes_per_block;
        let (errors, refused, log) = sampled_draw(GL_TEXTURE_2D, FS, || {
            // SAFETY: plain GL calls; the data is the imageSize bytes.
            unsafe {
                for p in [GL_TEXTURE_MIN_FILTER, GL_TEXTURE_MAG_FILTER] {
                    glTexParameteri(GL_TEXTURE_2D, p, GL_LINEAR as GLint);
                }
                glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE as GLint);
                glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE as GLint);
                let d = fill(n, 5);
                glCompressedTexImage2D(GL_TEXTURE_2D, 0, fmt, 16, 16, 0, n as GLsizei, d.as_ptr() as *const c_void);
            }
        });
        assert_eq!(errors, [], "{fmt:#x}: {log:?}");
        assert_eq!(refused, 0, "{fmt:#x}: {log:?}");
        assert!(refusal_lines(&log).is_empty(), "{fmt:#x}: {log:?}");
    }
}

/// Restores the platform's unlimited memory when a test ends.
struct Carve;
impl Drop for Carve {
    fn drop(&mut self) {
        with_platform(|p| p.cap = usize::MAX);
    }
}

/// (c) T14a: `texture.units.8_units.only_cube.2`, eight 256x256 cube maps
/// with every level, one draw sampling all eight units. On the board the
/// kept tiled copies (2-4 MiB each) found no GPU memory, no empty job held
/// the draw and the back end answered -E2BIG, which the GL core raises as
/// GL_OUT_OF_MEMORY: dEQP ends the image. No commit since b1b5b845 changes
/// that (docs/notes/v3d-refusals.md section F, cause (c)); only a larger
/// carve-out clears it (texture-2048.conf), which tests/v3d_units_cube_mem.rs
/// measures with the Zephyr allocator. Here the platform's memory is capped
/// 12 MiB above what the uploads hold, below what the eight units' kept
/// copies need, and the draw is an EXPECTED refusal: GL_OUT_OF_MEMORY, the
/// `no empty job holds the draw` refusal and `-7` (-E2BIG) in the log (the
/// `out of GPU memory` line is the board's OSAL's, not this platform's). Issue draft: "V3D: eight cube units
/// (texture.units.8_units.only_cube.2) lose the draw for lack of GPU memory
/// and abort the image". When that is fixed this test must be turned round.
#[test]
fn eight_cube_units_without_memory_still_refuse() {
    let _g = rig();
    let _c = Carve;
    let mut fs = String::from("#version 300 es\nprecision mediump float;\nin vec2 uv;\n");
    for i in 0..8 {
        fs += &format!("uniform samplerCube s{i};\n");
    }
    fs += "layout(location = 0) out vec4 c;\nvoid main() {\n vec4 r = vec4(0.0);\n";
    for i in 0..8 {
        fs += &format!(" r += texture(s{i}, vec3(uv - 0.5, 1.0));\n");
    }
    fs += " c = r * 0.125;\n}\n";
    let mut tex = [0u32; 8];
    // SAFETY: plain GL calls; every data pointer outlives its call.
    unsafe {
        glViewport(0, 0, 32, 32);
        glGenTextures(8, tex.as_mut_ptr());
        let px = vec![0x80u8; 256 * 256 * 4];
        for (i, t) in tex.iter().enumerate() {
            glActiveTexture(GL_TEXTURE0 + i as u32);
            glBindTexture(GL_TEXTURE_CUBE_MAP, *t);
            glTexParameteri(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_MIN_FILTER, GL_LINEAR_MIPMAP_LINEAR as GLint);
            let mut level = 0;
            let mut side = 256;
            while side >= 1 {
                for face in 0..6u32 {
                    glTexImage2D(
                        GL_TEXTURE_CUBE_MAP_POSITIVE_X + face,
                        level,
                        GL_RGBA8 as GLint,
                        side,
                        side,
                        0,
                        GL_RGBA,
                        GL_UNSIGNED_BYTE,
                        px.as_ptr() as *const c_void,
                    );
                }
                level += 1;
                side /= 2;
            }
        }
        assert_eq!(glGetError(), GL_NO_ERROR, "the uploads: {:?}", log_take());
    }
    let p = program(&fs);
    // SAFETY: plain GL calls.
    unsafe {
        for i in 0..8 {
            glUniform1i(glGetUniformLocation(p, CString::new(format!("s{i}")).unwrap().as_ptr()), i);
        }
    }
    with_platform(|pl| {
        let live: usize = pl.allocs.values().map(|a| a.size).sum();
        pl.cap = live + (12 << 20);
    });
    log_take();
    let errors = draw_all();
    let log = log_take();
    assert_eq!(errors, [GL_OUT_OF_MEMORY], "an expected refusal (cause (c)): {log:?}");
    assert!(log.iter().any(|l| l.contains("no empty job holds the draw")), "{log:?}");
    assert!(log.iter().any(|l| l.contains("was not recorded (-7)")), "-E2BIG: {log:?}");
    // SAFETY: plain GL calls.
    unsafe {
        glUseProgram(0);
        glDeleteProgram(p);
        glDeleteTextures(8, tex.as_ptr());
        while glGetError() != GL_NO_ERROR {}
    }
}

/// (d) TF1a, TF1b, TF2a, TF2b: the mat3 and mat4 line cases (27 and 32
/// fragment input scalars) through the real job builder, at every
/// precision (TF2a is `lowp_mat4`). b1b5b845 capped inputs at 24 and
/// refused them; the cap is gone, so every one draws with no error and no
/// refusal line, and the capture is right. (tests/es3_tf_v3d.rs holds the
/// same four inside its whole 252-case sweep; this is the one-file guard.)
#[test]
fn tf_mat3_mat4_lines_draw() {
    let _g = rig();
    let cases = [
        ("TF1a", Case::array_of("mat3", GL_LINES, GL_INTERLEAVED_ATTRIBS), 27),
        ("TF1b", Case::array_of("mat3", GL_LINES, GL_SEPARATE_ATTRIBS), 27),
        ("TF2a", Case::basic("mat4", GL_LINES, GL_INTERLEAVED_ATTRIBS), 32),
        ("TF2b", Case::basic("mat4", GL_LINES, GL_SEPARATE_ATTRIBS), 32),
    ];
    for (image, c, inputs) in cases {
        for prec in ["lowp", "mediump", "highp"] {
            let c = c.at(prec);
            assert_eq!(c.fragment_inputs(), inputs, "{image}: the input count");
            let o = run(&c);
            assert_eq!(o.errors, [], "{image} {}: {:?}", c.name(), o.log);
            assert!(refusal_lines(&o.log).is_empty(), "{image} {}: {:?}", c.name(), o.log);
            assert_eq!(o.captured, o.expected, "{image} {}: the capture", c.name());
        }
    }
}
