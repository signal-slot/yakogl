// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! A scissored or masked colour clear of several colour attachments of mixed
//! integer and float classes on the V3D back end (v3d-mrt-clear), through the
//! real GL core and the real V3D back end with the job builder stood in for
//! (the pattern of tests/es3_int_rt_v3d.rs's `gl_api`: a fake
//! `v3d_job_begin`/`v3d_job_draw`/`v3d_job_end` that records, the layout code
//! of `v3d_tex.c` dlopen'd, a platform whose memory answers).
//!
//! The whole-target clear (no scissor, full mask) is the TLB's: one clear
//! word per RT in `V3dJobClear::clear_rt`, and that is held here
//! (`unscissored_*`). A scissored or colour-masked clear of two or three
//! attachments is one clear quad whose program writes every RT the clear
//! names in its own class (`src/v3d/clear_mrt.rs`, `draw_clear_quad`;
//! docs/notes/v3d-mrt-clear.md); until it landed the back end refused it by
//! name ("a scissored or masked colour clear of several colour
//! attachments"). The tests assert what the job holds, in terms that fit
//! either design (a quad per class, or one quad writing every RT):
//!
//! - no refusal is logged and the GL reports no error;
//! - the clear is drawn: quads carry the GL's scissor box and colour mask, a
//!   quad writing an integer RT uses 32-bit TLB words with the raw clear
//!   words, a float one f16 pairs;
//! - a RT the clear does not name (glClearBuffer of one drawbuffer) is left
//!   loaded (`load_rt0` / `load_rt`), never cleared by the TLB, and no quad
//!   writes it.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_mrt_clear_v3d
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-v3d", not(feature = "report-es2")))]

use core::ffi::{c_char, c_int, c_void, CStr};
use std::collections::BTreeMap;
use std::ffi::CString;
use std::path::PathBuf;
use std::process::Command;
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

/// The attachment sets: two and three attachments mixing a float target
/// with RGBA32UI, RGBA16I and RG16I, and an integer target in front.
fn sets() -> Vec<(&'static str, Vec<GLenum>)> {
    vec![
        ("RGBA8+RGBA32UI", vec![GL_RGBA8, GL_RGBA32UI]),
        ("RGBA8+RG16I", vec![GL_RGBA8, GL_RG16I]),
        ("RGBA16I+RGBA8", vec![GL_RGBA16I, GL_RGBA8]),
        ("RGBA8+RGBA16I+RG16I", vec![GL_RGBA8, GL_RGBA16I, GL_RG16I]),
        ("RGBA32UI+RGBA8+RG16I", vec![GL_RGBA32UI, GL_RGBA8, GL_RG16I]),
    ]
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
}

const PLAIN: Scene = Scene { scissor: None, mask: [true; 4], clear: Clear::Each, draw: false };

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
            bufs.push(GL_COLOR_ATTACHMENT0 + i as GLenum);
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

/* ---- today: the TLB's whole-target clear --------------------------------- */

/// An unscissored, unmasked clear of each drawbuffer is the TLB's: no quad,
/// no refusal, one job whose clear words are each RT's own, narrowed per
/// internal type (`state::clear_words`), none of them loaded.
#[test]
fn unscissored_clear_of_several_attachments_is_the_tlbs() {
    let _g = rig();
    for (name, formats) in sets() {
        let r = run(&formats, &PLAIN);
        let s = &r.seen;
        assert_eq!(r.error, GL_NO_ERROR, "{name}: {:?}", s.log);
        assert_eq!(refusals(&r), 0, "{name}: {:?}", s.log);
        assert!(s.draws.is_empty(), "{name}: no quad: {:?}", s.log);
        assert_eq!((s.jobs.len(), s.clears.len()), (1, 1), "{name}");
        assert_eq!(s.jobs[0].rts as usize, formats.len(), "{name}: RTs of the job");
        assert!(s.jobs[0].typed, "{name}: a typed job");
        let c = &s.clears[0];
        for (n, &f) in formats.iter().enumerate() {
            assert!(!loaded(c, n), "{name}: RT {n} is cleared, not loaded");
            assert_eq!(c.clear_rt[n], expected_words(f), "{name}: RT {n} ({f:#x}) clear words");
        }
    }
}

/// glClear of the colour buffer over mixed attachments: also the TLB's, no
/// quad, no RT loaded.
#[test]
fn unscissored_glclear_of_several_attachments_is_the_tlbs() {
    let _g = rig();
    for (name, formats) in sets() {
        let r = run(&formats, &Scene { clear: Clear::All, ..PLAIN });
        let s = &r.seen;
        assert_eq!(r.error, GL_NO_ERROR, "{name}: {:?}", s.log);
        assert_eq!(refusals(&r), 0, "{name}");
        assert!(s.draws.is_empty() && s.clears.len() == 1, "{name}: {:?}", s.log);
        for n in 0..formats.len() {
            assert!(!loaded(&s.clears[0], n), "{name}: RT {n}");
        }
    }
}

/// glClearBuffer of one drawbuffer of several, unscissored: the TLB clears
/// that RT alone, the others are loaded (`clear_target_mask`).
#[test]
fn unscissored_clearbuffer_of_one_drawbuffer_loads_the_others() {
    let _g = rig();
    for (name, formats) in sets() {
        for i in 0..formats.len() {
            let r = run(&formats, &Scene { clear: Clear::One(i), ..PLAIN });
            let s = &r.seen;
            assert_eq!(r.error, GL_NO_ERROR, "{name}: {:?}", s.log);
            assert_eq!(refusals(&r), 0, "{name} drawbuffer {i}");
            assert!(s.draws.is_empty() && s.clears.len() == 1, "{name} drawbuffer {i}");
            for n in 0..formats.len() {
                assert_eq!(loaded(&s.clears[0], n), n != i, "{name}: drawbuffer {i}, RT {n}");
            }
            assert_eq!(s.clears[0].clear_rt[i], expected_words(formats[i]), "{name}: drawbuffer {i}");
        }
    }
}

/// An unscissored, unmasked clear fused with a draw stays the TLB's: the
/// draw alone is recorded and no RT is loaded.
#[test]
fn full_clear_fused_with_a_draw_stays_the_tlbs() {
    let _g = rig();
    for (name, formats) in sets() {
        let r = run(&formats, &Scene { draw: true, ..PLAIN });
        let s = &r.seen;
        assert_eq!(r.error, GL_NO_ERROR, "{name}: {:?}", s.log);
        assert_eq!(refusals(&r), 0, "{name}: {:?}", s.log);
        assert_eq!(s.draws.len(), 1, "{name}: the draw alone: {:?}", s.log);
        assert_eq!(s.jobs.len(), 1, "{name}");
        for n in 0..formats.len() {
            assert!(!loaded(&s.clears[0], n), "{name}: RT {n}");
        }
    }
}

/// The refusal that is gone stays gone: no scene below logs it, and the
/// application sees no error (the clear of each is asserted by its test).
#[test]
fn the_refusal_of_several_attachments_is_never_logged() {
    let _g = rig();
    let scenes: [(&str, Scene); 5] = [
        ("scissored", Scene { scissor: Some([3, 2, 9, 7]), ..PLAIN }),
        ("masked", Scene { mask: [true, false, false, true], ..PLAIN }),
        ("scissored glClear", Scene { scissor: Some([3, 2, 9, 7]), clear: Clear::All, ..PLAIN }),
        ("one drawbuffer, scissored", Scene { scissor: Some([0, 4, 10, 5]), clear: Clear::One(1), ..PLAIN }),
        ("masked and scissored", Scene { scissor: Some([1, 1, 5, 5]), mask: [false, true, true, false], ..PLAIN }),
    ];
    for (name, formats) in sets() {
        for (what, sc) in &scenes {
            let r = run(&formats, sc);
            let s = &r.seen;
            assert_eq!(refusals(&r), 0, "{name}, {what}: {:?}", s.log);
            assert!(!s.log.iter().any(|l| l.contains(REFUSAL)), "{name}, {what}: {:?}", s.log);
            assert!(!s.draws.is_empty(), "{name}, {what}: the clear is drawn");
            assert_eq!(r.error, GL_NO_ERROR, "{name}, {what}");
        }
    }
}

/* ---- the drawn clear: what the job holds ---------------------------------- */

/// The common assertions of a quad-drawn clear of the `cleared` drawbuffers
/// of `formats`: no refusal and no GL error; every RT loaded (the TLB
/// clears none of them); the clear quads (every draw but a fused draw's own)
/// are one per class or one in all, carry the GL's scissor box and colour
/// mask; an integer RT is written by a quad with 32-bit TLB words for it and
/// the raw clear words in a fragment block (glClear's integer values are
/// undefined, so not asserted for it).
fn assert_quad_clear(name: &str, formats: &[GLenum], r: &Run, sc: &Scene, cleared: &[usize]) {
    let s = &r.seen;
    assert_eq!(r.error, GL_NO_ERROR, "{name}: {:?}", s.log);
    assert_eq!(refusals(r), 0, "{name}: no refusal logged: {:?}", s.log);
    assert_eq!(s.jobs.len(), 1, "{name}: one job");
    assert_eq!(s.clears.len(), 1, "{name}");
    for n in 0..formats.len() {
        assert!(loaded(&s.clears[0], n), "{name}: RT {n} is loaded, the quad (or nothing) writes it");
    }
    let own = usize::from(sc.draw);
    assert!(s.draws.len() > own, "{name}: a clear quad is drawn: {} draws", s.draws.len());
    let quads = &s.draws[..s.draws.len() - own];
    assert!(quads.len() <= 3, "{name}: a quad per class or one quad, not one per RT and channel: {}", quads.len());
    for q in quads {
        assert_eq!(q.count, 6, "{name}: a quad");
        assert_eq!(q.st.colormask & 0xf, mask_bits(sc.mask), "{name}: the GL's colour mask");
        assert_eq!(q.st.scissor, sc.scissor.is_some(), "{name}: the scissor test");
        if let Some([x, y, w, h]) = sc.scissor {
            let b = [q.st.scissor_minx, q.st.scissor_miny, q.st.scissor_maxx, q.st.scissor_maxy];
            assert_eq!(b, [x as u32, y as u32, (x + w) as u32, (y + h) as u32], "{name}: the GL's box");
        }
    }
    for &i in cleared {
        if class(formats[i]) != Class::Float {
            assert!(quads.iter().any(|q| writes_32bit(q, i as u32)), "{name}: RT {i} is written with 32-bit words");
            if sc.clear != Clear::All {
                assert!(
                    quads.iter().any(|q| q.fs_storage.windows(4).any(|w| w == WORDS)),
                    "{name}: RT {i}: the raw words reach a quad: {quads:?}"
                );
            }
        }
    }
}

fn all(formats: &[GLenum]) -> Vec<usize> {
    (0..formats.len()).collect()
}

/// (b) A scissored clear of every attachment.
#[test]
fn scissored_clear_of_all_attachments_is_drawn() {
    let _g = rig();
    for (name, formats) in sets() {
        let sc = Scene { scissor: Some([3, 2, 9, 7]), ..PLAIN };
        assert_quad_clear(name, &formats, &run(&formats, &sc), &sc, &all(&formats));
    }
}

/// (b) glClear of every attachment, scissored.
#[test]
fn scissored_glclear_of_all_attachments_is_drawn() {
    let _g = rig();
    for (name, formats) in sets() {
        let sc = Scene { scissor: Some([3, 2, 9, 7]), clear: Clear::All, ..PLAIN };
        assert_quad_clear(name, &formats, &run(&formats, &sc), &sc, &all(&formats));
    }
}

/// (c) A partially masked clear (R and A only), unscissored.
#[test]
fn masked_clear_of_all_attachments_is_drawn() {
    let _g = rig();
    for (name, formats) in sets() {
        let sc = Scene { mask: [true, false, false, true], ..PLAIN };
        assert_quad_clear(name, &formats, &run(&formats, &sc), &sc, &all(&formats));
    }
}

/// (b)+(c) Scissored and masked together.
#[test]
fn scissored_and_masked_clear_of_all_attachments_is_drawn() {
    let _g = rig();
    for (name, formats) in sets() {
        let sc = Scene { scissor: Some([1, 1, 5, 5]), mask: [true, false, false, true], ..PLAIN };
        assert_quad_clear(name, &formats, &run(&formats, &sc), &sc, &all(&formats));
    }
}

/// (d) glClearBuffer of one drawbuffer of several while scissored: that RT
/// is cleared by a quad, the others stay loaded, and no quad writes an
/// integer RT the clear does not name.
#[test]
fn scissored_clearbuffer_of_one_drawbuffer_is_drawn() {
    let _g = rig();
    for (name, formats) in sets() {
        for i in 0..formats.len() {
            let sc = Scene { scissor: Some([0, 4, 10, 5]), clear: Clear::One(i), ..PLAIN };
            let r = run(&formats, &sc);
            assert_quad_clear(name, &formats, &r, &sc, &[i]);
            for q in &r.seen.draws {
                for (n, &f) in formats.iter().enumerate() {
                    if n != i && class(f) != Class::Float {
                        assert!(!writes_32bit(q, n as u32), "{name}: drawbuffer {i}: RT {n} is not written");
                    }
                }
            }
        }
    }
}

/// (d) The same, masked instead of scissored.
#[test]
fn masked_clearbuffer_of_one_drawbuffer_is_drawn() {
    let _g = rig();
    for (name, formats) in sets() {
        for i in 0..formats.len() {
            let sc = Scene { mask: [true, false, false, true], clear: Clear::One(i), ..PLAIN };
            assert_quad_clear(name, &formats, &run(&formats, &sc), &sc, &[i]);
        }
    }
}

/// (e) A scissored, masked clear fused with a draw into every attachment:
/// the clear quad(s) first, in the same job, then the application's draw last.
#[test]
fn scissored_masked_clear_fused_with_a_draw() {
    let _g = rig();
    for (name, formats) in sets() {
        let sc = Scene { scissor: Some([2, 2, 12, 8]), mask: [true, false, true, true], draw: true, ..PLAIN };
        let r = run(&formats, &sc);
        assert_quad_clear(name, &formats, &r, &sc, &all(&formats));
        let last = r.seen.draws.last().expect("the draw");
        assert_eq!(last.count, 6, "{name}: the application's draw is last");
    }
}
