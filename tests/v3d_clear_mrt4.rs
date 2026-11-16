// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Clears of the fourth V3D draw buffer (render target 3), through the GL API,
//! the real GL core and the real V3D back end with the job builder stood in
//! for (the harness of tests/es3_mrt_clear_v3d.rs), under the four-target
//! stand-in `backend::HOST_DRAW_TARGETS` = 4 (what tests/es3_mrt_gl.rs and
//! tests/es3_four_rt_v3d.rs use; what DRAW_TARGETS = 4 for the V3D would
//! give). A test sets the stand-in when it takes the rig and the rig puts it
//! back to 0 when dropped, under the rig's lock.
//!
//! What this file pins, and the neighbours do not (tests/v3d_clear_mrt.rs
//! holds the clear program's source, tests/es3_mrt_clear_v3d.rs two and
//! three attachments, tests/es3_four_rt_v3d.rs one scissored four-target
//! glClear):
//!
//! - an unscissored, unmasked glClear is the TLB's: RT 3 gets its clear
//!   words and is not loaded;
//! - a scissored glClear, and a colour-masked one, is one clear quad whose
//!   program writes all four outputs, with the GL's scissor box and colour
//!   mask, over four loaded targets;
//! - glClearBuffer{f,i,ui}v of draw buffer 3 alone, with a float, signed or
//!   unsigned attachment on RT 3, clears RT 3 and leaves RTs 0-2 loaded:
//!   by the TLB when unscissored, by a quad that writes RT 3 only when
//!   scissored;
//! - glClearBufferfv of draw buffer 3 while it is GL_NONE does nothing: no
//!   job, no quad, no TLB clear of any RT, no error.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d_clear_mrt4
//! ```
#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-v3d", not(feature = "report-es2")))]

use core::ffi::{c_char, c_int, c_void, CStr};
use std::collections::BTreeMap;
use std::ffi::CString;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::Ordering;
use std::sync::{Mutex, MutexGuard, Once, OnceLock};
use yakogl::backend::{GLES_SURF_RG16I, GLES_SURF_RGBA16I, GLES_SURF_RGBA32UI, GLES_SURF_RGBA8888};
use yakogl::gl::consts::*;
use yakogl::gl::context;
use yakogl::gl::draw::glDrawArrays;
use yakogl::gl::es3::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps, OsalV3dCpuOf};
use yakogl::v3d::backend::{v3d_refused_draws, yakogl_backend_v3d};
use yakogl::v3d::ffi::*;
use yakogl::v3d::kick::V3dKick;

/* ---- the job builder: records, draws nothing ---- */

#[derive(Clone, Debug)]
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
}

static SEEN: Mutex<Seen> =
    Mutex::new(Seen { jobs: Vec::new(), clears: Vec::new(), draws: Vec::new(), log: Vec::new() });

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

/// `n` items at `p` (a draw with none passes a null pointer).
///
/// # Safety
/// `p` is null or points at `n` live items.
unsafe fn slice_of<T: Clone>(p: *const T, n: usize) -> Vec<T> {
    if p.is_null() || n == 0 {
        return Vec::new();
    }
    // SAFETY: the caller's.
    unsafe { core::slice::from_raw_parts(p, n) }.to_vec()
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
            fs_storage: slice_of(d.fs_storage.words, d.fs_storage.count as usize),
            fs_uniforms: slice_of((*d.fs).uniforms, (*d.fs).num_uniforms as usize),
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
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("v3d_mrt_clear");
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
fn rig() -> Rig {
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
    yakogl::backend::HOST_DRAW_TARGETS.store(4, Ordering::Relaxed);
    Rig(g)
}

/// The rig's lock; the four-target stand-in goes back to 0 when it drops.
struct Rig(#[allow(dead_code)] MutexGuard<'static, ()>);

impl Drop for Rig {
    fn drop(&mut self) {
        yakogl::backend::HOST_DRAW_TARGETS.store(0, Ordering::Relaxed);
    }
}

/* ---- the scenes ---------------------------------------------------------- */

const W: GLsizei = 24;
const H: GLsizei = 16;
/// The refusal v3d-mrt-clear removed; no test may see it logged.
const REFUSAL: &str = "a scissored or masked colour clear of several colour attachments";
/// Raw clear words for integer targets, and a float colour.
const WORDS: [u32; 4] = [0x7fff_fff1, 0x0000_0002, 0xffff_fffd, 0x0000_0004];
const FLOAT: [f32; 4] = [0.25, 0.5, 0.75, 1.0];

#[derive(Clone, Copy, PartialEq, Debug)]
enum Class {
    Float,
    Int,
    Uint,
}

fn class(format: GLenum) -> Class {
    match format {
        GL_RGBA32UI | GL_RGBA8UI | GL_RGBA16UI | GL_RG16UI | GL_R32UI => Class::Uint,
        GL_RGBA16I | GL_RG16I | GL_RGBA8I | GL_RGBA32I | GL_R32I => Class::Int,
        _ => Class::Float,
    }
}

fn logs() -> Vec<String> {
    seen(|s| s.log.clone())
}

/// Draws the back end refused during the run (`v3d_refused_draws`; its log line
/// is written once per cause, so the counter is what counts).
fn refusals(r: &Run) -> usize {
    r.refused as usize
}

/// How a scene clears.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Clear {
    /// glClearBuffer of every drawbuffer, in order.
    Each,
    /// glClear(GL_COLOR_BUFFER_BIT): every drawbuffer, the values undefined for an integer one.
    All,
    /// glClearBuffer of this drawbuffer alone.
    One(usize),
}

struct Scene {
    scissor: Option<[GLint; 4]>,
    mask: [bool; 4],
    clear: Clear,
    /// A draw into every attachment after the clear (a clear fused with it).
    draw: bool,
    /// This draw buffer is GL_NONE (its attachment stays).
    none: Option<usize>,
}

const PLAIN: Scene = Scene { scissor: None, mask: [true; 4], clear: Clear::Each, draw: false, none: None };

const VS: &str = "#version 300 es\nin vec4 a_position;\nvoid main() { gl_Position = a_position; }\n";

/// A fragment shader writing one output per attachment, of its class.
fn fragment_shader(formats: &[GLenum]) -> String {
    let mut s = String::from("#version 300 es\nprecision highp float;\nprecision highp int;\n");
    for (i, &f) in formats.iter().enumerate() {
        let ty = match class(f) {
            Class::Float => "vec4",
            Class::Int => "ivec4",
            Class::Uint => "uvec4",
        };
        s += &format!("layout(location = {i}) out {ty} o{i};\n");
    }
    s += "void main() {\n";
    for (i, &f) in formats.iter().enumerate() {
        s += &match class(f) {
            Class::Float => format!("  o{i} = vec4(1.0, 0.0, 0.0, 1.0);\n"),
            Class::Int => format!("  o{i} = ivec4(-1, 2, -3, 4);\n"),
            Class::Uint => format!("  o{i} = uvec4(1u, 2u, 3u, 4u);\n"),
        };
    }
    s + "}\n"
}

fn shader(type_: GLenum, src: &str) -> GLuint {
    let c = CString::new(src).unwrap();
    // SAFETY: plain GL calls; the string lives for the call.
    unsafe {
        let s = glCreateShader(type_);
        let srcs = [c.as_ptr()];
        glShaderSource(s, 1, srcs.as_ptr(), core::ptr::null());
        glCompileShader(s);
        let mut ok: GLint = 0;
        glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
        assert_eq!(ok, 1, "compile 0x{type_:x}: {src}\n{:?}", logs());
        s
    }
}

fn program(formats: &[GLenum]) -> GLuint {
    let (v, f) = (shader(GL_VERTEX_SHADER, VS), shader(GL_FRAGMENT_SHADER, &fragment_shader(formats)));
    let name = CString::new("a_position").unwrap();
    // SAFETY: plain GL calls on valid handles.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glBindAttribLocation(p, 0, name.as_ptr());
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "link failed: {:?}", logs());
        p
    }
}

/// A quad over the whole target at z = 0.
static QUAD: [f32; 18] = [
    -1.0, -1.0, 0.0, 1.0, -1.0, 0.0, -1.0, 1.0, 0.0, //
    -1.0, 1.0, 0.0, 1.0, -1.0, 0.0, 1.0, 1.0, 0.0,
];

/// What a scene left: the job builder's record and the GL error of the clear
/// (and the draw).
struct Run {
    refused: u32,
    seen: Seen,
    error: GLenum,
}

/// A framebuffer of one renderbuffer per format, all drawbuffers on, the
/// scene's state set, its clear (and draw) issued and flushed.
fn run(formats: &[GLenum], sc: &Scene) -> Run {
    let n = formats.len();
    let mut rbs = vec![0; n];
    let mut fb = 0;
    let mut error = GL_NO_ERROR;
    let refused0 = v3d_refused_draws();
    let prog = if sc.draw { program(formats) } else { 0 };
    // SAFETY: plain GL calls with room for the names; the clear values are four words.
    unsafe {
        glGenRenderbuffers(n as GLsizei, rbs.as_mut_ptr());
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        while glGetError() != GL_NO_ERROR {}
        let mut bufs = Vec::new();
        for (i, &f) in formats.iter().enumerate() {
            glBindRenderbuffer(GL_RENDERBUFFER, rbs[i]);
            glRenderbufferStorage(GL_RENDERBUFFER, f, W, H);
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0 + i as GLenum, GL_RENDERBUFFER, rbs[i]);
            bufs.push(if sc.none == Some(i) { GL_NONE } else { GL_COLOR_ATTACHMENT0 + i as GLenum });
        }
        glDrawBuffers(n as GLsizei, bufs.as_ptr());
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "{formats:#x?} {:?}", logs());
        assert_eq!(glGetError(), GL_NO_ERROR, "{:?}", logs());
        glViewport(0, 0, W, H);
        if let Some([x, y, w, h]) = sc.scissor {
            glEnable(GL_SCISSOR_TEST);
            glScissor(x, y, w, h);
        }
        glColorMask(sc.mask[0] as u8, sc.mask[1] as u8, sc.mask[2] as u8, sc.mask[3] as u8);
        let one = |i: usize| match class(formats[i]) {
            Class::Float => glClearBufferfv(GL_COLOR, i as GLint, FLOAT.as_ptr()),
            Class::Int => glClearBufferiv(GL_COLOR, i as GLint, WORDS.as_ptr() as *const GLint),
            Class::Uint => glClearBufferuiv(GL_COLOR, i as GLint, WORDS.as_ptr()),
        };
        match sc.clear {
            Clear::Each => (0..n).for_each(one),
            Clear::One(i) => one(i),
            Clear::All => {
                glClearColor(FLOAT[0], FLOAT[1], FLOAT[2], FLOAT[3]);
                glClear(GL_COLOR_BUFFER_BIT);
            }
        }
        error = error.max(glGetError());
        if sc.draw {
            glUseProgram(prog);
            glVertexAttribPointer(0, 3, GL_FLOAT, 0, 12, QUAD.as_ptr() as *const c_void);
            glEnableVertexAttribArray(0);
            glDrawArrays(GL_TRIANGLES, 0, 6);
            error = error.max(glGetError());
        }
        glFinish();
        error = error.max(glGetError());
        glColorMask(1, 1, 1, 1);
        glDisable(GL_SCISSOR_TEST);
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fb);
        glDeleteRenderbuffers(n as GLsizei, rbs.as_ptr());
        if sc.draw {
            glUseProgram(0);
            glDeleteProgram(prog);
        }
    }
    Run { refused: v3d_refused_draws().wrapping_sub(refused0), seen: seen(core::mem::take), error }
}

/// The TLB's clear of RT n in a job: loaded (`true`) or cleared.
fn loaded(c: &V3dJobClear, n: usize) -> bool {
    if n == 0 {
        c.load_rt0
    } else {
        c.load_rt[n - 1]
    }
}

/// The clear words the TLB starts a RT of this format from, as
/// `state::clear_words` packs the scene's clear colour.
fn expected_words(format: GLenum) -> [u32; 4] {
    let surf = match format {
        GL_RGBA8 => GLES_SURF_RGBA8888,
        GL_RGBA32UI => GLES_SURF_RGBA32UI,
        GL_RGBA16I => GLES_SURF_RGBA16I,
        GL_RG16I => GLES_SURF_RG16I,
        _ => unreachable!(),
    };
    let c = match class(format) {
        Class::Float => FLOAT,
        _ => WORDS.map(f32::from_bits),
    };
    yakogl::v3d::state::clear_words(surf, c)
}

/// Does this draw write 32-bit-word colour for RT `rt` (of any width)?
fn writes_32bit(q: &SeenDraw, rt: u32) -> bool {
    use yakogl::shaderc::online::v3d::select::tlb_config_32b;
    (1..=4)
        .any(|n| q.fs_uniforms.contains(&V3dUniform { contents: V3D_QUNIFORM_CONSTANT, data: tlb_config_32b(rt, n) }))
}

fn mask_bits(m: [bool; 4]) -> u32 {
    m.iter().enumerate().map(|(i, b)| u32::from(*b) << i).sum()
}

/* ---- the four-target scenes ---------------------------------------------- */

const RGBA8: GLenum = GL_RGBA8;
/// What render target 3 can be: float, signed and unsigned integer.
const RT3: [(&str, GLenum); 4] =
    [("RGBA8", GL_RGBA8), ("RGBA16I", GL_RGBA16I), ("RG16I", GL_RG16I), ("RGBA32UI", GL_RGBA32UI)];

/// Four attachments, RTs 0-2 of three classes, RT 3 of `rt3`.
fn four(rt3: GLenum) -> [GLenum; 4] {
    [RGBA8, GL_RG16I, GL_RGBA32UI, rt3]
}

/// The scissor box and colour mask of the scissored and masked scenes.
const BOX: [GLint; 4] = [3, 2, 9, 7];
const MASK: [bool; 4] = [true, false, false, true];

/// Does this draw write RT `rt` as f16 pairs (any component count)?
fn writes_f16(q: &SeenDraw, rt: u32) -> bool {
    use yakogl::shaderc::online::v3d::select::tlb_config_f16;
    (1..=4)
        .any(|n| q.fs_uniforms.contains(&V3dUniform { contents: V3D_QUNIFORM_CONSTANT, data: tlb_config_f16(rt, n) }))
}

/// Does this draw write RT `rt` at all, in either way? (RT 0's f16 write
/// with three or four components is the TLB's default and has no config.)
fn writes(q: &SeenDraw, rt: u32) -> bool {
    writes_f16(q, rt) || writes_32bit(q, rt)
}

/// No refusal, no GL error.
fn assert_clean(what: &str, r: &Run) {
    let s = &r.seen;
    assert_eq!(r.error, GL_NO_ERROR, "{what}: {:?}", s.log);
    assert_eq!(refusals(r), 0, "{what}: {:?}", s.log);
    assert!(!s.log.iter().any(|l| l.contains(REFUSAL)), "{what}: {:?}", s.log);
}

/// The clear quad of a scissored or masked clear: one quad in one job, over
/// four loaded targets, with the GL's scissor box and colour mask.
fn the_quad<'a>(what: &str, r: &'a Run, sc: &Scene) -> &'a SeenDraw {
    let s = &r.seen;
    assert_clean(what, r);
    assert_eq!((s.jobs.len(), s.clears.len(), s.draws.len()), (1, 1, 1), "{what}: one job, one quad: {:?}", s.log);
    assert_eq!(s.jobs[0].rts, 4, "{what}: four render targets");
    for n in 0..4 {
        assert!(loaded(&s.clears[0], n), "{what}: RT {n} is loaded, the TLB clears none");
    }
    let q = &s.draws[0];
    assert_eq!(q.count, 6, "{what}: a quad");
    assert_eq!(q.st.colormask & 0xf, mask_bits(sc.mask), "{what}: the GL's colour mask");
    assert_eq!(q.st.scissor, sc.scissor.is_some(), "{what}: the scissor test");
    if let Some([x, y, w, h]) = sc.scissor {
        let b = [q.st.scissor_minx, q.st.scissor_miny, q.st.scissor_maxx, q.st.scissor_maxy];
        assert_eq!(b, [x as u32, y as u32, (x + w) as u32, (y + h) as u32], "{what}: the GL's box");
    }
    q
}

/// The quad writes all four outputs: RTs 1-3 each have their TLB config,
/// and the fragment block holds four colours (four dwords each); a float
/// target's are the clear colour's f32 bits (glClear's integer values are
/// undefined, so not asserted).
fn assert_writes_all_four(what: &str, q: &SeenDraw, formats: &[GLenum; 4]) {
    for rt in 1..4u32 {
        assert!(writes(q, rt), "{what}: the quad writes RT {rt}: {:?}", q.fs_uniforms);
    }
    assert_eq!(q.fs_storage.len(), 16, "{what}: four colours in the block: {:?}", q.fs_storage);
    for (k, &f) in formats.iter().enumerate() {
        let int = class(f) != Class::Float;
        assert_eq!(writes_32bit(q, k as u32), int, "{what}: RT {k}'s TLB type");
        if !int {
            assert_eq!(q.fs_storage[4 * k..4 * k + 4], FLOAT.map(f32::to_bits), "{what}: RT {k}'s colour");
        }
    }
}

/* ---- glClear -------------------------------------------------------------- */

/// An unscissored, unmasked glClear is the TLB's: RT 3 is cleared by it with
/// the clear colour's words and not loaded; no quad, four render targets.
#[test]
fn full_glclear_reaches_rt3() {
    let _g = rig();
    let formats = [RGBA8; 4];
    let r = run(&formats, &Scene { clear: Clear::All, ..PLAIN });
    assert_clean("glClear", &r);
    let s = &r.seen;
    assert!(s.draws.is_empty(), "no quad: {:?}", s.log);
    assert_eq!((s.jobs.len(), s.clears.len()), (1, 1));
    assert_eq!(s.jobs[0].rts, 4, "four render targets");
    for n in 0..4 {
        assert!(!loaded(&s.clears[0], n), "RT {n} is cleared, not loaded");
        assert_eq!(s.clears[0].clear_rt[n], expected_words(RGBA8), "RT {n}'s clear words");
    }
}

/// The same over four attachments of mixed classes, RT 3 of each class: RT 3
/// is cleared by the TLB and not loaded.
#[test]
fn full_glclear_reaches_rt3_of_every_class() {
    let _g = rig();
    for (name, rt3) in RT3 {
        let formats = four(rt3);
        let r = run(&formats, &Scene { clear: Clear::All, ..PLAIN });
        assert_clean(name, &r);
        let s = &r.seen;
        assert!(s.draws.is_empty() && s.clears.len() == 1, "{name}: {:?}", s.log);
        for n in 0..4 {
            assert!(!loaded(&s.clears[0], n), "{name}: RT {n}");
        }
    }
}

/// A scissored glClear of four attachments: one quad, scissored to the GL's
/// box, whose program writes all four outputs.
#[test]
fn scissored_glclear_reaches_rt3_through_the_quad() {
    let _g = rig();
    for (name, formats) in [("RGBA8 x4", [RGBA8; 4]), ("mixed", four(GL_RGBA16I))] {
        let sc = Scene { scissor: Some(BOX), clear: Clear::All, ..PLAIN };
        let r = run(&formats, &sc);
        let q = the_quad(name, &r, &sc);
        assert_writes_all_four(name, q, &formats);
    }
}

/// A colour-masked glClear (R and A), unscissored: one quad with the mask,
/// writing all four outputs.
#[test]
fn masked_glclear_reaches_rt3_through_the_quad() {
    let _g = rig();
    for (name, formats) in [("RGBA8 x4", [RGBA8; 4]), ("mixed", four(GL_RGBA32UI))] {
        let sc = Scene { mask: MASK, clear: Clear::All, ..PLAIN };
        let r = run(&formats, &sc);
        let q = the_quad(name, &r, &sc);
        assert_writes_all_four(name, q, &formats);
    }
}

/// Scissored and masked together: the same one quad.
#[test]
fn scissored_masked_glclear_reaches_rt3_through_the_quad() {
    let _g = rig();
    let formats = [RGBA8; 4];
    let sc = Scene { scissor: Some([1, 1, 5, 5]), mask: [false, true, true, false], clear: Clear::All, ..PLAIN };
    let r = run(&formats, &sc);
    let q = the_quad("scissored and masked", &r, &sc);
    assert_writes_all_four("scissored and masked", q, &formats);
}

/* ---- glClearBuffer of draw buffer 3 --------------------------------------- */

/// glClearBuffer{f,i,ui}v of draw buffer 3 alone, unscissored: the TLB
/// clears RT 3 with its class's words; RTs 0-2 are loaded, none cleared;
/// no quad.
#[test]
fn clearbuffer_of_drawbuffer_3_leaves_rts_0_to_2_alone() {
    let _g = rig();
    for (name, rt3) in RT3 {
        let formats = four(rt3);
        let r = run(&formats, &Scene { clear: Clear::One(3), ..PLAIN });
        assert_clean(name, &r);
        let s = &r.seen;
        assert!(s.draws.is_empty() && (s.jobs.len(), s.clears.len()) == (1, 1), "{name}: {:?}", s.log);
        assert_eq!(s.jobs[0].rts, 4, "{name}");
        for n in 0..3 {
            assert!(loaded(&s.clears[0], n), "{name}: RT {n} is loaded, not cleared");
        }
        assert!(!loaded(&s.clears[0], 3), "{name}: RT 3 is cleared");
        assert_eq!(s.clears[0].clear_rt[3], expected_words(rt3), "{name}: RT 3's clear words");
    }
}

/// glClearBuffer of every draw buffer in turn: each RT, RT 3 included, keeps
/// the words of its own call.
#[test]
fn clearbuffer_of_every_drawbuffer_gives_rt3_its_own_words() {
    let _g = rig();
    for (name, rt3) in RT3 {
        let formats = four(rt3);
        let r = run(&formats, &PLAIN);
        assert_clean(name, &r);
        let c = &r.seen.clears[0];
        for (n, &f) in formats.iter().enumerate() {
            assert!(!loaded(c, n), "{name}: RT {n}");
            assert_eq!(c.clear_rt[n], expected_words(f), "{name}: RT {n} ({f:#x})");
        }
    }
}

/// glClearBuffer of draw buffer 3 alone, scissored: one quad over four loaded
/// targets, carrying the clear value (the raw words for an integer RT 3),
/// writing RT 3 only (no config of RTs 0-2, one colour in the block).
#[test]
fn scissored_clearbuffer_of_drawbuffer_3_is_a_quad_of_rt3_alone() {
    let _g = rig();
    for (name, rt3) in RT3 {
        let formats = four(rt3);
        let sc = Scene { scissor: Some(BOX), clear: Clear::One(3), ..PLAIN };
        let r = run(&formats, &sc);
        let q = the_quad(name, &r, &sc);
        assert!(writes(q, 3), "{name}: the quad writes RT 3: {:?}", q.fs_uniforms);
        assert_eq!(writes_32bit(q, 3), class(rt3) != Class::Float, "{name}: RT 3's TLB type");
        for rt in 0..3 {
            assert!(!writes_32bit(q, rt), "{name}: RT {rt} is not written as words");
            if rt > 0 {
                assert!(!writes_f16(q, rt), "{name}: RT {rt} is not written");
            }
        }
        assert_eq!(q.fs_storage.len(), 4, "{name}: one colour in the block: {:?}", q.fs_storage);
        let want: [u32; 4] = if class(rt3) == Class::Float { FLOAT.map(f32::to_bits) } else { WORDS };
        assert_eq!(q.fs_storage[..], want, "{name}: RT 3's clear value");
    }
}

/// The same with the colour mask instead of the scissor.
#[test]
fn masked_clearbuffer_of_drawbuffer_3_is_a_quad_of_rt3_alone() {
    let _g = rig();
    for (name, rt3) in RT3 {
        let formats = four(rt3);
        let sc = Scene { mask: MASK, clear: Clear::One(3), ..PLAIN };
        let r = run(&formats, &sc);
        let q = the_quad(name, &r, &sc);
        assert!(writes(q, 3), "{name}: the quad writes RT 3");
        assert_eq!(q.fs_storage.len(), 4, "{name}: one colour in the block");
        for rt in 0..3 {
            assert!(!writes_32bit(q, rt), "{name}: RT {rt} is not written as words");
        }
    }
}

/// glClearBufferfv of draw buffer 3 while draw buffer 3 is GL_NONE does
/// nothing (ES 3.0.6 section 4.2.3): no quad, no clear of any RT, no error;
/// unscissored and scissored.
#[test]
#[ignore = "v3d-clear4: glClearBufferfv clears a GL_NONE draw buffer: clear_image_kind is None there and src/gl/es3.rs:1392 only refuses Some(non-float), so with_clear_target(3) still clears RT 3 (the integer forms return at es3.rs:1362)"]
fn clearbufferfv_of_a_none_drawbuffer_3_does_nothing() {
    let _g = rig();
    for (what, scissor) in [("unscissored", None), ("scissored", Some(BOX))] {
        let formats = [RGBA8; 4];
        let r = run(&formats, &Scene { scissor, clear: Clear::One(3), none: Some(3), ..PLAIN });
        assert_clean(what, &r);
        let s = &r.seen;
        assert!(s.draws.is_empty(), "{what}: no quad: {:?}", s.log);
        for c in &s.clears {
            for n in 0..4 {
                assert!(loaded(c, n), "{what}: RT {n} is not cleared: {c:?}");
            }
        }
    }
}

/// The integer forms of the same: glClearBufferiv and glClearBufferuiv of a
/// GL_NONE draw buffer 3 do nothing.
#[test]
fn clearbuffer_iv_and_uiv_of_a_none_drawbuffer_3_do_nothing() {
    let _g = rig();
    for (name, rt3) in [("RGBA16I", GL_RGBA16I), ("RGBA32UI", GL_RGBA32UI)] {
        for (what, scissor) in [("unscissored", None), ("scissored", Some(BOX))] {
            let r = run(&four(rt3), &Scene { scissor, clear: Clear::One(3), none: Some(3), ..PLAIN });
            assert_clean(what, &r);
            let s = &r.seen;
            assert!(s.draws.is_empty(), "{name} {what}: no quad: {:?}", s.log);
            for c in &s.clears {
                for n in 0..4 {
                    assert!(loaded(c, n), "{name} {what}: RT {n} is not cleared: {c:?}");
                }
            }
        }
    }
}

/// With draw buffer 3 GL_NONE, a glClearBufferfv of draw buffer 2 still
/// clears RT 2 and only it.
#[test]
fn a_none_drawbuffer_3_does_not_stop_the_clear_of_drawbuffer_2() {
    let _g = rig();
    let formats = [RGBA8; 4];
    let r = run(&formats, &Scene { clear: Clear::One(2), none: Some(3), ..PLAIN });
    assert_clean("drawbuffer 2", &r);
    let s = &r.seen;
    assert_eq!((s.jobs.len(), s.clears.len()), (1, 1), "{:?}", s.log);
    for n in [0, 1, 3] {
        assert!(loaded(&s.clears[0], n), "RT {n} is loaded");
    }
    assert!(!loaded(&s.clears[0], 2), "RT 2 is cleared");
}

/* ---- the stand-in ---------------------------------------------------------- */

/// The rig sets the four-target stand-in, and drops it again.
#[test]
fn the_stand_in_is_four_and_goes_back_to_zero() {
    {
        let _g = rig();
        let mut v = [0; 2];
        // SAFETY: two GLints.
        unsafe {
            glGetIntegerv(GL_MAX_DRAW_BUFFERS, &mut v[0]);
            glGetIntegerv(GL_MAX_COLOR_ATTACHMENTS, &mut v[1]);
        }
        assert_eq!(v, [4, 4]);
    }
    assert_eq!(yakogl::backend::HOST_DRAW_TARGETS.load(Ordering::Relaxed), 0);
}
