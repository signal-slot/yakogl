// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Depth/stencil beside FOUR colour targets on the V3D, through the real GL
//! core and the real V3D back end, with `backend::HOST_DRAW_TARGETS` standing
//! in for the knob of plan #368 (docs/notes/mrt4-v3d-pvr.md): a clear, a
//! depth-tested draw and the stores of the frame. The job builder is stood
//! in for as in tests/es3_mrt_ds_clear_v3d.rs (the same at three targets): it
//! records the job's target (`rts`, the address of each RT: an RT with an
//! address is stored), the draws (state words) and the `V3dJobClear` the job
//! ends with (clear words, loads, Z/S load and store), so what is held is the
//! back end's choice. No pixel is rasterised; the fragment outputs and the
//! read-back of RT 3 are tests/es3_mrt4_out_v3d.rs's and
//! tests/es3_readback4_v3d.rs's.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_mrt4_ds_v3d
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-v3d", not(feature = "report-es2")))]
use core::ffi::{c_char, c_int, c_void, CStr};
use std::collections::BTreeMap;
use std::ffi::CString;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Mutex, MutexGuard, Once, OnceLock};
use yakogl::gl::consts::*;
use yakogl::gl::context;
use yakogl::gl::draw::*;
use yakogl::gl::es3::*;
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
/* ---- the job builder: records, draws nothing ---- */

#[derive(Clone, Debug)]
#[allow(dead_code)] /* the clear-program fields are tests/es3_int_rt_v3d.rs's; kept so a draw reads whole */
struct SeenDraw {
    st: V3dJobState,
    /// The fragment block: the clear colour as raw words.
    fs_storage: Vec<u32>,
    fs_uniforms: Vec<V3dUniform>,
    count: u32,
}

#[derive(Default)]
struct Seen {
    jobs: Vec<V3dJobTarget>,
    clears: Vec<V3dJobClear>,
    draws: Vec<SeenDraw>,
    log: Vec<String>,
    /// Draws the back end refused (`v3d_refused_draws`) while the scene ran.
    refused: u32,
}

static SEEN: Mutex<Seen> =
    Mutex::new(Seen { jobs: Vec::new(), clears: Vec::new(), draws: Vec::new(), log: Vec::new(), refused: 0 });

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
        let rec = SeenDraw {
            st: *d.st,
            fs_storage: if d.fs_storage.words.is_null() {
                Vec::new()
            } else {
                core::slice::from_raw_parts(d.fs_storage.words, d.fs_storage.count as usize).to_vec()
            },
            fs_uniforms: if (*d.fs).uniforms.is_null() {
                Vec::new()
            } else {
                core::slice::from_raw_parts((*d.fs).uniforms, (*d.fs).num_uniforms as usize).to_vec()
            },
            count: d.count,
        };
        seen(|s| s.draws.push(rec));
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
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("v3d_copy_reasons");
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

/// The one context and back end of the module; the tests take turns and
/// each starts with nothing recorded and nothing pending.
fn rig() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    yakogl::backend::HOST_DRAW_TARGETS.store(4, core::sync::atomic::Ordering::Relaxed);
    ONCE.call_once(|| {
        // SAFETY: statics; the lock serialises every GL call of the module.
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

const W: GLsizei = 24;
const H: GLsizei = 16;

/// One framebuffer of `colours` (draw buffers 0..n) and a depth/stencil
/// renderbuffer of `ds`, `f` run under it, flushed. Returns what the job
/// builder recorded and the GL errors raised.
fn scene(colours: &[GLenum], ds: GLenum, f: impl FnOnce()) -> (Seen, Vec<GLenum>) {
    let (mut fb, mut dsrb) = (0, 0);
    let mut rbs = vec![0; colours.len()];
    // SAFETY: plain GL calls with room for the names each writes.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glGenRenderbuffers(colours.len() as GLsizei, rbs.as_mut_ptr());
        let mut bufs = vec![];
        for (i, &c) in colours.iter().enumerate() {
            glBindRenderbuffer(GL_RENDERBUFFER, rbs[i]);
            glRenderbufferStorage(GL_RENDERBUFFER, c, W, H);
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0 + i as u32, GL_RENDERBUFFER, rbs[i]);
            bufs.push(GL_COLOR_ATTACHMENT0 + i as u32);
        }
        glDrawBuffers(bufs.len() as GLsizei, bufs.as_ptr());
        if ds != 0 {
            glGenRenderbuffers(1, &mut dsrb);
            glBindRenderbuffer(GL_RENDERBUFFER, dsrb);
            glRenderbufferStorage(GL_RENDERBUFFER, ds, W, H);
            if ds == GL_DEPTH24_STENCIL8 {
                glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_DEPTH_STENCIL_ATTACHMENT, GL_RENDERBUFFER, dsrb);
            } else {
                glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_RENDERBUFFER, dsrb);
            }
        }
        let log = seen(|s| s.log.clone());
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "{log:?}");
        glViewport(0, 0, W, H);
        while glGetError() != GL_NO_ERROR {}
        seen(|s| *s = Seen::default());
        let before = yakogl::v3d::backend::v3d_refused_draws();
        f();
        glFinish();
        let refused = yakogl::v3d::backend::v3d_refused_draws().wrapping_sub(before);
        let mut errs = vec![];
        loop {
            let e = glGetError();
            if e == GL_NO_ERROR {
                break;
            }
            errs.push(e);
        }
        glColorMask(1, 1, 1, 1);
        glDepthMask(1);
        glStencilMask(0xff);
        glDisable(GL_SCISSOR_TEST);
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fb);
        glDeleteRenderbuffers(colours.len() as GLsizei, rbs.as_ptr());
        if dsrb != 0 {
            glDeleteRenderbuffers(1, &dsrb);
        }
        let mut s = seen(core::mem::take);
        s.refused = refused;
        (s, errs)
    }
}

/// Four colour attachments of the classes the TLB mixes, beside the packed
/// depth/stencil and the depth-only format.
fn combos() -> Vec<(Vec<GLenum>, GLenum)> {
    vec![
        (vec![GL_RGBA8; 4], GL_DEPTH24_STENCIL8),
        (vec![GL_RGBA8; 4], GL_DEPTH_COMPONENT16),
        (vec![GL_RGBA8, GL_RGBA32UI, GL_RGBA16F, GL_RG16I], GL_DEPTH24_STENCIL8),
        (vec![GL_RGBA16F, GL_RG16I, GL_RGBA8, GL_RGBA8], GL_DEPTH_COMPONENT16),
    ]
}

fn name(cols: &[GLenum], ds: GLenum) -> String {
    format!("{cols:x?} + {ds:#x}")
}

fn has_stencil(ds: GLenum) -> bool {
    ds == GL_DEPTH24_STENCIL8
}

fn clear_all() {
    // SAFETY: plain GL calls.
    unsafe {
        glClearColor(0.25, 0.5, 0.75, 1.0);
        glClearDepthf(0.25);
        glClearStencil(0x5a);
        glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT);
    }
}

/// Every colour RT the job configures is stored: its raster address is set
/// (RT 0 `address`, RT n `rt[n - 1].address`; address 0 is a GL_NONE buffer,
/// never stored).
fn stored_rts(j: &V3dJobTarget) -> usize {
    usize::from(j.address != 0) + j.rt.iter().filter(|r| r.address != 0).count()
}

fn all_loaded(c: &V3dJobClear) -> bool {
    c.load_rt0 && c.load_rt.iter().all(|l| *l)
}

fn none_loaded(c: &V3dJobClear) -> bool {
    !c.load_rt0 && c.load_rt.iter().all(|l| !*l)
}

fn shader(ty: GLenum, src: &str) -> GLuint {
    let c = CString::new(src).unwrap();
    // SAFETY: plain GL calls; the string lives for the call.
    unsafe {
        let s = glCreateShader(ty);
        let srcs = [c.as_ptr()];
        glShaderSource(s, 1, srcs.as_ptr(), core::ptr::null());
        glCompileShader(s);
        let mut ok: GLint = 0;
        glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
        assert_eq!(ok, 1, "compile {ty:#x}: {src}\n{:?}", seen(|s| s.log.clone()));
        s
    }
}

/// The current program: `a_position` at attribute 0 and one `out` per entry
/// of `outs` ("vec4", "ivec4" or "uvec4"), each written a constant.
fn program(outs: &[&str]) -> GLuint {
    let mut fs = String::from("#version 300 es\nprecision highp float;\nprecision highp int;\n");
    for (i, t) in outs.iter().enumerate() {
        fs += &format!("layout(location = {i}) out {t} o{i};\n");
    }
    fs += "void main() {\n";
    for (i, t) in outs.iter().enumerate() {
        fs += &match *t {
            "vec4" => format!("  o{i} = vec4(0.5, 1.0, 0.2, 0.5);\n"),
            "ivec4" => format!("  o{i} = ivec4(1, 2, 3, 4);\n"),
            _ => format!("  o{i} = uvec4(1u, 2u, 3u, 4u);\n"),
        };
    }
    fs += "}\n";
    let v =
        shader(GL_VERTEX_SHADER, "#version 300 es\nin vec4 a_position;\nvoid main() { gl_Position = a_position; }\n");
    let f = shader(GL_FRAGMENT_SHADER, &fs);
    let name = CString::new("a_position").unwrap();
    // SAFETY: plain GL calls on handles just made.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glBindAttribLocation(p, 0, name.as_ptr());
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "link: {:?}", seen(|s| s.log.clone()));
        glUseProgram(p);
        glDeleteShader(v);
        glDeleteShader(f);
        p
    }
}

/// One triangle over the whole target, at depth `z`.
fn draw_triangle(z: f32) {
    static TRI: Mutex<[f32; 12]> = Mutex::new([0.0; 12]);
    let mut t = TRI.lock().unwrap_or_else(|e| e.into_inner());
    *t = [-1.0, -1.0, z, 1.0, 3.0, -1.0, z, 1.0, -1.0, 3.0, z, 1.0];
    // SAFETY: plain GL calls; the array is a static read by the draw call.
    unsafe {
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, t.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glDrawArrays(GL_TRIANGLES, 0, 3);
    }
}

/// The program's outputs for the colours of a combination.
fn outs_of(cols: &[GLenum]) -> Vec<&'static str> {
    cols.iter()
        .map(|&c| match c {
            GL_RGBA32UI => "uvec4",
            GL_RG16I => "ivec4",
            _ => "vec4",
        })
        .collect()
}

/// A depth-tested draw of every output of `cols` under `f`'s state.
fn drawn(cols: &[GLenum], z: f32, f: impl FnOnce()) {
    let p = program(&outs_of(cols));
    f();
    draw_triangle(z);
    // SAFETY: plain GL calls.
    unsafe {
        glDisable(GL_STENCIL_TEST);
        glDisable(GL_DEPTH_TEST);
        glUseProgram(0);
        glDeleteProgram(p);
    }
}

/* ---- glClear of everything: the TLB's ---------------------------------------- */

/// glClear(COLOR|DEPTH|STENCIL) over four colours and the Z/S: one job of
/// four RTs, no quad, every RT (RT 3 is `load_rt[2]`) cleared by the tile
/// buffer, the Z/S clear words the job's, nothing loaded; a job that only
/// cleared stores no Z/S, and every colour RT has its address.
#[test]
fn a_whole_clear_of_four_colours_and_depth_stencil_is_the_tlbs() {
    let _g = rig();
    for (cols, ds) in combos() {
        let w = name(&cols, ds);
        let (s, errs) = scene(&cols, ds, clear_all);
        assert!(errs.is_empty() && s.refused == 0, "{w}: {errs:x?} {:?}", s.log);
        assert_eq!((s.jobs.len(), s.draws.len(), s.clears.len()), (1, 0, 1), "{w}: one job, no quad");
        assert_eq!(s.jobs[0].rts, 4, "{w}: four render targets");
        assert_eq!(stored_rts(&s.jobs[0]), 4, "{w}: four colour RTs have a store address");
        let c = &s.clears[0];
        assert!(none_loaded(c), "{w}: no RT, RT 3 included, starts from a load");
        assert_eq!(c.z, 0.25, "{w}: the Z clear word");
        assert_ne!(c.zs_address, 0, "{w}: the Z/S buffer is the job's");
        if has_stencil(ds) {
            assert_eq!(c.stencil, 0x5a, "{w}: the stencil clear word");
        }
        assert!(!c.zs_load && !c.zs_store, "{w}: cleared, not loaded, not stored");
        assert_eq!((c.s_address, c.s_load, c.s_store), (0, false, false), "{w}: one buffer");
    }
}

/// A Z/S-only glClear beside four colours: the colours are loaded, RT 3 too,
/// the Z/S cleared.
#[test]
fn a_depth_stencil_only_clear_loads_all_four_colours() {
    let _g = rig();
    for (cols, ds) in combos() {
        let w = name(&cols, ds);
        let (s, errs) = scene(&cols, ds, || unsafe {
            glClearDepthf(0.75);
            glClearStencil(3);
            glClear(GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT);
        });
        assert!(errs.is_empty() && s.refused == 0, "{w}: {errs:x?} {:?}", s.log);
        assert_eq!((s.jobs.len(), s.draws.len(), s.clears.len()), (1, 0, 1), "{w}");
        assert!(all_loaded(&s.clears[0]), "{w}: the four colours are loaded");
        assert_eq!(s.clears[0].z, 0.75, "{w}");
    }
}

/* ---- glClearBufferfi with glClearBufferfv(COLOR, 3) -------------------------- */

/// glClearBufferfi(DEPTH_STENCIL) alone, four colours: the TLB's Z/S clear,
/// the colours (RT 3 too) loaded, never cleared.
#[test]
fn a_clear_buffer_fi_beside_four_colours_loads_them() {
    let _g = rig();
    for (cols, ds) in combos().into_iter().filter(|(_, ds)| has_stencil(*ds)) {
        let w = name(&cols, ds);
        let (s, errs) = scene(&cols, ds, || unsafe { glClearBufferfi(GL_DEPTH_STENCIL, 0, 0.5, 7) });
        assert!(errs.is_empty() && s.refused == 0, "{w}: {errs:x?} {:?}", s.log);
        assert_eq!((s.jobs.len(), s.draws.len(), s.clears.len()), (1, 0, 1), "{w}");
        let c = &s.clears[0];
        assert_eq!((c.z, c.stencil), (0.5, 7), "{w}");
        assert!(all_loaded(c), "{w}: the four colours are loaded");
        assert!(!c.zs_load && !c.zs_store, "{w}");
    }
}

/// glClearBufferfi(DEPTH_STENCIL) and glClearBufferfv(COLOR, 3) in one frame,
/// in either order: one job of four RTs and no quad. The Z/S clear is the
/// job's words; RT 3 alone is cleared by the tile buffer (`load_rt[2]` false,
/// its clear word in `rgba8_rt[2]`), RT 0-2 are loaded, never cleared.
#[test]
fn a_clear_buffer_fi_and_a_clear_of_draw_buffer_3_are_one_job_of_tlb_clears() {
    let _g = rig();
    for fi_first in [true, false] {
        for (cols, ds) in combos().into_iter().filter(|(c, ds)| has_stencil(*ds) && c[3] == GL_RGBA8) {
            let w = format!("{} fi first {fi_first}", name(&cols, ds));
            let (s, errs) = scene(&cols, ds, || unsafe {
                let v = [0.1f32, 0.2, 0.3, 0.4];
                if fi_first {
                    glClearBufferfi(GL_DEPTH_STENCIL, 0, 0.5, 7);
                }
                glClearBufferfv(GL_COLOR, 3, v.as_ptr());
                if !fi_first {
                    glClearBufferfi(GL_DEPTH_STENCIL, 0, 0.5, 7);
                }
            });
            assert!(errs.is_empty() && s.refused == 0, "{w}: {errs:x?} {:?}", s.log);
            assert_eq!(s.jobs.len(), 1, "{w}: one job: {:?}", s.log);
            assert_eq!(s.jobs[0].rts, 4, "{w}");
            assert_eq!(s.draws.len(), 0, "{w}: no quad, RT 3 is cleared by the tile buffer");
            let c = &s.clears[0];
            assert_eq!((c.z, c.stencil), (0.5, 7), "{w}: the Z/S clear words");
            assert!(c.load_rt0 && c.load_rt[..2].iter().all(|l| *l), "{w}: RT 0-2 are loaded");
            assert!(!c.load_rt[2], "{w}: RT 3 starts from its clear");
            assert_eq!(c.rgba8_rt[2], 0x664c_331a, "{w}: RT 3's clear colour word (0.1, 0.2, 0.3, 0.4)");
            assert!(!c.zs_load, "{w}");
        }
    }
}

/* ---- a depth-tested draw of four outputs -------------------------------------- */

/// A draw that writes all four outputs with the depth test on (LESS) over a
/// cleared Z/S: one job of four RTs, one draw whose state tests and writes
/// depth and has the full colour mask, so every RT is written exactly where
/// the test passes (the TLB applies the Z test once for the four RTs); the
/// Z/S, cleared in the same job, is stored, and the four colours are stored.
#[test]
fn a_depth_tested_draw_writes_four_colours_where_the_test_passes() {
    let _g = rig();
    for (cols, ds) in combos() {
        let w = name(&cols, ds);
        let (s, errs) = scene(&cols, ds, || {
            clear_all();
            drawn(&cols, -0.5, || unsafe {
                glEnable(GL_DEPTH_TEST);
                glDepthFunc(GL_LESS);
            });
        });
        assert!(errs.is_empty() && s.refused == 0, "{w}: {errs:x?} {:?}", s.log);
        assert_eq!((s.jobs.len(), s.draws.len(), s.clears.len()), (1, 1, 1), "{w}: {:?}", s.log);
        assert_eq!(s.jobs[0].rts, 4, "{w}");
        let d = &s.draws[0];
        assert!(d.st.depth_enabled && d.st.depth_writemask, "{w}: depth tested and written");
        assert_ne!(d.st.depth_func, V3D_COMPARE_FUNC_ALWAYS, "{w}: a real test");
        assert_eq!(d.st.colormask, 0xf, "{w}: every channel of every RT");
        assert!(!d.st.stencil_enabled, "{w}: stencil off");
        let c = &s.clears[0];
        assert!(none_loaded(c), "{w}: the colours were cleared in the job, none loaded");
        assert_eq!(c.z, 0.25, "{w}: the Z clear is the job's");
        assert!(c.zs_store, "{w}: the draw wrote the Z/S, so it is stored");
        assert!(!c.zs_load, "{w}: it was cleared in this job");
        assert_eq!(stored_rts(&s.jobs[0]), 4, "{w}: four colours stored");
    }
}

/// The depth test with the stencil test on: the draw's state carries both.
#[test]
fn a_depth_and_stencil_tested_draw_of_four_colours() {
    let _g = rig();
    let (cols, ds) = (vec![GL_RGBA8; 4], GL_DEPTH24_STENCIL8);
    let (s, errs) = scene(&cols, ds, || {
        clear_all();
        drawn(&cols, 0.0, || unsafe {
            glEnable(GL_DEPTH_TEST);
            glDepthFunc(GL_LEQUAL);
            glEnable(GL_STENCIL_TEST);
            glStencilFunc(GL_EQUAL, 0x5a, 0xff);
            glStencilOp(GL_KEEP, GL_KEEP, GL_INCR);
        });
    });
    assert!(errs.is_empty() && s.refused == 0, "{errs:x?} {:?}", s.log);
    assert_eq!((s.jobs.len(), s.draws.len()), (1, 1), "{:?}", s.log);
    let d = &s.draws[0];
    assert!(d.st.depth_enabled && d.st.stencil_enabled);
    assert_eq!(d.st.colormask, 0xf);
    assert_eq!(s.jobs[0].rts, 4);
    assert!(s.clears[0].zs_store);
}

/// A depth test with a depth mask of 0 leaves the Z untouched: the four
/// colours are stored, the Z/S is not (nothing wrote it).
#[test]
fn a_depth_test_without_a_depth_write_stores_the_colours_only() {
    let _g = rig();
    let (cols, ds) = (vec![GL_RGBA8; 4], GL_DEPTH_COMPONENT16);
    let (s, errs) = scene(&cols, ds, || {
        clear_all();
        drawn(&cols, -0.5, || unsafe {
            glEnable(GL_DEPTH_TEST);
            glDepthMask(0);
        });
    });
    assert!(errs.is_empty() && s.refused == 0, "{errs:x?} {:?}", s.log);
    assert_eq!((s.jobs.len(), s.draws.len()), (1, 1), "{:?}", s.log);
    let d = &s.draws[0];
    assert!(d.st.depth_enabled && !d.st.depth_writemask);
    assert_eq!(stored_rts(&s.jobs[0]), 4);
    assert!(!s.clears[0].zs_store, "no Z write, no Z store");
}

/* ---- the frame's stores --------------------------------------------------------- */

/// A frame in two flushes (clear, finish, draw): the second job loads the four
/// colours (RT 3 too), starts the Z/S from the same clear words
/// (`ZsKnown::Cleared`), and stores four colours and the Z the draw wrote.
#[test]
fn a_second_job_loads_four_colours_and_stores_them_again() {
    let _g = rig();
    for (cols, ds) in combos() {
        let w = name(&cols, ds);
        let (s, errs) = scene(&cols, ds, || {
            clear_all();
            // SAFETY: a plain GL call.
            unsafe { glFinish() };
            drawn(&cols, -0.5, || unsafe {
                glEnable(GL_DEPTH_TEST);
                glDepthFunc(GL_LESS);
            });
        });
        assert!(errs.is_empty() && s.refused == 0, "{w}: {errs:x?} {:?}", s.log);
        assert_eq!((s.jobs.len(), s.draws.len(), s.clears.len()), (2, 1, 2), "{w}: {:?}", s.log);
        assert!(none_loaded(&s.clears[0]), "{w}: job 1 cleared");
        assert!(!s.clears[0].zs_store, "{w}: job 1 wrote no Z");
        let c = &s.clears[1];
        assert_eq!(s.jobs[1].rts, 4, "{w}");
        assert!(all_loaded(c), "{w}: job 2 loads four colours, RT 3 included");
        assert_eq!(c.z, 0.25, "{w}: job 2's Z starts from job 1's clear word");
        assert!(c.zs_store, "{w}: the draw's Z is stored");
        assert_eq!(stored_rts(&s.jobs[1]), 4, "{w}: and the four colours");
    }
}

/// Four colours without a depth/stencil attachment: the same job without a
/// Z/S, four stores.
#[test]
fn four_colours_without_depth_stencil_have_no_z_s_buffer() {
    let _g = rig();
    let cols = vec![GL_RGBA8; 4];
    let (s, errs) = scene(&cols, 0, || unsafe {
        glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
    });
    assert!(errs.is_empty() && s.refused == 0, "{errs:x?} {:?}", s.log);
    assert_eq!((s.jobs.len(), s.draws.len(), s.clears.len()), (1, 0, 1), "{:?}", s.log);
    assert_eq!(s.jobs[0].rts, 4);
    assert_eq!(stored_rts(&s.jobs[0]), 4);
    assert_eq!(s.clears[0].zs_address, 0, "no Z/S buffer");
    assert!(none_loaded(&s.clears[0]));
}
