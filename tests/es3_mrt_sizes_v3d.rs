// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Colour attachments of different sizes on the V3D host back end
//! (t-mrt-sizes-v3d), through the GL API, the real GL core and the real
//! V3D back end, with the job builder stood in for (the harness of
//! tests/es3_int_rt_v3d.rs `gl_api` and tests/es3_mrt_ds_clear_v3d.rs): it
//! records each job's `V3dJobTarget` (the frame's `width`/`height`, RT 0's
//! `stride`, RT 1..3's `rt[i].address`/`stride`), the draws, the
//! `V3dJobClear` each job ends with, the log and the refusals.
//!
//! ES 3.0 4.4.4.2 allows attachments of different sizes and 4.4.1 renders
//! their intersection; the GL core calls such a framebuffer complete and
//! hands each target its own size (docs/notes/v3d-mrt-sizes.md (a)). Today
//! `extra_targets` (src/v3d/backend.rs) refuses the draw by name, "colour
//! attachments of different sizes", and RT 1..3's stride is RT 0's
//! (`begin_job`, note (b)). The GL-API counterparts of the note's host tests
//! ((f) 1, 2, 3, 4, 5, 7; 6 and 8 are src/v3d/backend_tests.rs's own):
//!
//! - today, pinned: the refusal and the error the application sees, nothing
//!   recorded, the context usable after; a GL_NONE slot is not a size; a
//!   depth attachment smaller than the colour does not limit the frame;
//! - expected, `#[ignore]`d until the v3d-mrt-sizes change: the frame is the
//!   intersection, each RT keeps its own pitch, the depth attachment limits
//!   the frame, a fused clear clears the intersection, a resize between two
//!   draws is another job.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_mrt_sizes_v3d
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_mrt_sizes_v3d -- --ignored
//! ```
#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-v3d", not(feature = "report-es2")))]

use core::ffi::{c_char, c_int, c_void, CStr};
use std::collections::BTreeMap;
use std::ffi::CString;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Mutex, MutexGuard, Once, OnceLock};
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
use yakogl::v3d::backend::{target_pitch, v3d_refused_draws, yakogl_backend_v3d};
use yakogl::v3d::ffi::*;
use yakogl::v3d::kick::V3dKick;

/* ---- the job builder: records, draws nothing ---- */

#[derive(Default)]
struct Seen {
    jobs: Vec<V3dJobTarget>,
    clears: Vec<V3dJobClear>,
    /// The vertex count of each draw the job builder was handed.
    draws: Vec<u32>,
    log: Vec<String>,
    /// Draws the back end refused (`v3d_refused_draws`) while the scene ran.
    refused: u32,
    /// The GL errors raised while the scene ran, in order.
    errs: Vec<GLenum>,
}

static SEEN: Mutex<Seen> = Mutex::new(Seen {
    jobs: Vec::new(),
    clears: Vec::new(),
    draws: Vec::new(),
    log: Vec::new(),
    refused: 0,
    errs: Vec::new(),
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
    // SAFETY: the job's BCL is live.
    unsafe {
        (*j.bcl).used += 120;
    }
    seen(|s| s.draws.push(d.count));
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
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("v3d_mrt_sizes");
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

static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

/// The one context and back end of the file; the tests take turns and each
/// starts with nothing recorded and nothing pending.
fn rig() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    ONCE.call_once(|| {
        // SAFETY: statics; the lock serialises every GL call of the file.
        unsafe {
            osal_set_ops(osal());
            yakogl::backend::g_backend = &yakogl_backend_v3d;
            assert_eq!(yakogl::backend::init(), 0, "{:?}", seen(|s| s.log.clone()));
            assert!(!context::gl_context_create(32, 32).is_null(), "{:?}", seen(|s| s.log.clone()));
        }
    });
    // SAFETY: plain GL calls.
    unsafe {
        glFinish();
        while glGetError() != GL_NO_ERROR {}
    }
    seen(|s| *s = Seen::default());
    g
}

/* ---- the framebuffers, the program, the scene ---- */

/// A renderbuffer: its sized format, width and height.
#[derive(Clone, Copy, Debug)]
struct Rb(GLenum, GLsizei, GLsizei);

/// Colour attachment 0 of every case, 64x64.
const RT0: Rb = Rb(GL_RGBA8, 64, 64);
/// The smaller attachment of note (f) 1: 32x48, inside RT0 on both axes.
const SMALL: Rb = Rb(GL_RGBA8, 32, 48);
/// The wider attachment of note (f) 2: 96x32, wider and lower than RT0.
const WIDE: Rb = Rb(GL_RGBA8, 96, 32);
/// The depth attachment of note (f) 4: 32x32, smaller than the colour.
const DEPTH_SMALL: Rb = Rb(GL_DEPTH_COMPONENT16, 32, 32);

fn renderbuffer(r: Rb) -> GLuint {
    let mut rb = 0;
    // SAFETY: plain GL calls with room for one name.
    unsafe {
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorage(GL_RENDERBUFFER, r.0, r.1, r.2);
        assert_eq!(glGetError(), GL_NO_ERROR, "storage {r:?}");
    }
    rb
}

/// A bound framebuffer of renderbuffers on colour attachments 0.., with a
/// depth renderbuffer when asked; deleted when dropped.
struct Fbo {
    fb: GLuint,
    rbs: Vec<GLuint>,
}

impl Fbo {
    /// `colours` on GL_COLOR_ATTACHMENT0.., every one a draw buffer unless
    /// `bufs` names them; `depth` on GL_DEPTH_ATTACHMENT. The viewport is
    /// colour 0's size. The framebuffer must be complete (ES 3.0: sizes may
    /// differ).
    fn new(colours: &[Rb], bufs: Option<&[GLenum]>, depth: Option<Rb>) -> Fbo {
        let mut fb = 0;
        // SAFETY: room for one name.
        unsafe {
            glGenFramebuffers(1, &mut fb);
            glBindFramebuffer(GL_FRAMEBUFFER, fb);
        }
        let mut rbs = Vec::new();
        for (i, r) in colours.iter().enumerate() {
            let rb = renderbuffer(*r);
            // SAFETY: plain GL call.
            unsafe {
                glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0 + i as GLenum, GL_RENDERBUFFER, rb)
            };
            rbs.push(rb);
        }
        if let Some(d) = depth {
            let rb = renderbuffer(d);
            // SAFETY: plain GL call.
            unsafe { glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_RENDERBUFFER, rb) };
            rbs.push(rb);
        }
        let all: Vec<GLenum> = (0..colours.len()).map(|i| GL_COLOR_ATTACHMENT0 + i as GLenum).collect();
        let bufs = bufs.unwrap_or(&all);
        // SAFETY: `bufs` has `len` enums.
        unsafe {
            glDrawBuffers(bufs.len() as GLsizei, bufs.as_ptr());
            glViewport(0, 0, colours[0].1, colours[0].2);
        }
        let log = seen(|s| s.log.clone());
        // SAFETY: plain GL calls.
        unsafe {
            assert_eq!(glGetError(), GL_NO_ERROR, "{colours:?} {depth:?}: {log:?}");
            let st = glCheckFramebufferStatus(GL_FRAMEBUFFER);
            assert_eq!(st, GL_FRAMEBUFFER_COMPLETE, "{colours:?} {depth:?}: {log:?}");
        }
        Fbo { fb, rbs }
    }

    /// Colour attachment `i`'s renderbuffer given new storage: another size,
    /// the same attachment point.
    fn resize(&self, i: usize, r: Rb) {
        // SAFETY: a name this test made.
        unsafe {
            glBindRenderbuffer(GL_RENDERBUFFER, self.rbs[i]);
            glRenderbufferStorage(GL_RENDERBUFFER, r.0, r.1, r.2);
            assert_eq!(glGetError(), GL_NO_ERROR, "re-storage {r:?}");
            assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
        }
    }
}

impl Drop for Fbo {
    fn drop(&mut self) {
        // SAFETY: names this test made.
        unsafe {
            glDisable(GL_SCISSOR_TEST);
            glColorMask(1, 1, 1, 1);
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteFramebuffers(1, &self.fb);
            glDeleteRenderbuffers(self.rbs.len() as GLsizei, self.rbs.as_ptr());
            glUseProgram(0);
        }
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

/// The current program: `n` `vec4` outputs at locations 0..n (one per
/// draw-buffer slot, so `rt_mask` fits the job's render targets).
fn program(n: usize) {
    let mut fs = String::from("#version 300 es\nprecision highp float;\n");
    for i in 0..n {
        fs += &format!("layout(location = {i}) out vec4 o{i};\n");
    }
    fs += "void main() {\n";
    for i in 0..n {
        fs += &format!("o{i} = vec4(0.25, 0.5, 0.75, {}.0);\n", i + 1);
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

/// One triangle over the framebuffer (not flushed: `scene` does that).
fn triangle() {
    static TRI: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 3.0, -1.0, 0.0, 1.0, -1.0, 3.0, 0.0, 1.0];
    // SAFETY: TRI is static and outlives the draw.
    unsafe {
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, TRI.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glDrawArrays(GL_TRIANGLES, 0, 3);
    }
}

/// glClearColor + glClear(GL_COLOR_BUFFER_BIT), the whole target.
fn clear_colour() {
    // SAFETY: plain GL calls.
    unsafe {
        glClearColor(0.25, 0.5, 0.75, 1.0);
        glClear(GL_COLOR_BUFFER_BIT);
    }
}

/// `f` run under the bound framebuffer and flushed: what the job builder
/// recorded, the GL errors raised and the draws the back end refused.
fn scene(f: impl FnOnce()) -> Seen {
    // SAFETY: plain GL calls.
    unsafe { while glGetError() != GL_NO_ERROR {} }
    seen(|s| *s = Seen::default());
    let before = v3d_refused_draws();
    f();
    // SAFETY: plain GL calls.
    unsafe {
        glFinish();
    }
    let refused = v3d_refused_draws().wrapping_sub(before);
    let mut errs = vec![];
    loop {
        // SAFETY: plain GL call.
        let e = unsafe { glGetError() };
        if e == GL_NO_ERROR {
            break;
        }
        errs.push(e);
    }
    let mut s = seen(core::mem::take);
    (s.refused, s.errs) = (refused, errs);
    s
}

/* ---- what to expect ---- */

/// The refusal's name, as `extra_targets` words it.
const REFUSAL: &str = "colour attachments of different sizes";

/// The error a V3D refusal of legal GL state reaches the application as:
/// the back end answers -ENOTSUP (`refuse_unsupported`) and `submit_draw`
/// (src/gl/draw.rs) maps that to GL_INVALID_OPERATION, where every other
/// errno is GL_OUT_OF_MEMORY and aborts a dEQP session.
const REFUSAL_ERROR: GLenum = GL_INVALID_OPERATION;

/// The raster stride of a `w`-wide render target at `bpp` bytes a pixel: the
/// pair-padded pitch (`target_pitch`, the GL core's `tex_pitch`) in bytes.
fn stride(w: GLsizei, bpp: u32) -> u32 {
    target_pitch(w as u32) * bpp
}

/// Refused as the application sees it today: one error, one refused draw,
/// nothing recorded; the log names the cause the first time only
/// (`count_refusal`), so a "draw refused" line, when there is one, is this
/// refusal's.
fn assert_refused(what: &str, s: &Seen) {
    assert_eq!(s.errs, [REFUSAL_ERROR], "{what}: {:?}", s.log);
    assert_eq!(s.refused, 1, "{what}: one refused draw");
    assert!(s.jobs.is_empty() && s.draws.is_empty() && s.clears.is_empty(), "{what}: nothing recorded");
    for l in s.log.iter().filter(|l| l.contains("draw refused")) {
        assert!(l.ends_with(REFUSAL), "{what}: {l}");
    }
}

/// Taken: no error, no refusal, one job of `rts` render targets with
/// `draws` draws, ended once. The job.
fn assert_one_job(what: &str, s: &Seen, rts: u32, draws: usize) -> V3dJobTarget {
    assert!(s.errs.is_empty() && s.refused == 0, "{what}: {:x?} {:?}", s.errs, s.log);
    assert_eq!((s.jobs.len(), s.draws.len(), s.clears.len()), (1, draws, 1), "{what}: one job: {:?}", s.log);
    assert_eq!(s.jobs[0].rts, rts, "{what}: render targets");
    s.jobs[0]
}

/* ---- equal sizes: the job as it is, the baseline of every case below ----- */

/// Two 64x64 RGBA8 attachments: one job at 64x64 with two render targets,
/// RT 0's stride and RT 1's both 64 pixels x 4 bytes, RT 1 at its own
/// address.
#[test]
fn equal_attachments_draw_at_their_size() {
    let _g = rig();
    let fbo = Fbo::new(&[RT0, RT0], None, None);
    program(2);
    let s = scene(triangle);
    drop(fbo);
    let j = assert_one_job("64x64 + 64x64", &s, 2, 1);
    assert_eq!((j.width, j.height), (64, 64));
    assert_eq!(j.stride, stride(64, 4));
    assert_ne!(j.rt[0].address, 0, "RT 1 has its address");
    assert_eq!(j.rt[0].stride, stride(64, 4));
    assert_eq!(j.rt[1].address, 0, "no RT 2");
}

/* ---- today: the refusal by name, both orders ----------------------------- */

/// RT 0 64x64 and RT 1 32x48 (note (f) 1, the first order): the GL core
/// calls the framebuffer complete and hands the draw over; `extra_targets`
/// refuses it by name, the application sees one GL error, nothing reaches
/// the job builder.
#[test]
fn a_smaller_second_attachment_is_refused_by_name_today() {
    let _g = rig();
    let fbo = Fbo::new(&[RT0, SMALL], None, None);
    program(2);
    let s = scene(triangle);
    drop(fbo);
    assert_refused("64x64 + 32x48", &s);
}

/// The other order (note (f) 1): RT 0 32x48 and RT 1 64x64 is refused the
/// same way; the size of the first attachment is not what decides.
#[test]
fn a_larger_second_attachment_is_refused_by_name_today() {
    let _g = rig();
    let fbo = Fbo::new(&[SMALL, RT0], None, None);
    program(2);
    let s = scene(triangle);
    drop(fbo);
    assert_refused("32x48 + 64x64", &s);
}

/// RT 1 wider and lower than RT 0 (96x32 against 64x64, note (f) 2): refused
/// today; neither axis has to be the smaller for the refusal.
#[test]
fn a_wider_second_attachment_is_refused_by_name_today() {
    let _g = rig();
    let fbo = Fbo::new(&[RT0, WIDE], None, None);
    program(2);
    let s = scene(triangle);
    drop(fbo);
    assert_refused("64x64 + 96x32", &s);
}

/// After the refusal the context stays usable: a draw of equal sizes on the
/// next framebuffer is recorded as one job.
#[test]
fn the_context_works_after_the_refusal() {
    let _g = rig();
    let fbo = Fbo::new(&[RT0, SMALL], None, None);
    program(2);
    let s = scene(triangle);
    drop(fbo);
    assert_refused("64x64 + 32x48", &s);
    let fbo = Fbo::new(&[RT0, RT0], None, None);
    program(2);
    let s = scene(triangle);
    drop(fbo);
    let j = assert_one_job("64x64 + 64x64 after the refusal", &s, 2, 1);
    assert_eq!((j.width, j.height), (64, 64));
}

/* ---- a GL_NONE slot is not a size (note (f) 3) ---------------------------- */

/// Attachment 1 is 32x48 but draw buffer 1 is GL_NONE and attachment 2 is
/// 64x64: the GL core hands slot 1 over as no memory and size 0x0
/// (`fb_target_n` skips a GL_NONE draw buffer, note (a)), `extra_targets`
/// skips it before the size check, and the draw is taken: three render
/// targets, RT 1 configured with address 0 and stride 0, RT 2 at its own
/// address at the frame's stride. The same before and after the change.
#[test]
fn a_gl_none_slot_is_not_a_size() {
    let _g = rig();
    let bufs = [GL_COLOR_ATTACHMENT0, GL_NONE, GL_COLOR_ATTACHMENT0 + 2];
    let fbo = Fbo::new(&[RT0, SMALL, RT0], Some(&bufs), None);
    program(3);
    let s = scene(triangle);
    drop(fbo);
    let j = assert_one_job("64x64 + (none 32x48) + 64x64", &s, 3, 1);
    assert_eq!((j.width, j.height), (64, 64));
    assert_eq!((j.rt[0].address, j.rt[0].stride), (0, 0), "the GL_NONE slot: configured, never stored");
    assert_ne!(j.rt[1].address, 0, "RT 2 has its address");
    assert_eq!(j.rt[1].stride, stride(64, 4));
}

/// The same framebuffer with draw buffer 1 named: the 32x48 attachment is a
/// size again and the draw is refused.
#[test]
fn naming_the_slot_makes_its_size_count_today() {
    let _g = rig();
    let fbo = Fbo::new(&[RT0, SMALL, RT0], None, None);
    program(3);
    let s = scene(triangle);
    drop(fbo);
    assert_refused("64x64 + 32x48 + 64x64", &s);
}

/* ---- a depth attachment smaller than the colour (note (f) 4, (e) 4) ------- */

/// Colour 64x64 with a 32x32 depth renderbuffer: complete in ES 3.0, taken,
/// and drawn at the colour's 64x64 today against a private depth buffer of
/// that size (nothing in src/v3d reads `ds_w`/`ds_h`: the silent gap of note
/// (e) 4). GL renders 32x32. Pinned as it is; the expected frame is
/// `the_depth_attachment_limits_the_frame`.
#[test]
fn a_smaller_depth_attachment_does_not_limit_the_frame_today() {
    let _g = rig();
    let fbo = Fbo::new(&[RT0], None, Some(DEPTH_SMALL));
    program(1);
    let s = scene(triangle);
    drop(fbo);
    let j = assert_one_job("64x64 + depth 32x32", &s, 0, 1);
    assert_eq!((j.width, j.height), (64, 64), "the colour's size, not the intersection");
    assert_ne!(s.clears[0].zs_address, 0, "a depth buffer of the back end's own");
}

/* ---- a clear of unequal attachments (note (f) 5) -------------------------- */

/// glClear(GL_COLOR_BUFFER_BIT), no scissor, full mask, over 64x64 and
/// 32x32 attachments: the clear-only draw carries both targets and is
/// refused by the same name today; the application sees the same error as
/// for a draw.
#[test]
fn a_clear_of_unequal_attachments_is_refused_by_name_today() {
    let _g = rig();
    let fbo = Fbo::new(&[RT0, Rb(GL_RGBA8, 32, 32)], None, None);
    let s = scene(clear_colour);
    drop(fbo);
    assert_refused("clear 64x64 + 32x32", &s);
}

/// The same clear under a scissor: `extra_targets` runs before the clear
/// quad is drawn, so the size refusal is the one named today (after the
/// size change the clear is the several-attachment quad of v3d-mrt-clear,
/// `src/v3d/clear_mrt.rs`, over the intersection; note (f) 5).
#[test]
fn a_scissored_clear_of_unequal_attachments_is_refused_for_its_sizes_today() {
    let _g = rig();
    let fbo = Fbo::new(&[RT0, Rb(GL_RGBA8, 32, 32)], None, None);
    let s = scene(|| {
        // SAFETY: plain GL calls.
        unsafe {
            glEnable(GL_SCISSOR_TEST);
            glScissor(2, 2, 8, 8);
        }
        clear_colour();
    });
    drop(fbo);
    assert_refused("scissored clear 64x64 + 32x32", &s);
}

/* ---- a resize between two draws (note (f) 7) ------------------------------ */

/// Two draws into {64x64, 64x64} with attachment 1 re-stored at 32x48
/// between them, no flush of the test's own: the first draw is one job, the
/// second is refused (its own error, one refused draw), and the first job
/// is what was recorded.
#[test]
fn a_resize_between_two_draws_refuses_the_second_today() {
    let _g = rig();
    let fbo = Fbo::new(&[RT0, RT0], None, None);
    program(2);
    let s = scene(|| {
        triangle();
        fbo.resize(1, SMALL);
        triangle();
    });
    drop(fbo);
    assert_eq!(s.errs, [REFUSAL_ERROR], "{:?}", s.log);
    assert_eq!(s.refused, 1);
    assert_eq!((s.jobs.len(), s.draws.len(), s.clears.len()), (1, 1, 1), "the first draw's job: {:?}", s.log);
    assert_eq!((s.jobs[0].width, s.jobs[0].height), (64, 64));
}

/* ---- expected: the intersection, each RT its own pitch --------------------
 *
 * What the v3d-mrt-sizes change must give (docs/notes/v3d-mrt-sizes.md
 * (e)): the frame is the minimum over the written targets and the depth
 * attachment, RT 1..3 store at their own pitch, a fused clear clears the
 * frame, and a changed intersection is another job. Ignored until then. */

/// Note (f) 1: RT 0 64x64 and RT 1 32x48, both orders. The draw is taken,
/// one job at 32x48 (the intersection), RT 0 at its own stride (64 or 32
/// pixels x 4 bytes) and RT 1 at its own; the GL error is none.
#[test]
#[ignore = "v3d-mrt-sizes: refused today (docs/notes/v3d-mrt-sizes.md (f) 1, (e) 1-2)"]
fn colour_attachments_of_different_sizes_draw_in_their_intersection() {
    let _g = rig();
    for (a, b) in [(RT0, SMALL), (SMALL, RT0)] {
        let fbo = Fbo::new(&[a, b], None, None);
        program(2);
        let s = scene(triangle);
        drop(fbo);
        let j = assert_one_job(&format!("{a:?} + {b:?}"), &s, 2, 1);
        assert_eq!((j.width, j.height), (32, 48), "{a:?} + {b:?}: the intersection");
        assert_eq!(j.stride, stride(a.1, 4), "{a:?} + {b:?}: RT 0's own pitch");
        assert_ne!(j.rt[0].address, 0);
        assert_eq!(j.rt[0].stride, stride(b.1, 4), "{a:?} + {b:?}: RT 1's own pitch");
    }
    /* an ES 3.0 format past the first: its own pitch at its own bytes a
     * pixel (RGBA16F, 8), a typed job */
    let half = Rb(GL_RGBA16F, 32, 48);
    let fbo = Fbo::new(&[RT0, half], None, None);
    program(2);
    let s = scene(triangle);
    drop(fbo);
    let j = assert_one_job("64x64 + RGBA16F 32x48", &s, 2, 1);
    assert!(j.typed);
    assert_eq!((j.width, j.height), (32, 48));
    assert_eq!((j.stride, j.rt[0].stride), (stride(64, 4), stride(32, 8)));
}

/// Note (f) 2: RT 1 wider than RT 0 (96x32 against 64x64): the frame is
/// 64x32 and RT 1's stride is 96 pixels x 4 bytes, not the frame's 64 (the
/// old `target_pitch(w)` of RT 0's width would make its rows walk into the
/// next).
#[test]
#[ignore = "v3d-mrt-sizes: refused today (docs/notes/v3d-mrt-sizes.md (f) 2, (e) 2)"]
fn an_attachment_past_the_first_keeps_its_own_pitch() {
    let _g = rig();
    let fbo = Fbo::new(&[RT0, WIDE], None, None);
    program(2);
    let s = scene(triangle);
    drop(fbo);
    let j = assert_one_job("64x64 + 96x32", &s, 2, 1);
    assert_eq!((j.width, j.height), (64, 32), "the intersection");
    assert_eq!(j.stride, stride(64, 4));
    assert_eq!(j.rt[0].stride, stride(96, 4), "RT 1's own pitch");
}

/// Note (f) 4, (e) 1 and 4: colour 64x64 with a 32x32 depth attachment
/// (`ds_w`/`ds_h` 32x32, `ds_persist`): the frame is 32x32.
#[test]
#[ignore = "v3d-mrt-sizes: the depth size is not read today (docs/notes/v3d-mrt-sizes.md (f) 4, (e) 4)"]
fn the_depth_attachment_limits_the_frame() {
    let _g = rig();
    let fbo = Fbo::new(&[RT0], None, Some(DEPTH_SMALL));
    program(1);
    let s = scene(triangle);
    drop(fbo);
    let j = assert_one_job("64x64 + depth 32x32", &s, 0, 1);
    assert_eq!((j.width, j.height), (32, 32), "the intersection with the depth attachment");
    assert_eq!(j.stride, stride(64, 4), "RT 0's own pitch");
    assert_ne!(s.clears[0].zs_address, 0);
}

/// Note (f) 5: glClear over 64x64 and 32x32 attachments, no scissor, full
/// mask: one job at 32x32, the TLB's clear of both render targets (neither
/// loaded), no quad; RT 0 stored 32x32 (the frame) at its own stride.
#[test]
#[ignore = "v3d-mrt-sizes: refused today (docs/notes/v3d-mrt-sizes.md (f) 5, (d) Clear)"]
fn a_fused_clear_of_unequal_attachments_clears_the_intersection() {
    let _g = rig();
    let fbo = Fbo::new(&[RT0, Rb(GL_RGBA8, 32, 32)], None, None);
    let s = scene(clear_colour);
    drop(fbo);
    let j = assert_one_job("clear 64x64 + 32x32", &s, 2, 0);
    assert_eq!((j.width, j.height), (32, 32), "the intersection");
    assert_eq!((j.stride, j.rt[0].stride), (stride(64, 4), stride(32, 4)));
    let c = &s.clears[0];
    assert!(!c.load_rt0 && !c.load_rt[0], "cleared, not loaded");
    assert_eq!(c.rgba8, c.rgba8_rt[0], "one clear colour for both");
}

/// Note (f) 7, (e) 2: two draws into {64x64, 64x64} with attachment 1
/// re-stored at 32x48 between them: the intersection changed, so the second
/// draw is another job (`g.j.target != target`), at 32x48 with RT 1's new
/// pitch; no error.
#[test]
#[ignore = "v3d-mrt-sizes: refused today (docs/notes/v3d-mrt-sizes.md (f) 7, (e) 2)"]
fn a_job_is_not_continued_across_an_intersection_change() {
    let _g = rig();
    let fbo = Fbo::new(&[RT0, RT0], None, None);
    program(2);
    let s = scene(|| {
        triangle();
        fbo.resize(1, SMALL);
        triangle();
    });
    drop(fbo);
    assert!(s.errs.is_empty() && s.refused == 0, "{:x?} {:?}", s.errs, s.log);
    assert_eq!((s.jobs.len(), s.draws.len(), s.clears.len()), (2, 2, 2), "two jobs: {:?}", s.log);
    assert_eq!((s.jobs[0].width, s.jobs[0].height), (64, 64));
    assert_eq!((s.jobs[1].width, s.jobs[1].height), (32, 48), "the second job: the new intersection");
    assert_eq!(s.jobs[1].rt[0].stride, stride(32, 4), "RT 1's new pitch");
}
