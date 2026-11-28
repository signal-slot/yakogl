// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The life of a 4x renderbuffer on the V3D: delete, re-store and reuse, through
//! the GL API, the real GL core and the real V3D back end (`draw_call`) on the
//! host, with the job builder stood in for (the harness of
//! tests/es3_msaa_v3d_sizes.rs; it draws no pixels). docs/notes/es3-msaa-renderbuffers.md.
//!
//! - a 4x renderbuffer is drawn into and resolved, deleted, and a new one
//!   (which may take the name back) is drawn into with another colour and
//!   resolved: the draws carry another `(target_id, target_gen)`, the job
//!   starts from the clear colour (`load_rt0` false) and not from the old
//!   samples, and the resolve is the new renderbuffer's;
//! - an attached 4x renderbuffer is stored again as 1x and back as 4x between
//!   draws: each draw's `samples` follows, the generation moves at every
//!   storage, the id stays, and a 4x job after the re-storage does not load
//!   the earlier samples;
//! - a 4x renderbuffer deleted while attached leaves the framebuffer without
//!   it: the framebuffer is incomplete and a draw reaches no job.
//!
//! The resolved bytes are the renderbuffer's own (nothing is drawn on the
//! host), so what "no leak" can check here is identity and job state, plus
//! that every resolved pixel is the same. BOARD-UNVERIFIED: no Pi 4 run.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table,msaa-fbo --test es3_msaa_v3d_life
//! ```

#![cfg(all(
    feature = "host-test",
    feature = "shaderc-online",
    feature = "gpu-v3d",
    feature = "msaa-fbo",
    not(feature = "report-es2")
))]

use core::ffi::{c_char, c_int, c_void, CStr};
use std::collections::BTreeMap;
use std::ffi::CString;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Mutex, MutexGuard, Once, OnceLock};
use yakogl::backend::{GlesDraw, GLES_MAX_TARGETS};
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
    tiles: (u32, u32),
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
    seen(|s| s.tilings.push(Tiling { rts, msaa, max_bpp, tile: (t.tile_w, t.tile_h), tiles: (t.tiles_x, t.tiles_y) }));
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

const W: GLsizei = 40;
const H: GLsizei = 40;

struct Fbo {
    fb: GLuint,
    rbs: Vec<GLuint>,
}

impl Drop for Fbo {
    fn drop(&mut self) {
        // SAFETY: names this test made (a deleted renderbuffer name is ignored).
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteFramebuffers(1, &self.fb);
            glDeleteRenderbuffers(self.rbs.len() as GLsizei, self.rbs.as_ptr());
            glUseProgram(0);
        }
    }
}

/// A new RGBA8 renderbuffer of `samples` (0: plain storage), W x H.
fn renderbuffer(samples: GLsizei) -> GLuint {
    let mut rb = 0;
    // SAFETY: plain GL calls; room for one name.
    unsafe {
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorageMultisample(GL_RENDERBUFFER, samples, GL_RGBA8, W, H);
    }
    assert_eq!(err(), GL_NO_ERROR, "storage at {samples} samples");
    rb
}

/// Store `rb` again with `samples` (0: plain storage).
fn restore(rb: GLuint, samples: GLsizei) {
    // SAFETY: plain GL calls on a renderbuffer this test made.
    unsafe {
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        if samples == 0 {
            glRenderbufferStorage(GL_RENDERBUFFER, GL_RGBA8, W, H);
        } else {
            glRenderbufferStorageMultisample(GL_RENDERBUFFER, samples, GL_RGBA8, W, H);
        }
    }
    assert_eq!(err(), GL_NO_ERROR, "re-storage at {samples} samples");
}

/// A bound framebuffer with one RGBA8 renderbuffer of `samples`; complete.
fn framebuffer(samples: GLsizei) -> Fbo {
    let mut fb = 0;
    let rb = renderbuffer(samples);
    // SAFETY: plain GL calls.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb);
        glViewport(0, 0, W, H);
        assert_eq!(err(), GL_NO_ERROR);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
    }
    Fbo { fb, rbs: vec![rb] }
}

fn status() -> GLenum {
    // SAFETY: a plain GL call.
    unsafe { glCheckFramebufferStatus(GL_FRAMEBUFFER) }
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

/// A program that writes the colour `c`.
fn program(c: [f32; 3]) {
    let v = shader(GL_VERTEX_SHADER, "#version 300 es\nin vec4 p;\nvoid main() { gl_Position = p; }\n");
    let f = shader(
        GL_FRAGMENT_SHADER,
        &format!(
            "#version 300 es\nprecision highp float;\nlayout(location = 0) out vec4 o;\nvoid main() {{ o = vec4({:?}, {:?}, {:?}, 1.0); }}\n",
            c[0], c[1], c[2]
        ),
    );
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

/// One triangle draw (no clear unless `clear`), flushed; what the back end was
/// handed (None when the draw never reached it) and the job builder's record.
/// The GL error is left for the caller.
fn draw(clear: bool) -> (Option<GlSeen>, Seen) {
    static TRI: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 1.0, -1.0, 0.0, 1.0, -1.0, 1.0, 0.0, 1.0];
    seen(|s| *s = Seen::default());
    // SAFETY: TRI is static and outlives the draw.
    unsafe {
        if clear {
            glClearColor(0.0, 0.0, 1.0, 1.0);
            glClear(GL_COLOR_BUFFER_BIT);
        }
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, TRI.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glDrawArrays(GL_TRIANGLES, 0, 3);
        glFinish();
    }
    let s = seen(core::mem::take);
    let d = s.gl.iter().rev().find(|d| d.draws).cloned();
    (d, s)
}

/// A draw that must succeed.
fn draw_ok(clear: bool) -> (GlSeen, Seen) {
    let (d, s) = draw(clear);
    assert_eq!(err(), GL_NO_ERROR, "{:?}", s.log);
    (d.expect("the draw reached the back end"), s)
}

/// Resolve blit of `read` into a fresh 1x framebuffer, then every corner and
/// edge middle read back; the blit's error and the bytes.
fn resolve_and_read(read: GLuint) -> (GLenum, Vec<[u8; 4]>) {
    let dst = framebuffer(0);
    // SAFETY: plain GL calls on names this test made.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, read);
        glBindFramebuffer(GL_DRAW_FRAMEBUFFER, dst.fb);
        while glGetError() != GL_NO_ERROR {}
        glBlitFramebuffer(0, 0, W, H, 0, 0, W, H, GL_COLOR_BUFFER_BIT, GL_NEAREST);
    }
    let e = err();
    let mut px = Vec::new();
    let (r, t) = (W - 1, H - 1);
    // SAFETY: four bytes per read, inside the framebuffer.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, dst.fb);
        for (x, y) in [(0, 0), (r, 0), (0, t), (r, t), (r / 2, 0), (r / 2, t), (0, t / 2), (r, t / 2), (20, 20)] {
            let mut p = [0xAAu8; 4];
            glReadPixels(x, y, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, p.as_mut_ptr() as *mut c_void);
            assert_eq!(err(), GL_NO_ERROR, "glReadPixels at ({x}, {y})");
            px.push(p);
        }
        glBindFramebuffer(GL_FRAMEBUFFER, read);
    }
    drop(dst);
    (e, px)
}

/// Resolved bytes: the blit raised nothing and every pixel is the same (the
/// stand-in job builder draws none; the renderbuffer's own zeros come back).
fn assert_resolved(read: GLuint, what: &str) {
    let (e, px) = resolve_and_read(read);
    assert_eq!(e, GL_NO_ERROR, "{what}: the resolve blit");
    assert!(px.iter().all(|p| *p == px[0]), "{what}: {px:?}");
    assert_eq!(px[0], [0, 0, 0, 0], "{what}: nothing of an earlier draw, nothing drawn on the host");
}

/// The tile the job asked for at 4x is the 32x32 of RGBA8 (v3d_frame.c).
fn assert_4x_job(s: &Seen, what: &str) {
    assert_eq!(s.jobs.len(), 1, "{what}: one job: {:?}", s.log);
    let t = s.jobs[0];
    assert_eq!((t.width, t.height, t.msaa), (W as u32, H as u32, true), "{what}: {t:?}");
    assert!(t.address != 0 && t.ms_address != 0, "{what}: {t:?}");
}

fn rb_samples(rb: GLuint) -> GLint {
    let mut v = -1;
    // SAFETY: one value; a renderbuffer this test made.
    unsafe {
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glGetRenderbufferParameteriv(GL_RENDERBUFFER, GL_RENDERBUFFER_SAMPLES, &mut v);
    }
    assert_eq!(err(), GL_NO_ERROR);
    v
}

/* ---- delete and reuse ---------------------------------------------------- */

/// Draw red, resolve, delete the renderbuffer, make a new one (detached name
/// possibly reused) and draw green: the second draw has its own identity and
/// starts from the clear, and its resolve is the new renderbuffer's.
#[test]
fn delete_then_new_renderbuffer_does_not_leak() {
    let _g = context();
    let fbo = framebuffer(4);
    program([1.0, 0.0, 0.0]);
    let (d1, s1) = draw_ok(true);
    assert_eq!((d1.samples, d1.rc), (4, 0), "{d1:?}");
    assert_4x_job(&s1, "red");
    assert!(!s1.clears.last().unwrap().load_rt0);
    let old = fbo.rbs[0];
    assert_resolved(fbo.fb, "red");

    // SAFETY: names this test made.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, fbo.fb);
        glDeleteRenderbuffers(1, &old);
    }
    assert_eq!(err(), GL_NO_ERROR);
    assert_ne!(status(), GL_FRAMEBUFFER_COMPLETE, "the colour attachment is gone");

    let rb = renderbuffer(4);
    // SAFETY: names this test made.
    unsafe {
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb);
    }
    assert_eq!(err(), GL_NO_ERROR);
    assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE);
    program([0.0, 1.0, 0.0]);
    let (d2, s2) = draw_ok(true);
    assert_eq!((d2.samples, d2.rc), (4, 0), "{d2:?}");
    assert_4x_job(&s2, "green");
    assert!(!s2.clears.last().unwrap().load_rt0, "a new renderbuffer: no samples to load, from the clear colour");
    assert!(d2.id.0 != 0 && d2.id.1 != 0, "{d2:?}");
    assert_eq!(d2.id, (d2.ids[0], d2.gens[0]));
    assert!(d2.id != d1.id, "another renderbuffer or another generation: {d1:?} {d2:?}");
    assert_resolved(fbo.fb, "green");
    // SAFETY: a name this test made.
    unsafe { glDeleteRenderbuffers(1, &rb) };
}

/// The same, with the new renderbuffer made first, so that no name is reused:
/// the deleted renderbuffer's id is never handed to the new one.
#[test]
fn new_renderbuffer_has_another_id_than_the_deleted_one() {
    let _g = context();
    let a = framebuffer(4);
    program([1.0, 0.0, 0.0]);
    let (da, _) = draw_ok(true);
    let b = framebuffer(4);
    program([0.0, 1.0, 0.0]);
    let (db, sb) = draw_ok(true);
    assert_ne!(da.id.0, db.id.0, "two live renderbuffers: two ids");
    assert!(!sb.clears.last().unwrap().load_rt0);
    drop(a);
    // SAFETY: a name this test made.
    unsafe { glBindFramebuffer(GL_FRAMEBUFFER, b.fb) };
    program([0.0, 0.0, 1.0]);
    let (db2, _) = draw_ok(false);
    assert_eq!(db2.id, db.id, "the survivor keeps its pair after the other's deletion");
    let c = framebuffer(4);
    program([1.0, 1.0, 0.0]);
    let (dc, sc) = draw_ok(true);
    assert!(dc.id.0 != da.id.0 && dc.id.0 != db.id.0, "{da:?} {db:?} {dc:?}");
    assert!(!sc.clears.last().unwrap().load_rt0);
    assert_resolved(c.fb, "the third");
}

/// A redraw without a clear on the new renderbuffer loads only what the new
/// one's own first job kept (`load_rt0` on the second job of the new pair).
#[test]
fn second_job_loads_only_the_new_renderbuffers_samples() {
    let _g = context();
    let fbo = framebuffer(4);
    program([1.0, 0.0, 0.0]);
    let (_, _) = draw_ok(true);
    let old = fbo.rbs[0];
    // SAFETY: names this test made.
    unsafe { glDeleteRenderbuffers(1, &old) };
    let rb = renderbuffer(4);
    // SAFETY: names this test made.
    unsafe { glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb) };
    program([0.0, 1.0, 0.0]);
    let (d1, s1) = draw_ok(false);
    assert!(!s1.clears.last().unwrap().load_rt0, "a never-drawn 4x target has nothing to load: {d1:?}");
    let (d2, s2) = draw_ok(false);
    assert_eq!(d2.id, d1.id);
    assert!(s2.clears.last().unwrap().load_rt0, "the new renderbuffer's own samples are loaded");
    // SAFETY: a name this test made.
    unsafe { glDeleteRenderbuffers(1, &rb) };
}

/* ---- re-store as 1x and back ----------------------------------------------- */

/// 4 -> 1 -> 4 between draws: the draw's samples follow, the generation moves
/// at each storage with the id kept, and the 4x job after the round trip does
/// not load the samples of the first one.
#[test]
fn restore_one_x_and_back_between_draws() {
    let _g = context();
    let fbo = framebuffer(4);
    let rb = fbo.rbs[0];
    program([1.0, 0.0, 0.0]);
    let (d4, s4) = draw_ok(true);
    assert_eq!((d4.samples, d4.rc), (4, 0));
    assert_4x_job(&s4, "first 4x");

    restore(rb, 0);
    assert_eq!(rb_samples(rb), 0);
    // SAFETY: the framebuffer is this test's.
    unsafe { glBindFramebuffer(GL_FRAMEBUFFER, fbo.fb) };
    assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE);
    program([0.0, 1.0, 0.0]);
    let (d1, s1) = draw_ok(true);
    assert_eq!((d1.samples, d1.rc, d1.targets.len()), (1, 0, 1), "{d1:?}");
    assert!(s1.jobs.iter().all(|j| !j.msaa), "the 1x draw is a 1x job: {:?}", s1.jobs);
    assert_eq!(d1.id.0, d4.id.0, "the id is the renderbuffer's");
    assert_ne!(d1.id.1, d4.id.1, "new storage, new generation");

    restore(rb, 4);
    assert_eq!(rb_samples(rb), 4);
    // SAFETY: the framebuffer is this test's.
    unsafe { glBindFramebuffer(GL_FRAMEBUFFER, fbo.fb) };
    assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE);
    program([0.0, 0.0, 1.0]);
    let (d4b, s4b) = draw_ok(false);
    assert_eq!((d4b.samples, d4b.rc), (4, 0), "{d4b:?}");
    assert_4x_job(&s4b, "second 4x");
    assert_eq!(d4b.id.0, d4.id.0);
    assert!(d4b.id.1 != d4.id.1 && d4b.id.1 != d1.id.1, "a generation is not reused: {d4:?} {d1:?} {d4b:?}");
    assert!(
        !s4b.clears.last().unwrap().load_rt0,
        "the samples of the first 4x storage are not loaded into the new storage"
    );
    assert_resolved(fbo.fb, "after the round trip");
}

/// Re-storing as 4x again (same parameters) between draws is new storage too:
/// the next 4x job does not load, and the generation moves.
#[test]
fn restore_four_x_again_drops_the_samples() {
    let _g = context();
    let fbo = framebuffer(4);
    program([1.0, 0.0, 0.0]);
    let (d1, _) = draw_ok(true);
    let (d2, s2) = draw_ok(false);
    assert_eq!(d2.id, d1.id);
    assert!(s2.clears.last().unwrap().load_rt0, "a redraw loads the kept samples");
    restore(fbo.rbs[0], 4);
    // SAFETY: the framebuffer is this test's.
    unsafe { glBindFramebuffer(GL_FRAMEBUFFER, fbo.fb) };
    let (d3, s3) = draw_ok(false);
    assert_eq!(d3.id.0, d1.id.0);
    assert_ne!(d3.id.1, d1.id.1);
    assert!(!s3.clears.last().unwrap().load_rt0, "new storage: nothing to load");
}

/// A 1x -> 4x re-storage: the framebuffer stays complete (one attachment), the
/// resolve blit takes the new storage and not an old one.
#[test]
fn restore_one_x_to_four_x_resolves() {
    let _g = context();
    let fbo = framebuffer(0);
    program([1.0, 0.0, 0.0]);
    let (d1, _) = draw_ok(true);
    assert_eq!(d1.samples, 1);
    restore(fbo.rbs[0], 4);
    // SAFETY: the framebuffer is this test's.
    unsafe { glBindFramebuffer(GL_FRAMEBUFFER, fbo.fb) };
    assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE);
    let (d4, s4) = draw_ok(true);
    assert_eq!(d4.samples, 4, "{d4:?}");
    assert_4x_job(&s4, "4x after 1x");
    assert_eq!(d4.id.0, d1.id.0);
    assert_ne!(d4.id.1, d1.id.1);
    assert_resolved(fbo.fb, "4x after 1x");
}

/* ---- delete while attached --------------------------------------------------- */

/// Delete a 4x renderbuffer while it is attached to the bound framebuffer:
/// the framebuffer is incomplete, a draw is refused with
/// GL_INVALID_FRAMEBUFFER_OPERATION and reaches no job.
#[test]
fn delete_while_attached_then_redraw() {
    let _g = context();
    let fbo = framebuffer(4);
    program([1.0, 0.0, 0.0]);
    let (_, s) = draw_ok(true);
    assert_4x_job(&s, "before the delete");
    let rb = fbo.rbs[0];
    // SAFETY: a name this test made.
    unsafe { glDeleteRenderbuffers(1, &rb) };
    assert_eq!(err(), GL_NO_ERROR);
    assert_eq!(status(), GL_FRAMEBUFFER_INCOMPLETE_MISSING_ATTACHMENT);
    let (d, s) = draw(false);
    assert_eq!(err(), GL_INVALID_FRAMEBUFFER_OPERATION, "{:?}", s.log);
    assert!(d.is_none(), "an incomplete framebuffer draws nothing: {d:?}");
    assert!(s.jobs.is_empty(), "{:?}", s.jobs);
}

/// After that, a fresh 4x renderbuffer on the same framebuffer draws again
/// and resolves; the failed draw left nothing pending.
#[test]
fn delete_while_attached_then_recover() {
    let _g = context();
    let fbo = framebuffer(4);
    program([1.0, 0.0, 0.0]);
    let (d1, _) = draw_ok(true);
    let rb = fbo.rbs[0];
    // SAFETY: names this test made.
    unsafe { glDeleteRenderbuffers(1, &rb) };
    let (_, _) = draw(false);
    assert_eq!(err(), GL_INVALID_FRAMEBUFFER_OPERATION);
    let rb2 = renderbuffer(4);
    // SAFETY: names this test made.
    unsafe { glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb2) };
    assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE);
    let (d2, s2) = draw_ok(true);
    assert_eq!((d2.samples, d2.rc), (4, 0));
    assert_4x_job(&s2, "recovered");
    assert!(d2.id != d1.id);
    assert!(!s2.clears.last().unwrap().load_rt0);
    assert_resolved(fbo.fb, "recovered");
    // SAFETY: a name this test made.
    unsafe { glDeleteRenderbuffers(1, &rb2) };
}

/// Delete a 4x renderbuffer that is the read side of a resolve: the blit is
/// refused (the read framebuffer is incomplete), not served from the old samples.
#[test]
fn resolve_from_a_deleted_renderbuffer_is_refused() {
    let _g = context();
    let fbo = framebuffer(4);
    program([1.0, 0.0, 0.0]);
    let _ = draw_ok(true);
    let rb = fbo.rbs[0];
    // SAFETY: a name this test made.
    unsafe { glDeleteRenderbuffers(1, &rb) };
    assert_eq!(err(), GL_NO_ERROR);
    let (e, _) = resolve_and_read_raw(fbo.fb);
    assert_eq!(e, GL_INVALID_FRAMEBUFFER_OPERATION);
}

/// The blit only, no readback.
fn resolve_and_read_raw(read: GLuint) -> (GLenum, ()) {
    let dst = framebuffer(0);
    // SAFETY: plain GL calls on names this test made.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, read);
        glBindFramebuffer(GL_DRAW_FRAMEBUFFER, dst.fb);
        while glGetError() != GL_NO_ERROR {}
        glBlitFramebuffer(0, 0, W, H, 0, 0, W, H, GL_COLOR_BUFFER_BIT, GL_NEAREST);
    }
    (err(), ())
}
