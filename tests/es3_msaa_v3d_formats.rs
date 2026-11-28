// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! 4x multisample renderbuffers of every colour-renderable sized format on the
//! V3D, through the GL API, the real GL core and the real V3D back end
//! (`draw_call`) on the host (docs/notes/es3-msaa-renderbuffers.md).
//!
//! Per format: glGetInternalformativ's sample counts, a 4x renderbuffer, its
//! completeness, a draw that reaches the back end with 4 samples and the
//! format's target, a resolve blit into a 1x renderbuffer of the same format
//! and a glReadPixels of the result. As in tests/es3_msaa_v3d_sizes.rs the job
//! builder is stood in for and draws no pixels, so the readback is the 1x
//! renderbuffer's own bytes (zero); the colour values a board would give are
//! BOARD-UNVERIFIED and sit in `#[ignore]`d tests. Integer formats are
//! refused: GL_NUM_SAMPLE_COUNTS 0 and glRenderbufferStorageMultisample with
//! samples > 0 gives GL_INVALID_OPERATION (ES 3.0.6 4.4.2.1).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table,msaa-fbo --test es3_msaa_v3d_formats
//! ```

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
        "#version 300 es\nprecision highp float;\nlayout(location = 0) out vec4 o;\nvoid main() { o = vec4(0.25, 0.5, 0.75, 1.0); }\n",
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

/// The frame the tests use: 40x40, as multisample.fbo_max_samples draws.
const W: GLsizei = 40;
const H: GLsizei = 40;

/// Colour-renderable normalised and float formats of ES 3.0 (floats with
/// GL_EXT_color_buffer_float), with their channel bits (r, g, b, a) and
/// whether a read is 8-bit (`false`) or float.
const COLOUR: &[(&str, GLenum, [u32; 4], bool)] = &[
    ("RGBA8", GL_RGBA8, [8, 8, 8, 8], false),
    ("RGB8", GL_RGB8, [8, 8, 8, 0], false),
    ("RGB565", GL_RGB565, [5, 6, 5, 0], false),
    ("RGBA4", GL_RGBA4, [4, 4, 4, 4], false),
    ("RGB5_A1", GL_RGB5_A1, [5, 5, 5, 1], false),
    ("RGB10_A2", GL_RGB10_A2, [10, 10, 10, 2], false),
    ("SRGB8_ALPHA8", GL_SRGB8_ALPHA8, [8, 8, 8, 8], false),
    ("R8", GL_R8, [8, 0, 0, 0], false),
    ("RG8", GL_RG8, [8, 8, 0, 0], false),
    ("RGBA16F", GL_RGBA16F, [16, 16, 16, 16], true),
    ("RGB16F", GL_RGB16F, [16, 16, 16, 0], true),
    ("RG16F", GL_RG16F, [16, 16, 0, 0], true),
    ("R16F", GL_R16F, [16, 0, 0, 0], true),
    ("RGBA32F", GL_RGBA32F, [32, 32, 32, 32], true),
    ("RG32F", GL_RG32F, [32, 32, 0, 0], true),
    ("R32F", GL_R32F, [32, 0, 0, 0], true),
];

/// ES 3.0 colour formats that are integer (no multisampling in ES 3.0).
const INTEGER: &[(&str, GLenum)] = &[
    ("R8I", GL_R8I),
    ("R8UI", GL_R8UI),
    ("RG8I", GL_RG8I),
    ("RG8UI", GL_RG8UI),
    ("RGBA8I", GL_RGBA8I),
    ("RGBA8UI", GL_RGBA8UI),
    ("R16I", GL_R16I),
    ("R16UI", GL_R16UI),
    ("RGBA16I", GL_RGBA16I),
    ("RGBA16UI", GL_RGBA16UI),
    ("R32I", GL_R32I),
    ("R32UI", GL_R32UI),
    ("RG32I", GL_RG32I),
    ("RG32UI", GL_RG32UI),
    ("RGBA32I", GL_RGBA32I),
    ("RGBA32UI", GL_RGBA32UI),
    ("RGB10_A2UI", GL_RGB10_A2UI),
];

/// glGetInternalformativ(GL_RENDERBUFFER, format, GL_SAMPLES): the counts, or
/// the error of the query (GL_INVALID_ENUM: the GL core takes the name for no
/// renderbuffer format at all).
fn sample_counts(format: GLenum) -> Result<Vec<GLint>, GLenum> {
    let mut n: GLint = -777;
    // SAFETY: plain GL calls; one GLint out, then room for `n` values.
    unsafe {
        while glGetError() != GL_NO_ERROR {}
        glGetInternalformativ(GL_RENDERBUFFER, format, GL_NUM_SAMPLE_COUNTS, 1, &mut n);
        let e = err();
        if e != GL_NO_ERROR {
            return Err(e);
        }
        let mut v = vec![-777; n.max(0) as usize];
        if n > 0 {
            glGetInternalformativ(GL_RENDERBUFFER, format, GL_SAMPLES, n, v.as_mut_ptr());
            assert_eq!(err(), GL_NO_ERROR, "GL_SAMPLES of {format:#x}");
        }
        Ok(v)
    }
}

/// A bound `W`x`H` framebuffer with one renderbuffer of `samples` and
/// `format`; the storage error comes back as `Err`.
fn framebuffer(samples: GLsizei, format: GLenum) -> Result<Fbo, GLenum> {
    let (mut fb, mut rb) = (0, 0);
    // SAFETY: room for one name each; plain GL calls.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        while glGetError() != GL_NO_ERROR {}
        glRenderbufferStorageMultisample(GL_RENDERBUFFER, samples, format, W, H);
        let e = err();
        let f = Fbo { fb, rbs: vec![rb] };
        if e != GL_NO_ERROR {
            return Err(e);
        }
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb);
        glViewport(0, 0, W, H);
        assert_eq!(err(), GL_NO_ERROR);
        Ok(f)
    }
}

fn status() -> GLenum {
    // SAFETY: a plain GL call.
    unsafe { glCheckFramebufferStatus(GL_FRAMEBUFFER) }
}

fn rb_samples(rb: GLuint) -> GLint {
    let mut v: GLint = -777;
    // SAFETY: plain GL calls; one GLint out.
    unsafe {
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glGetRenderbufferParameteriv(GL_RENDERBUFFER, GL_RENDERBUFFER_SAMPLES, &mut v);
    }
    assert_eq!(err(), GL_NO_ERROR);
    v
}

/// Clear to (0.25, 0.5, 0.75, 1) and draw the lower left triangle, flushed;
/// the last application draw the back end saw.
fn clear_and_draw() -> Result<(GlSeen, Seen), GLenum> {
    static TRI: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 1.0, -1.0, 0.0, 1.0, -1.0, 1.0, 0.0, 1.0];
    seen(|s| *s = Seen::default());
    // SAFETY: TRI is static and outlives the draw.
    unsafe {
        glClearColor(0.25, 0.5, 0.75, 1.0);
        glClear(GL_COLOR_BUFFER_BIT);
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, TRI.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glDrawArrays(GL_TRIANGLES, 0, 3);
        glFinish();
    }
    let s = seen(core::mem::take);
    let e = err();
    if e != GL_NO_ERROR {
        return Err(e);
    }
    let d = s.gl.iter().rev().find(|d| d.draws).cloned().expect("the draw reached the back end");
    Ok((d, s))
}

/// Resolve `read` into a fresh 1x framebuffer of `format`, read the pixel at
/// (x, y) as 4 floats (`float`) or 4 bytes; the blit's error and the values
/// as f32 (bytes / 255).
fn resolve_and_read(read: &Fbo, format: GLenum, float: bool, probes: &[(GLint, GLint)]) -> (GLenum, Vec<[f32; 4]>) {
    let dst = framebuffer(0, format).expect("a 1x renderbuffer of the format");
    assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "{format:#x} at 1x");
    // SAFETY: plain GL calls on names this test made.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, read.fb);
        glBindFramebuffer(GL_DRAW_FRAMEBUFFER, dst.fb);
        while glGetError() != GL_NO_ERROR {}
        glBlitFramebuffer(0, 0, W, H, 0, 0, W, H, GL_COLOR_BUFFER_BIT, GL_NEAREST);
    }
    let e = err();
    let mut px = Vec::new();
    // SAFETY: sixteen bytes per read, inside the framebuffer.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, dst.fb);
        for &(x, y) in probes {
            if float {
                let mut p = [-7.0f32; 4];
                glReadPixels(x, y, 1, 1, GL_RGBA, GL_FLOAT, p.as_mut_ptr() as *mut c_void);
                assert_eq!(err(), GL_NO_ERROR, "glReadPixels float at ({x}, {y}) of {format:#x}");
                px.push(p);
            } else {
                let mut p = [0xAAu8; 4];
                glReadPixels(x, y, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, p.as_mut_ptr() as *mut c_void);
                assert_eq!(err(), GL_NO_ERROR, "glReadPixels at ({x}, {y}) of {format:#x}");
                px.push(p.map(|b| b as f32 / 255.0));
            }
        }
    }
    drop(dst);
    (e, px)
}

const PROBES: [(GLint, GLint); 3] = [(1, 1), (W / 2, H / 2), (W - 2, 2)];

/// The formats the GL core reports 4 samples for (with the V3D's capability
/// on), and the ones it reports none for.
fn split() -> (Vec<usize>, Vec<usize>) {
    let (mut on, mut off) = (vec![], vec![]);
    for (i, c) in COLOUR.iter().enumerate() {
        if sample_counts(c.1) == Ok(vec![4]) {
            on.push(i);
        } else {
            off.push(i);
        }
    }
    (on, off)
}

/// The sample counts the GL core answers, per format: 4 for the 8-bit and
/// packed formats and the 16F ones the V3D draws; the names it cannot (the
/// 32F ones, R11F_G11F_B10F) list the 1 a single-sample store keeps, or none.
#[test]
fn sample_counts_per_format() {
    let _g = context();
    for (name, f, _, _) in COLOUR {
        let c = sample_counts(*f);
        assert!(
            matches!(&c, Ok(v) if v == &[4] || v == &[1] || v.is_empty()) || c == Err(GL_INVALID_ENUM),
            "{name}: {c:?}"
        );
    }
    for name in ["RGBA8", "RGB8", "RGB565", "RGBA4", "RGB5_A1", "RGB10_A2", "SRGB8_ALPHA8", "R8", "RG8"] {
        let (_, f, _, _) = COLOUR.iter().find(|c| c.0 == name).unwrap();
        assert_eq!(sample_counts(*f), Ok(vec![4]), "{name}");
    }
}

/// Every integer format: no sample counts and GL_INVALID_OPERATION from
/// glRenderbufferStorageMultisample with 4 samples (ES 3.0.6 4.4.2.1).
#[test]
fn integer_formats_are_refused() {
    let _g = context();
    for (name, f) in INTEGER {
        assert_eq!(sample_counts(*f), Ok(vec![]), "{name}");
        assert_eq!(framebuffer(4, *f).err(), Some(GL_INVALID_OPERATION), "{name}: storage at 4 samples");
        // SAFETY: plain GL calls.
        unsafe { glBindFramebuffer(GL_FRAMEBUFFER, 0) };
        assert_eq!(framebuffer(1, *f).err(), Some(GL_INVALID_OPERATION), "{name}: storage at 1 sample");
        // SAFETY: plain GL calls.
        unsafe { glBindFramebuffer(GL_FRAMEBUFFER, 0) };
    }
}

/// A bound framebuffer of `format` at `samples`, drawn into by
/// `clear_and_draw`: the result of the draw.
fn draw_in(samples: GLsizei, format: GLenum, name: &str) -> (Fbo, Result<(GlSeen, Seen), GLenum>) {
    let fbo = framebuffer(samples, format).unwrap_or_else(|e| panic!("{name}: storage {e:#x}"));
    assert_eq!(rb_samples(fbo.rbs[0]), samples, "{name}");
    assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "{name}");
    /* dropping a framebuffer unbinds the program */
    program();
    let r = clear_and_draw();
    (fbo, r)
}

/// Every format the GL core reports 4x for: stored with 4 samples, complete,
/// the draw reaches the back end at 4 samples on one target (the 1x draw
/// into the same format is accepted too), the job is a 4x job over a sample
/// buffer, and the resolve blit and the readback raise no error.
#[test]
fn draw_and_resolve_per_format() {
    let _g = context();
    let (on, _) = split();
    assert!(on.len() >= 9, "the 8-bit and packed formats at least: {on:?}");
    for i in on {
        let (name, f, _, float) = COLOUR[i];
        let (_one, r1) = draw_in(0, f, name);
        let (d1, _) = r1.unwrap_or_else(|e| panic!("{name}: 1x draw {e:#x}"));
        assert_eq!((d1.samples, d1.rc), (1, 0), "{name}");
        let (fbo, r) = draw_in(4, f, name);
        let (d, s) = r.unwrap_or_else(|e| panic!("{name}: 4x draw {e:#x}"));
        assert_eq!((d.samples, d.targets.len(), d.rc), (4, 1, 0), "{name}: {:?}", s.log);
        assert!(d.id.0 != 0 && d.id.1 != 0, "{name}: {d:?}");
        assert_eq!((d.ids[0], d.gens[0]), d.id, "{name}: target_id is targets[0]'s");
        assert_eq!(s.jobs.len(), 1, "{name}: one job: {:?}", s.log);
        let t = s.jobs[0];
        assert_eq!((t.width, t.height, t.msaa), (W as u32, H as u32, true), "{name}: {t:?}");
        assert!(t.address != 0 && t.ms_address != 0 && t.ms_address % 4096 == 0, "{name}: {t:?}");
        let (e, px) = resolve_and_read(&fbo, f, float, &PROBES);
        assert_eq!(e, GL_NO_ERROR, "{name}: the resolve blit");
        /* nothing is drawn on the host: the 1x target's own bytes, the same everywhere */
        assert_eq!(px.len(), PROBES.len());
        assert!(px.iter().all(|p| p.iter().all(|v| v.is_finite()) && *p == px[0]), "{name}: {px:?}");
    }
}

/// A resolve blit with nothing drawn, per reported format: no error, finite
/// values, the same at every probe (the 1x renderbuffer's own bytes).
#[test]
fn resolve_per_format() {
    let _g = context();
    let (on, _) = split();
    assert!(on.len() >= 9, "the 8-bit and packed formats at least: {on:?}");
    for i in on {
        let (name, f, _, float) = COLOUR[i];
        let fbo = framebuffer(4, f).unwrap_or_else(|e| panic!("{name}: storage {e:#x}"));
        assert_eq!(rb_samples(fbo.rbs[0]), 4, "{name}");
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "{name}");
        let (e, px) = resolve_and_read(&fbo, f, float, &PROBES);
        assert_eq!(e, GL_NO_ERROR, "{name}: the resolve blit");
        assert!(px.iter().all(|p| p.iter().all(|v| v.is_finite()) && *p == px[0]), "{name}: {px:?}");
    }
}

/// The formats the GL core reports no 4x for (the 32F ones: GL_INVALID_ENUM at
/// the query) take 4 samples as an error (never a silent 1x store).
#[test]
fn unreported_formats_are_refused_at_4x() {
    let _g = context();
    let (_, off) = split();
    for i in off {
        let (name, f, _, _) = COLOUR[i];
        let c = sample_counts(f);
        let r = framebuffer(4, f);
        assert!(r.is_err(), "{name}: counts {c:?}: 4 samples must not be stored at 1x");
        // SAFETY: a plain GL call.
        unsafe { glBindFramebuffer(GL_FRAMEBUFFER, 0) };
    }
}

/// The pixel a board gives: the clear colour (0.25, 0.5, 0.75, 1) at the
/// upper right of the frame (outside the triangle), within one step of the
/// format's precision; channels the format lacks read 0 (alpha 1).
#[test]
#[ignore = "V3D (src/v3d, driver/src/v3d/v3d_job.c): the host job builder draws no pixels; board-only (BOARD-UNVERIFIED, no Pi 4 run)"]
fn resolved_clear_colour_per_format() {
    let _g = context();
    program();
    let (on, _) = split();
    for i in on {
        let (name, f, bits, float) = COLOUR[i];
        let (fbo, r) = draw_in(4, f, name);
        r.unwrap_or_else(|e| panic!("{name}: {e:#x}"));
        let (e, px) = resolve_and_read(&fbo, f, float, &[(W - 2, H - 2)]);
        assert_eq!(e, GL_NO_ERROR, "{name}");
        let want = [0.25f32, 0.5, 0.75, 1.0];
        for c in 0..4 {
            let w = if bits[c] == 0 {
                if c == 3 {
                    1.0
                } else {
                    0.0
                }
            } else if f == GL_SRGB8_ALPHA8 && c < 3 {
                /* the clear colour is linear; the stored value is encoded
                 * and a read of an sRGB target is not decoded */
                let l = want[c];
                if l <= 0.003_130_8 {
                    l * 12.92
                } else {
                    1.055 * l.powf(1.0 / 2.4) - 0.055
                }
            } else {
                want[c]
            };
            let tol = if bits[c] >= 16 { 0.002 } else { 1.5 / ((1u32 << bits[c].min(16)) - 1) as f32 };
            assert!((px[0][c] - w).abs() <= tol.max(1.0 / 255.0), "{name}: channel {c} {} vs {w}", px[0][c]);
        }
    }
}
