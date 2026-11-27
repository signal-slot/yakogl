// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! 4x multisample framebuffer objects on the V3D (t-msaa-v3d), through the GL
//! API, the real GL core and the real V3D back end (`yakogl_backend_v3d`,
//! its `draw_call`), on the host. The job builder is stood in for (it records
//! what the back end hands it and draws nothing: the harness of
//! tests/es3_four_rt_v3d.rs) and the tile rule is the real
//! driver/src/v3d/v3d_frame.c. docs/notes/v3d-msaa-es3.md (3)-(4),
//! docs/notes/es3-msaa-renderbuffers.md (c2).
//!
//! The back end's capability is the build's: `max_samples` is 4 with the
//! `msaa-fbo` feature and 0 without it (src/v3d/backend.rs). So the file has
//! two halves, each compiled in its build:
//!
//! - without `msaa-fbo` (the `*_today` tests): GL_MAX_SAMPLES is 1,
//!   `glRenderbufferStorageMultisample` gives GL_INVALID_OPERATION for 2 or
//!   more samples, one sample is plain storage, and an FBO draw is one
//!   sample;
//! - with `msaa-fbo` (the `*_4x_*` tests): GL_MAX_SAMPLES is 4, 4x storage of
//!   every multisample-renderable ES 3.0 format, and a draw into a 4x FBO of
//!   one RT, of an ES 3.0 format (sRGB8_A8), of two RTs and of {RGBA8,
//!   sRGB8_A8} is one 4x job of the real `draw_call`: `msaa`, each RT with
//!   its own sample buffer (laid out at the RT's own cpp,
//!   `msaa_hw::v3d::ms_layout`), the tile the real tile rule gives Mesa's
//!   `v3d_choose_tile_size` for it, then a resolve blit. The draws carry the
//!   GL core's draw identity (`GlesDraw::target_ids` / `target_gens`).
//!
//! What a host test cannot see: the control list (the stand-in job builder
//! emits no words; tools/v3d/job_check.c part_msaa pins the C builder's
//! loads and stores) and pixels (nothing is drawn, so the resolve blit copies
//! what the renderbuffer holds, zeros). BOARD-UNVERIFIED: no Pi 4 run.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_msaa_v3d
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table,msaa-fbo --test es3_msaa_v3d
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-v3d", not(feature = "report-es2")))]

use core::ffi::{c_char, c_int, c_void, CStr};
use std::collections::BTreeMap;
use std::ffi::CString;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Mutex, MutexGuard, Once, OnceLock};
use yakogl::backend::{GlesDraw, GLES_MAX_TARGETS, GLES_PRIM_TRIANGLES, GLES_SURF_RGBA8888, GLES_SURF_SRGB8_A8};
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
use yakogl::v3d::backend::yakogl_backend_v3d;
use yakogl::v3d::ffi::*;
use yakogl::v3d::kick::V3dKick;
use yakogl::v3d::state;

/// One draw as the V3D back end was handed it, and what it answered.
#[derive(Clone, Debug)]
struct GlSeen {
    /// An application draw (vertices), not a clear alone.
    draws: bool,
    /// `targets[..num_targets]`: (gpu, format).
    targets: Vec<(u64, u8)>,
    samples: u8,
    /// `target_id`, `target_gen` and the per-target arrays.
    id: (u32, u32),
    ids: [u32; GLES_MAX_TARGETS],
    gens: [u32; GLES_MAX_TARGETS],
    /// `draw_call`'s answer (0, or the refusal's -errno).
    rc: i32,
}

/* ---- the job builder, the tile rule and the platform (tests/es3_four_rt_v3d.rs) ---- */

#[derive(Default)]
struct Seen {
    jobs: Vec<V3dJobTarget>,
    clears: Vec<V3dJobClear>,
    /// The vertex count of each draw the job builder was handed.
    draws: Vec<u32>,
    tilings: Vec<Tiling>,
    /// What the V3D back end was handed by each draw, and its answer.
    gl: Vec<GlSeen>,
    log: Vec<String>,
}

/// One call of the tile rule: what was asked, the tile it answered.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Tiling {
    rts: u32,
    msaa: bool,
    max_bpp: u32,
    tile: (u32, u32),
}

static SEEN: Mutex<Seen> = Mutex::new(Seen {
    jobs: Vec::new(),
    clears: Vec::new(),
    draws: Vec::new(),
    tilings: Vec::new(),
    gl: Vec::new(),
    log: Vec::new(),
});

fn seen<R>(f: impl FnOnce(&mut Seen) -> R) -> R {
    f(&mut SEEN.lock().unwrap_or_else(|e| e.into_inner()))
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
    seen(|s| s.jobs.push(t));
    t.width != 0 && t.height != 0
}

#[no_mangle]
unsafe extern "C" fn v3d_job_draw(j: *mut V3dJob, d: *const V3dJobDraw, _layout: *mut c_void) -> i32 {
    // SAFETY: the backend passes its live job and a draw whose pointers live for the call.
    let (j, d) = unsafe { (&mut *j, &*d) };
    j.draws += 1;
    // SAFETY: the job's BCL is live; the draw's slices are as long as it says.
    unsafe {
        (*j.bcl).used += 120;
        seen(|s| s.draws.push(d.count));
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
    // SAFETY: the backend passes its live job and clear; its lists are live.
    unsafe {
        seen(|s| s.clears.push(*c));
        (*(*j).bcl).used += 1;
        (*(*j).rcl).used = 200;
    }
    true
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
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("v3d_msaa");
        std::fs::create_dir_all(&dir).expect("the scratch directory");
        let so = dir.join(format!("v3d_msaa-{}.so", std::process::id()));
        /* the compiler's own temporary files next to the object */
        let st = Command::new("cc")
            .current_dir(&root)
            .env("TMPDIR", &dir)
            .args(["-std=c11", "-O1", "-shared", "-fPIC", "-Wall", "-Wextra", "-Werror"])
            .args(["-Idriver/src/v3d", "-Idriver/src/v3d/generated"])
            .args(["-Ithird_party/mesa-26.2.1/src/broadcom", "-Ithird_party/mesa-26.2.1/src"])
            .arg("-Ithird_party/mesa-26.2.1/include")
            .args([
                "driver/src/v3d/v3d_tex.c",
                "driver/src/v3d/v3d_tiling_mesa.c",
                "driver/src/v3d/v3d_frame.c",
                "-lm",
                "-o",
            ])
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
    trampoline!(v3d_frame_alloc_sizes(t: *const V3dTiling, draws: u32, page: u32, ta: *mut u32, ts: *mut u32));
}

/// The real tile rule (driver/src/v3d/v3d_frame.c), each call recorded:
/// the render-target count, MSAA and largest internal bpp the back end
/// asked for, and the tile it got.
fn tiling(rts: u32, msaa: bool, max_bpp: u32, t: *const V3dTiling) {
    // SAFETY: the library filled `t` just before.
    let t = unsafe { *t };
    seen(|s| s.tilings.push(Tiling { rts, msaa, max_bpp, tile: (t.tile_w, t.tile_h) }));
}

#[no_mangle]
unsafe extern "C" fn v3d_frame_tiling(width: u32, height: u32, t: *mut V3dTiling) {
    // SAFETY: the C function's signature, filling the backend's out-parameter.
    let f: unsafe extern "C" fn(u32, u32, *mut V3dTiling) = unsafe { std::mem::transmute(sym("v3d_frame_tiling")) };
    // SAFETY: as above.
    unsafe { f(width, height, t) };
    tiling(1, false, 0, t);
}

#[no_mangle]
unsafe extern "C" fn v3d_frame_tiling_rts(width: u32, height: u32, rts: u32, t: *mut V3dTiling) {
    // SAFETY: the C function's signature, filling the backend's out-parameter.
    let f: unsafe extern "C" fn(u32, u32, u32, *mut V3dTiling) =
        unsafe { std::mem::transmute(sym("v3d_frame_tiling_rts")) };
    // SAFETY: as above.
    unsafe { f(width, height, rts, t) };
    tiling(rts, false, 0, t);
}

#[no_mangle]
unsafe extern "C" fn v3d_frame_tiling_ms(width: u32, height: u32, rts: u32, msaa: bool, t: *mut V3dTiling) {
    // SAFETY: the C function's signature, filling the backend's out-parameter.
    let f: unsafe extern "C" fn(u32, u32, u32, bool, *mut V3dTiling) =
        unsafe { std::mem::transmute(sym("v3d_frame_tiling_ms")) };
    // SAFETY: as above.
    unsafe { f(width, height, rts, msaa, t) };
    tiling(rts, msaa, 0, t);
}

#[no_mangle]
unsafe extern "C" fn v3d_frame_tiling_bpp(
    width: u32,
    height: u32,
    rts: u32,
    msaa: bool,
    max_bpp: u32,
    t: *mut V3dTiling,
) {
    // SAFETY: the C function's signature, filling the backend's out-parameter.
    let f: unsafe extern "C" fn(u32, u32, u32, bool, u32, *mut V3dTiling) =
        unsafe { std::mem::transmute(sym("v3d_frame_tiling_bpp")) };
    // SAFETY: as above.
    unsafe { f(width, height, rts, msaa, max_bpp, t) };
    tiling(rts, msaa, max_bpp, t);
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

/* ---- the context ---- */

/// The V3D back end with its draw entry watched: what the GL core hands it,
/// then the real `draw_call`'s answer.
unsafe extern "C" fn spy_draw(d: *const GlesDraw) -> i32 {
    // SAFETY: the GL core hands a live draw.
    let dd = unsafe { &*d };
    // SAFETY: the real back end's entry, with the same draw.
    let rc = unsafe { (yakogl_backend_v3d.draw.expect("the V3D draws"))(d) };
    let n = (dd.num_targets.max(1) as usize).min(GLES_MAX_TARGETS);
    seen(|s| {
        s.gl.push(GlSeen {
            draws: dd.index_count != 0,
            targets: dd.targets[..n].iter().map(|t| (t.gpu, t.format)).collect(),
            samples: dd.samples,
            id: (dd.target_id, dd.target_gen),
            ids: dd.target_ids,
            gens: dd.target_gens,
            rc,
        })
    });
    rc
}

fn spy_backend() -> &'static yakogl::backend::YakoglBackend {
    let b = yakogl::backend::YakoglBackend { draw: Some(spy_draw), ..yakogl_backend_v3d };
    Box::leak(Box::new(b))
}

static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

/// The one context and back end of the file; the tests take turns and each
/// starts with nothing recorded and nothing pending.
fn context() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    ONCE.call_once(|| {
        // SAFETY: statics; the lock serialises every GL call of the file.
        unsafe {
            osal_set_ops(osal());
            yakogl::backend::g_backend = spy_backend();
            assert_eq!(yakogl::backend::init(), 0, "{:?}", seen(|s| s.log.clone()));
            assert!(!context::gl_context_create(8, 8).is_null(), "{:?}", seen(|s| s.log.clone()));
        }
    });
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glFinish();
        while glGetError() != GL_NO_ERROR {}
    }
    seen(|s| *s = Seen::default());
    g
}

/// GL_NO_ERROR, or the first error of the queue (the queue is drained).
fn err() -> GLenum {
    // SAFETY: plain GL calls.
    unsafe {
        let e = glGetError();
        while glGetError() != GL_NO_ERROR {}
        e
    }
}

/* ---- framebuffers, the program, the draw ------------------------------- */

struct Fbo {
    fb: GLuint,
    rbs: Vec<GLuint>,
}

impl Drop for Fbo {
    fn drop(&mut self) {
        // SAFETY: names this test made.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteFramebuffers(1, &self.fb);
            glDeleteRenderbuffers(self.rbs.len() as GLsizei, self.rbs.as_ptr());
            glUseProgram(0);
        }
    }
}

/// The error of `glRenderbufferStorageMultisample(.., samples, format, 8, 8)`
/// on a fresh renderbuffer, and the renderbuffer.
fn storage(samples: GLsizei, format: GLenum) -> (GLenum, GLuint) {
    let mut rb = 0;
    // SAFETY: plain GL calls.
    unsafe {
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        while glGetError() != GL_NO_ERROR {}
        glRenderbufferStorageMultisample(GL_RENDERBUFFER, samples, format, 8, 8);
    }
    (err(), rb)
}

/// A bound 8x8 framebuffer with a renderbuffer of `samples` on each of
/// `formats`' colour attachments, all draw buffers. A storage error comes back
/// as `Err(error)`.
fn framebuffer(samples: GLsizei, formats: &[GLenum]) -> Result<Fbo, GLenum> {
    let mut fb = 0;
    // SAFETY: room for one name; plain GL call.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
    }
    let mut fbo = Fbo { fb, rbs: Vec::new() };
    for (i, f) in formats.iter().enumerate() {
        let (e, rb) = storage(samples, *f);
        fbo.rbs.push(rb);
        if e != GL_NO_ERROR {
            return Err(e);
        }
        // SAFETY: a renderbuffer this test made.
        unsafe { glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0 + i as GLenum, GL_RENDERBUFFER, rb) };
    }
    let bufs: Vec<GLenum> = (0..formats.len()).map(|i| GL_COLOR_ATTACHMENT0 + i as GLenum).collect();
    // SAFETY: plain GL calls.
    unsafe {
        glDrawBuffers(bufs.len() as GLsizei, bufs.as_ptr());
        glViewport(0, 0, 8, 8);
        assert_eq!(err(), GL_NO_ERROR);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "{formats:#x?}");
    }
    Ok(fbo)
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

/// The current program: `n` float outputs.
fn program(n: usize) {
    let mut fs = String::from("#version 300 es\nprecision highp float;\n");
    for i in 0..n {
        fs += &format!("layout(location = {i}) out vec4 o{i};\n");
    }
    fs += "void main() {\n";
    for i in 0..n {
        fs += &format!("o{i} = vec4(0.25, 0.5, 0.75, 1.0);\n");
    }
    fs += "}\n";
    let v = shader(GL_VERTEX_SHADER, "#version 300 es\nin vec4 p;\nvoid main() { gl_Position = p; }\n");
    let f = shader(GL_FRAGMENT_SHADER, &fs);
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1);
        glUseProgram(p);
    }
}

/// Draw one triangle, flushed; what the back end was handed for it (and
/// answered), and what the job builder recorded.
fn draw() -> (GlSeen, Seen) {
    static TRI: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 3.0, -1.0, 0.0, 1.0, -1.0, 3.0, 0.0, 1.0];
    seen(|s| *s = Seen::default());
    // SAFETY: TRI is static and outlives the draw.
    unsafe {
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, TRI.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glDrawArrays(GL_TRIANGLES, 0, 3);
        glFinish();
    }
    let s = seen(core::mem::take);
    assert_eq!(err(), GL_NO_ERROR, "{:?}", s.log);
    let d = s.gl.iter().rev().find(|d| d.draws).cloned().expect("the draw reached the back end");
    (d, s)
}

/// Resolve blit of `read`'s colour 0 into a fresh single-sample RGBA8
/// framebuffer of the same size; the GL error.
fn resolve_blit(read: &Fbo) -> GLenum {
    let dst = framebuffer(0, &[GL_RGBA8]).expect("a single-sample target");
    // SAFETY: plain GL calls on names this test made.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, read.fb);
        glBindFramebuffer(GL_DRAW_FRAMEBUFFER, dst.fb);
        glReadBuffer(GL_COLOR_ATTACHMENT0);
        while glGetError() != GL_NO_ERROR {}
        glBlitFramebuffer(0, 0, 8, 8, 0, 0, 8, 8, GL_COLOR_BUFFER_BIT, GL_NEAREST);
    }
    let e = err();
    drop(dst);
    e
}

fn geti(pname: GLenum) -> GLint {
    let mut v = -1;
    // SAFETY: one value.
    unsafe { glGetIntegerv(pname, &mut v) };
    v
}

fn rb_samples(rb: GLuint) -> GLint {
    let mut v = -1;
    // SAFETY: one value; a renderbuffer this test made.
    unsafe {
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glGetRenderbufferParameteriv(GL_RENDERBUFFER, GL_RENDERBUFFER_SAMPLES, &mut v);
    }
    v
}

fn internalformat(format: GLenum, pname: GLenum) -> GLint {
    let mut v = -1;
    // SAFETY: room for one value.
    unsafe { glGetInternalformativ(GL_RENDERBUFFER, format, pname, 1, &mut v) };
    assert_eq!(err(), GL_NO_ERROR);
    v
}

const RGBA8: [GLenum; 1] = [GL_RGBA8];
const ES3_FORMAT: [GLenum; 1] = [GL_SRGB8_ALPHA8];
const TWO_RTS: [GLenum; 2] = [GL_RGBA8, GL_RGBA8];

/* ---- the expected job words (note (3), after Mesa) ----------------------- */

/// What the 4x job of `formats` should carry: one `multisample_mode_4x` bit
/// for the job, the tile of Mesa's `v3d_choose_tile_size` for V3D 4.x (index =
/// RT class 0/1/2 for 1/2/3-4 RTs + 2 for 4x + the largest internal bpp 0/1/2
/// for 32/64/128), and for each RT a kept sample buffer (ALL_SAMPLES, the
/// RT's own buffer, UIF at twice the size, the format's own cpp) besides its
/// resolve (decimate 4X, the raster target).
#[derive(Debug, PartialEq)]
struct JobWords {
    multisample_mode_4x: bool,
    tile: (u32, u32),
    kept_stores: usize,
    resolve_stores: usize,
    sample_buffer_cpp: Vec<u32>,
}

fn expected_job(formats: &[GLenum]) -> JobWords {
    const TILES: [(u32, u32); 7] = [(64, 64), (64, 32), (32, 32), (32, 16), (16, 16), (16, 8), (8, 8)];
    /* RGBA8 is internal type 8 at 32 bpp; sRGB8_A8, RGB10_A2 and the 16F rows
     * are 16F at 64 bpp (note (2)) */
    let bpp = |f: GLenum| usize::from(matches!(f, GL_SRGB8_ALPHA8 | GL_RGB10_A2 | GL_RGBA16F));
    let cpp = |f: GLenum| if f == GL_RGBA16F { 8 } else { 4 };
    let rt_class = [0usize, 0, 1, 2, 2][formats.len().min(4)];
    let max_bpp = formats.iter().map(|f| bpp(*f)).max().unwrap_or(0);
    JobWords {
        multisample_mode_4x: true,
        tile: TILES[rt_class + 2 + max_bpp],
        kept_stores: formats.len(),
        resolve_stores: formats.len(),
        sample_buffer_cpp: formats.iter().map(|f| cpp(*f)).collect(),
    }
}

/// The table above against the note's own numbers: one RGBA8 at 4x is 32x32,
/// a 64-bpp RT alone 32x16, {RGBA8, sRGB8_A8} 16x16 (index 1+2+1), four RTs
/// with a 64-bpp one 16x8.
#[test]
fn the_expected_tile_sizes_are_the_notes() {
    assert_eq!(expected_job(&RGBA8).tile, (32, 32));
    assert_eq!(expected_job(&ES3_FORMAT).tile, (32, 16));
    assert_eq!(expected_job(&[GL_RGBA8, GL_SRGB8_ALPHA8]).tile, (16, 16));
    assert_eq!(expected_job(&TWO_RTS).tile, (32, 16));
    assert_eq!(expected_job(&[GL_RGBA8, GL_RGBA8, GL_RGBA8, GL_SRGB8_ALPHA8]).tile, (16, 8));
}

/* ---- the capability off: the build without `msaa-fbo` ------------------- */

/// `GL_MAX_SAMPLES` is 1: the V3D's `max_samples` is 0 without the feature.
#[cfg(not(feature = "msaa-fbo"))]
#[test]
fn max_samples_is_one_today() {
    let _g = context();
    assert_eq!(yakogl_backend_v3d.max_samples, 0);
    assert_eq!(geti(GL_MAX_SAMPLES), 1);
}

/// `glGetInternalformativ`: one sample count, 1, for a renderable format; no
/// counts for an integer one (ES 3.0 4.4.2.1).
#[cfg(not(feature = "msaa-fbo"))]
#[test]
fn internalformat_counts_today() {
    let _g = context();
    for f in [GL_RGBA8, GL_SRGB8_ALPHA8, GL_RGB10_A2, GL_RGB565, GL_R8, GL_RG8, GL_RGBA4, GL_RGB5_A1, GL_RGB8] {
        assert_eq!(internalformat(f, GL_NUM_SAMPLE_COUNTS), 1, "{f:#x}");
        assert_eq!(internalformat(f, GL_SAMPLES), 1, "{f:#x}");
    }
    for f in [GL_R8I, GL_RG16UI, GL_RGBA32I, GL_RGB10_A2UI] {
        assert_eq!(internalformat(f, GL_NUM_SAMPLE_COUNTS), 0, "{f:#x}");
    }
}

/// GL_SAMPLES and GL_SAMPLE_BUFFERS of the window and of framebuffers of
/// zero-sample and one-sample storage, and the renderbuffer's own count.
#[cfg(not(feature = "msaa-fbo"))]
#[test]
fn samples_queries_today() {
    let _g = context();
    assert_eq!((geti(GL_SAMPLE_BUFFERS), geti(GL_SAMPLES)), (0, 0), "the window of the 8x8 test context");
    let fbo = framebuffer(0, &RGBA8).unwrap();
    assert_eq!((geti(GL_SAMPLE_BUFFERS), geti(GL_SAMPLES)), (0, 0));
    assert_eq!(rb_samples(fbo.rbs[0]), 0);
    drop(fbo);
    /* one sample is plain storage "with sample buffers" */
    let fbo = framebuffer(1, &RGBA8).unwrap();
    assert_eq!((geti(GL_SAMPLE_BUFFERS), geti(GL_SAMPLES)), (1, 1));
    assert_eq!(rb_samples(fbo.rbs[0]), 1);
}

/// 4 samples on RGBA8: `GL_INVALID_OPERATION`, no storage (the count is past
/// GL_MAX_SAMPLES, 1).
#[cfg(not(feature = "msaa-fbo"))]
#[test]
fn four_samples_of_rgba8_is_invalid_operation_today() {
    let _g = context();
    assert_eq!(storage(4, GL_RGBA8).0, GL_INVALID_OPERATION);
    assert_eq!(storage(2, GL_RGBA8).0, GL_INVALID_OPERATION);
}

/// 4 samples on an ES 3.0 colour format: the same error at storage time.
/// Integer formats are refused by the spec itself at any count above 0.
#[cfg(not(feature = "msaa-fbo"))]
#[test]
fn four_samples_of_an_es3_format_is_invalid_operation_today() {
    let _g = context();
    for f in [GL_SRGB8_ALPHA8, GL_RGB10_A2, GL_RGBA16F, GL_R8, GL_RG8, GL_RGB565, GL_RGB8] {
        assert_eq!(storage(4, f).0, GL_INVALID_OPERATION, "{f:#x}");
    }
    for f in [GL_R8I, GL_RGBA8UI, GL_RG16I, GL_RGBA32UI] {
        assert_eq!(storage(1, f).0, GL_INVALID_OPERATION, "{f:#x}: no multisample integer format");
        assert_eq!(storage(4, f).0, GL_INVALID_OPERATION, "{f:#x}");
    }
}

/// Two colour attachments at 4 samples: the first attachment already fails.
#[cfg(not(feature = "msaa-fbo"))]
#[test]
fn four_samples_of_two_attachments_is_invalid_operation_today() {
    let _g = context();
    assert_eq!(framebuffer(4, &TWO_RTS).err(), Some(GL_INVALID_OPERATION));
    assert_eq!(framebuffer(4, &[GL_RGBA8, GL_SRGB8_ALPHA8]).err(), Some(GL_INVALID_OPERATION));
}

/// One sample: a draw into one and into two such attachments is a
/// single-sampled job of the real `draw_call` (the back end gets `samples`
/// 1, and the renderbuffers' identity), and a resolve blit into a
/// single-sample framebuffer works.
#[cfg(not(feature = "msaa-fbo"))]
#[test]
fn one_sample_draws_and_blits_today() {
    let _g = context();
    for formats in [&RGBA8[..], &ES3_FORMAT[..], &TWO_RTS[..]] {
        let fbo = framebuffer(1, formats).unwrap();
        program(formats.len());
        let (d, s) = draw();
        assert_eq!((d.samples, d.targets.len(), d.rc), (1, formats.len(), 0), "{formats:#x?}: {:?}", s.log);
        assert!(d.ids[..formats.len()].iter().all(|&id| id != 0), "the renderbuffers' identity: {d:?}");
        assert_eq!(d.id, (d.ids[0], d.gens[0]), "target_id is targets[0]'s");
        let t = s.jobs.last().expect("a job");
        assert!(!t.msaa && t.ms_address == 0, "{t:?}");
        assert_eq!(resolve_blit(&fbo), GL_NO_ERROR, "{formats:#x?}");
    }
}

/* ---- either build ------------------------------------------------------- */

/// A negative count is `GL_INVALID_VALUE`, checked before the limit.
#[test]
fn a_negative_count_is_invalid_value() {
    let _g = context();
    assert_eq!(storage(-1, GL_RGBA8).0, GL_INVALID_VALUE);
}

/// A blit *into* a multisample framebuffer is refused by the spec (a
/// request of 1 sample: 1, or 4 with the capability on).
#[test]
fn a_blit_into_a_multisample_framebuffer_is_invalid_operation() {
    let _g = context();
    let ms = framebuffer(1, &RGBA8).unwrap();
    let single = framebuffer(0, &[GL_RGBA8]).unwrap();
    // SAFETY: plain GL calls on names this test made.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, single.fb);
        glBindFramebuffer(GL_DRAW_FRAMEBUFFER, ms.fb);
        while glGetError() != GL_NO_ERROR {}
        glBlitFramebuffer(0, 0, 8, 8, 0, 0, 8, 8, GL_COLOR_BUFFER_BIT, GL_NEAREST);
    }
    assert_eq!(err(), GL_INVALID_OPERATION);
}

/// An integer format stays refused at any count, 4x or not: the spec's rule,
/// not a limit.
#[test]
fn integer_formats_stay_refused() {
    let _g = context();
    assert_eq!(storage(4, GL_RGBA32UI).0, GL_INVALID_OPERATION);
    assert_eq!(storage(1, GL_RGBA8I).0, GL_INVALID_OPERATION);
}

/// A hand-made 4x draw of the window shape (one RGBA8 target) passes the
/// V3D's state layer, and so does an sRGB one; only 1 and 4 samples are
/// modes of the V3D.
#[test]
fn the_state_layer_takes_a_4x_draw() {
    let mut d = GlesDraw::ZERO;
    d.target_gpu = 0x0100_0000;
    (d.target_w, d.target_h, d.target_format, d.target_alpha_bits) = (8, 8, GLES_SURF_RGBA8888, 8);
    (d.samples, d.prim, d.index_count, d.line_width) = (4, GLES_PRIM_TRIANGLES, 3, 1.0);
    (d.viewport, d.depth_range, d.color_mask, d.color_write) = ([0, 0, 8, 8], [0.0, 1.0], 0xf, true);
    assert_eq!(state::refusal(&d), None);
    assert!(state::job_state(&d).is_ok());
    d.target_format = GLES_SURF_SRGB8_A8;
    assert_eq!(state::refusal(&d), None, "the format is a drawable row");
    d.samples = 2;
    assert!(state::refusal(&d).is_some(), "only 1 and 4");
}

/* ---- the capability on: the build with `msaa-fbo` ---------------------- */

/// `GL_MAX_SAMPLES` is the V3D's `max_samples`, 4 (note (4)).
#[cfg(feature = "msaa-fbo")]
#[test]
fn max_samples_is_four() {
    let _g = context();
    assert_eq!(yakogl_backend_v3d.max_samples, 4);
    assert_eq!(geti(GL_MAX_SAMPLES), 4);
}

/// The queries: sample count 4 for the multisample-renderable formats, none
/// for the integer ones.
#[cfg(feature = "msaa-fbo")]
#[test]
fn internalformat_counts_four() {
    let _g = context();
    for f in [GL_RGBA8, GL_SRGB8_ALPHA8, GL_RGB10_A2, GL_RGB565, GL_R8, GL_RG8] {
        assert_eq!(internalformat(f, GL_NUM_SAMPLE_COUNTS), 1, "{f:#x}");
        assert_eq!(internalformat(f, GL_SAMPLES), 4, "{f:#x}");
    }
    for f in [GL_R8I, GL_RGBA32UI] {
        assert_eq!(internalformat(f, GL_NUM_SAMPLE_COUNTS), 0, "{f:#x}");
    }
}

/// The bytes a sample of `format` takes in its RT's sample buffer.
#[cfg(feature = "msaa-fbo")]
fn sample_cpp(format: GLenum) -> u32 {
    expected_job(&[format]).sample_buffer_cpp[0]
}

/// One 4x framebuffer of `formats`: storage, completeness, GL_SAMPLES, the
/// draw as the real V3D `draw_call` takes it (4 samples, every target, each
/// renderbuffer's identity, not refused), the job it makes (4x, the RT
/// count, a sample buffer per RT laid out at the RT's own cpp, the tile the
/// real tile rule gives), a second draw that loads the kept samples, and the
/// resolve blit.
#[cfg(feature = "msaa-fbo")]
fn expect_4x(formats: &[GLenum]) {
    let _g = context();
    let fbo = framebuffer(4, formats).expect("4 samples are legal storage");
    assert_eq!(rb_samples(fbo.rbs[0]), 4);
    assert_eq!(geti(GL_SAMPLES), 4);
    program(formats.len());
    let (d, s) = draw();
    assert_eq!((d.samples, d.targets.len(), d.rc), (4, formats.len(), 0), "{formats:#x?}: {:?}", s.log);
    let n = formats.len();
    assert!(d.ids[..n].iter().all(|&id| id != 0) && d.gens[..n].iter().all(|&g| g != 0), "{d:?}");
    assert_eq!(d.id, (d.ids[0], d.gens[0]), "target_id is targets[0]'s");
    assert!(d.ids[n..].iter().all(|&id| id == 0), "no renderbuffer past num_targets: {d:?}");
    let w = expected_job(formats);
    assert_eq!(s.jobs.len(), 1, "one job: {:?}", s.log);
    let t = s.jobs[0];
    assert_eq!((t.msaa, t.rts.max(1) as usize), (w.multisample_mode_4x, n), "{t:?}");
    let kept: Vec<(u32, u32)> = (0..n)
        .map(|k| {
            if k == 0 {
                (t.ms_address, t.ms_height_in_ub)
            } else {
                (t.rt[k - 1].ms_address, t.rt[k - 1].ms_height_in_ub)
            }
        })
        .collect();
    assert_eq!(
        kept.iter().filter(|(a, _)| *a != 0 && a % 4096 == 0).count(),
        w.kept_stores,
        "a sample buffer per RT: {t:?}"
    );
    let resolves = (0..n).filter(|&k| if k == 0 { t.address != 0 } else { t.rt[k - 1].address != 0 }).count();
    assert_eq!(resolves, w.resolve_stores, "each RT has its raster target, the resolve's: {t:?}");
    for (k, f) in formats.iter().enumerate() {
        let l = yakogl::msaa_hw::v3d::ms_layout(8, 8, sample_cpp(*f)).expect("a layout");
        assert_eq!(kept[k].1, l.height_in_ub, "RT {k} ({f:#x}): laid out at {} bytes a sample", w.sample_buffer_cpp[k]);
    }
    let tiling = *s.tilings.iter().rev().find(|t| t.msaa).expect("the job asked the tile rule at 4x");
    assert_eq!(tiling.tile, w.tile, "{formats:#x?}: {tiling:?}");
    let c = s.clears.last().expect("the job ended");
    assert!(!c.load_rt0 && c.load_rt[..n - 1].iter().all(|l| !l), "nothing kept yet: from the clear colour");
    /* the next job finds the samples the first one stored */
    let (_, s2) = draw();
    assert_eq!(s2.jobs.len(), 1);
    assert_eq!(s2.jobs[0], t, "the same sample buffers");
    let c = s2.clears.last().expect("the job ended");
    assert!(c.load_rt0 && c.load_rt[..n - 1].iter().all(|l| *l), "kept: loaded: {c:?}");
    assert_eq!(resolve_blit(&fbo), GL_NO_ERROR);
}

/// RGBA8 at 4 samples, one RT: 32x32 tile, a kept sample buffer and the
/// resolve into the renderbuffer (the window's shape, as an FBO).
#[cfg(feature = "msaa-fbo")]
#[test]
fn rgba8_4x_draw_and_resolve() {
    assert_eq!(expected_job(&RGBA8).tile, (32, 32));
    expect_4x(&RGBA8);
}

/// An ES 3.0 colour format at 4 samples: the sample buffer in the format's
/// cpp, 64-bpp tile 32x16 (sRGB8_A8 is 16F at 64 bpp). Was the refusal
/// "4x multisampling of an ES 3.0 colour format".
#[cfg(feature = "msaa-fbo")]
#[test]
fn srgb8_alpha8_4x_draw_and_resolve() {
    assert_eq!(expected_job(&ES3_FORMAT).tile, (32, 16));
    expect_4x(&ES3_FORMAT);
}

/// Two RGBA8 attachments at 4 samples: tile 32x16 (RT class 1 + 2 + bpp 0),
/// a sample buffer and a resolve for each RT. Was the refusal "4x
/// multisampling of several colour attachments".
#[cfg(feature = "msaa-fbo")]
#[test]
fn two_rgba8_4x_draw_and_resolve() {
    assert_eq!(expected_job(&TWO_RTS).tile, (32, 16));
    expect_4x(&TWO_RTS);
}

/// {RGBA8, sRGB8_A8}: tile 16x16 (index 1+2+1), the note's example: both
/// former refusals at once.
#[cfg(feature = "msaa-fbo")]
#[test]
fn rgba8_and_srgb_4x_draw_and_resolve() {
    assert_eq!(expected_job(&[GL_RGBA8, GL_SRGB8_ALPHA8]).tile, (16, 16));
    expect_4x(&[GL_RGBA8, GL_SRGB8_ALPHA8]);
}

/// The other multisample-renderable colour formats of ES 3.0 4.4.2.1 at 4
/// samples.
#[cfg(feature = "msaa-fbo")]
#[test]
fn other_es3_formats_4x_storage() {
    let _g = context();
    for f in [GL_RGB10_A2, GL_R8, GL_RG8, GL_RGB565, GL_RGBA4, GL_RGB5_A1, GL_RGB8] {
        let (e, rb) = storage(4, f);
        assert_eq!((e, rb_samples(rb)), (GL_NO_ERROR, 4), "{f:#x}");
    }
}

/// The samples go with the renderbuffer's contents: after a whole
/// glInvalidateFramebuffer of the 4x attachment, and after its storage is
/// specified again (another generation, the same id), the next job starts
/// from the clear colour instead of loading stale samples. A
/// glInvalidateSubFramebuffer keeps them.
#[cfg(feature = "msaa-fbo")]
#[test]
fn the_samples_go_with_the_contents() {
    let _g = context();
    let fbo = framebuffer(4, &RGBA8).unwrap();
    program(1);
    let loads = || draw().1.clears.last().expect("the job ended").load_rt0;
    let (first, _) = draw();
    assert!(loads(), "kept between jobs");
    let att = [GL_COLOR_ATTACHMENT0];
    // SAFETY: one enum.
    unsafe { glInvalidateSubFramebuffer(GL_FRAMEBUFFER, 1, att.as_ptr(), 0, 0, 4, 4) };
    assert!(loads(), "a sub-rectangle keeps them");
    // SAFETY: one enum.
    unsafe { glInvalidateFramebuffer(GL_FRAMEBUFFER, 1, att.as_ptr()) };
    assert_eq!(err(), GL_NO_ERROR);
    assert!(!loads(), "a whole invalidate drops them");
    assert!(loads(), "and the next job keeps them again");
    // SAFETY: the bound renderbuffer is this framebuffer's.
    unsafe {
        glBindRenderbuffer(GL_RENDERBUFFER, fbo.rbs[0]);
        glRenderbufferStorageMultisample(GL_RENDERBUFFER, 4, GL_RGBA8, 8, 8);
    }
    let (again, s) = draw();
    assert_eq!(again.id.0, first.id.0, "the same object");
    assert_ne!(again.id.1, first.id.1, "new storage");
    assert!(!s.clears.last().expect("the job ended").load_rt0, "its old samples are not loaded");
}
