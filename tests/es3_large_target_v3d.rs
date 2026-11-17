// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Targets of 4096 a side on the V3D host back end (#1261), through the GL
//! API, the real GL core and the real V3D back end. dEQP's shader executor
//! (glsShaderExecUtil.cpp) draws its inputs into a target about 4096 pixels
//! wide and 2-6 rows high; while the tile memory was sized for 2048 a side
//! (`v3d::backend::MAX_TARGET`) every such draw was refused, and Pi 4 images
//! S03-S08 (`shaders.builtin_functions.precision`) aborted on their first
//! case (docs/notes/v3d-large-targets.md). The GL core reports 4096 for
//! GL_MAX_RENDERBUFFER_SIZE, GL_MAX_TEXTURE_SIZE and GL_MAX_VIEWPORT_DIMS
//! (src/gl/state.rs), and so is the tile memory sized now.
//!
//! The job builder and the GPU are stood in for (as tests/es3_mrt4_sizes_v3d.rs):
//! each job's `V3dJobTarget` is recorded, and at job end RT 0's raster store
//! is cleared with the job's clear colour unless loaded and, when the job
//! drew, filled with the colour the test's program writes (`DRAW`), inside
//! the job's `width` x `height`, never past the allocation (an overrun is
//! recorded). So a read-back through glReadPixels shows the frame the back
//! end chose. Pinned:
//!
//! - 4096 x 4, 4 x 4096 and 4096 x 6 RGBA8 renderbuffers: complete, one job
//!   the size of the target, the clear and the draw taken, every pixel read
//!   back, no refusal;
//! - 4097 on a side is GL_INVALID_VALUE at the storage (past
//!   GL_MAX_RENDERBUFFER_SIZE), so the GL API makes no target the tile
//!   memory cannot take;
//! - a target past 4096 or an empty one that reaches the back end anyway (the
//!   GL core's own draw, resized by a spy in front of the back end) is
//!   refused by a text naming its cause: past 4096 with -ENOTSUP, which the
//!   GL core raises as GL_INVALID_OPERATION, empty with -EINVAL
//!   (GL_OUT_OF_MEMORY: a malformed draw); and the context works after.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_large_target_v3d
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-v3d", feature = "host-test", not(feature = "report-es2")))]

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
use yakogl::v3d::backend::{v3d_refused_draws, yakogl_backend_v3d, MAX_TARGET};
use yakogl::v3d::ffi::*;
use yakogl::v3d::kick::V3dKick;

/* ---- the job builder and the GPU: records, fills the raster store ---- */

#[derive(Default)]
struct Seen {
    jobs: Vec<V3dJobTarget>,
    log: Vec<String>,
    /// Raster stores the stand-in GPU would have written past their allocation.
    overruns: u32,
}

static SEEN: Mutex<Seen> = Mutex::new(Seen { jobs: Vec::new(), log: Vec::new(), overruns: 0 });

fn seen<R>(f: impl FnOnce(&mut Seen) -> R) -> R {
    f(&mut SEEN.lock().unwrap_or_else(|e| e.into_inner()))
}

fn log_lines() -> Vec<String> {
    seen(|s| s.log.clone())
}

/// What `FS` writes, vec4(0.25, 0.5, 0.75, 1.0), as RGBA8.
const DRAW: [u8; 4] = [64, 128, 191, 255];

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
    seen(|s| s.jobs.push(t));
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

/// `w` x `h` pixels of `px` at `addr`, `stride` bytes a row, inside the
/// platform allocation that holds `addr`.
fn fill(addr: u32, stride: u32, w: u32, h: u32, px: [u8; 4]) {
    if addr == 0 || w == 0 || h == 0 {
        return;
    }
    let ok = with_platform(|p| match p.allocs.range_mut(..=addr as u64).next_back() {
        Some((base, a)) => {
            let off = (addr as u64 - base) as usize;
            let last = off + (h as usize - 1) * stride as usize + w as usize * 4;
            if last > a.size {
                return false;
            }
            // SAFETY: the allocation is `size` bytes of u64s and `last` is inside it.
            let bytes = unsafe { core::slice::from_raw_parts_mut(a.mem.as_mut_ptr() as *mut u8, a.size) };
            for y in 0..h as usize {
                for x in 0..w as usize {
                    let o = off + y * stride as usize + x * 4;
                    bytes[o..o + 4].copy_from_slice(&px);
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

#[no_mangle]
unsafe extern "C" fn v3d_job_end(
    j: *mut V3dJob,
    c: *const V3dJobClear,
    _tile_list_start: *mut u32,
    _tile_list_end: *mut u32,
) -> bool {
    // SAFETY: the backend passes its live job and clear; its lists are live.
    let (t, c, drew) = unsafe {
        (*(*j).bcl).used += 1;
        (*(*j).rcl).used = 200;
        ((*j).t, *c, (*j).draws > 0)
    };
    /* RGBA8, RT 0 only (this file's targets) */
    if !c.load_rt0 {
        fill(t.address, t.stride, t.width, t.height, c.rgba8.to_le_bytes());
    }
    if drew {
        fill(t.address, t.stride, t.width, t.height, DRAW);
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
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("es3_large_target_v3d");
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
    seen(|x| x.log.push(s));
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

/// The one context and back end of the module; the tests take turns and
/// each starts with nothing recorded and nothing pending.
fn rig() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    ONCE.call_once(|| {
        // SAFETY: statics; the lock serialises every GL call of the module.
        unsafe {
            osal_set_ops(osal());
            yakogl::backend::g_backend = spy_backend();
            assert_eq!(yakogl::backend::init(), 0, "{:?}", log_lines());
            assert!(!context::gl_context_create(32, 32).is_null(), "{:?}", log_lines());
        }
    });
    *RESIZE.lock().unwrap_or_else(|e| e.into_inner()) = None;
    // SAFETY: plain GL calls.
    unsafe {
        glFinish();
        while glGetError() != GL_NO_ERROR {}
    }
    seen(|s| *s = Seen::default());
    g
}

/* ---- helpers ---- */

const VS: &str = "#version 300 es\nin vec4 p;\nvoid main() { gl_Position = p; }\n";
const FS: &str = "#version 300 es\nprecision mediump float;\nlayout(location = 0) out vec4 c;\n\
                  void main() { c = vec4(0.25, 0.5, 0.75, 1.0); }\n";

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

/// Make `FS` (with `VS`) the current program.
fn program() {
    let (v, f) = (shader(GL_VERTEX_SHADER, VS), shader(GL_FRAGMENT_SHADER, FS));
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
    }
}

fn draw_triangle() {
    static TRI: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 3.0, -1.0, 0.0, 1.0, -1.0, 3.0, 0.0, 1.0];
    // SAFETY: TRI is static and outlives the draw.
    unsafe {
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, TRI.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glDrawArrays(GL_TRIANGLES, 0, 3);
    }
}

/// A framebuffer of one `w` x `h` RGBA8 renderbuffer, current, with the
/// viewport on it: (framebuffer, renderbuffer, the storage's error).
fn fbo(w: GLsizei, h: GLsizei) -> (GLuint, GLuint, GLenum) {
    let (mut fb, mut rb) = (0, 0);
    // SAFETY: plain GL calls with room for one name each.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorage(GL_RENDERBUFFER, GL_RGBA8, w, h);
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

/// The bound framebuffer's colour attachment 0, `w` x `h` RGBA8 pixels.
fn read(w: u32, h: u32) -> Vec<u8> {
    let mut px = vec![0xEEu8; (w * h * 4) as usize];
    // SAFETY: plain GL calls; `px` holds `w` x `h` RGBA8 pixels.
    unsafe {
        glReadPixels(0, 0, w as GLsizei, h as GLsizei, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr() as *mut c_void);
    }
    assert_eq!(err(), GL_NO_ERROR, "glReadPixels {w}x{h}");
    px
}

/// Every pixel of `px` is `want`.
fn assert_all(what: &str, px: &[u8], w: u32, want: [u8; 4]) {
    for (n, p) in px.chunks(4).enumerate() {
        assert_eq!(p, want, "{what}: pixel ({}, {})", n as u32 % w, n as u32 / w);
    }
}

/// The context works: a clear and a draw into a small RGBA8 framebuffer
/// raise nothing, are not refused, and read back.
fn still_usable() {
    let (fb, rb, e) = fbo(24, 16);
    assert_eq!(e, GL_NO_ERROR);
    let base = v3d_refused_draws();
    program();
    // SAFETY: plain GL calls.
    unsafe {
        glClearColor(0.0, 0.0, 1.0, 1.0);
        glClear(GL_COLOR_BUFFER_BIT);
        draw_triangle();
        glFinish();
    }
    assert_eq!(err(), GL_NO_ERROR, "{:?}", log_lines());
    assert_eq!(v3d_refused_draws(), base, "{:?}", log_lines());
    assert_all("24x16 after", &read(24, 16), 24, DRAW);
    delete(fb, rb);
}

/* ---- 4096 a side: taken ---- */

/// The tile memory's size is the GL core's: the back end's `MAX_TARGET`
/// is GL_MAX_RENDERBUFFER_SIZE, GL_MAX_TEXTURE_SIZE and each of
/// GL_MAX_VIEWPORT_DIMS.
#[test]
fn the_tile_memory_is_sized_for_what_the_gl_core_reports() {
    let _g = rig();
    let (mut rb, mut tex, mut vp) = (0, 0, [0; 2]);
    // SAFETY: plain queries with live out pointers.
    unsafe {
        glGetIntegerv(GL_MAX_RENDERBUFFER_SIZE, &mut rb);
        glGetIntegerv(GL_MAX_TEXTURE_SIZE, &mut tex);
        glGetIntegerv(GL_MAX_VIEWPORT_DIMS, vp.as_mut_ptr());
    }
    assert_eq!((rb, tex, vp), (4096, 4096, [4096, 4096]));
    assert_eq!(MAX_TARGET, 4096);
}

/// The executor's shapes (4096 x 2..6) and the transposed one: complete,
/// one job the target's size, the clear read back, then the draw read back,
/// with nothing refused and no store past the renderbuffer.
#[test]
fn a_4096_wide_target_clears_draws_and_reads_back() {
    let _g = rig();
    program();
    for (w, h) in [(4096u32, 4u32), (4, 4096), (4096, 6)] {
        seen(|s| *s = Seen::default());
        let base = v3d_refused_draws();
        let (fb, rb, e) = fbo(w as GLsizei, h as GLsizei);
        assert_eq!(e, GL_NO_ERROR, "{w}x{h}: storage");
        // SAFETY: plain GL call.
        let status = unsafe { glCheckFramebufferStatus(GL_FRAMEBUFFER) };
        assert_eq!(status, GL_FRAMEBUFFER_COMPLETE, "{w}x{h}");
        // SAFETY: plain GL calls.
        unsafe {
            glClearColor(1.0, 0.0, 0.0, 1.0);
            glClear(GL_COLOR_BUFFER_BIT);
            glFinish();
        }
        assert_eq!(err(), GL_NO_ERROR, "{w}x{h} clear: {:?}", log_lines());
        assert_all(&format!("{w}x{h} clear"), &read(w, h), w, [255, 0, 0, 255]);
        draw_triangle();
        // SAFETY: plain GL call.
        unsafe { glFinish() };
        assert_eq!(err(), GL_NO_ERROR, "{w}x{h} draw: {:?}", log_lines());
        assert_all(&format!("{w}x{h} draw"), &read(w, h), w, DRAW);
        assert_eq!(v3d_refused_draws(), base, "{w}x{h}: {:?}", log_lines());
        let (jobs, overruns, log) = seen(|s| (s.jobs.clone(), s.overruns, s.log.clone()));
        assert_eq!(overruns, 0, "{w}x{h}: a store left its allocation");
        assert!(!jobs.is_empty(), "{w}x{h}: a job");
        assert!(jobs.iter().all(|t| (t.width, t.height) == (w, h)), "{w}x{h}: every job the whole target");
        assert!(!log.iter().any(|l| l.contains("draw refused")), "{w}x{h}: {log:?}");
        delete(fb, rb);
    }
    still_usable();
}

/* ---- past 4096, or empty: refused by name ---- */

/// The GL API stops a side past 4096 at the storage (GL_INVALID_VALUE),
/// so no such target reaches the back end from the API.
#[test]
fn a_side_past_4096_is_invalid_value_at_the_storage() {
    let _g = rig();
    for (w, h) in [(4097, 4), (4, 4097)] {
        let (fb, rb, e) = fbo(w, h);
        assert_eq!(e, GL_INVALID_VALUE, "{w}x{h}");
        delete(fb, rb);
    }
    still_usable();
}

/// The GL core's own draw of a 4096 x 4 framebuffer, handed to the back
/// end as `w` x `h`: the GL error and whether `why` was logged.
fn resized_draw(w: u32, h: u32, why: &str) -> (GLenum, bool) {
    let (fb, rb, e) = fbo(4096, 4);
    assert_eq!(e, GL_NO_ERROR);
    program();
    let base = v3d_refused_draws();
    *RESIZE.lock().unwrap_or_else(|e| e.into_inner()) = Some((w, h));
    draw_triangle();
    *RESIZE.lock().unwrap_or_else(|e| e.into_inner()) = None;
    // SAFETY: plain GL call.
    unsafe { glFinish() };
    let e = err();
    assert_eq!(v3d_refused_draws().wrapping_sub(base), 1, "{w}x{h}: counted as refused");
    let named = log_lines().iter().any(|l| l.contains("draw refused") && l.contains(why));
    delete(fb, rb);
    (e, named)
}

/// Past 4096 a side, where the tile memory stops: legal-looking state the
/// back end cannot render, -ENOTSUP, GL_INVALID_OPERATION, named for that
/// cause; the context works after.
#[test]
fn a_target_past_4096_is_refused_by_its_own_name() {
    let _g = rig();
    let why = "a target larger than the tile memory is sized for (4096x4096)";
    for (w, h) in [(4097, 4), (4096, 4097)] {
        assert_eq!(resized_draw(w, h, why), (GL_INVALID_OPERATION, true), "{w}x{h}: {:?}", log_lines());
    }
    still_usable();
}

/// An empty target is a malformed draw: -EINVAL, which the GL core raises
/// as GL_OUT_OF_MEMORY, named for that cause and not for the tile memory;
/// the context works after.
#[test]
fn an_empty_target_is_refused_by_its_own_name() {
    let _g = rig();
    let why = "an empty target (0 pixels on a side)";
    for (w, h) in [(0, 4), (4096, 0)] {
        assert_eq!(resized_draw(w, h, why), (GL_OUT_OF_MEMORY, true), "{w}x{h}: {:?}", log_lines());
    }
    assert!(!log_lines().iter().any(|l| l.contains("tile memory")), "{:?}", log_lines());
    still_usable();
}
