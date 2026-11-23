// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The `transform_feedback.random_full_array_capture.*` cases of
//! dEQP-GLES3, drawn with transform feedback through the V3D back end and
//! the REAL job builder (plan 399, item tf2c-host-test; #1261).
//!
//! On the Pi 4 at `b1b5b845` dEQP ended image TF2c at
//! `random_full_array_capture.interleaved.lines.1` because the job builder
//! refused the draw. No host test modelled the group
//! (docs/notes/v3d-refusals.md, the TF2c row). tests/es3_tf_v3d.rs covers
//! `basic_types` and `array`; this file covers the 60 random programs.
//!
//! The programs are the board's: `RandomCase::init`
//! (es3fTransformFeedbackTests.cpp) is ported draw for draw, on a port of
//! `de::Random` and `deInt32Hash`, the way tests/es3_shaderc_infolog.rs does
//! (that file's generator is the model; it is copied here because the two
//! files must stay independent). `genShaderSources` gives the text. Every
//! varying is captured whole (`gl_Position` and `gl_PointSize` too, when the
//! case uses them), drawn to the default framebuffer with no rasterizer
//! discard, as dEQP does.
//!
//! The harness is tests/es3_tf_v3d.rs's: `driver/src/v3d/v3d_job.c`,
//! `v3d_frame.c` and the layout code built with `cc` into one shared object
//! and loaded with `dlopen`, behind `yakogl_backend_v3d` and the GL core with
//! the in-process compiler. Only the platform (memory that answers, jobs that
//! finish at once) is stood in for.
//!
//! What is held, for each case: the program links, the draw raises no GL
//! error (no GL_OUT_OF_MEMORY, which is how a refused draw shows) and the
//! library logs no refusal.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_tf_random_capture_v3d
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
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("es3_tf_random_capture_v3d");
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

/* ---- dEQP's random numbers ------------------------------------------------ */

/// `de::Random` (xorshift128, deRandom.c).
struct DeRandom {
    x: u32,
    y: u32,
    z: u32,
    w: u32,
}

impl DeRandom {
    fn new(seed: u32) -> DeRandom {
        DeRandom {
            x: (seed as i32).wrapping_neg() as u32 ^ 123_456_789,
            y: 362_436_069u32.wrapping_mul(seed),
            z: 521_288_629 ^ (seed >> 7),
            w: 88_675_123 ^ (seed << 3),
        }
    }

    fn get_u32(&mut self) -> u32 {
        let w = self.w;
        let t = self.x ^ (self.x << 11);
        self.x = self.y;
        self.y = self.z;
        self.z = w;
        self.w = (w ^ (w >> 19)) ^ (t ^ (t >> 8));
        self.w
    }

    fn get_float(&mut self) -> f32 {
        (self.get_u32() & 0x0FFF_FFFF) as f32 / (0x0FFF_FFFFu32 + 1) as f32
    }

    /// `getInt(min, max)`, inclusive.
    fn get_int(&mut self, min: i32, max: i32) -> i32 {
        min + (self.get_u32() % ((max as u32).wrapping_sub(min as u32).wrapping_add(1))) as i32
    }

    /// `choose<T>`: reservoir sampling of one.
    fn choose<T: Copy>(&mut self, items: &[T]) -> T {
        let mut val = items[0];
        for (ndx, it) in items.iter().enumerate().skip(1) {
            if self.get_int(0, ndx as i32) < 1 {
                val = *it;
            }
        }
        val
    }
}

/// `deInt32Hash`.
fn de_int32_hash(a: u32) -> u32 {
    let mut key = a;
    key = (key ^ 61) ^ (key >> 16);
    key = key.wrapping_add(key << 3);
    key ^= key >> 4;
    key = key.wrapping_mul(0x27d4_eb2d);
    key ^ (key >> 15)
}

/* ---- the program spec ----------------------------------------------------- */

/// A basic type: (name, is float, columns, rows).
type Ty = (&'static str, bool, usize, usize);

/// `RandomCase::init`'s `typeCandidates`, in its order.
const TYPE_CANDIDATES: [Ty; 21] = [
    ("float", true, 1, 1),
    ("vec2", true, 1, 2),
    ("vec3", true, 1, 3),
    ("vec4", true, 1, 4),
    ("int", false, 1, 1),
    ("ivec2", false, 1, 2),
    ("ivec3", false, 1, 3),
    ("ivec4", false, 1, 4),
    ("uint", false, 1, 1),
    ("uvec2", false, 1, 2),
    ("uvec3", false, 1, 3),
    ("uvec4", false, 1, 4),
    ("mat2", true, 2, 2),
    ("mat2x3", true, 2, 3),
    ("mat2x4", true, 2, 4),
    ("mat3x2", true, 3, 2),
    ("mat3", true, 3, 3),
    ("mat3x4", true, 3, 4),
    ("mat4x2", true, 4, 2),
    ("mat4x3", true, 4, 3),
    ("mat4", true, 4, 4),
];

const PRECISIONS: [&str; 3] = ["lowp", "mediump", "highp"];
const RANDOM_INTERPS: [&str; 3] = ["flat", "smooth", "centroid"];

#[derive(Clone, Debug)]
struct Varying {
    name: String,
    ty: Ty,
    prec: &'static str,
    /// 0 = not an array.
    array: usize,
    interp: &'static str,
}

/// One random case: its name below `functional.transform_feedback.`, the
/// varyings and the built-ins it captures.
#[derive(Clone, Debug)]
struct Case {
    name: String,
    prim: GLenum,
    mode: GLenum,
    varyings: Vec<Varying>,
    use_position: bool,
    use_point_size: bool,
}

/// `RandomCase::init` up to the varyings: (varyings, usePosition, usePointSize).
fn random_varyings(separate: bool, seed: u32) -> (Vec<Varying>, bool, bool) {
    const MAX_ATTRIBUTE_VECTORS: i32 = 16;
    let mut rnd = DeRandom::new(seed);
    let use_position = rnd.get_float() < 0.7;
    let use_point_size = rnd.get_float() < 0.1;
    let to_use = rnd.get_int(1, MAX_ATTRIBUTE_VECTORS - 1 - use_point_size as i32);
    let (mut have, mut ndx) = (0i32, 0);
    let mut out = Vec::new();
    while have < to_use {
        let max_vecs = if separate { 2.min(to_use - have) } else { to_use - have };
        let end = match max_vecs {
            4.. => 21,
            3 => 18,
            2 if separate => 13,
            2 => 15,
            _ => 12,
        };
        let t = rnd.choose(&TYPE_CANDIDATES[..end]);
        let prec = rnd.choose(&PRECISIONS);
        let interp = if t.1 { rnd.choose(&RANDOM_INTERPS) } else { "flat" };
        let num_vecs = t.2 as i32;
        let num_comps = (t.2 * t.3) as i32;
        let max_array_len = 1.max(if separate { 4 / num_comps } else { max_vecs / num_vecs });
        let use_array = rnd.get_float() < 0.3;
        let array_len = if use_array { rnd.get_int(1, max_array_len) } else { 1 };
        out.push(Varying {
            name: format!("v_var{ndx}"),
            ty: t,
            prec,
            array: if use_array { array_len as usize } else { 0 },
            interp,
        });
        have += array_len * num_vecs;
        ndx += 1;
    }
    (out, use_position, use_point_size)
}

/// The 60 mustpass cases of `random_full_array_capture`.
fn cases() -> Vec<Case> {
    let mut v = Vec::new();
    for (mode_name, mode) in [("separate", GL_SEPARATE_ATTRIBS), ("interleaved", GL_INTERLEAVED_ATTRIBS)] {
        for (prim_name, prim) in [("points", GL_POINTS), ("lines", GL_LINES), ("triangles", GL_TRIANGLES)] {
            for ndx in 0..10u32 {
                let seed = de_int32_hash(mode) ^ de_int32_hash(prim) ^ de_int32_hash(ndx);
                let (varyings, use_position, use_point_size) = random_varyings(mode == GL_SEPARATE_ATTRIBS, seed);
                v.push(Case {
                    name: format!("random_full_array_capture.{mode_name}.{prim_name}.{}", ndx + 1),
                    prim,
                    mode,
                    varyings,
                    use_position,
                    use_point_size,
                });
            }
        }
    }
    v
}

/// (attribute name, declared type, rows, path in the varying) for each
/// vector of a varying: one per column of each element.
fn vectors_of(v: &Varying) -> Vec<(String, String, usize, String)> {
    let base = v.name.strip_prefix("v_").unwrap();
    let (name, _, cols, rows) = v.ty;
    let mut out = Vec::new();
    for e in 0..v.array.max(1) {
        for c in 0..cols {
            let mut a = format!("a_{base}");
            let mut path = String::new();
            if v.array != 0 {
                a += &format!("_e{e}");
                path += &format!("[{e}]");
            }
            let t = if cols > 1 {
                a += &format!("_c{c}");
                path += &format!("[{c}]");
                format!("vec{rows}")
            } else {
                name.to_string()
            };
            out.push((a, t, rows, path));
        }
    }
    out
}

/// `genShaderSources` for `c`.
fn sources(c: &Case) -> (String, String) {
    let mut vs = String::from("#version 300 es\nin highp vec4 a_position;\n");
    let mut fs = String::from(
        "#version 300 es\nlayout(location = 0) out mediump vec4 o_color;\nuniform highp vec4 u_scale;\nuniform highp vec4 u_bias;\n",
    );
    if c.use_point_size {
        vs += "in highp float a_pointSize;\n";
    }
    for v in &c.varyings {
        for (a, t, _, _) in vectors_of(v) {
            vs += &format!("in {} {t} {a};\n", v.prec);
        }
    }
    for (dir, s) in [("out", &mut vs), ("in", &mut fs)] {
        for v in &c.varyings {
            let arr = if v.array != 0 { format!("[{}]", v.array) } else { String::new() };
            s.push_str(&format!("{} {dir} {} {} {}{arr};\n", v.interp, v.prec, v.ty.0, v.name));
        }
    }
    vs += "\nvoid main (void)\n{\n\tgl_Position = a_position;\n";
    fs += "\nvoid main (void)\n{\n\thighp vec4 res = vec4(0.0);\n";
    if c.use_point_size {
        vs += "\tgl_PointSize = a_pointSize;\n";
    } else if c.prim == GL_POINTS {
        vs += "\tgl_PointSize = 1.0;\n";
    }
    for v in &c.varyings {
        for (a, _, rows, path) in vectors_of(v) {
            vs += &format!("\t{}{path} = {a};\n", v.name);
            fs += &match rows {
                2 => format!("\tres += vec2({}{path}).xxyy;\n", v.name),
                3 => format!("\tres += vec3({}{path}).xyzx;\n", v.name),
                _ => format!("\tres += vec4({}{path});\n", v.name),
            };
        }
    }
    fs += "\to_color = res * u_scale + u_bias;\n}\n";
    vs += "}\n";
    (vs, fs)
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

/// What a case did: every GL error after the draw and the finish, and the
/// library's log lines of the draw.
#[derive(Debug)]
struct Outcome {
    errors: Vec<GLenum>,
    log: Vec<String>,
}

/// One attribute of a case: name, location, rows, `None` for a float or
/// `Some(signed)` for an integer, and four vertices of data.
type Attr = (String, GLuint, usize, Option<bool>, Vec<u32>);

/// Draw `c` as dEQP does and report what happened. A program that does not
/// link is a panic: the cases are the board's, and they linked there.
fn run(c: &Case) -> Outcome {
    let (vs, fs) = sources(c);
    let count = if c.prim == GL_TRIANGLES { 3 } else { 4 };
    let positions: [f32; 16] = [-0.5, -0.5, 0.0, 1.0, 0.5, -0.5, 0.0, 1.0, 0.5, 0.5, 0.0, 1.0, -0.5, 0.5, 0.0, 1.0];
    let sizes: [f32; 4] = [1.0, 2.0, 3.0, 4.0];
    let mut attrs: Vec<Attr> = Vec::new();
    let mut loc = if c.use_point_size { 2 } else { 1 };
    for v in &c.varyings {
        for (a, _, rows, _) in vectors_of(v) {
            let k = attrs.len();
            let float = v.ty.1;
            let signed = v.ty.0.starts_with('i');
            let data = (0..4 * rows)
                .map(|i| if float { ((k * 64 + i) as f32 * 0.25 + 0.5).to_bits() } else { (k * 64 + i) as u32 + 3 })
                .collect();
            attrs.push((a, loc, rows, if float { None } else { Some(signed) }, data));
            loc += 1;
        }
    }
    let mut names: Vec<CString> = Vec::new();
    if c.use_position {
        names.push(CString::new("gl_Position").unwrap());
    }
    if c.use_point_size {
        names.push(CString::new("gl_PointSize").unwrap());
    }
    for v in &c.varyings {
        names.push(CString::new(v.name.as_str()).unwrap());
    }
    /* a separate capture holds at most GL_MAX_TRANSFORM_FEEDBACK_SEPARATE_ATTRIBS (4) varyings: dEQP stops
     * selecting at the limit (the shaders still declare every varying) */
    if c.mode == GL_SEPARATE_ATTRIBS {
        names.truncate(4);
    }
    let nbufs = if c.mode == GL_SEPARATE_ATTRIBS { names.len() } else { 1 };
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
        if c.use_point_size {
            glBindAttribLocation(p, 1, c"a_pointSize".as_ptr());
        }
        for (a, l, _, _, _) in &attrs {
            let n = CString::new(a.as_str()).unwrap();
            glBindAttribLocation(p, *l, n.as_ptr());
        }
        let ptrs: Vec<*const c_char> = names.iter().map(|n| n.as_ptr()).collect();
        glTransformFeedbackVaryings(p, ptrs.len() as GLsizei, ptrs.as_ptr(), c.mode);
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "{}: link failed: {:?}\n{vs}\n{fs}", c.name, log_take());
        glUseProgram(p);
        glUniform4f(glGetUniformLocation(p, c"u_scale".as_ptr()), 0.25, 0.25, 0.25, 0.25);
        glUniform4f(glGetUniformLocation(p, c"u_bias".as_ptr()), 0.0, 0.0, 0.0, 0.0);

        let mut bufs = vec![0; nbufs];
        glGenBuffers(nbufs as GLsizei, bufs.as_mut_ptr());
        for (i, b) in bufs.iter().enumerate() {
            glBindBuffer(GL_TRANSFORM_FEEDBACK_BUFFER, *b);
            /* room for every vertex of the draw (at most 4 vertices of 64 scalars) and then some */
            glBufferData(GL_TRANSFORM_FEEDBACK_BUFFER, 4096, core::ptr::null(), GL_DYNAMIC_READ);
            glBindBufferBase(GL_TRANSFORM_FEEDBACK_BUFFER, i as GLuint, *b);
        }
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glEnableVertexAttribArray(0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, positions.as_ptr().cast());
        if c.use_point_size {
            glEnableVertexAttribArray(1);
            glVertexAttribPointer(1, 1, GL_FLOAT, 0, 0, sizes.as_ptr().cast());
        }
        for (_, l, rows, class, data) in &attrs {
            glEnableVertexAttribArray(*l);
            let at = data.as_ptr().cast::<c_void>();
            match class {
                None => glVertexAttribPointer(*l, *rows as GLint, GL_FLOAT, 0, 0, at),
                Some(signed) => {
                    let t = if *signed { GL_INT } else { GL_UNSIGNED_INT };
                    glVertexAttribIPointer(*l, *rows as GLint, t, 0, at);
                }
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
        for i in 0..nbufs {
            glBindBufferBase(GL_TRANSFORM_FEEDBACK_BUFFER, i as GLuint, 0);
        }
        glDeleteBuffers(nbufs as GLsizei, bufs.as_ptr());
        glUseProgram(0);
        glDeleteProgram(p);
        for k in 0..loc {
            glDisableVertexAttribArray(k);
        }
        while glGetError() != GL_NO_ERROR {}
        Outcome { errors, log }
    }
}

/// Draw one case of the 60 by its name and hold that nothing was refused.
fn draw(name: &str) {
    let _g = rig();
    let c = cases().into_iter().find(|c| c.name == name).expect("a mustpass case");
    let o = run(&c);
    assert_eq!(o.errors, [], "{name}: {:?}", o.log);
    assert!(!o.log.iter().any(|l| l.contains("refused")), "{name}: {:?}", o.log);
}

#[test]
fn the_60_cases_are_the_mustpass_list() {
    let list = std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tools/cts/gles3-mustpass.txt"))
        .expect("the mustpass list");
    let mut want: Vec<String> = list
        .lines()
        .filter_map(|l| l.strip_prefix("dEQP-GLES3.functional.transform_feedback."))
        .filter(|l| l.starts_with("random_full_array_capture."))
        .map(String::from)
        .collect();
    let mut got: Vec<String> = cases().into_iter().map(|c| c.name).collect();
    want.sort();
    got.sort();
    assert_eq!(got, want);
}

/* One test a case, so that a refused case is ignored by name. */
macro_rules! case_tests {
    ($($test:ident => $name:literal,)*) => {
        $(
            #[test]
            fn $test() {
                draw(concat!("random_full_array_capture.", $name));
            }
        )*
    };
}

case_tests! {
    interleaved_lines_1 => "interleaved.lines.1",
    interleaved_lines_2 => "interleaved.lines.2",
    interleaved_lines_3 => "interleaved.lines.3",
    interleaved_lines_4 => "interleaved.lines.4",
    interleaved_lines_5 => "interleaved.lines.5",
    interleaved_lines_6 => "interleaved.lines.6",
    interleaved_lines_7 => "interleaved.lines.7",
    interleaved_lines_8 => "interleaved.lines.8",
    interleaved_lines_9 => "interleaved.lines.9",
    interleaved_lines_10 => "interleaved.lines.10",
    separate_lines_1 => "separate.lines.1",
    separate_lines_2 => "separate.lines.2",
    separate_lines_3 => "separate.lines.3",
    separate_lines_4 => "separate.lines.4",
    separate_lines_5 => "separate.lines.5",
    separate_lines_6 => "separate.lines.6",
    separate_lines_7 => "separate.lines.7",
    separate_lines_8 => "separate.lines.8",
    separate_lines_9 => "separate.lines.9",
    separate_lines_10 => "separate.lines.10",
    interleaved_points_1 => "interleaved.points.1",
    interleaved_points_2 => "interleaved.points.2",
    interleaved_points_3 => "interleaved.points.3",
    interleaved_points_4 => "interleaved.points.4",
    interleaved_points_5 => "interleaved.points.5",
    interleaved_points_6 => "interleaved.points.6",
    interleaved_points_7 => "interleaved.points.7",
    interleaved_points_8 => "interleaved.points.8",
    interleaved_points_9 => "interleaved.points.9",
    interleaved_points_10 => "interleaved.points.10",
    separate_points_1 => "separate.points.1",
    separate_points_2 => "separate.points.2",
    separate_points_3 => "separate.points.3",
    separate_points_4 => "separate.points.4",
    separate_points_5 => "separate.points.5",
    separate_points_6 => "separate.points.6",
    separate_points_7 => "separate.points.7",
    separate_points_8 => "separate.points.8",
    separate_points_9 => "separate.points.9",
    separate_points_10 => "separate.points.10",
    interleaved_triangles_1 => "interleaved.triangles.1",
    interleaved_triangles_2 => "interleaved.triangles.2",
    interleaved_triangles_3 => "interleaved.triangles.3",
    interleaved_triangles_4 => "interleaved.triangles.4",
    interleaved_triangles_5 => "interleaved.triangles.5",
    interleaved_triangles_6 => "interleaved.triangles.6",
    interleaved_triangles_7 => "interleaved.triangles.7",
    interleaved_triangles_8 => "interleaved.triangles.8",
    interleaved_triangles_9 => "interleaved.triangles.9",
    interleaved_triangles_10 => "interleaved.triangles.10",
    separate_triangles_1 => "separate.triangles.1",
    separate_triangles_2 => "separate.triangles.2",
    separate_triangles_3 => "separate.triangles.3",
    separate_triangles_4 => "separate.triangles.4",
    separate_triangles_5 => "separate.triangles.5",
    separate_triangles_6 => "separate.triangles.6",
    separate_triangles_7 => "separate.triangles.7",
    separate_triangles_8 => "separate.triangles.8",
    separate_triangles_9 => "separate.triangles.9",
    separate_triangles_10 => "separate.triangles.10",
}
