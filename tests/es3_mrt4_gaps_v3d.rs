// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! glDrawBuffers lists with gaps that reach colour attachment 3, on the V3D
//! host back end (plan #368, RC14 on the Pi 4): `{NONE, NONE, NONE, ATT3}`,
//! `{ATT0, NONE, NONE, ATT3}`, `{NONE, ATT1, NONE, ATT3}`, and a framebuffer
//! whose only colour attachment is 3.
//!
//! `backend::HOST_DRAW_TARGETS` stands in for a build whose `DRAW_TARGETS` is
//! 4 (docs/notes/mrt4-v3d-pvr.md); the GL core is the real one, the back end
//! is the real V3D one, the job builder records and a stand-in GPU gives
//! every colour render target the texel its job clears it to (the rig of
//! tests/es3_mrt_clear_v3d.rs, plus that fill).
//!
//! What the GL core hands the back end for a list (src/gl/draw.rs,
//! `resolve_targets`, review R-09): the list starts at the first slot that
//! writes, `targets[0].out_shift` says how many leading draw buffers it
//! dropped, later gaps are `gpu` 0 targets. The V3D back end draws a list
//! whose draw buffer 0 is written (`{ATT0, NONE, NONE, ATT3}`: four RTs, RT 1
//! and RT 2 configured with address 0, RT 3 stored into attachment 3). It
//! clears a list with a non-zero `out_shift` (a clear has no fragment
//! outputs, so the job's RT k is the list's k-th target) and REFUSES a draw
//! into one by name (`state::unsupported`, src/v3d/state.rs: "draw buffer 0
//! is GL_NONE while a later draw buffer is written"): the shader writes
//! output i to RT i, and mapping the job's RTs back to draw-buffer order is
//! `src/v3d/backend.rs`'s (`extra_targets`, `Target`). So three of the four
//! shapes are cleared but not drawn today. The plain tests pin this; the
//! `#[ignore]`d one holds ES 3.0's answer, named `defect: ...`.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_mrt4_gaps_v3d
//! ```
#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-v3d", not(feature = "report-es2")))]

use core::ffi::{c_char, c_int, c_void, CStr};
use std::collections::BTreeMap;
use std::ffi::CString;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Mutex, MutexGuard, Once, OnceLock};
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
#[allow(dead_code)] // the fields the other tests of this rig read
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
        gpu_clears(&(*j).t, &*c);
        seen(|s| s.clears.push(*c));
        (*(*j).bcl).used += 1;
        (*(*j).rcl).used = 200;
    }
    true
}

/// The stand-in GPU: a render target the job does not load, and whose
/// address is set, holds the job's clear word (RGBA8) in every pixel after it.
fn gpu_clears(t: &V3dJobTarget, c: &V3dJobClear) {
    let word = |n: usize| match (t.typed, n) {
        (true, n) => c.clear_rt[n][0],
        (false, 0) => c.rgba8,
        (false, n) => c.rgba8_rt[n - 1],
    };
    let rts = (t.rts as usize).clamp(1, 4);
    for n in 0..rts {
        let (address, stride, load) = if n == 0 {
            (t.address, t.stride, c.load_rt0)
        } else {
            (t.rt[n - 1].address, t.rt[n - 1].stride, c.load_rt[n - 1])
        };
        if address == 0 || load {
            continue;
        }
        with_platform(|p| {
            let (base, a) =
                p.allocs.range_mut(..=u64::from(address)).next_back().expect("a render target is allocated");
            let off = (u64::from(address) - base) as usize;
            let bytes = a.mem.as_mut_ptr() as *mut u8;
            for y in 0..t.height as usize {
                for x in 0..t.width as usize {
                    let at = off + y * stride as usize + x * 4;
                    assert!(at + 4 <= a.size);
                    // SAFETY: inside the allocation (asserted).
                    unsafe { core::ptr::copy_nonoverlapping(word(n).to_le_bytes().as_ptr(), bytes.add(at), 4) };
                }
            }
        });
    }
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

/* ---- the scenes ---------------------------------------------------------- */

const W: GLsizei = 24;
const H: GLsizei = 16;
const A: [GLenum; 4] = [GL_COLOR_ATTACHMENT0, GL_COLOR_ATTACHMENT1, GL_COLOR_ATTACHMENT2, GL_COLOR_ATTACHMENT3];
const N: GLenum = GL_NONE;
/// The clear colour of every scene, and the bytes an RGBA8 target holds for it.
const CLEAR: [f32; 4] = [0.25, 0.5, 0.75, 1.0];

fn clear_bytes() -> [u8; 4] {
    yakogl::v3d::state::clear_words(yakogl::backend::GLES_SURF_RGBA8888, CLEAR)[0].to_le_bytes()
}

/// A list, the attachments it is made against.
struct Shape {
    name: &'static str,
    attached: &'static [usize],
    list: [GLenum; 4],
}

/// The one list with a written draw buffer 0 that the V3D draws today.
const GAP_AT_0_3: Shape = Shape { name: "{A0,NONE,NONE,A3}", attached: &[0, 1, 2, 3], list: [A[0], N, N, A[3]] };
/// The lists whose draw the V3D refuses today (`out_shift` 3, 1 and 3); their clears are drawn.
const SHIFTED_LISTS: [Shape; 3] = [
    Shape { name: "{NONE,NONE,NONE,A3}", attached: &[0, 1, 2, 3], list: [N, N, N, A[3]] },
    Shape { name: "{NONE,A1,NONE,A3}", attached: &[0, 1, 2, 3], list: [N, A[1], N, A[3]] },
    Shape { name: "only A3 attached", attached: &[3], list: [N, N, N, A[3]] },
];

const VS: &str = "#version 300 es\nin vec4 a_position;\nvoid main() { gl_Position = a_position; }\n";
const COLOURS: [[f32; 4]; 4] = [[1.0, 0.0, 0.0, 1.0], [0.0, 1.0, 0.0, 1.0], [0.0, 0.0, 1.0, 1.0], [1.0, 1.0, 0.0, 1.0]];

fn logs() -> Vec<String> {
    seen(|s| s.log.clone())
}

/// A fragment shader writing `layout(location = i) out vec4` for each `i` of `outs`.
fn program(outs: &[usize]) -> GLuint {
    let mut f = String::from("#version 300 es\nprecision highp float;\n");
    for &i in outs {
        f += &format!("layout(location = {i}) out vec4 o{i};\n");
    }
    f += "void main() {\n";
    for &i in outs {
        let c = COLOURS[i];
        f += &format!("  o{i} = vec4({:?}, {:?}, {:?}, {:?});\n", c[0], c[1], c[2], c[3]);
    }
    f += "}\n";
    let mk = |ty: GLenum, src: &str| {
        let c = CString::new(src).unwrap();
        // SAFETY: plain GL calls; the string lives for the call.
        unsafe {
            let s = glCreateShader(ty);
            glShaderSource(s, 1, [c.as_ptr()].as_ptr(), core::ptr::null());
            glCompileShader(s);
            let mut ok: GLint = 0;
            glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
            assert_eq!(ok, 1, "compile: {src}\n{:?}", logs());
            s
        }
    };
    let (v, f) = (mk(GL_VERTEX_SHADER, VS), mk(GL_FRAGMENT_SHADER, &f));
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

/// A bound framebuffer with one RGBA8 renderbuffer on each attachment of `attached`.
struct Fb {
    fb: GLuint,
    rbs: Vec<GLuint>,
}

impl Fb {
    fn new(attached: &[usize]) -> Fb {
        let (mut fb, mut rbs) = (0, vec![0; attached.len()]);
        // SAFETY: plain GL calls with room for the names.
        unsafe {
            glGenFramebuffers(1, &mut fb);
            glGenRenderbuffers(rbs.len() as GLsizei, rbs.as_mut_ptr());
            glBindFramebuffer(GL_FRAMEBUFFER, fb);
            for (&i, &rb) in attached.iter().zip(&rbs) {
                glBindRenderbuffer(GL_RENDERBUFFER, rb);
                glRenderbufferStorage(GL_RENDERBUFFER, GL_RGBA8, W, H);
                glFramebufferRenderbuffer(GL_FRAMEBUFFER, A[i], GL_RENDERBUFFER, rb);
            }
            assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "{attached:?} {:?}", logs());
            glViewport(0, 0, W, H);
            assert_eq!(glGetError(), GL_NO_ERROR, "{:?}", logs());
        }
        Fb { fb, rbs }
    }

    fn list(&self, l: &[GLenum]) {
        // SAFETY: plain GL call over a live slice.
        unsafe { glDrawBuffers(l.len() as GLsizei, l.as_ptr()) };
    }
}

impl Drop for Fb {
    fn drop(&mut self) {
        // SAFETY: names this value made.
        unsafe {
            glFinish();
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteFramebuffers(1, &self.fb);
            glDeleteRenderbuffers(self.rbs.len() as GLsizei, self.rbs.as_ptr());
            glUseProgram(0);
        }
    }
}

/// What the back end recorded for a closure's GL calls, flushed.
struct Run {
    seen: Seen,
    /// Draws the V3D back end refused (`v3d_refused_draws`).
    refused: u32,
    error: GLenum,
}

fn capture(f: impl FnOnce()) -> Run {
    let refused0 = v3d_refused_draws();
    // SAFETY: plain GL calls.
    unsafe {
        glFinish();
        while glGetError() != GL_NO_ERROR {}
    }
    seen(|s| *s = Seen::default());
    f();
    // SAFETY: plain GL calls.
    let error = unsafe {
        glFinish();
        glGetError()
    };
    Run { seen: seen(core::mem::take), refused: v3d_refused_draws().wrapping_sub(refused0), error }
}

fn draw(prog: GLuint) {
    // SAFETY: QUAD is static and outlives the draw.
    unsafe {
        glUseProgram(prog);
        glVertexAttribPointer(0, 3, GL_FLOAT, 0, 12, QUAD.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glDrawArrays(GL_TRIANGLES, 0, 6);
    }
}

fn clear_buffer(drawbuffer: GLint) {
    // SAFETY: four floats.
    unsafe { glClearBufferfv(GL_COLOR, drawbuffer, CLEAR.as_ptr()) };
}

fn clear_all() {
    // SAFETY: plain GL calls.
    unsafe {
        glClearColor(CLEAR[0], CLEAR[1], CLEAR[2], CLEAR[3]);
        glClear(GL_COLOR_BUFFER_BIT);
    }
}

/// Pixel (3, 2) of an attachment, read with glReadBuffer.
fn read(att: usize) -> [u8; 4] {
    let mut px = [0xeeu8; 4];
    // SAFETY: room for one RGBA8 pixel.
    unsafe {
        glReadBuffer(A[att]);
        glReadPixels(3, 2, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr() as *mut c_void);
        assert_eq!(glGetError(), GL_NO_ERROR, "read of attachment {att}: {:?}", logs());
    }
    px
}

/// The addresses of RT 0..3 of a job.
fn rt_addresses(t: &V3dJobTarget) -> [u32; 4] {
    [t.address, t.rt[0].address, t.rt[1].address, t.rt[2].address]
}

/// The memory address of each attachment of a four-attachment framebuffer:
/// one dense draw's job names them.
fn attachment_addresses(fb: &Fb) -> [u32; 4] {
    fb.list(&A);
    let p = program(&[0, 1, 2, 3]);
    let r = capture(|| draw(p));
    assert_eq!((r.refused, r.error, r.seen.jobs.len()), (0, GL_NO_ERROR, 1), "{:?}", r.seen.log);
    assert_eq!(r.seen.jobs[0].rts, 4);
    let a = rt_addresses(&r.seen.jobs[0]);
    assert!(a.iter().all(|&x| x != 0) && a.iter().enumerate().all(|(i, x)| !a[..i].contains(x)), "{a:x?}");
    a
}

/// Does the draw's fragment stream carry the TLB config of `rt` (RT 1..3; RT
/// 0's is the default and writes none)?
fn configures(q: &SeenDraw, rt: u32) -> bool {
    use yakogl::shaderc::online::v3d::select::tlb_config_f16;
    q.fs_uniforms.contains(&V3dUniform { contents: V3D_QUNIFORM_CONSTANT, data: tlb_config_f16(rt, 4) })
}

/* ---- {A0, NONE, NONE, A3}: written draw buffer 0, drawn today ------------ */

/// Four outputs into {A0, NONE, NONE, A3}: one job of four RTs, RT 0 and RT 3
/// stored into attachments 0 and 3, RT 1 and RT 2 configured with address 0
/// (never stored), and the fragment stream writes the TLB of RT 1..3 (the
/// outputs of the NONE slots are written, the RTs drop them).
#[test]
fn a0_none_none_a3_draws_rt0_and_rt3() {
    let _g = rig();
    let fb = Fb::new(&[0, 1, 2, 3]);
    let at = attachment_addresses(&fb);
    fb.list(&GAP_AT_0_3.list);
    let p = program(&[0, 1, 2, 3]);
    let r = capture(|| draw(p));
    let s = &r.seen;
    assert_eq!((r.error, r.refused), (GL_NO_ERROR, 0), "{:?}", s.log);
    assert_eq!((s.jobs.len(), s.draws.len()), (1, 1), "{:?}", s.log);
    assert_eq!(s.jobs[0].rts, 4, "RT 3 is reached, so four are configured");
    assert_eq!(rt_addresses(&s.jobs[0]), [at[0], 0, 0, at[3]]);
    for rt in 1..=3 {
        assert!(configures(&s.draws[0], rt), "TLB config of RT {rt}");
    }
}

/// A shader writing only location 3 (and one writing 0 and 3) into the same
/// list: the TLB writes are the RTs it names, no more.
#[test]
fn a0_none_none_a3_writes_only_the_locations_the_shader_has() {
    let _g = rig();
    let fb = Fb::new(&[0, 1, 2, 3]);
    let at = attachment_addresses(&fb);
    fb.list(&GAP_AT_0_3.list);
    for outs in [&[3usize][..], &[0, 3][..]] {
        let p = program(outs);
        let r = capture(|| draw(p));
        let s = &r.seen;
        assert_eq!((r.error, r.refused), (GL_NO_ERROR, 0), "{outs:?}: {:?}", s.log);
        assert_eq!(s.draws.len(), 1, "{outs:?}");
        assert_eq!(rt_addresses(&s.jobs[0]), [at[0], 0, 0, at[3]], "{outs:?}");
        for rt in 1..=3u32 {
            assert_eq!(configures(&s.draws[0], rt), outs.contains(&(rt as usize)), "{outs:?}: RT {rt}");
        }
    }
}

/// glClearBuffer of draw buffer 3 clears attachment 3 and loads the others;
/// of draw buffer 0, attachment 0 alone. Read back with glReadBuffer.
#[test]
fn a0_none_none_a3_clearbuffer_reaches_attachment_3() {
    let _g = rig();
    let fb = Fb::new(&[0, 1, 2, 3]);
    fb.list(&GAP_AT_0_3.list);
    let r = capture(|| clear_buffer(3));
    let s = &r.seen;
    assert_eq!((r.error, r.refused), (GL_NO_ERROR, 0), "{:?}", s.log);
    assert_eq!((s.jobs.len(), s.clears.len(), s.draws.len()), (1, 1, 0), "{:?}", s.log);
    let c = &s.clears[0];
    assert_eq!(
        (c.load_rt0, c.load_rt[0], c.load_rt[1], c.load_rt[2]),
        (true, true, true, false),
        "only RT 3 is cleared"
    );
    assert_eq!(read(3), clear_bytes());
    assert_eq!(read(0), [0; 4], "attachment 0 was loaded, not cleared");
    let r = capture(|| clear_buffer(0));
    let c = &r.seen.clears[0];
    assert_eq!((r.error, r.refused), (GL_NO_ERROR, 0));
    assert_eq!((c.load_rt0, c.load_rt[2]), (false, true), "only RT 0 is cleared");
    assert_eq!(read(0), clear_bytes());
}

/// glClear clears the two draw buffers the list writes, attachments 0 and 3,
/// and leaves 1 and 2 (they are not draw buffers) alone.
#[test]
fn a0_none_none_a3_glclear_reaches_0_and_3_only() {
    let _g = rig();
    let fb = Fb::new(&[0, 1, 2, 3]);
    fb.list(&GAP_AT_0_3.list);
    let r = capture(clear_all);
    let s = &r.seen;
    assert_eq!((r.error, r.refused), (GL_NO_ERROR, 0), "{:?}", s.log);
    assert_eq!((s.jobs.len(), s.clears.len(), s.draws.len()), (1, 1, 0), "{:?}", s.log);
    assert_eq!((read(0), read(3)), (clear_bytes(), clear_bytes()));
    assert_eq!((read(1), read(2)), ([0; 4], [0; 4]), "attachments 1 and 2 are not draw buffers");
}

/// glClearBuffer of a draw buffer that is GL_NONE (1 or 2) clears nothing
/// (ES 3.0 4.2.3), and raises no error.
#[test]
fn a0_none_none_a3_clearbuffer_of_a_none_slot_clears_nothing() {
    let _g = rig();
    let fb = Fb::new(&[0, 1, 2, 3]);
    fb.list(&GAP_AT_0_3.list);
    for slot in [1, 2] {
        let r = capture(|| clear_buffer(slot));
        assert_eq!((r.error, r.refused), (GL_NO_ERROR, 0), "draw buffer {slot}: {:?}", r.seen.log);
        for att in 0..4 {
            assert_eq!(read(att), [0; 4], "draw buffer {slot}: attachment {att}");
        }
    }
}

/* ---- the lists that start at a later attachment: cleared, draw refused -- */

/// `{NONE,NONE,NONE,A3}`, `{NONE,A1,NONE,A3}` and a framebuffer whose only
/// colour attachment is 3 have `out_shift` 3, 1 and 3: the back end refuses
/// the draw ("draw buffer 0 is GL_NONE while a later draw buffer is
/// written") and records nothing for it.
#[test]
fn lists_with_draw_buffer_0_none_refuse_the_draw() {
    let _g = rig();
    for sh in &SHIFTED_LISTS {
        let fb = Fb::new(sh.attached);
        fb.list(&sh.list);
        let p = program(&[0, 1, 2, 3]);
        let r = capture(|| draw(p));
        assert!(r.seen.draws.is_empty(), "{}: nothing drawn", sh.name);
        assert_eq!(r.refused, 1, "{}: refused", sh.name);
        assert_eq!(r.error, GL_INVALID_OPERATION, "{}: the refusal is a GL error", sh.name);
        assert_eq!(read(3), [0; 4], "{}: attachment 3 untouched", sh.name);
    }
}

/// A clear of a shifted list is not refused (a clear has no fragment
/// outputs, so the job's RT k is the list's k-th target): glClear clears the
/// attachments the list writes and leaves the others alone, glClearBuffer of
/// a GL_NONE draw buffer clears nothing.
#[test]
fn lists_with_draw_buffer_0_none_clear_their_draw_buffers() {
    let _g = rig();
    for sh in &SHIFTED_LISTS {
        let fb = Fb::new(sh.attached);
        fb.list(&sh.list);
        let r = capture(|| clear_buffer(0));
        assert_eq!((r.refused, r.error), (0, GL_NO_ERROR), "{}: clearbuffer 0: {:?}", sh.name, r.seen.log);
        for &att in sh.attached {
            assert_eq!(read(att), [0; 4], "{}: clearbuffer of NONE draw buffer 0, attachment {att}", sh.name);
        }
        let r = capture(clear_all);
        assert_eq!((r.refused, r.error), (0, GL_NO_ERROR), "{}: glClear: {:?}", sh.name, r.seen.log);
        assert_eq!((r.seen.jobs.len(), r.seen.clears.len()), (1, 1), "{}: one clear", sh.name);
        for &att in sh.attached {
            let want = if sh.list[att] == A[att] { clear_bytes() } else { [0; 4] };
            assert_eq!(read(att), want, "{}: glClear, attachment {att}", sh.name);
        }
        /* scissored: the clear quad (`clear_mrt`, one output per RT of the
         * job), not refused either. The stand-in GPU does not run draws, so
         * only that it is recorded is checked here. */
        let r = capture(|| {
            // SAFETY: plain GL calls.
            unsafe {
                glEnable(GL_SCISSOR_TEST);
                glScissor(1, 1, 2, 2);
            }
            clear_all();
            // SAFETY: plain GL call.
            unsafe { glDisable(GL_SCISSOR_TEST) };
        });
        assert_eq!((r.refused, r.error), (0, GL_NO_ERROR), "{}: scissored glClear: {:?}", sh.name, r.seen.log);
        assert_eq!(r.seen.draws.len(), 1, "{}: the clear quad", sh.name);
    }
}

/// A glClear left pending and then a draw into a shifted list: the GL core
/// fuses the clear into the draw, and the back end refuses the draw (R-09),
/// so the fused clear goes with it. Pins today's answer until the draw is
/// drawn (`defect_shifted_lists_draw_attachment_3`).
#[test]
fn a_clear_fused_into_a_refused_shifted_draw_is_refused_with_it() {
    let _g = rig();
    let sh = &SHIFTED_LISTS[0];
    let fb = Fb::new(sh.attached);
    fb.list(&sh.list);
    let p = program(&[3]);
    let r = capture(|| {
        clear_all();
        draw(p);
    });
    assert_eq!((r.refused, r.error, r.seen.draws.len()), (1, GL_INVALID_OPERATION, 0), "{:?}", r.seen.log);
    assert_eq!(read(3), [0; 4], "the fused clear was not recorded");
}

/* ---- ES 3.0's answer for those lists ------------------------------------ */

/// ES 3.0 4.2.1: a draw into `{NONE,NONE,NONE,A3}` writes attachment 3, the
/// fragment's `location = 3` output. The job stores attachment 3.
#[test]
#[ignore = "defect: the V3D refuses a draw into a glDrawBuffers list whose draw buffer 0 is GL_NONE (src/v3d/state.rs refusal, review R-09; the job's RTs must be remapped to draw-buffer order in src/v3d/backend.rs extra_targets/Target first)"]
fn defect_shifted_lists_draw_attachment_3() {
    let _g = rig();
    for sh in &SHIFTED_LISTS {
        let fb = Fb::new(sh.attached);
        let want = (sh.attached.len() == 4).then(|| attachment_addresses(&fb)[3]);
        fb.list(&sh.list);
        let p = program(&[0, 1, 2, 3]);
        let r = capture(|| draw(p));
        assert_eq!((r.refused, r.error, r.seen.draws.len()), (0, GL_NO_ERROR, 1), "{}: {:?}", sh.name, r.seen.log);
        let at = rt_addresses(&r.seen.jobs[0]);
        assert!(
            at.iter().any(|&a| a != 0 && want.is_none_or(|w| w == a)),
            "{}: a job that stores attachment 3",
            sh.name
        );
    }
}

/// ES 3.0 4.2.3: glClearBuffer of draw buffer 3 clears attachment 3, and
/// glReadBuffer(GL_COLOR_ATTACHMENT3) reads the colour back.
#[test]
fn shifted_lists_clear_and_read_back_attachment_3() {
    let _g = rig();
    for sh in &SHIFTED_LISTS {
        let fb = Fb::new(sh.attached);
        fb.list(&sh.list);
        let r = capture(|| clear_buffer(3));
        assert_eq!((r.refused, r.error), (0, GL_NO_ERROR), "{}: {:?}", sh.name, r.seen.log);
        assert_eq!(read(3), clear_bytes(), "{}: clearbuffer", sh.name);
    }
}
