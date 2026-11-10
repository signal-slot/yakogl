// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! A colour clear beside a depth/stencil clear, over several colour
//! attachments, under a scissor and a colour mask, through the real GL core
//! and the real V3D back end (t-mrt-ds-clear). The job builder is stood in
//! for as in tests/es3_int_rt_v3d.rs: it records the job's target, the draws
//! (the clear quads) and the `V3dJobClear` the job ends with, so what is
//! held is the back end's choice: the TLB clear words, the Z/S clear in
//! `v3d_job_clear`, which buffers are loaded and stored, and the refusal.
//!
//! Pinned as the back end behaves today:
//! - the whole-framebuffer clear (no scissor, full mask) is the TLB's, for
//!   every colour attachment and the Z/S buffer;
//! - a Z/S-only clear, scissored, beside colour attachments is a quad of the
//!   Z/S buffer alone; the colour attachments are loaded, not cleared;
//! - a scissored or masked colour clear of several colour attachments
//!   beside a Z/S clear is one quad: its program writes every colour
//!   attachment in its own class (`src/v3d/clear_mrt.rs`,
//!   `draw_clear_quad`), its state clears the depth and the stencil with it.
//!   Until v3d-mrt-clear the back end refused it by name ('a scissored or
//!   masked colour clear of several colour attachments').
//!
//! Mesa 26.2.1 (third_party, v3dx_draw.c `v3d_clear`, v3dx_rcl.c) is the
//! reference: the Z/S clear value goes into the job's clear Z/S words, and
//! GFXH-1461 (v3dx_draw.c:1676) makes a lone depth or stencil clear beside a
//! loaded other half a drawn quad.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_mrt_ds_clear_v3d
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
use yakogl::gl::es3::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::state::*;
use yakogl::gl::types::*;
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
            fs_storage: core::slice::from_raw_parts(d.fs_storage.words, d.fs_storage.count as usize).to_vec(),
            fs_uniforms: core::slice::from_raw_parts((*d.fs).uniforms, (*d.fs).num_uniforms as usize).to_vec(),
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
/// The (colour attachments, depth/stencil) combinations of the cases: the
/// ES 3.0 integer and float classes beside RGBA8, with the packed
/// depth/stencil and the depth-only format.
fn combos() -> Vec<(Vec<GLenum>, GLenum)> {
    vec![
        (vec![GL_RGBA8, GL_RGBA32UI], GL_DEPTH24_STENCIL8),
        (vec![GL_RGBA16F, GL_RG16I], GL_DEPTH_COMPONENT16),
        (vec![GL_RGBA8, GL_RGBA32UI, GL_RGBA16F], GL_DEPTH24_STENCIL8),
        (vec![GL_RGBA16F, GL_RG16I, GL_RGBA8], GL_DEPTH_COMPONENT16),
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

fn scissor() {
    // SAFETY: plain GL calls.
    unsafe {
        glEnable(GL_SCISSOR_TEST);
        glScissor(3, 2, 9, 7);
    }
}

/// The box of `scissor()` as the quad's state holds it (an FBO's rows).
const BOX: [u32; 4] = [3, 2, 12, 9];

fn quad_box(d: &SeenDraw) -> [u32; 4] {
    [d.st.scissor_minx, d.st.scissor_miny, d.st.scissor_maxx, d.st.scissor_maxy]
}

/* ---- the whole-framebuffer clear: the TLB's -------------------------------- */

/// glClear of COLOR|DEPTH|STENCIL over every attachment, no scissor, full
/// mask: one job, no quad, every colour attachment cleared by the tile
/// buffer (none loaded), the Z/S clear words in the job's clear, and nothing
/// of the Z/S loaded; a job that only cleared stores nothing (`flush_job`:
/// the next one starts from the same clear, `ZsKnown::Cleared`).
#[test]
fn a_whole_clear_of_colour_and_depth_stencil_is_the_tlbs() {
    let _g = rig();
    for (cols, ds) in combos() {
        let w = name(&cols, ds);
        let (s, errs) = scene(&cols, ds, clear_all);
        assert!(errs.is_empty() && s.refused == 0, "{w}: {errs:x?} {:?}", s.log);
        assert_eq!((s.jobs.len(), s.draws.len(), s.clears.len()), (1, 0, 1), "{w}: no quad");
        assert_eq!(s.jobs[0].rts as usize, cols.len(), "{w}");
        let c = &s.clears[0];
        assert!(!c.load_rt0, "{w}: RT 0 starts from the clear");
        for n in 1..cols.len() {
            assert!(!c.load_rt[n - 1], "{w}: RT {n} starts from the clear");
        }
        assert_eq!(c.z, 0.25, "{w}: the Z clear word");
        assert_ne!(c.zs_address, 0, "{w}: the Z/S buffer is the job's");
        if has_stencil(ds) {
            assert_eq!(c.stencil, 0x5a, "{w}: the stencil clear word");
        }
        assert!(!c.zs_load && !c.zs_store, "{w}: cleared, not loaded, not stored");
        assert_eq!((c.s_address, c.s_load, c.s_store), (0, false, false), "{w}: one buffer");
    }
}

/// glClearBufferfi(DEPTH_STENCIL), unscissored, beside the colour
/// attachments: the TLB's Z/S clear, the colours loaded (never cleared).
#[test]
fn a_whole_clear_buffer_fi_is_the_tlbs_and_loads_the_colours() {
    let _g = rig();
    for (cols, ds) in combos().into_iter().filter(|(_, ds)| has_stencil(*ds)) {
        let w = name(&cols, ds);
        let (s, errs) = scene(&cols, ds, || unsafe { glClearBufferfi(GL_DEPTH_STENCIL, 0, 0.5, 7) });
        assert!(errs.is_empty(), "{w}: {errs:x?}");
        assert_eq!((s.jobs.len(), s.draws.len(), s.clears.len()), (1, 0, 1), "{w}");
        let c = &s.clears[0];
        assert_eq!((c.z, c.stencil), (0.5, 7), "{w}");
        assert!(c.load_rt0 && c.load_rt[..cols.len() - 1].iter().all(|l| *l), "{w}: the colours are loaded");
        assert!(!c.zs_load && !c.zs_store, "{w}");
    }
}

/// A partial colour mask with a clear of the Z/S alone: the colour is not
/// being cleared, so no quad and no refusal; the colours are loaded.
#[test]
fn a_colour_mask_does_not_touch_a_depth_stencil_only_clear() {
    let _g = rig();
    for (cols, ds) in combos() {
        let w = name(&cols, ds);
        let (s, errs) = scene(&cols, ds, || unsafe {
            glColorMask(1, 0, 1, 0);
            glClearDepthf(0.75);
            glClear(GL_DEPTH_BUFFER_BIT);
        });
        assert!(errs.is_empty() && s.refused == 0, "{w}: {errs:x?} {:?}", s.log);
        assert_eq!((s.jobs.len(), s.draws.len(), s.clears.len()), (1, 0, 1), "{w}");
        let c = &s.clears[0];
        assert_eq!(c.z, 0.75, "{w}");
        assert!(c.load_rt0 && c.load_rt[..cols.len() - 1].iter().all(|l| *l), "{w}: loaded");
    }
}

/* ---- a scissored Z/S clear beside colour attachments: one Z/S quad --------- */

/// glClear(DEPTH) under a scissor, colour attachments present: the TLB
/// cannot clear a box, so one quad of the depth buffer alone is drawn
/// (colour mask 0, depth test on and writing, stencil untouched), and the
/// colour attachments stay loaded -- no RT starts from a clear. The Z/S
/// buffer is stored (the quad wrote it) and, on a buffer nothing has
/// defined, not loaded.
#[test]
fn a_scissored_depth_clear_is_a_depth_quad_and_the_colours_stay_loaded() {
    let _g = rig();
    for (cols, ds) in combos() {
        let w = name(&cols, ds);
        let (s, errs) = scene(&cols, ds, || unsafe {
            scissor();
            glClearDepthf(0.5);
            glClear(GL_DEPTH_BUFFER_BIT);
        });
        assert!(errs.is_empty() && s.refused == 0, "{w}: {errs:x?} {:?}", s.log);
        assert_eq!((s.jobs.len(), s.draws.len(), s.clears.len()), (1, 1, 1), "{w}");
        let q = &s.draws[0];
        assert_eq!(q.st.colormask, 0, "{w}: no colour written");
        assert!(q.st.depth_enabled && q.st.depth_writemask, "{w}: depth written");
        assert!(!q.st.stencil_enabled, "{w}: stencil untouched");
        assert!(q.st.scissor, "{w}");
        assert_eq!(quad_box(q), BOX, "{w}");
        let c = &s.clears[0];
        assert!(c.load_rt0 && c.load_rt[..cols.len() - 1].iter().all(|l| *l), "{w}: colour loaded, not cleared");
        assert_ne!(c.zs_address, 0, "{w}");
        assert!(c.zs_store, "{w}: the quad wrote it");
        assert!(!c.zs_load, "{w}: a buffer nothing defined yet is not loaded");
    }
}

/// glClearBufferfi(DEPTH_STENCIL) under a scissor beside colour attachments:
/// one quad writing depth and stencil, no colour.
#[test]
fn a_scissored_clear_buffer_fi_is_a_depth_stencil_quad() {
    let _g = rig();
    for (cols, ds) in combos().into_iter().filter(|(_, ds)| has_stencil(*ds)) {
        let w = name(&cols, ds);
        let (s, errs) = scene(&cols, ds, || unsafe {
            scissor();
            glClearBufferfi(GL_DEPTH_STENCIL, 0, 0.5, 7);
        });
        assert!(errs.is_empty() && s.refused == 0, "{w}: {errs:x?} {:?}", s.log);
        assert_eq!((s.jobs.len(), s.draws.len(), s.clears.len()), (1, 1, 1), "{w}");
        let q = &s.draws[0];
        assert_eq!(q.st.colormask, 0, "{w}");
        assert!(q.st.depth_enabled && q.st.depth_writemask && q.st.stencil_enabled, "{w}: depth and stencil");
        assert_eq!(quad_box(q), BOX, "{w}");
        let c = &s.clears[0];
        assert!(c.load_rt0 && c.load_rt[..cols.len() - 1].iter().all(|l| *l), "{w}: colour loaded");
        assert!(c.zs_store, "{w}");
    }
}

/// A scissored stencil-only clear writes the stencil half alone.
#[test]
fn a_scissored_stencil_clear_is_a_stencil_quad() {
    let _g = rig();
    let (cols, ds) = (vec![GL_RGBA8, GL_RGBA32UI], GL_DEPTH24_STENCIL8);
    let (s, errs) = scene(&cols, ds, || unsafe {
        scissor();
        glClearStencil(0x11);
        glClear(GL_STENCIL_BUFFER_BIT);
    });
    assert!(errs.is_empty() && s.refused == 0, "{errs:x?} {:?}", s.log);
    assert_eq!((s.jobs.len(), s.draws.len()), (1, 1));
    let q = &s.draws[0];
    assert_eq!(q.st.colormask, 0);
    assert!(q.st.stencil_enabled && !q.st.depth_writemask, "stencil only");
    assert!(s.clears[0].load_rt0 && s.clears[0].load_rt[0]);
}

/// After a whole clear (job 1: the TLB clears the colours and the Z/S, which
/// stores nothing), a scissored depth clear is job 2: the colours load, the
/// Z/S starts from the same clear words again (`ZsKnown::Cleared`; not
/// loaded), the quad's depth is stored.
#[test]
fn a_scissored_depth_clear_after_a_whole_clear_loads_the_colours() {
    let _g = rig();
    for (cols, ds) in combos() {
        let w = name(&cols, ds);
        let (s, errs) = scene(&cols, ds, || unsafe {
            clear_all();
            glFinish();
            scissor();
            glClearDepthf(0.5);
            glClear(GL_DEPTH_BUFFER_BIT);
        });
        assert!(errs.is_empty() && s.refused == 0, "{w}: {errs:x?} {:?}", s.log);
        assert_eq!((s.jobs.len(), s.draws.len(), s.clears.len()), (2, 1, 2), "{w}");
        assert!(!s.clears[0].load_rt0 && !s.clears[0].zs_store, "{w}: job 1 cleared");
        let c = &s.clears[1];
        assert!(c.load_rt0 && c.load_rt[..cols.len() - 1].iter().all(|l| *l), "{w}: job 2 loads the colours");
        assert_eq!(c.z, 0.25, "{w}: job 2's Z starts from job 1's clear word");
        assert!(!c.zs_load && c.zs_store, "{w}");
    }
}

/* ---- the colour clear of several attachments beside the Z/S clear ---------- */

/// Scissor and mask together on glClear(COLOR) alone (no Z/S in the call):
/// one quad of the colours alone, the colour mask and the scissor box its
/// own, neither depth nor stencil written, the colours loaded.
#[test]
fn a_scissored_and_masked_colour_only_clear_is_one_quad_of_the_colours() {
    let _g = rig();
    for (cols, ds) in combos() {
        let w = name(&cols, ds);
        let (s, errs) = scene(&cols, ds, || unsafe {
            scissor();
            glColorMask(0, 1, 0, 1);
            glClear(GL_COLOR_BUFFER_BIT);
        });
        assert!(errs.is_empty() && s.refused == 0, "{w}: {errs:x?} {:?}", s.log);
        assert_eq!((s.jobs.len(), s.draws.len(), s.clears.len()), (1, 1, 1), "{w}: one job, one quad");
        let q = &s.draws[0];
        assert_eq!((q.st.colormask, q.st.scissor), (0b1010, true), "{w}");
        assert_eq!(quad_box(q), BOX, "{w}");
        assert!(!q.st.depth_enabled && !q.st.stencil_enabled, "{w}: the colours alone");
        let c = &s.clears[0];
        assert!(c.load_rt0 && c.load_rt[..cols.len() - 1].iter().all(|l| *l), "{w}: the colours are loaded");
    }
}

/// After such a clear the context goes on: the next whole clear is the
/// TLB's again, with nothing drawn.
#[test]
fn a_whole_clear_after_a_drawn_one_is_the_tlbs_again() {
    let _g = rig();
    let (cols, ds) = (vec![GL_RGBA8, GL_RGBA32UI], GL_DEPTH24_STENCIL8);
    let (s, errs) = scene(&cols, ds, || unsafe {
        scissor();
        clear_all();
        glFinish();
        glDisable(GL_SCISSOR_TEST);
        seen(|s| *s = Seen::default());
        clear_all();
    });
    assert!(errs.is_empty(), "{errs:x?}");
    assert_eq!((s.jobs.len(), s.draws.len(), s.clears.len()), (1, 0, 1), "{:?}", s.log);
}

/* ---- the drawn result ------------------------------------------------------- */

/// The several-attachment clear as Mesa's st does it (st_cb_clear.c:440-528):
/// one quad that writes every colour attachment (its mask), the colours
/// loaded by the job; no refusal, no error. Scissored, the quad covers the
/// box and clears the depth and the stencil with the colour, and the Z/S is
/// stored; a colour mask alone leaves the Z/S to the TLB (whole tiles, as
/// the one-attachment masked clear does), the quad writing colour only.
fn assert_drawn(w: &str, s: &Seen, errs: &[GLenum], cols: usize, ds: GLenum, mask: u32, scissored: bool) {
    assert!(errs.is_empty() && s.refused == 0, "{w}: {errs:x?} {:?}", s.log);
    assert_eq!((s.jobs.len(), s.draws.len(), s.clears.len()), (1, 1, 1), "{w}: one job, one quad");
    let q = &s.draws[0];
    assert_eq!(q.st.colormask, mask, "{w}");
    assert_eq!(q.st.scissor, scissored, "{w}");
    assert_eq!(s.jobs[0].rts as usize, cols, "{w}");
    let c = &s.clears[0];
    assert!(c.load_rt0 && c.load_rt[..cols - 1].iter().all(|l| *l), "{w}: the colours are loaded");
    if scissored {
        assert!(q.st.depth_enabled && q.st.depth_writemask, "{w}: depth");
        assert_eq!(q.st.stencil_enabled, has_stencil(ds), "{w}: stencil");
        assert!(c.zs_store, "{w}");
    } else {
        assert!(!q.st.depth_enabled && !q.st.stencil_enabled, "{w}: the quad writes colour only");
        assert_eq!(c.z, 0.25, "{w}: the Z clear is the TLB's");
        if has_stencil(ds) {
            assert_eq!(c.stencil, 0x5a, "{w}: the stencil clear is the TLB's");
        }
        assert!(!c.zs_load && !c.zs_store, "{w}: cleared, not loaded, not stored");
    }
}

#[test]
fn a_scissored_clear_of_several_attachments_and_depth_stencil_is_one_quad() {
    let _g = rig();
    for (cols, ds) in combos() {
        let w = name(&cols, ds);
        let (s, errs) = scene(&cols, ds, || {
            scissor();
            clear_all();
        });
        assert_drawn(&w, &s, &errs, cols.len(), ds, 0xf, true);
        assert_eq!(quad_box(&s.draws[0]), BOX, "{w}");
    }
}

#[test]
fn a_masked_clear_of_several_attachments_and_depth_stencil_is_one_quad() {
    let _g = rig();
    for (cols, ds) in combos() {
        let w = name(&cols, ds);
        let (s, errs) = scene(&cols, ds, || unsafe {
            glColorMask(1, 0, 1, 0);
            clear_all();
        });
        assert_drawn(&w, &s, &errs, cols.len(), ds, 0b0101, false);
    }
}

#[test]
fn a_scissored_and_masked_clear_of_several_attachments_and_depth_stencil_is_one_quad() {
    let _g = rig();
    for (cols, ds) in combos() {
        let w = name(&cols, ds);
        let (s, errs) = scene(&cols, ds, || unsafe {
            scissor();
            glColorMask(0, 1, 1, 0);
            clear_all();
        });
        assert_drawn(&w, &s, &errs, cols.len(), ds, 0b0110, true);
    }
}
