// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! 4x depth/stencil resolve on the V3D at the sizes dEQP draws (119x131 in
//! fbo.msaa, 40x40 in multisample.fbo_max_samples) and one tile-aligned size
//! (64x64), for DEPTH24_STENCIL8, DEPTH_COMPONENT16 and DEPTH_COMPONENT32F,
//! through the GL API, the real GL core and the real V3D back end
//! (`draw_call`) on the host. docs/notes/es3-msaa-renderbuffers.md (g), (h).
//!
//! Each case: a 4x FBO of RGBA8 plus the depth format, a depth/stencil-tested
//! draw, a GL_NEAREST resolve blit of depth (and stencil) into a 1x FBO of the
//! same formats, then a depth/stencil-tested draw into the 1x FBO. The job
//! builder is stood in for (as in es3_msaa_v3d_sizes.rs) and draws no pixels,
//! and the GL core has no depth read-back, so the depth and stencil values are
//! BOARD-UNVERIFIED (no Pi 4 run): the checks are the 4x job, the blit's
//! absence of error and the 1x job that follows. Nothing fails today, so no
//! test is pinned or `#[ignore]`d.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table,msaa-fbo --test es3_msaa_v3d_sizes
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

/* ---- framebuffers with depth and stencil, the program, the draws ------- */

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
            glDisable(GL_DEPTH_TEST);
            glDisable(GL_STENCIL_TEST);
        }
    }
}

/// The depth/stencil renderbuffer formats of the file.
const D24S8: GLenum = GL_DEPTH24_STENCIL8;
const D16: GLenum = GL_DEPTH_COMPONENT16;
const D32F: GLenum = GL_DEPTH_COMPONENT32F;

/// The attachment point of a depth/stencil format, and the blit mask of it.
fn attachment(fmt: GLenum) -> GLenum {
    if fmt == D24S8 {
        GL_DEPTH_STENCIL_ATTACHMENT
    } else {
        GL_DEPTH_ATTACHMENT
    }
}

fn blit_mask(fmt: GLenum) -> GLbitfield {
    if fmt == D24S8 {
        GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT
    } else {
        GL_DEPTH_BUFFER_BIT
    }
}

/// A bound `w`x`h` framebuffer with an RGBA8 renderbuffer and a `fmt`
/// depth/stencil renderbuffer, both of `samples` (0: plain storage); storage
/// and completeness are asserted.
fn framebuffer(samples: GLsizei, fmt: GLenum, w: GLsizei, h: GLsizei) -> Fbo {
    let (mut fb, mut rb) = (0, [0; 2]);
    // SAFETY: room for the names; plain GL calls.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glGenRenderbuffers(2, rb.as_mut_ptr());
        glBindRenderbuffer(GL_RENDERBUFFER, rb[0]);
        glRenderbufferStorageMultisample(GL_RENDERBUFFER, samples, GL_RGBA8, w, h);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb[0]);
        glBindRenderbuffer(GL_RENDERBUFFER, rb[1]);
        glRenderbufferStorageMultisample(GL_RENDERBUFFER, samples, fmt, w, h);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, attachment(fmt), GL_RENDERBUFFER, rb[1]);
        glViewport(0, 0, w, h);
        assert_eq!(err(), GL_NO_ERROR, "{w}x{h} {fmt:#x} at {samples} samples");
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "{w}x{h} {fmt:#x}");
    }
    Fbo { fb, rbs: rb.to_vec() }
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

fn program() {
    let v = shader(GL_VERTEX_SHADER, "#version 300 es\nin vec4 p;\nvoid main() { gl_Position = p; }\n");
    let f = shader(
        GL_FRAGMENT_SHADER,
        "#version 300 es\nprecision highp float;\nlayout(location = 0) out vec4 o;\nvoid main() { o = vec4(1.0, 0.0, 0.0, 1.0); }\n",
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

/// Clear colour, depth 1 and stencil 0, then draw the triangle of the lower
/// left half at depth `z` with the depth test LESS (and, with stencil,
/// REPLACE by 1), flushed; what the back end was handed and what the job
/// builder recorded.
fn draw_ds(fmt: GLenum, z: f32) -> (GlSeen, Seen) {
    let tri: [f32; 12] = [-1.0, -1.0, z, 1.0, 1.0, -1.0, z, 1.0, -1.0, 1.0, z, 1.0];
    seen(|s| *s = Seen::default());
    // SAFETY: `tri` outlives the draw (glFinish).
    unsafe {
        glClearColor(0.0, 0.0, 1.0, 1.0);
        glClearDepthf(1.0);
        glClearStencil(0);
        glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT);
        glEnable(GL_DEPTH_TEST);
        glDepthFunc(GL_LESS);
        if fmt == D24S8 {
            glEnable(GL_STENCIL_TEST);
            glStencilFunc(GL_ALWAYS, 1, 0xff);
            glStencilOp(GL_KEEP, GL_KEEP, GL_REPLACE);
        }
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, tri.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glDrawArrays(GL_TRIANGLES, 0, 3);
        glFinish();
    }
    let s = seen(core::mem::take);
    assert_eq!(err(), GL_NO_ERROR, "{:?}", s.log);
    let d = s.gl.iter().rev().find(|d| d.draws).cloned().expect("the draw reached the back end");
    (d, s)
}

/// The resolve blit of the depth (and stencil) of `read` into `dst`, same
/// rectangle, GL_NEAREST: the blit's first error.
fn resolve_ds(read: &Fbo, dst: &Fbo, fmt: GLenum, w: GLint, h: GLint) -> GLenum {
    // SAFETY: plain GL calls on names this test made.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, read.fb);
        glBindFramebuffer(GL_DRAW_FRAMEBUFFER, dst.fb);
        while glGetError() != GL_NO_ERROR {}
        glBlitFramebuffer(0, 0, w, h, 0, 0, w, h, blit_mask(fmt), GL_NEAREST);
    }
    err()
}

/// A depth/stencil-tested draw (depth 0.75 LESS, stencil EQUAL 1) into the
/// bound 1x framebuffer after the resolve: what the back end was handed and
/// what the job builder recorded.
fn test_draw_1x(fmt: GLenum) -> (GlSeen, Seen) {
    let tri: [f32; 12] = [-1.0, -1.0, 0.75, 1.0, 1.0, -1.0, 0.75, 1.0, -1.0, 1.0, 0.75, 1.0];
    seen(|s| *s = Seen::default());
    // SAFETY: `tri` outlives the draw (glFinish).
    unsafe {
        glEnable(GL_DEPTH_TEST);
        glDepthFunc(GL_LESS);
        if fmt == D24S8 {
            glEnable(GL_STENCIL_TEST);
            glStencilFunc(GL_EQUAL, 1, 0xff);
            glStencilOp(GL_KEEP, GL_KEEP, GL_KEEP);
        }
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, tri.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glDrawArrays(GL_TRIANGLES, 0, 3);
        glFinish();
    }
    let s = seen(core::mem::take);
    assert_eq!(err(), GL_NO_ERROR, "{:?}", s.log);
    let d = s.gl.iter().rev().find(|d| d.draws).cloned().expect("the test draw reached the back end");
    (d, s)
}

/// One format at one size: the 4x FBO with depth/stencil is complete and
/// draws as 4x; the depth/stencil resolve blit into a 1x FBO of the same
/// format raises no error; a depth/stencil-tested draw into the 1x FBO
/// reaches the back end as a 1x job of the frame's size and is accepted.
/// The depth and stencil values themselves are not read: the job builder is
/// stood in for and draws no pixels, and the GL core has no depth read-back
/// (BOARD-UNVERIFIED, no Pi 4 run).
fn expect(fmt: GLenum, w: u32, h: u32) {
    let _g = context();
    let src = framebuffer(4, fmt, w as GLsizei, h as GLsizei);
    let mut rb_samples = -1;
    // SAFETY: one value; the renderbuffer is this framebuffer's.
    unsafe {
        glBindRenderbuffer(GL_RENDERBUFFER, src.rbs[1]);
        glGetRenderbufferParameteriv(GL_RENDERBUFFER, GL_RENDERBUFFER_SAMPLES, &mut rb_samples);
    }
    assert_eq!(rb_samples, 4, "{w}x{h} {fmt:#x}");
    program();
    let (d, s) = draw_ds(fmt, 0.5);
    assert_eq!((d.samples, d.rc), (4, 0), "{w}x{h} {fmt:#x}: {:?}", s.log);
    assert!(d.id.0 != 0 && d.id.1 != 0, "{d:?}");
    assert_eq!(d.id, (d.ids[0], d.gens[0]), "target_id is targets[0]'s");
    assert_eq!(d.targets.len(), 1, "{d:?}");
    assert_eq!(s.jobs.len(), 1, "{w}x{h} {fmt:#x}: {:?}", s.log);
    assert_eq!((s.jobs[0].width, s.jobs[0].height, s.jobs[0].msaa), (w, h, true), "{:?}", s.jobs[0]);
    let dst = framebuffer(0, fmt, w as GLsizei, h as GLsizei);
    let e = resolve_ds(&src, &dst, fmt, w as GLint, h as GLint);
    assert_eq!(e, GL_NO_ERROR, "{w}x{h} {fmt:#x}: the depth/stencil resolve blit");
    // SAFETY: plain GL call.
    unsafe { glBindFramebuffer(GL_FRAMEBUFFER, dst.fb) };
    let (d2, s2) = test_draw_1x(fmt);
    assert_eq!((d2.samples, d2.rc), (1, 0), "{w}x{h} {fmt:#x}: {:?}", s2.log);
    assert_ne!(d2.id, d.id, "the destination is another target");
    assert_eq!(s2.jobs.len(), 1, "{w}x{h} {fmt:#x}: {:?}", s2.log);
    assert_eq!((s2.jobs[0].width, s2.jobs[0].height, s2.jobs[0].msaa), (w, h, false), "{:?}", s2.jobs[0]);
}

/// The sizes: fbo.msaa's, multisample.fbo_max_samples', and one that is whole
/// 32x32 tiles of 4x (64x64).
macro_rules! sizes {
    ($($name:ident: $fmt:expr;)*) => {$(
        mod $name {
            use super::*;
            #[test]
            fn size_119x131() { expect($fmt, 119, 131); }
            #[test]
            fn size_40x40() { expect($fmt, 40, 40); }
            #[test]
            fn size_64x64() { expect($fmt, 64, 64); }
        }
    )*};
}

sizes! {
    depth24_stencil8: D24S8;
    depth_component16: D16;
    depth_component32f: D32F;
}
