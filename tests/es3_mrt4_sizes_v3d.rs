// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Four colour attachments of different sizes on the V3D host back end
//! (v3d-sizes4-test), the V3D mirror of pvr-sizes4-test and the four-target
//! extension of tests/es3_mrt_sizes_v3d.rs, through the GL API, the real GL
//! core and the real V3D back end, under the four-target stand-in
//! (`backend::HOST_DRAW_TARGETS` = 4, as tests/es3_rt_masks_v3d.rs). The job
//! builder is stood in for and records each job's `V3dJobTarget` (the frame's
//! `width`/`height`, RT 0's `stride`, RT 1..3's `rt[i].address`/`stride`, RT 3
//! being `rt[2]`), the draws, the `V3dJobClear` and the log.
//!
//! The stand-in GPU runs a job's clear and its draws on the raster stores
//! only: at job end every render target not loaded is cleared with its clear
//! colour and, when the job drew, filled with the colour the test's program
//! writes (`DRAW`, a full-screen triangle, no blend), each inside the job's
//! `width` x `height` at that RT's own address and stride, never past its
//! allocation (an overrun is recorded). So the read-back of attachment 3
//! shows the frame the back end chose: the intersection or not.
//!
//! ES 3.0 4.4.4.2 allows attachments of different sizes and 4.4.1 renders
//! their intersection (docs/notes/v3d-mrt-sizes.md). Today `extra_targets`
//! (src/v3d/backend.rs) refuses a draw whose attachments past the first differ
//! in size from the first, by name, "colour attachments of different sizes",
//! and RT 1..3's stride is RT 0's (`begin_job`). So:
//!
//! - today, pinned: four equal attachments (the baseline: completeness, job,
//!   strides, clear, draw and read-back of attachment 3); every mix with
//!   attachment 3 the smallest, the largest, wider-lower, or all four
//!   different is complete, and the draw and the clear are refused by name
//!   (one GL error, nothing recorded, attachment 3 reads back untouched); a
//!   GL_NONE slot is not a size; the context works after;
//! - expected, `#[ignore]`d until the v3d-mrt-sizes change: the frame is the
//!   intersection, RT 3 (`rt[2]`) keeps its own address and pitch, a fused
//!   clear and a draw fill the common area of attachment 3 and leave the rest
//!   of it alone.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_mrt4_sizes_v3d
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_mrt4_sizes_v3d -- --ignored
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
    /// Stores of the stand-in GPU that would have left an allocation.
    overruns: u32,
}

static SEEN: Mutex<Seen> = Mutex::new(Seen {
    jobs: Vec::new(),
    clears: Vec::new(),
    draws: Vec::new(),
    log: Vec::new(),
    refused: 0,
    errs: Vec::new(),
    overruns: 0,
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

/// The colour the test's program writes to render target `i` (R = 0.2 (i+1),
/// G 0.5, B 0.75, A 1) as RGBA8 bytes: what a draw leaves in the stand-in.
const DRAW: [[u8; 4]; 4] = [[51, 128, 191, 255], [102, 128, 191, 255], [153, 128, 191, 255], [204, 128, 191, 255]];

/// Fill `w` x `h` pixels of the raster store at GPU address `addr`, `stride`
/// bytes a row, with `px`; a store that would leave its allocation is not
/// written and counted in `Seen::overruns`.
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
        seen(|s| s.clears.push(*c));
        (*(*j).bcl).used += 1;
        (*(*j).rcl).used = 200;
        ((*j).t, *c, (*j).draws > 0)
    };
    /* RGBA8 targets only (this file's formats): RT 0 then rt[0..3] */
    let rts = t.rts.max(1) as usize;
    for (i, draw) in DRAW.iter().enumerate().take(rts.min(4)) {
        let (addr, stride, load, word) = if i == 0 {
            (t.address, t.stride, c.load_rt0, c.rgba8)
        } else {
            (t.rt[i - 1].address, t.rt[i - 1].stride, c.load_rt[i - 1], c.rgba8_rt[i - 1])
        };
        if !load {
            fill(addr, stride, t.width, t.height, word.to_le_bytes());
        }
        if drew {
            fill(addr, stride, t.width, t.height, *draw);
        }
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
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("v3d_mrt4_sizes");
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

/* ---- the context ---- */

static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

/// The one context and back end of the file; the tests take turns and each
/// starts with nothing recorded and nothing pending.
fn rig() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    yakogl::backend::HOST_DRAW_TARGETS.store(4, core::sync::atomic::Ordering::Relaxed);
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

/// A renderbuffer: its sized format, width and height (RGBA8 throughout).
#[derive(Clone, Copy, Debug)]
struct Rb(GLenum, GLsizei, GLsizei);

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
        fs += &format!("o{i} = vec4({:.1}, 0.5, 0.75, 1.0);\n", 0.2 * (i + 1) as f32);
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

/// The four attachment sizes the mixes are made of (RGBA8).
const A: Rb = Rb(GL_RGBA8, 64, 64);
const B: Rb = Rb(GL_RGBA8, 48, 56);
const C: Rb = Rb(GL_RGBA8, 32, 48);
/// Wider than A and lower: neither axis is the smaller.
const D: Rb = Rb(GL_RGBA8, 96, 32);
const E: Rb = Rb(GL_RGBA8, 80, 40);

/// A mix of four attachments and its name.
struct Mix {
    name: &'static str,
    rbs: [Rb; 4],
}

impl Mix {
    /// The intersection: the smallest width and the smallest height.
    fn common(&self) -> (u32, u32) {
        let w = self.rbs.iter().map(|r| r.1).min().unwrap();
        let h = self.rbs.iter().map(|r| r.2).min().unwrap();
        (w as u32, h as u32)
    }
}

/// Attachment 3 the smallest: 64x64 x3 and 32x48. Common 32x48.
const SMALLEST3: Mix = Mix { name: "attachment 3 smallest", rbs: [A, A, A, C] };
/// Attachment 3 the largest: 32x48 x3 and 64x64. Common 32x48.
const LARGEST3: Mix = Mix { name: "attachment 3 largest", rbs: [C, C, C, A] };
/// Attachment 3 wider and lower than the rest: common 64x32.
const WIDER3: Mix = Mix { name: "attachment 3 wider and lower", rbs: [A, A, A, D] };
/// All four different, attachment 3 neither smallest nor largest. Common
/// 32x40: the width from attachment 2, the height from attachment 3.
const ALL_DIFFERENT: Mix = Mix { name: "all four different", rbs: [A, B, C, E] };
const MIXES: [&Mix; 4] = [&SMALLEST3, &LARGEST3, &WIDER3, &ALL_DIFFERENT];

/// The refusal's name, as `extra_targets` words it.
const REFUSAL: &str = "colour attachments of different sizes";

/// The error this V3D refusal reaches the application as (`refuse_unsupported`
/// answers -ENOTSUP, `submit_draw` maps it to GL_INVALID_OPERATION).
const REFUSAL_ERROR: GLenum = GL_INVALID_OPERATION;

/// What `glClearColor(0.25, 0.5, 0.75, 1)` leaves in an RGBA8 store.
const CLEARED: [u8; 4] = [64, 128, 191, 255];

/// The raster stride of a `w`-wide render target at `bpp` bytes a pixel.
fn stride(w: GLsizei, bpp: u32) -> u32 {
    target_pitch(w as u32) * bpp
}

/// Refused as the application sees it today: one error, one refused draw,
/// nothing recorded; a "draw refused" log line, when there is one, names
/// this refusal.
fn assert_refused(what: &str, s: &Seen) {
    assert_eq!(s.errs, [REFUSAL_ERROR], "{what}: {:?}", s.log);
    assert_eq!(s.refused, 1, "{what}: one refused draw");
    assert!(s.jobs.is_empty() && s.draws.is_empty() && s.clears.is_empty(), "{what}: nothing recorded");
    for l in s.log.iter().filter(|l| l.contains("draw refused")) {
        assert!(l.ends_with(REFUSAL), "{what}: {l}");
    }
}

/// Taken: no error, no refusal, no overrun, one job of `rts` render targets
/// with `draws` draws, ended once. The job.
fn assert_one_job(what: &str, s: &Seen, rts: u32, draws: usize) -> V3dJobTarget {
    assert!(s.errs.is_empty() && s.refused == 0, "{what}: {:x?} {:?}", s.errs, s.log);
    assert_eq!(s.overruns, 0, "{what}: a store left its allocation");
    assert_eq!((s.jobs.len(), s.draws.len(), s.clears.len()), (1, draws, 1), "{what}: one job: {:?}", s.log);
    assert_eq!(s.jobs[0].rts, rts, "{what}: render targets");
    s.jobs[0]
}

/// Colour attachment `i` of the bound framebuffer, `w` x `h` RGBA8 pixels
/// (its own size), read through glReadPixels.
fn read_attachment(i: u32, w: u32, h: u32) -> Vec<u8> {
    let mut px = vec![0xEEu8; (w * h * 4) as usize];
    // SAFETY: plain GL calls; `px` holds `w` x `h` RGBA8 pixels.
    unsafe {
        glReadBuffer(GL_COLOR_ATTACHMENT0 + i);
        glReadPixels(0, 0, w as GLsizei, h as GLsizei, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr() as *mut c_void);
        assert_eq!(glGetError(), GL_NO_ERROR, "read attachment {i}");
    }
    px
}

/// `px` (`w` pixels a row) is `fill` in the top-left `fw` x `fh` pixels and
/// zero elsewhere: the common area filled, the rest of the attachment
/// untouched.
fn assert_filled(what: &str, px: &[u8], w: u32, fw: u32, fh: u32, fill: [u8; 4]) {
    for (n, p) in px.chunks(4).enumerate() {
        let (x, y) = (n as u32 % w, n as u32 / w);
        let want = if x < fw && y < fh { fill } else { [0; 4] };
        assert_eq!(p, want, "{what}: pixel ({x}, {y})");
    }
}

/* ---- four equal attachments: the baseline ---------------------------------- */

/// Four 64x64 RGBA8 attachments are complete and drawn today: one job at
/// 64x64 with four render targets, RT 0's stride and RT 1..3's all 64 pixels
/// x 4 bytes, each at its own address (RT 3 is `rt[2]`); attachment 3 reads
/// back what the draw wrote.
#[test]
fn four_equal_attachments_draw_at_their_size() {
    let _g = rig();
    let fbo = Fbo::new(&[A, A, A, A], None, None);
    program(4);
    let s = scene(triangle);
    let j = assert_one_job("4 x 64x64", &s, 4, 1);
    assert_eq!((j.width, j.height), (64, 64));
    assert_eq!(j.stride, stride(64, 4));
    for (n, rt) in j.rt.iter().enumerate().take(3) {
        assert_ne!(rt.address, 0, "RT {} has its address", n + 1);
        assert_eq!(rt.stride, stride(64, 4), "RT {}'s stride", n + 1);
    }
    assert!(j.address != j.rt[0].address && j.rt[0].address != j.rt[1].address && j.rt[1].address != j.rt[2].address);
    assert_filled("draw", &read_attachment(3, 64, 64), 64, 64, 64, DRAW[3]);
    assert_filled("draw, attachment 0", &read_attachment(0, 64, 64), 64, 64, 64, DRAW[0]);
    drop(fbo);
}

/// glClear over four equal attachments is one fused TLB clear of four render
/// targets (no draw, none loaded), and attachment 3 reads back the colour.
#[test]
fn a_clear_of_four_equal_attachments_clears_attachment_3() {
    let _g = rig();
    let fbo = Fbo::new(&[A, A, A, A], None, None);
    let s = scene(clear_colour);
    let j = assert_one_job("clear 4 x 64x64", &s, 4, 0);
    assert_eq!((j.width, j.height), (64, 64));
    let c = &s.clears[0];
    assert!(!c.load_rt0 && c.load_rt == [false; 3], "cleared, not loaded");
    assert_eq!(c.rgba8.to_le_bytes(), CLEARED);
    assert_eq!(c.rgba8_rt[2], c.rgba8, "RT 3 takes the same colour");
    assert_filled("clear", &read_attachment(3, 64, 64), 64, 64, 64, CLEARED);
    drop(fbo);
}

/* ---- completeness and today's refusal, every mix --------------------------- */

/// Every mix is a complete framebuffer (ES 3.0 4.4.4.2: sizes may differ;
/// `Fbo::new` asserts GL_FRAMEBUFFER_COMPLETE and no error), and the GL
/// core still reports GL_MAX_DRAW_BUFFERS 4 beside it.
#[test]
fn every_mix_of_four_sizes_is_complete() {
    let _g = rig();
    for m in MIXES {
        let fbo = Fbo::new(&m.rbs, None, None);
        let mut n: GLint = 0;
        // SAFETY: plain GL call, one GLint out.
        unsafe { glGetIntegerv(GL_MAX_DRAW_BUFFERS, &mut n) };
        assert_eq!(n, 4, "{}", m.name);
        drop(fbo);
    }
}

/// The draw into each mix is refused by name today, one GL error and nothing
/// recorded, and attachment 3 reads back untouched (all zero).
#[test]
fn a_draw_into_each_mix_is_refused_by_name_today() {
    let _g = rig();
    for m in MIXES {
        let fbo = Fbo::new(&m.rbs, None, None);
        program(4);
        let s = scene(triangle);
        assert_refused(m.name, &s);
        let r = m.rbs[3];
        assert_filled(m.name, &read_attachment(3, r.1 as u32, r.2 as u32), r.1 as u32, 0, 0, [0; 4]);
        drop(fbo);
    }
}

/// The fused glClear over each mix is refused by the same name, attachment 3
/// untouched.
#[test]
fn a_clear_of_each_mix_is_refused_by_name_today() {
    let _g = rig();
    for m in MIXES {
        let fbo = Fbo::new(&m.rbs, None, None);
        let s = scene(clear_colour);
        assert_refused(m.name, &s);
        let r = m.rbs[3];
        assert_filled(m.name, &read_attachment(3, r.1 as u32, r.2 as u32), r.1 as u32, 0, 0, [0; 4]);
        drop(fbo);
    }
}

/// The scissored clear reaches `extra_targets` before the clear quad, so it
/// is refused for the sizes too.
#[test]
fn a_scissored_clear_of_each_mix_is_refused_by_name_today() {
    let _g = rig();
    for m in MIXES {
        let fbo = Fbo::new(&m.rbs, None, None);
        let s = scene(|| {
            // SAFETY: plain GL calls.
            unsafe {
                glEnable(GL_SCISSOR_TEST);
                glScissor(2, 2, 8, 8);
            }
            clear_colour();
        });
        assert_refused(m.name, &s);
        drop(fbo);
    }
}

/// After the refusal the context stays usable: four equal attachments on the
/// next framebuffer draw as one job.
#[test]
fn the_context_works_after_the_refusal() {
    let _g = rig();
    let fbo = Fbo::new(&SMALLEST3.rbs, None, None);
    program(4);
    let s = scene(triangle);
    assert_refused(SMALLEST3.name, &s);
    drop(fbo);
    let fbo = Fbo::new(&[A, A, A, A], None, None);
    program(4);
    let s = scene(triangle);
    let j = assert_one_job("4 x 64x64 after the refusal", &s, 4, 1);
    assert_eq!((j.width, j.height), (64, 64));
    drop(fbo);
}

/* ---- which attachments count ------------------------------------------------ */

/// Attachment 3 is 32x48 against three 64x64, but only draw buffers 0..2 are
/// named: attachment 3 is not written, so its size is not a size. Three
/// render targets, the frame 64x64.
#[test]
fn an_attachment_that_is_not_a_draw_buffer_is_not_a_size() {
    let _g = rig();
    let bufs = [GL_COLOR_ATTACHMENT0, GL_COLOR_ATTACHMENT0 + 1, GL_COLOR_ATTACHMENT0 + 2];
    let fbo = Fbo::new(&SMALLEST3.rbs, Some(&bufs), None);
    program(3);
    let s = scene(triangle);
    let j = assert_one_job("64x64 x3 + unnamed 32x48", &s, 3, 1);
    assert_eq!((j.width, j.height), (64, 64));
    assert_filled("unnamed attachment 3", &read_attachment(3, 32, 48), 32, 0, 0, [0; 4]);
    drop(fbo);
}

/// Attachment 2 is 32x48 but draw buffer 2 is GL_NONE; attachment 3 is
/// 64x64: the GL_NONE slot is no memory and no size, four render targets,
/// RT 2 configured with address 0 and stride 0, RT 3 at its own address and
/// the frame's stride; attachment 3 reads back the draw.
#[test]
fn a_gl_none_slot_beside_attachment_3_is_not_a_size() {
    let _g = rig();
    let bufs = [GL_COLOR_ATTACHMENT0, GL_COLOR_ATTACHMENT0 + 1, GL_NONE, GL_COLOR_ATTACHMENT0 + 3];
    let fbo = Fbo::new(&[A, A, C, A], Some(&bufs), None);
    program(4);
    let s = scene(triangle);
    let j = assert_one_job("64x64 x2 + (none 32x48) + 64x64", &s, 4, 1);
    assert_eq!((j.width, j.height), (64, 64));
    assert_eq!((j.rt[1].address, j.rt[1].stride), (0, 0), "the GL_NONE slot: configured, never stored");
    assert_ne!(j.rt[2].address, 0, "RT 3 has its address");
    assert_eq!(j.rt[2].stride, stride(64, 4));
    assert_filled("RT 3", &read_attachment(3, 64, 64), 64, 64, 64, DRAW[3]);
    drop(fbo);
}

/// The same framebuffer with draw buffer 2 named: the 32x48 attachment is a
/// size again and the draw is refused.
#[test]
fn naming_the_small_slot_makes_its_size_count_today() {
    let _g = rig();
    let fbo = Fbo::new(&[A, A, C, A], None, None);
    program(4);
    let s = scene(triangle);
    assert_refused("64x64 x2 + 32x48 + 64x64", &s);
    drop(fbo);
}

/// Two draws into four 64x64 attachments with attachment 3 re-stored at 32x48
/// between them: the first draw is one job, the second is refused, and the
/// first job is what was recorded.
#[test]
fn a_resize_of_attachment_3_between_two_draws_refuses_the_second_today() {
    let _g = rig();
    let fbo = Fbo::new(&[A, A, A, A], None, None);
    program(4);
    let s = scene(|| {
        triangle();
        fbo.resize(3, C);
        triangle();
    });
    drop(fbo);
    assert_eq!(s.errs, [REFUSAL_ERROR], "{:?}", s.log);
    assert_eq!(s.refused, 1);
    assert_eq!((s.jobs.len(), s.draws.len(), s.clears.len()), (1, 1, 1), "the first draw's job: {:?}", s.log);
    assert_eq!((s.jobs[0].width, s.jobs[0].height), (64, 64));
}

/* ---- expected: the intersection, RT 3 its own address and pitch ------------
 *
 * What the v3d-mrt-sizes change must give (docs/notes/v3d-mrt-sizes.md (e))
 * for four targets. Ignored until then. */

/// The draw into each mix is one job of four render targets at the
/// intersection, each RT at its own pitch (RT 3 is `rt[2]`), no store past an
/// allocation; attachment 3 reads back the draw inside the common area and
/// is untouched outside it.
#[test]
#[ignore = "v3d-mrt-sizes: refused today (docs/notes/v3d-mrt-sizes.md (f) 1, (e) 1-2)"]
fn a_draw_into_each_mix_fills_the_intersection() {
    let _g = rig();
    for m in MIXES {
        let fbo = Fbo::new(&m.rbs, None, None);
        program(4);
        let s = scene(triangle);
        let j = assert_one_job(m.name, &s, 4, 1);
        let (cw, ch) = m.common();
        assert_eq!((j.width, j.height), (cw, ch), "{}: the intersection", m.name);
        assert_eq!(j.stride, stride(m.rbs[0].1, 4), "{}: RT 0's own pitch", m.name);
        for n in 0..3 {
            assert_ne!(j.rt[n].address, 0, "{}: RT {}", m.name, n + 1);
            assert_eq!(j.rt[n].stride, stride(m.rbs[n + 1].1, 4), "{}: RT {}'s own pitch", m.name, n + 1);
        }
        for (i, r) in m.rbs.iter().enumerate() {
            let px = read_attachment(i as u32, r.1 as u32, r.2 as u32);
            assert_filled(&format!("{} attachment {i}", m.name), &px, r.1 as u32, cw, ch, DRAW[i]);
        }
        drop(fbo);
    }
}

/// The fused glClear over each mix is one job of four render targets at the
/// intersection, none loaded, no quad; attachment 3 reads back the clear
/// colour inside the common area and is untouched outside it.
#[test]
#[ignore = "v3d-mrt-sizes: refused today (docs/notes/v3d-mrt-sizes.md (f) 5, (d) Clear)"]
fn a_fused_clear_of_each_mix_clears_the_intersection() {
    let _g = rig();
    for m in MIXES {
        let fbo = Fbo::new(&m.rbs, None, None);
        let s = scene(clear_colour);
        let j = assert_one_job(m.name, &s, 4, 0);
        let (cw, ch) = m.common();
        assert_eq!((j.width, j.height), (cw, ch), "{}: the intersection", m.name);
        assert_eq!(j.rt[2].stride, stride(m.rbs[3].1, 4), "{}: RT 3's own pitch", m.name);
        let c = &s.clears[0];
        assert!(!c.load_rt0 && c.load_rt == [false; 3], "{}: cleared, not loaded", m.name);
        let r = m.rbs[3];
        let px = read_attachment(3, r.1 as u32, r.2 as u32);
        assert_filled(m.name, &px, r.1 as u32, cw, ch, CLEARED);
        drop(fbo);
    }
}

/// Two draws into four 64x64 attachments with attachment 3 re-stored at 32x48
/// between them: the intersection changed, so the second draw is another job
/// at 32x48 with RT 3's new pitch; no error.
#[test]
#[ignore = "v3d-mrt-sizes: refused today (docs/notes/v3d-mrt-sizes.md (f) 7, (e) 2)"]
fn a_job_is_not_continued_across_a_resize_of_attachment_3() {
    let _g = rig();
    let fbo = Fbo::new(&[A, A, A, A], None, None);
    program(4);
    let s = scene(|| {
        triangle();
        fbo.resize(3, C);
        triangle();
    });
    drop(fbo);
    assert!(s.errs.is_empty() && s.refused == 0, "{:x?} {:?}", s.errs, s.log);
    assert_eq!((s.jobs.len(), s.draws.len(), s.clears.len()), (2, 2, 2), "two jobs: {:?}", s.log);
    assert_eq!((s.jobs[0].width, s.jobs[0].height), (64, 64));
    assert_eq!((s.jobs[1].width, s.jobs[1].height), (32, 48), "the second job: the new intersection");
    assert_eq!(s.jobs[1].rt[2].stride, stride(32, 4), "RT 3's new pitch");
}
