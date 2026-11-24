// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The 4096-wide targets of dEQP-GLES3's shader executor, drawn with the
//! highp precision shaders of Pi 4 images S03a and S07, through the GL API,
//! the real GL core, the real V3D back end and the REAL job builder (plan 399,
//! item s03a-host-test; #1261).
//!
//! On the Pi 4 at `b1b5b845` dEQP ended S03a at
//! `shaders.builtin_functions.precision.abs.highp_fragment.vec2` and S07 at
//! `shaders.builtin_functions.precision.reflect.highp_fragment.vec3`: the
//! executor (`glsShaderExecUtil.cpp`, `FragmentOutExecutor`) draws its inputs
//! into a target 4096 pixels wide and 2-6 rows high, the V3D back end refused
//! it for being larger than the 2048 x 2048 tile memory (refusal A5,
//! docs/notes/v3d-refusals.md, the S03a and S07 rows), the draw came back as
//! `GL_OUT_OF_MEMORY` and dEQP aborted the image. Commit `6d2e7d0d` sized the
//! tile memory for 4096 (`v3d::backend::MAX_TARGET`). tests/es3_large_target_v3d.rs
//! pins that with a stand-in job builder; this file draws the executor's own
//! shapes through the real one (`driver/src/v3d/v3d_job.c`, `v3d_frame.c`,
//! built with `cc` into a shared object and loaded with `dlopen`, as
//! tests/es3_tf_random_capture_v3d.rs does).
//!
//! What the executor does here is read from docs/notes/v3d-large-targets.md
//! (the CTS source was not on this host): a framebuffer object with one colour
//! renderbuffer, 4096 x 2..6, the viewport on it, a flat-varying vertex
//! shader, a highp fragment shader and one draw that covers the target. ASSUMED:
//! the format. ES 3.0 has no float renderbuffer without
//! GL_EXT_color_buffer_float, which the GL core does not list, so the highp
//! outputs are packed with `floatBitsToUint` into a `GL_RGBA32UI` target
//! (128 bpp, "128-bpp outputs included" in the note). The program, the
//! target and the draw are the case's; the inputs are one value.
//!
//! The GPU is stood in for: the real job builder builds the job (and refuses
//! what it cannot take, `v3d_job_draw`'s status), and at job end the stand-in
//! stores the values the shader writes, computed on the host from the same
//! formulas, into the job's target, inside its `width` x `height`, never past
//! the allocation. So the read-back shows that the frame the back end chose
//! is the whole target, with the values `abs` and `reflect` give; it does not
//! show the shader's arithmetic, which only the board can.
//!
//! Held, for 4096 x 2, 3, 4, 5 and 6 with `abs(vec2)` and `reflect(vec3)`:
//!
//! - the framebuffer is complete and the draw raises no GL error and is not
//!   refused (the back end's refused-draw count is unchanged, the library
//!   logs no refusal, no draw returned `-ENOTSUP` or `-EINVAL` from the job
//!   builder);
//! - every draw builds one job, the size of the target, with one job draw;
//! - every pixel reads back as the expected `abs` or `reflect` value;
//! - a target past 4096 that reaches the back end anyway (the GL core's own
//!   draw, resized by a spy in front of it: the GL API itself stops at 4096
//!   with `GL_INVALID_VALUE`) is refused with -ENOTSUP, which the GL core
//!   raises as `GL_INVALID_OPERATION`.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_precision_target_v3d
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
use yakogl::backend::{GlesDraw, YakoglBackend};
use yakogl::gl::buffer::glBindBuffer;
use yakogl::gl::consts::*;
use yakogl::gl::context;
use yakogl::gl::draw::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::{glEnableVertexAttribArray, glVertexAttribPointer};
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps, OsalV3dCpuOf};
use yakogl::v3d::backend::{v3d_refused_draws, yakogl_backend_v3d};
use yakogl::v3d::ffi::*;
use yakogl::v3d::kick::V3dKick;

/// Every log line the library wrote since the test began.
static LOG: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn log_take() -> Vec<String> {
    core::mem::take(&mut *LOG.lock().unwrap_or_else(|e| e.into_inner()))
}

/// Every line since the test began, kept.
fn log_peek() -> Vec<String> {
    LOG.lock().unwrap_or_else(|e| e.into_inner()).clone()
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
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("es3_precision_target_v3d");
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

/* ---- the job builder: the real one, observed; the GPU: stood in for ---- */

#[derive(Default)]
struct Seen {
    /// Target size of every job begun.
    jobs: Vec<(u32, u32)>,
    /// Status of every `v3d_job_draw`.
    draws: Vec<i32>,
    /// Draws of every job ended.
    drawn: Vec<u32>,
    /// `v3d_job_begin` / `v3d_job_end` answers that were false.
    failed: u32,
    /// Raster stores the stand-in GPU would have written past their allocation.
    overruns: u32,
}

static SEEN: Mutex<Seen> =
    Mutex::new(Seen { jobs: Vec::new(), draws: Vec::new(), drawn: Vec::new(), failed: 0, overruns: 0 });

fn seen<R>(f: impl FnOnce(&mut Seen) -> R) -> R {
    f(&mut SEEN.lock().unwrap_or_else(|e| e.into_inner()))
}

/// What the stand-in GPU stores for a drawn job: the RGBA32UI pixel the
/// current program writes, as 16 bytes.
static PIXEL: Mutex<[u8; 16]> = Mutex::new([0; 16]);

/// `px` stored over `w` x `h` pixels at `addr`, `stride` bytes a row, inside
/// the platform allocation that holds `addr`.
fn fill(addr: u32, stride: u32, w: u32, h: u32, px: [u8; 16]) {
    if addr == 0 || w == 0 || h == 0 {
        return;
    }
    let ok = with_platform(|p| match p.allocs.range_mut(..=addr as u64).next_back() {
        Some((base, a)) => {
            let off = (addr as u64 - base) as usize;
            let last = off + (h as usize - 1) * stride as usize + w as usize * 16;
            if last > a.size {
                return false;
            }
            // SAFETY: the allocation is `size` bytes of u64s and `last` is inside it.
            let bytes = unsafe { core::slice::from_raw_parts_mut(a.mem.as_mut_ptr() as *mut u8, a.size) };
            for y in 0..h as usize {
                for x in 0..w as usize {
                    let o = off + y * stride as usize + x * 16;
                    bytes[o..o + 16].copy_from_slice(&px);
                }
            }
            true
        }
        None => true,
    });
    if !ok {
        seen(|s| s.overruns += 1);
    }
}

fn real<T: Copy>(name: &str) -> T {
    let p = sym(name);
    // SAFETY: `T` is the function pointer type of that C function (src/v3d/ffi.rs).
    unsafe { core::mem::transmute_copy(&p) }
}

#[no_mangle]
unsafe extern "C" fn v3d_job_begin(
    j: *mut V3dJob,
    t: *const V3dJobTarget,
    bcl: *mut V3dCl,
    rcl: *mut V3dCl,
    ind: *mut V3dCl,
) -> bool {
    let f: unsafe extern "C" fn(*mut V3dJob, *const V3dJobTarget, *mut V3dCl, *mut V3dCl, *mut V3dCl) -> bool =
        real("v3d_job_begin");
    // SAFETY: the backend's live job, target and lists, passed on.
    let (ok, w, h) = unsafe { (f(j, t, bcl, rcl, ind), (*t).width, (*t).height) };
    seen(|s| {
        s.jobs.push((w, h));
        s.failed += !ok as u32;
    });
    ok
}

#[no_mangle]
unsafe extern "C" fn v3d_job_draw(j: *mut V3dJob, d: *const V3dJobDraw, layout: *mut c_void) -> i32 {
    let f: unsafe extern "C" fn(*mut V3dJob, *const V3dJobDraw, *mut c_void) -> i32 = real("v3d_job_draw");
    // SAFETY: the backend's live job and draw, passed on.
    let rc = unsafe { f(j, d, layout) };
    seen(|s| s.draws.push(rc));
    rc
}

#[no_mangle]
unsafe extern "C" fn v3d_job_end(
    j: *mut V3dJob,
    c: *const V3dJobClear,
    tile_list_start: *mut u32,
    tile_list_end: *mut u32,
) -> bool {
    let f: unsafe extern "C" fn(*mut V3dJob, *const V3dJobClear, *mut u32, *mut u32) -> bool = real("v3d_job_end");
    // SAFETY: the backend's live job, passed on; read before the call.
    let (ok, t, drew) = unsafe {
        let (drew, t) = ((*j).draws, (*j).t);
        (f(j, c, tile_list_start, tile_list_end), t, drew)
    };
    seen(|s| {
        s.drawn.push(drew);
        s.failed += !ok as u32;
    });
    if ok && drew > 0 {
        let px = *PIXEL.lock().unwrap_or_else(|e| e.into_inner());
        fill(t.address, t.stride, t.width, t.height, px);
    }
    ok
}

/* ---- the spy: the GL core's own draw, resized on request ---- */

/// The target size the spy hands the back end instead of the GL core's
/// (`None`: the draw is passed on as it is).
static RESIZE: Mutex<Option<(u32, u32)>> = Mutex::new(None);

unsafe extern "C" fn spy_draw(d: *const GlesDraw) -> i32 {
    let draw = yakogl_backend_v3d.draw.expect("the V3D draws");
    match *RESIZE.lock().unwrap_or_else(|e| e.into_inner()) {
        Some((w, h)) => {
            // SAFETY: the GL core passes a live draw; the copy lives for the call.
            let mut c = unsafe { *d };
            (c.target_w, c.target_h) = (w, h);
            // SAFETY: the back end's entry point with a draw that lives for the call.
            unsafe { draw(&c) }
        }
        // SAFETY: the GL core's draw, passed on.
        None => unsafe { draw(d) },
    }
}

fn spy_backend() -> &'static YakoglBackend {
    Box::leak(Box::new(YakoglBackend { draw: Some(spy_draw), ..yakogl_backend_v3d }))
}

/* ---- the context ---- */

static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

/// The one context and back end of the file; the tests take turns and each
/// starts with no error pending, nothing recorded and the log empty.
fn rig() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    ONCE.call_once(|| {
        // SAFETY: statics; the lock serialises every GL call of the file.
        unsafe {
            osal_set_ops(osal());
            yakogl::backend::g_backend = spy_backend();
            yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            assert_eq!(yakogl::backend::init(), 0, "{:?}", log_take());
            assert!(!context::gl_context_create(32, 32).is_null(), "{:?}", log_take());
        }
    });
    *RESIZE.lock().unwrap_or_else(|e| e.into_inner()) = None;
    // SAFETY: plain GL calls on the one context.
    unsafe {
        glUseProgram(0);
        glFinish();
        while glGetError() != GL_NO_ERROR {}
    }
    log_take();
    seen(|s| *s = Seen::default());
    g
}

/* ---- the executor's programs ---- */

const VS: &str = "#version 300 es
in highp vec4 a_position;
in highp vec3 a_in0;
flat out highp vec3 v_in0;
void main() { gl_Position = a_position; v_in0 = a_in0; }
";

/// `abs(vec2)`, highp, packed as the executor packs a float output.
const FS_ABS: &str = "#version 300 es
flat in highp vec3 v_in0;
layout(location = 0) out highp uvec4 o_out0;
void main() {
    highp vec2 r = abs(v_in0.xy);
    o_out0 = uvec4(floatBitsToUint(r.x), floatBitsToUint(r.y), 0u, 0u);
}
";

/// `reflect(vec3, vec3)`, highp: I is the input, N = (0, 0, 1).
const FS_REFLECT: &str = "#version 300 es
flat in highp vec3 v_in0;
layout(location = 0) out highp uvec4 o_out0;
void main() {
    highp vec3 r = reflect(v_in0, vec3(0.0, 0.0, 1.0));
    o_out0 = uvec4(floatBitsToUint(r.x), floatBitsToUint(r.y), floatBitsToUint(r.z), 0u);
}
";

/// The one input value of the three vertices.
static IN3: [f32; 9] = [-1.5, 2.25, 3.0, -1.5, 2.25, 3.0, -1.5, 2.25, 3.0];

/// What `abs(IN3.xy)` is, as the pixel's four u32.
fn want_abs() -> [u32; 4] {
    [1.5f32.to_bits(), 2.25f32.to_bits(), 0, 0]
}

/// What `reflect(IN3, (0, 0, 1))` is, I - 2 dot(N, I) N: (-1.5, 2.25, -3).
fn want_reflect() -> [u32; 4] {
    [(-1.5f32).to_bits(), 2.25f32.to_bits(), (-3.0f32).to_bits(), 0]
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
        assert_eq!(ok, 1, "{text:?} did not compile: {:?}", log_take());
        s
    }
}

/// Link and use (`VS`, `fs`) and point the attributes at `IN3` on three
/// vertices of a triangle that covers the target.
fn program(fs: &str) -> GLuint {
    static TRI: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 3.0, -1.0, 0.0, 1.0, -1.0, 3.0, 0.0, 1.0];
    let (v, f) = (shader(GL_VERTEX_SHADER, VS), shader(GL_FRAGMENT_SHADER, fs));
    // SAFETY: plain GL calls; the arrays are static.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glBindAttribLocation(p, 0, c"a_position".as_ptr());
        glBindAttribLocation(p, 1, c"a_in0".as_ptr());
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "link: {:?}", log_take());
        glUseProgram(p);
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, TRI.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glVertexAttribPointer(1, 3, GL_FLOAT, 0, 0, IN3.as_ptr() as *const c_void);
        glEnableVertexAttribArray(1);
        p
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

/// A framebuffer of one `w` x `h` RGBA32UI renderbuffer, current, with the
/// viewport on it: (framebuffer, renderbuffer, the storage's error).
fn fbo(w: GLsizei, h: GLsizei) -> (GLuint, GLuint, GLenum) {
    let (mut fb, mut rb) = (0, 0);
    // SAFETY: plain GL calls with room for one name each.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorage(GL_RENDERBUFFER, GL_RGBA32UI, w, h);
        let e = err();
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb);
        glViewport(0, 0, w, h);
        (fb, rb, e)
    }
}

fn delete(fb: GLuint, rb: GLuint) {
    // SAFETY: plain GL calls on names made here.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fb);
        glDeleteRenderbuffers(1, &rb);
    }
}

fn pixel_bytes(px: [u32; 4]) -> [u8; 16] {
    let mut b = [0u8; 16];
    for (i, v) in px.iter().enumerate() {
        b[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
    }
    b
}

/// The bound framebuffer's colour attachment 0, `w` x `h` RGBA32UI pixels.
fn read(w: u32, h: u32) -> Vec<u32> {
    let mut px = vec![0xEEEE_EEEEu32; (w * h * 4) as usize];
    // SAFETY: plain GL call; `px` holds `w` x `h` RGBA32UI pixels.
    unsafe {
        glReadPixels(
            0,
            0,
            w as GLsizei,
            h as GLsizei,
            GL_RGBA_INTEGER,
            GL_UNSIGNED_INT,
            px.as_mut_ptr() as *mut c_void,
        );
    }
    assert_eq!(err(), GL_NO_ERROR, "glReadPixels {w}x{h}");
    px
}

/// One executor draw of `fs` into a `w` x `h` target: nothing refused, one
/// job the size of the target with one draw, every pixel `want`.
fn executor_draw(what: &str, fs: &str, want: [u32; 4], w: u32, h: u32) {
    let _g = rig();
    executor_draw_locked(what, fs, want, w, h);
}

fn executor_draw_locked(what: &str, fs: &str, want: [u32; 4], w: u32, h: u32) {
    *PIXEL.lock().unwrap_or_else(|e| e.into_inner()) = pixel_bytes(want);
    let (fb, rb, e) = fbo(w as GLsizei, h as GLsizei);
    assert_eq!(e, GL_NO_ERROR, "{what} {w}x{h}: storage");
    // SAFETY: plain GL call.
    let status = unsafe { glCheckFramebufferStatus(GL_FRAMEBUFFER) };
    assert_eq!(status, GL_FRAMEBUFFER_COMPLETE, "{what} {w}x{h}");
    let p = program(fs);
    let base = v3d_refused_draws();
    log_take();
    seen(|s| *s = Seen::default());
    // SAFETY: plain GL calls; the vertex data is static.
    unsafe {
        glDrawArrays(GL_TRIANGLES, 0, 3);
        glFinish();
    }
    assert_eq!(err(), GL_NO_ERROR, "{what} {w}x{h}: {:?}", log_peek());
    assert_eq!(v3d_refused_draws(), base, "{what} {w}x{h}: refused: {:?}", log_peek());
    let log = log_peek();
    assert!(!log.iter().any(|l| l.contains("refused")), "{what} {w}x{h}: {log:?}");
    let (jobs, draws, drawn, failed, overruns) =
        seen(|s| (s.jobs.clone(), s.draws.clone(), s.drawn.clone(), s.failed, s.overruns));
    assert_eq!(failed, 0, "{what} {w}x{h}: the job builder said no");
    assert_eq!(draws, [V3D_JOB_OK], "{what} {w}x{h}: one job draw, taken (no -ENOTSUP, no -EINVAL)");
    assert_eq!(jobs, [(w, h)], "{what} {w}x{h}: one job, the size of the target");
    assert_eq!(drawn, [1], "{what} {w}x{h}: the job holds the draw");
    assert_eq!(overruns, 0, "{what} {w}x{h}: a store left its allocation");
    let px = read(w, h);
    for (n, q) in px.chunks(4).enumerate() {
        assert_eq!(q, want, "{what} {w}x{h}: pixel ({}, {})", n as u32 % w, n as u32 / w);
    }
    // SAFETY: plain GL call on a program made here.
    unsafe { glDeleteProgram(p) };
    delete(fb, rb);
}

/// S03a: `shaders.builtin_functions.precision.abs.highp_fragment.vec2`, at
/// every height the executor sizes its target to.
#[test]
fn abs_vec2_highp_fragment_draws_to_4096_by_2_to_6() {
    for h in 2..=6 {
        executor_draw("abs.highp_fragment.vec2", FS_ABS, want_abs(), 4096, h);
    }
}

/// S07: `shaders.builtin_functions.precision.reflect.highp_fragment.vec3`.
#[test]
fn reflect_vec3_highp_fragment_draws_to_4096_by_2_to_6() {
    for h in 2..=6 {
        executor_draw("reflect.highp_fragment.vec3", FS_REFLECT, want_reflect(), 4096, h);
    }
}

/// The GL API stops a side past 4096 at the storage (GL_INVALID_VALUE); a
/// target that reaches the back end anyway (the spy resizes the GL core's own
/// draw) is refused with -ENOTSUP, which the GL core raises as
/// GL_INVALID_OPERATION, named for the tile memory; the context draws
/// 4096 x 4 after.
#[test]
fn a_target_wider_than_4096_is_refused_with_enotsup() {
    let _g = rig();
    let (fb, rb, e) = fbo(4097, 4);
    assert_eq!(e, GL_INVALID_VALUE, "the storage past 4096");
    delete(fb, rb);
    let (fb, rb, e) = fbo(4096, 4);
    assert_eq!(e, GL_NO_ERROR);
    let p = program(FS_ABS);
    let base = v3d_refused_draws();
    log_take();
    *RESIZE.lock().unwrap_or_else(|e| e.into_inner()) = Some((4097, 4));
    // SAFETY: plain GL calls; the vertex data is static.
    unsafe {
        glDrawArrays(GL_TRIANGLES, 0, 3);
        glFinish();
    }
    *RESIZE.lock().unwrap_or_else(|e| e.into_inner()) = None;
    assert_eq!(err(), GL_INVALID_OPERATION, "-ENOTSUP is GL_INVALID_OPERATION: {:?}", log_peek());
    assert_eq!(v3d_refused_draws().wrapping_sub(base), 1, "counted as refused");
    let why = "a target larger than the tile memory is sized for (4096x4096)";
    assert!(log_peek().iter().any(|l| l.contains("draw refused") && l.contains(why)), "{:?}", log_peek());
    // SAFETY: plain GL call on a program made here.
    unsafe { glDeleteProgram(p) };
    delete(fb, rb);
    /* and the context works after */
    executor_draw_locked("after the refusal", FS_ABS, want_abs(), 4096, 4);
}
