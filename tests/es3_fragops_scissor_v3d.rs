// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! dEQP-GLES3 `fragment_ops.scissor.*` clears on the V3D back end, on the host
//! (image FO6 of the Pi 4 stopped at `fragment_ops.scissor.clear_int_buffer`,
//! docs/es3/pi4-gles3-fails.tsv; e883be8f added the integer clear quad).
//!
//! The GL calls go through the GL core into the REAL V3D back end
//! (`yakogl_backend_v3d`); the job builder (driver/src/v3d/v3d_job.c) is
//! stood in for by fakes that record each job's target, TLB clear words and
//! every draw's scissor box and colour mask, as tests/es3_formats_v3d.rs
//! `gl_rig` does. The host has no V3D rasteriser and no tile buffer, so what
//! is held is the back end's decision for each clear and the state it hands
//! the job builder:
//!
//! - a clear of the whole target with every channel written is the TLB's
//!   (the job's clear words, no draw); the fake "GPU" stores those words in
//!   the raster, so glReadPixels sees them;
//! - a scissored clear, or one with a colour mask that leaves a channel out,
//!   is the clear quad: a draw whose scissor box is the GL box clamped to the
//!   target (an empty box: no draw at all, the target keeps its pixels) and
//!   whose colour mask is the GL one; the job loads the target first, so the
//!   pixels outside the box are the ones from before. The quad's own
//!   fragments are not run here, so "inside the box" is the recorded
//!   scissor box, not a pixel;
//! - a refused clear is a GL error (the back end's refusal reaches the
//!   application as GL_OUT_OF_MEMORY today, docs/es3-cts-failures.md) and
//!   a refusal line in the platform log; the refusal's name is asserted.
//!
//! Refused today, pinned by a test that is not ignored, with the expected
//! result in an `#[ignore = "<refusal name>"]` test beside it:
//! `a scissored or masked colour clear of several colour attachments`.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_fragops_scissor_v3d
//! ```
//!
//! The file compiles to nothing under report-es2 (the ES 3.0 clears are not
//! there).

#![cfg(all(
    feature = "host-test",
    feature = "gpu-v3d",
    feature = "shaderc-online",
    not(feature = "report-es2"),
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
#![allow(clippy::too_many_arguments)]

use core::ffi::{c_char, c_int, c_void, CStr};
use std::collections::BTreeMap;
use std::ffi::CString;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Mutex, MutexGuard, Once, OnceLock};

use yakogl::gl::consts::*;
use yakogl::gl::context::{self, GlContext};
use yakogl::gl::es3::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::state::*;
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps, OsalV3dCpuOf};
use yakogl::v3d::backend::*;
use yakogl::v3d::ffi::*;
use yakogl::v3d::kick::V3dKick;

/* ---- the real layout and packers, built from driver/src/v3d/v3d_tex.c ------ */

extern "C" {
    fn dlopen(path: *const c_char, flags: c_int) -> *mut c_void;
    fn dlsym(h: *mut c_void, name: *const c_char) -> *mut c_void;
}

/// `driver/src/v3d/v3d_tex.c` and the Mesa tiling it includes, as a shared
/// object of this test's own, reached through the handle only (RTLD_LOCAL),
/// as tests/es3_formats_v3d.rs builds it. The back end's own start-up lays
/// out its black texture with it.
fn lib() -> *mut c_void {
    static L: OnceLock<usize> = OnceLock::new();
    *L.get_or_init(|| {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("es3_fragops_scissor_v3d");
        std::fs::create_dir_all(&dir).expect("the scratch directory");
        let so = dir.join(format!("v3d_tex-{}.so", std::process::id()));
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

/* The signatures are src/v3d/ffi.rs's. Kept as written: a macro call wider
 * than the width is wrapped by some rustfmt releases and left by others, so
 * the gate's `cargo fmt --check` must not depend on the host's release. */
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
}

/* ---- the job builder, stood in for (driver/src/v3d/v3d_job.c) -------------- */

/// One draw of a job: the scissor box (`min_x, min_y, max_x, max_y`, as the
/// job builder gets it) when the scissor test is on, and the colour mask.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SeenDraw {
    scissor: Option<[u32; 4]>,
    colormask: u32,
    /// The depth test is on and writes (a depth clear quad: func ALWAYS).
    depth_write: bool,
    /// The stencil test is on: the front face's write mask and reference.
    stencil: Option<(u32, u32)>,
}

#[derive(Clone, Debug)]
struct SeenJob {
    target: V3dJobTarget,
    clear: Option<V3dJobClear>,
    draws: Vec<SeenDraw>,
}

static JOBS: Mutex<Vec<SeenJob>> = Mutex::new(Vec::new());

fn jobs() -> MutexGuard<'static, Vec<SeenJob>> {
    JOBS.lock().unwrap_or_else(|e| e.into_inner())
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
    jobs().push(SeenJob { target: t, clear: None, draws: Vec::new() });
    t.width != 0 && t.height != 0
}

#[no_mangle]
unsafe extern "C" fn v3d_job_draw(j: *mut V3dJob, d: *const V3dJobDraw, _layout: *mut c_void) -> i32 {
    // SAFETY: the backend passes its live job and a draw whose pointers live for the call.
    let (j, d) = unsafe { (&mut *j, &*d) };
    // SAFETY: the draw's state record is live for the call.
    let st = unsafe { &*d.st };
    j.draws += 1;
    // SAFETY: the job's BCL is live.
    unsafe { (*j.bcl).used += 120 };
    if let Some(job) = jobs().last_mut() {
        job.draws.push(SeenDraw {
            scissor: st.scissor.then_some([st.scissor_minx, st.scissor_miny, st.scissor_maxx, st.scissor_maxy]),
            colormask: st.colormask,
            depth_write: st.depth_enabled && st.depth_writemask,
            stencil: st.stencil_enabled.then_some((st.stencil_front.write_mask, st.stencil_front.ref_)),
        });
    }
    V3D_JOB_OK
}

#[no_mangle]
unsafe extern "C" fn v3d_job_end(
    j: *mut V3dJob,
    c: *const V3dJobClear,
    _tile_list_start: *mut u32,
    _tile_list_end: *mut u32,
) -> bool {
    // SAFETY: the backend passes its live job and clear.
    let (j, c) = unsafe { (&mut *j, *c) };
    // SAFETY: the job's lists are live.
    unsafe {
        (*j.bcl).used += 1;
        (*j.rcl).used = 200;
    }
    if let Some(job) = jobs().last_mut() {
        job.clear = Some(c);
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

/// The tile-size table of v3d_frame.c (Mesa's v3d_choose_tile_size), as the
/// backend's unit tests fake it.
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
unsafe extern "C" fn v3d_frame_alloc_sizes(t: *const V3dTiling, _draws: u32, _page: u32, ta: *mut u32, ts: *mut u32) {
    // SAFETY: the backend's parameters.
    unsafe {
        let n = (*t).tiles_x * (*t).tiles_y;
        *ta = n * 128 + 8192;
        *ts = n * 256;
    }
}

/* ---- the platform ------------------------------------------------------------ */

/// GPU addresses are 32-bit (the V3D's MMU), from here up.
const GPU_BASE: u64 = 0x0100_0000;

struct Alloc {
    /// u64 backing: every u32 view of it is aligned.
    mem: Box<[u64]>,
    size: usize,
}

#[derive(Default)]
struct Platform {
    next: u64,
    allocs: BTreeMap<u64, Alloc>,
    kicks: Vec<V3dKick>,
    log: Vec<String>,
}

static PLATFORM: Mutex<Option<Platform>> = Mutex::new(None);

fn with_platform<R>(f: impl FnOnce(&mut Platform) -> R) -> R {
    let mut g = PLATFORM.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(Platform::default))
}

/// The CPU view of `n` bytes at GPU address `gpu`.
fn platform_cpu(p: &mut Platform, gpu: u64, n: usize) -> *mut u8 {
    let (base, a) = p.allocs.range_mut(..=gpu).next_back().expect("an allocation below the address");
    let off = (gpu - base) as usize;
    assert!(off + n <= a.size, "0x{gpu:x}+{n} is past its allocation");
    // SAFETY: inside the allocation.
    unsafe { (a.mem.as_mut_ptr() as *mut u8).add(off) }
}

unsafe extern "C" fn p_gpu_alloc(_heap: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
    with_platform(|p| {
        if size == 0 || size > 256 << 20 {
            return -12;
        }
        let mut mem = vec![0u64; size.div_ceil(8)].into_boxed_slice();
        let gpu = GPU_BASE + p.next;
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

/// The bytes one pixel of the job's RT 0 holds after a TLB clear, or None
/// for a format this file does not model.
fn clear_texel(t: &V3dJobTarget, c: &V3dJobClear) -> Option<Vec<u8>> {
    if !t.typed {
        return Some(c.rgba8.to_le_bytes().to_vec());
    }
    let w = c.clear_rt[0];
    let words = |n: usize| -> Vec<u8> { w[..n].iter().flat_map(|x| x.to_le_bytes()).collect() };
    match t.rt_format[0] {
        V3D_OUTPUT_IMAGE_FORMAT_RGBA32I | V3D_OUTPUT_IMAGE_FORMAT_RGBA32UI => Some(words(4)),
        V3D_OUTPUT_IMAGE_FORMAT_RGBA16F => Some(words(2)),
        V3D_OUTPUT_IMAGE_FORMAT_R32F => Some(words(1)),
        _ => None,
    }
}

/// The "GPU": every job completes inside its submit (as src/osal/linux.c). A
/// job that does not load RT 0 leaves the TLB clear's texel in every pixel
/// of the raster store (`address`); a job that loads it leaves the raster as
/// it was. The clear quads' fragments are not run.
unsafe extern "C" fn p_submit(kick: *const c_void, len: usize) -> i64 {
    if len != size_of::<V3dKick>() {
        return -22;
    }
    // SAFETY: the backend's live descriptor of that size.
    let k = unsafe { *(kick as *const V3dKick) };
    let fence = with_platform(|p| {
        p.kicks.push(k);
        p.kicks.len()
    });
    let job = jobs().get(fence - 1).cloned().expect("the job this kick runs was begun and ended");
    let (t, c) = (job.target, job.clear.expect("the job was ended"));
    if !c.load_rt0 && t.address != 0 {
        if let Some(texel) = clear_texel(&t, &c) {
            with_platform(|p| {
                let row = (t.stride as usize).max(t.width as usize * texel.len());
                let base = platform_cpu(p, u64::from(t.address), row * t.height as usize);
                for y in 0..t.height as usize {
                    for x in 0..t.width as usize {
                        // SAFETY: inside the span platform_cpu checked.
                        unsafe {
                            core::ptr::copy_nonoverlapping(
                                texel.as_ptr(),
                                base.add(y * row + x * texel.len()),
                                texel.len(),
                            )
                        };
                    }
                }
            });
        }
    }
    fence as i64
}

unsafe extern "C" fn p_wait(_fence: i64, _timeout_us: i64) -> i32 {
    0
}

unsafe extern "C" fn p_completed() -> i64 {
    with_platform(|p| p.kicks.len() as i64)
}

unsafe extern "C" fn p_query(what: u32, out: *mut c_void, out_sz: usize) -> i32 {
    use yakogl::osal::{
        OSAL_QUERY_V3D_BIN_POOL, OSAL_QUERY_V3D_CPU_OF_GPU, OSAL_QUERY_V3D_JOB_OPEN, OSAL_QUERY_V3D_VPM_BYTES,
    };
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
    if what == OSAL_QUERY_V3D_JOB_OPEN && out_sz == 4 {
        return 0;
    }
    if what == OSAL_QUERY_V3D_BIN_POOL {
        return -61; /* ENODATA, as zephyr_v3d.c answers before the first job */
    }
    -22 /* no TFU, the default budgets, no clock */
}

/// The library logs every line as `log("%s", line)` (`osal::log_str`), so a
/// two-pointer function takes it; on x86-64 and AArch64 Linux a variadic call
/// passes those two arguments as a plain one does.
unsafe extern "C" fn p_log2(_fmt: *const c_char, line: *const c_char) {
    // SAFETY: the library passes a NUL-terminated line.
    let s = unsafe { CStr::from_ptr(line) }.to_string_lossy().into_owned();
    with_platform(|p| p.log.push(s));
}

type LogFn = unsafe extern "C" fn(*const c_char, ...);

unsafe extern "C" fn p_init(_platform_arg: *mut c_void) -> i32 {
    0
}

/// The platform's table (made once, at run time: the log hook is a cast).
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

/* ---- the context ------------------------------------------------------------- */

const CANVAS: (i32, i32) = (16, 16);

static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();
static mut CTX: *mut GlContext = core::ptr::null_mut();

/// The one context and backend of the file, set up once; the tests take turns.
fn rig() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    ONCE.call_once(|| {
        // SAFETY: statics; the lock serialises every GL call of the file.
        unsafe {
            osal_set_ops(osal());
            yakogl::backend::g_backend = &yakogl_backend_v3d;
            assert_eq!(yakogl::backend::init(), 0, "{:?}", logs());
            CTX = context::gl_context_create(CANVAS.0 as u32, CANVAS.1 as u32);
            assert!(!CTX.is_null(), "{:?}", logs());
        }
    });
    // SAFETY: plain GL calls.
    unsafe {
        glFinish();
        while glGetError() != GL_NO_ERROR {}
        glDisable(GL_SCISSOR_TEST);
        glColorMask(1, 1, 1, 1);
        glDepthMask(1);
        glStencilMask(!0);
    }
    g
}

fn logs() -> Vec<String> {
    with_platform(|p| p.log.clone())
}

/* ---- the cases' scaffolding ---------------------------------------------------- */

const SIDE: i32 = 8;

/// A framebuffer of renderbuffers: one per colour format, plus an optional
/// DEPTH24_STENCIL8 one. Deleted on drop.
struct Fb {
    fbo: GLuint,
    rbs: Vec<GLuint>,
}

impl Fb {
    fn new(colours: &[GLenum], depth_stencil: bool) -> Fb {
        let mut fb = Fb { fbo: 0, rbs: Vec::new() };
        // SAFETY: plain GL calls on names made here, with live out pointers.
        unsafe {
            glGenFramebuffers(1, &mut fb.fbo);
            glBindFramebuffer(GL_FRAMEBUFFER, fb.fbo);
            for (i, &f) in colours.iter().enumerate() {
                let rb = fb.renderbuffer(f);
                glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0 + i as u32, GL_RENDERBUFFER, rb);
            }
            if depth_stencil {
                let rb = fb.renderbuffer(GL_DEPTH24_STENCIL8);
                glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_DEPTH_STENCIL_ATTACHMENT, GL_RENDERBUFFER, rb);
            }
            let bufs: Vec<GLenum> = (0..colours.len() as u32).map(|i| GL_COLOR_ATTACHMENT0 + i).collect();
            if !bufs.is_empty() {
                glDrawBuffers(bufs.len() as GLsizei, bufs.as_ptr());
            }
            glViewport(0, 0, SIDE, SIDE);
        }
        fb
    }

    fn renderbuffer(&mut self, format: GLenum) -> GLuint {
        let mut rb = 0u32;
        // SAFETY: plain GL calls, a live out pointer.
        unsafe {
            glGenRenderbuffers(1, &mut rb);
            glBindRenderbuffer(GL_RENDERBUFFER, rb);
            glRenderbufferStorage(GL_RENDERBUFFER, format, SIDE, SIDE);
        }
        self.rbs.push(rb);
        rb
    }

    fn status(&self) -> GLenum {
        // SAFETY: a plain GL call.
        unsafe { glCheckFramebufferStatus(GL_FRAMEBUFFER) }
    }
}

impl Drop for Fb {
    fn drop(&mut self) {
        // SAFETY: names made by `new`.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteFramebuffers(1, &self.fbo);
            glDeleteRenderbuffers(self.rbs.len() as GLsizei, self.rbs.as_ptr());
        }
    }
}

fn err() -> GLenum {
    // SAFETY: a plain GL call.
    unsafe { glGetError() }
}

/// What a GL call sequence did to the back end: the jobs it made, the GL
/// error it left (the first one; the queue is emptied) and the refusal lines
/// it logged.
struct Effect {
    jobs: Vec<SeenJob>,
    error: GLenum,
    refusals: Vec<String>,
    /// Draws the back end refused (`v3d_refused_draws`) while `f` ran. The
    /// log names a cause once per process, so the count is the per-case proof.
    refused: u32,
}

fn observe(f: impl FnOnce()) -> Effect {
    // SAFETY: plain GL calls.
    unsafe {
        glFinish();
        while glGetError() != GL_NO_ERROR {}
    }
    let (j0, l0, r0) = (jobs().len(), logs().len(), v3d_refused_draws());
    f();
    // SAFETY: a plain GL call.
    unsafe { glFinish() };
    let error = err();
    while err() != GL_NO_ERROR {}
    /* the job list is never trimmed: a kick's fence is its index */
    let refusals = logs()[l0..].to_vec();
    Effect { jobs: jobs()[j0..].to_vec(), error, refusals, refused: v3d_refused_draws().wrapping_sub(r0) }
}

impl Effect {
    /// All draws of all jobs.
    fn draws(&self) -> Vec<SeenDraw> {
        self.jobs.iter().flat_map(|j| j.draws.iter().copied()).collect()
    }

    fn assert_no_error(&self) {
        assert_eq!(self.error, GL_NO_ERROR, "{:?}", self.refusals);
        assert_eq!(self.refused, 0, "{:?}", self.refusals);
    }

    /// The clear was refused as a draw of several attachments is today: one
    /// refused draw, GL_OUT_OF_MEMORY, nothing recorded, and the refusal's
    /// name in the platform log (written once per process, so any line since
    /// start-up).
    fn assert_refused_several_rt(&self, what: &str) {
        assert_eq!(self.error, GL_OUT_OF_MEMORY, "{what}: {:?}", self.refusals);
        assert_eq!(self.refused, 1, "{what}");
        assert!(self.jobs.is_empty(), "{what}: nothing is recorded");
        assert!(logs().iter().any(|l| l.contains(SEVERAL_RT)), "{what}: {:?}", logs());
    }
}

/// glReadPixels of the whole target in the given format and type, `bpp` bytes
/// a pixel, from the current read buffer.
fn read(format: GLenum, type_: GLenum, bpp: usize) -> Vec<u8> {
    let mut px = vec![0xEEu8; SIDE as usize * SIDE as usize * bpp];
    // SAFETY: a buffer of SIDE x SIDE pixels of `bpp` bytes.
    unsafe { glReadPixels(0, 0, SIDE, SIDE, format, type_, px.as_mut_ptr() as *mut c_void) };
    px
}

fn texel_at(px: &[u8], x: i32, y: i32, bpp: usize) -> &[u8] {
    let o = (y * SIDE + x) as usize * bpp;
    &px[o..o + bpp]
}

/// What the back end does with a clear under one scissor box.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Want {
    /// The box covers the target: the TLB's clear, no draw.
    Tlb,
    /// The clear quad, scissored to this box (min_x, min_y, max_x, max_y);
    /// an empty box that is on the target's far side clamps to [0, 0, 0, 0].
    Quad([u32; 4]),
    /// A box with no area (width or height 0): the back end is not called.
    Nothing,
}

/// The scissor boxes of the dEQP cases and of the generator's edge cases
/// (es3fScissorTests, es2fColorClearTest's `iterate`): inside, the whole
/// target, partly outside on each side, wholly outside, empty. On an 8x8
/// target.
const BOXES: [([i32; 4], Want); 9] = [
    ([2, 3, 4, 3], Want::Quad([2, 3, 6, 6])),
    ([0, 0, SIDE, SIDE], Want::Tlb),
    ([-4, -4, 8, 8], Want::Quad([0, 0, 4, 4])),
    ([5, 5, 10, 10], Want::Quad([5, 5, 8, 8])),
    ([-100, -100, 300, 300], Want::Tlb),
    ([20, 20, 4, 4], Want::Quad([0, 0, 0, 0])),
    ([-20, 2, 4, 4], Want::Quad([0, 0, 0, 0])),
    ([3, 3, 0, 4], Want::Nothing),
    ([3, 3, 4, 0], Want::Nothing),
];

fn scissor(b: [i32; 4]) {
    // SAFETY: plain GL calls.
    unsafe {
        glEnable(GL_SCISSOR_TEST);
        glScissor(b[0], b[1], b[2], b[3]);
    }
}

fn no_scissor() {
    // SAFETY: a plain GL call.
    unsafe { glDisable(GL_SCISSOR_TEST) };
}

/* ---- the colour formats ---------------------------------------------------- */

const FA: [f32; 4] = [0.25, 0.5, 0.75, 1.0];
const FB: [f32; 4] = [1.0, 0.0, 0.0, 0.0];
const IA: [i32; 4] = [-1, 2, 3, 4];
const IB: [i32; 4] = [7, -8, 9, -10];
const UA: [u32; 4] = [1, 2, 3, 4];
const UB: [u32; 4] = [5, 6, 7, 8];

fn words<T: Copy, const N: usize>(v: [T; N], f: impl Fn(T) -> [u8; 4]) -> Vec<u8> {
    v.into_iter().flat_map(f).collect()
}

/// One colour format: how it is cleared (value `0` or `1`), the bytes a pixel
/// holds after that clear, and how glReadPixels reads it back (None: the fake
/// GPU does not store that format, only the decision is checked).
struct Fmt {
    name: &'static str,
    gl: GLenum,
    clear: fn(usize),
    read: Option<(GLenum, GLenum, usize)>,
    texel: fn(usize) -> Vec<u8>,
}

const FORMATS: [Fmt; 7] = [
    Fmt {
        name: "RGBA8 glClearBufferfv",
        gl: GL_RGBA8,
        // SAFETY (all `clear` closures): plain GL calls, four values each.
        clear: |w| unsafe { glClearBufferfv(GL_COLOR, 0, [FA, FB][w].as_ptr()) },
        read: Some((GL_RGBA, GL_UNSIGNED_BYTE, 4)),
        texel: |w| [[0x40, 0x80, 0xbf, 0xff], [0xff, 0, 0, 0]][w].to_vec(),
    },
    Fmt {
        name: "RGBA8 glClear",
        gl: GL_RGBA8,
        clear: |w| unsafe {
            let c = [FA, FB][w];
            glClearColor(c[0], c[1], c[2], c[3]);
            glClear(GL_COLOR_BUFFER_BIT);
        },
        read: Some((GL_RGBA, GL_UNSIGNED_BYTE, 4)),
        texel: |w| [[0x40, 0x80, 0xbf, 0xff], [0xff, 0, 0, 0]][w].to_vec(),
    },
    Fmt {
        name: "RGBA16F glClearBufferfv",
        gl: GL_RGBA16F,
        clear: |w| unsafe { glClearBufferfv(GL_COLOR, 0, [FA, FB][w].as_ptr()) },
        read: Some((GL_RGBA, GL_FLOAT, 16)),
        texel: |w| words([FA, FB][w], |x| x.to_le_bytes()),
    },
    Fmt {
        name: "RGBA32I glClearBufferiv",
        gl: GL_RGBA32I,
        clear: |w| unsafe { glClearBufferiv(GL_COLOR, 0, [IA, IB][w].as_ptr()) },
        read: Some((GL_RGBA_INTEGER, GL_INT, 16)),
        texel: |w| words([IA, IB][w], |x| x.to_le_bytes()),
    },
    Fmt {
        name: "RGBA32UI glClearBufferuiv",
        gl: GL_RGBA32UI,
        clear: |w| unsafe { glClearBufferuiv(GL_COLOR, 0, [UA, UB][w].as_ptr()) },
        read: Some((GL_RGBA_INTEGER, GL_UNSIGNED_INT, 16)),
        texel: |w| words([UA, UB][w], |x| x.to_le_bytes()),
    },
    Fmt {
        name: "RGBA8I glClearBufferiv",
        gl: GL_RGBA8I,
        clear: |w| unsafe { glClearBufferiv(GL_COLOR, 0, [IA, IB][w].as_ptr()) },
        read: None,
        texel: |_| Vec::new(),
    },
    Fmt {
        name: "RG16I glClearBufferiv",
        gl: GL_RG16I,
        clear: |w| unsafe { glClearBufferiv(GL_COLOR, 0, [IA, IB][w].as_ptr()) },
        read: None,
        texel: |_| Vec::new(),
    },
];

impl Fmt {
    /// The colour mask the job builder gets for "every channel on": the GL
    /// core drops the alpha bit of a target without alpha (RG16I).
    fn mask_all(&self) -> u32 {
        if self.gl == GL_RG16I {
            0b0111
        } else {
            0b1111
        }
    }

    fn pixels(&self) -> Option<Vec<u8>> {
        // SAFETY: a plain GL call.
        self.read.map(|(f, t, bpp)| unsafe {
            glFinish();
            read(f, t, bpp)
        })
    }
}

/// Every pixel of `px` inside `b` (min_x, min_y, max_x, max_y) if `inside`,
/// else outside it, holds `texel`.
fn region_is(px: &[u8], bpp: usize, b: [u32; 4], inside: bool, texel: &[u8]) -> bool {
    (0..SIDE).all(|y| {
        (0..SIDE).all(|x| {
            let in_box = (x as u32) >= b[0] && (x as u32) < b[2] && (y as u32) >= b[1] && (y as u32) < b[3];
            in_box != inside || texel_at(px, x, y, bpp) == texel
        })
    })
}

/* ---- colour clears under a scissor box ------------------------------------- */

/// `fragment_ops.scissor.clear_color` and `clear_int_buffer`, for each colour
/// format: every box of the table is accepted, and the back end clears with
/// the TLB when the box covers the target, otherwise with a clear quad
/// scissored to the box clamped to the target, loading the target first.
/// Pixels outside the box keep the earlier clear's value.
#[test]
fn colour_clears_under_a_scissor_box_are_accepted() {
    let _g = rig();
    for fmt in &FORMATS {
        let fb = Fb::new(&[fmt.gl], false);
        assert_eq!(fb.status(), GL_FRAMEBUFFER_COMPLETE, "{}", fmt.name);
        for (b, want) in BOXES {
            let what = format!("{} box {b:?}", fmt.name);
            no_scissor();
            let e = observe(|| (fmt.clear)(0));
            e.assert_no_error();
            assert_eq!(e.jobs.len(), 1, "{what}: the unscissored clear");
            let before = fmt.pixels();
            if let Some(px) = &before {
                let (_, _, bpp) = fmt.read.unwrap();
                assert!(region_is(px, bpp, [0, 0, 8, 8], true, &(fmt.texel)(0)), "{what}: the first clear");
            }

            scissor(b);
            let e = observe(|| (fmt.clear)(1));
            assert_eq!(e.error, GL_NO_ERROR, "{what}: {:?}", e.refusals);
            match want {
                Want::Nothing => {
                    assert!(e.jobs.is_empty(), "{what}: an empty box makes no job");
                }
                Want::Tlb => {
                    assert_eq!((e.jobs.len(), e.draws().len()), (1, 0), "{what}: the TLB clear");
                    assert!(!e.jobs[0].clear.unwrap().load_rt0, "{what}: the tile buffer starts cleared");
                }
                Want::Quad(q) => {
                    assert_eq!(e.jobs.len(), 1, "{what}");
                    assert!(e.jobs[0].clear.unwrap().load_rt0, "{what}: the quad draws over the old pixels");
                    assert_eq!(
                        e.draws(),
                        [SeenDraw { scissor: Some(q), colormask: fmt.mask_all(), depth_write: false, stencil: None }],
                        "{what}"
                    );
                }
            }
            // the pixels the clear must not touch (and, for the TLB, all of them)
            if let (Some(px), Some((_, _, bpp))) = (fmt.pixels(), fmt.read) {
                match want {
                    Want::Tlb => assert!(region_is(&px, bpp, [0, 0, 8, 8], true, &(fmt.texel)(1)), "{what}"),
                    Want::Quad(q) => assert!(region_is(&px, bpp, q, false, &(fmt.texel)(0)), "{what}: outside the box"),
                    Want::Nothing => assert!(region_is(&px, bpp, [0, 0, 8, 8], true, &(fmt.texel)(0)), "{what}"),
                }
            }
        }
        no_scissor();
    }
}

/// A colour mask that leaves a channel out is also a clear quad (the TLB
/// clears every channel), with or without a box, and the mask reaches the
/// job builder as written.
#[test]
fn masked_colour_clears_are_a_quad_with_the_mask() {
    let _g = rig();
    for fmt in &FORMATS {
        let fb = Fb::new(&[fmt.gl], false);
        assert_eq!(fb.status(), GL_FRAMEBUFFER_COMPLETE, "{}", fmt.name);
        for (mask, bits) in [([1, 0, 1, 1], 0b1101), ([0, 1, 0, 0], 0b0010), ([1, 1, 1, 0], 0b0111)] {
            for b in [None, Some([2, 3, 4, 3])] {
                let what = format!("{} mask {mask:?} box {b:?}", fmt.name);
                match b {
                    Some(b) => scissor(b),
                    None => no_scissor(),
                }
                // SAFETY: a plain GL call.
                unsafe { glColorMask(mask[0], mask[1], mask[2], mask[3]) };
                let e = observe(|| (fmt.clear)(1));
                // SAFETY: a plain GL call.
                unsafe { glColorMask(1, 1, 1, 1) };
                e.assert_no_error();
                assert_eq!(e.jobs.len(), 1, "{what}");
                assert_eq!(
                    e.draws(),
                    [SeenDraw {
                        scissor: b.map(|_| [2, 3, 6, 6]),
                        colormask: bits & fmt.mask_all(),
                        depth_write: false,
                        stencil: None
                    }],
                    "{what}"
                );
            }
        }
        no_scissor();
    }
}

/// A colour mask that writes nothing: no error, and nothing is drawn.
#[test]
fn a_clear_with_every_channel_masked_draws_nothing() {
    let _g = rig();
    for fmt in &FORMATS {
        let _fb = Fb::new(&[fmt.gl], false);
        scissor([2, 3, 4, 3]);
        // SAFETY: a plain GL call.
        unsafe { glColorMask(0, 0, 0, 0) };
        let e = observe(|| (fmt.clear)(1));
        // SAFETY: a plain GL call.
        unsafe { glColorMask(1, 1, 1, 1) };
        e.assert_no_error();
        assert!(e.draws().is_empty(), "{}: {:?}", fmt.name, e.draws());
    }
    no_scissor();
}

/* ---- depth and stencil clears under a scissor box ----------------------------- */

/// How a depth/stencil clear is written.
#[derive(Clone, Copy, Debug)]
enum Ds {
    /// glClear(DEPTH | STENCIL), optionally with COLOR.
    DepthAndStencil(bool),
    ClearDepth,
    ClearStencil,
    BufferFi,
    BufferFvDepth,
    BufferIvStencil,
}

impl Ds {
    fn run(self) {
        // SAFETY: plain GL calls.
        unsafe {
            glClearDepthf(0.5);
            glClearStencil(5);
            match self {
                Ds::DepthAndStencil(colour) => {
                    glClear(GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT | if colour { GL_COLOR_BUFFER_BIT } else { 0 })
                }
                Ds::ClearDepth => glClear(GL_DEPTH_BUFFER_BIT),
                Ds::ClearStencil => glClear(GL_STENCIL_BUFFER_BIT),
                Ds::BufferFi => glClearBufferfi(GL_DEPTH_STENCIL, 0, 0.25, 7),
                Ds::BufferFvDepth => glClearBufferfv(GL_DEPTH, 0, [0.25f32].as_ptr()),
                Ds::BufferIvStencil => glClearBufferiv(GL_STENCIL, 0, [7i32].as_ptr()),
            }
        }
    }

    /// (colour, depth, stencil reference) the clear writes.
    fn writes(self) -> (bool, bool, Option<u32>) {
        match self {
            Ds::DepthAndStencil(c) => (c, true, Some(5)),
            Ds::ClearDepth | Ds::BufferFvDepth => (false, true, None),
            Ds::ClearStencil => (false, false, Some(5)),
            Ds::BufferFi => (false, true, Some(7)),
            Ds::BufferIvStencil => (false, false, Some(7)),
        }
    }
}

const DS_FORMS: [Ds; 7] = [
    Ds::DepthAndStencil(false),
    Ds::DepthAndStencil(true),
    Ds::ClearDepth,
    Ds::ClearStencil,
    Ds::BufferFi,
    Ds::BufferFvDepth,
    Ds::BufferIvStencil,
];

/// `fragment_ops.scissor.clear_depth` and `clear_stencil`, on an RGBA8 +
/// DEPTH24_STENCIL8 framebuffer: a box inside the target is a clear quad
/// that writes the depth (func ALWAYS, writes on) and the stencil (REPLACE
/// of the clear value, write mask 0xff) under the scissor and leaves the
/// colour mask at 0 unless COLOR is cleared too; an empty box makes no job.
/// With the box covering the target the depth and stencil are the TLB's
/// when both are cleared together (no draw), and a single one of them is a
/// quad without a scissor (the TLB would clear the other too).
#[test]
fn depth_and_stencil_clears_under_a_scissor_box() {
    let _g = rig();
    let fb = Fb::new(&[GL_RGBA8], true);
    assert_eq!(fb.status(), GL_FRAMEBUFFER_COMPLETE);
    for form in DS_FORMS {
        let (colour, depth, stencil) = form.writes();
        for (b, want) in BOXES {
            let what = format!("{form:?} box {b:?}");
            scissor(b);
            let e = observe(|| form.run());
            assert_eq!(e.error, GL_NO_ERROR, "{what}: {:?}", e.refusals);
            let quad = |s: Option<[u32; 4]>| SeenDraw {
                scissor: s,
                colormask: if colour { 15 } else { 0 },
                depth_write: depth,
                stencil: stencil.map(|r| (0xff, r)),
            };
            match want {
                Want::Nothing => assert!(e.jobs.is_empty(), "{what}"),
                Want::Quad(q) => {
                    assert_eq!(e.jobs.len(), 1, "{what}");
                    assert_eq!(e.draws(), [quad(Some(q))], "{what}");
                }
                Want::Tlb => {
                    assert_eq!(e.jobs.len(), 1, "{what}");
                    let both = depth && stencil.is_some();
                    if both {
                        assert!(e.draws().is_empty(), "{what}: the TLB clears depth and stencil together");
                        let c = e.jobs[0].clear.unwrap();
                        let (z, s) = if matches!(form, Ds::BufferFi) { (0.25, 7) } else { (0.5, 5) };
                        assert_eq!((c.z, c.stencil), (z, s), "{what}");
                    } else {
                        assert_eq!(e.draws(), [quad(None)], "{what}: a lone depth or stencil clear is a quad");
                    }
                }
            }
        }
    }
    no_scissor();
}

/* ---- several colour attachments ----------------------------------------------- */

/// An RGBA8 and an RGBA32I attachment: a clear of the whole target that
/// writes every channel is the TLB's, with no error.
#[test]
fn an_unscissored_clear_of_two_attachments_is_accepted() {
    let _g = rig();
    let _fb = Fb::new(&[GL_RGBA8, GL_RGBA32I], false);
    no_scissor();
    let e = observe(|| {
        // SAFETY: plain GL calls.
        unsafe {
            glClearBufferfv(GL_COLOR, 0, FA.as_ptr());
            glClearBufferiv(GL_COLOR, 1, IA.as_ptr());
        }
    });
    e.assert_no_error();
    assert!(e.draws().is_empty(), "{:?}", e.draws());
}

/// The cases that reach the one refusal of the several-attachment clear:
/// (name, how the clear is written, with a scissor box or a colour mask).
fn two_attachment_clears() -> [(&'static str, fn()); 4] {
    [
        // SAFETY (all of them): plain GL calls.
        ("glClear of both, scissored", || {
            scissor([2, 3, 4, 3]);
            unsafe { glClear(GL_COLOR_BUFFER_BIT) }
        }),
        ("glClearBufferiv of attachment 1, scissored", || {
            scissor([2, 3, 4, 3]);
            unsafe { glClearBufferiv(GL_COLOR, 1, IA.as_ptr()) }
        }),
        ("glClearBufferfv of attachment 0, masked", || {
            no_scissor();
            unsafe {
                glColorMask(1, 0, 1, 1);
                glClearBufferfv(GL_COLOR, 0, FA.as_ptr());
            }
        }),
        ("glClear of both, scissored and masked", || {
            scissor([-4, -4, 8, 8]);
            unsafe {
                glColorMask(0, 1, 1, 1);
                glClear(GL_COLOR_BUFFER_BIT);
            }
        }),
    ]
}

const SEVERAL_RT: &str = "a scissored or masked colour clear of several colour attachments";

/// Pinned as it is today: the back end refuses the clear by name, nothing is
/// recorded, the GL core reports GL_OUT_OF_MEMORY (docs/es3-cts-failures.md),
/// and the context is usable afterwards (the next unscissored clear works).
#[test]
fn a_scissored_or_masked_clear_of_two_attachments_is_refused_today() {
    let _g = rig();
    let _fb = Fb::new(&[GL_RGBA8, GL_RGBA32I], false);
    for (what, run) in two_attachment_clears() {
        let e = observe(run);
        // SAFETY: a plain GL call.
        unsafe { glColorMask(1, 1, 1, 1) };
        no_scissor();
        e.assert_refused_several_rt(what);
        let after = observe(|| {
            // SAFETY: a plain GL call.
            unsafe { glClear(GL_COLOR_BUFFER_BIT) }
        });
        after.assert_no_error();
        assert_eq!(after.jobs.len(), 1, "{what}: the next clear works");
    }
}

/// The same with two integer attachments and with a float beside an integer
/// one: any second attachment brings the refusal.
#[test]
fn the_refusal_does_not_depend_on_the_attachments_classes() {
    let _g = rig();
    for pair in [[GL_RGBA8, GL_RGBA16F], [GL_RGBA32I, GL_RGBA32UI], [GL_RGBA32UI, GL_RGBA8]] {
        let _fb = Fb::new(&pair, false);
        scissor([2, 3, 4, 3]);
        let e = observe(|| {
            // SAFETY: a plain GL call.
            unsafe { glClear(GL_COLOR_BUFFER_BIT) }
        });
        no_scissor();
        e.assert_refused_several_rt(&format!("{pair:x?}"));
    }
}

/// What a back end that does the several-attachment clear must give: no
/// error, one job that loads the attachments, and a clear quad scissored to
/// the box clamped to the target, with the mask as written.
#[test]
#[ignore = "a scissored or masked colour clear of several colour attachments"]
fn a_scissored_or_masked_clear_of_two_attachments_is_a_quad() {
    let _g = rig();
    let _fb = Fb::new(&[GL_RGBA8, GL_RGBA32I], false);
    for (what, run) in two_attachment_clears() {
        let e = observe(run);
        // SAFETY: a plain GL call.
        unsafe { glColorMask(1, 1, 1, 1) };
        no_scissor();
        assert_eq!(e.error, GL_NO_ERROR, "{what}: {:?}", e.refusals);
        assert_eq!(e.jobs.len(), 1, "{what}");
        assert!(e.jobs[0].clear.unwrap().load_rt0, "{what}");
        assert_eq!(e.draws().len(), 1, "{what}");
    }
}
