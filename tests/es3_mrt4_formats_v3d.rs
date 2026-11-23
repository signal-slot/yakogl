// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Every colour-renderable ES 3.0 format on the fourth V3D colour attachment
//! (render target 3), RGBA8 on attachments 0-2, on the host (plan #368, item
//! v3d-fmt4-test).
//!
//! The harness is tests/v3d_clear_mrt4.rs's: the real GL core and the real V3D
//! back end with the job builder stood in for by a recorder, under the
//! four-target stand-in `backend::HOST_DRAW_TARGETS` = 4 (`rig` sets it and
//! puts 0 back). Not repeated here: the clears of draw buffer 3 by class
//! (tests/v3d_clear_mrt4.rs). The tile size and blend of four targets are
//! es3_mrt4_tiles_v3d.rs's and es3_mrt4_blend_v3d.rs's where they exist.
//!
//! Per format, the framebuffer {RGBA8, RGBA8, RGBA8, F} with all four draw
//! buffers and a glClearBuffer of draw buffer 3 is pinned for:
//!
//! - completeness, and the formats the GL core refuses as renderbuffer
//!   storage today (RGBA32F, RG32F, R11F_G11F_B10F: GL_EXT_color_buffer_float
//!   is unlisted);
//! - the words the back end hands the job builder for RT 3, written out from
//!   Mesa 26.2.1 (`v3dX(get_internal_type_bpp_for_output_format)`,
//!   `clamp_for_format_and_type`, `v3d_tlb_clear`, docs/notes/mrt4-v3d-pvr.md
//!   section 1, docs/notes/v3d-fragment-out.md): four render targets, a typed
//!   job with `rt_format[3]` the output image format, the internal type, bpp
//!   and clamp of `state::rt_row`, the tile bpp `v3d_frame_tiling_bpp` is asked
//!   for (RT 3 alone raises it above RGBA8's 32), RT 3's raster stride, RTs
//!   0-2 loaded and RT 3 cleared by the TLB with the words of its class;
//! - glReadBuffer / glReadPixels of attachment 3: RT 3's memory is seeded
//!   after the clear (the recorder draws nothing) and must come back by the
//!   read type, so the read is RT 3's own address and row.
//!
//! What a host test cannot show: that the TLB stores RT 3 (the simulator
//! reference is v3d-sim4's, silicon the board's). The C `rt_internal` table
//! (`driver/src/v3d/v3d_job.c`) is not reached; the Rust row it mirrors is.
//!
//! R32F is a colour target only with GL_EXT_color_buffer_float (ES 3.0
//! table 3.13), which the build does not list; `rig` stands in for a build
//! that does (`framebuffer::HOST_R32F_RT`) so the V3D's R32F row is still
//! pinned, and puts it back when it drops.
//!
//! The defects found are `#[ignore]`d with their owner:
//!
//! - GL core and V3D state: RGBA32F, RG32F and R11F_G11F_B10F are refused and
//!   `state::rt_format` has no row for them.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_mrt4_formats_v3d
//! ```
#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-v3d", not(feature = "report-es2")))]

use core::ffi::{c_char, c_int, c_void, CStr};
use std::collections::BTreeMap;
use std::ffi::CString;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::Ordering;
use std::sync::{Mutex, MutexGuard, Once, OnceLock};
use yakogl::backend::*;
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

#[derive(Default)]
struct Seen {
    jobs: Vec<V3dJobTarget>,
    clears: Vec<V3dJobClear>,
    draws: u32,
    log: Vec<String>,
    /// `v3d_frame_tiling_bpp` calls: (rts, msaa, max_bpp).
    tilings: Vec<(u32, bool, u32)>,
}

static SEEN: Mutex<Seen> =
    Mutex::new(Seen { jobs: Vec::new(), clears: Vec::new(), draws: 0, log: Vec::new(), tilings: Vec::new() });

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
unsafe extern "C" fn v3d_job_draw(j: *mut V3dJob, _d: *const V3dJobDraw, _layout: *mut c_void) -> i32 {
    // SAFETY: the backend passes its live job and a draw whose pointers live for the call.
    let j = unsafe { &mut *j };
    j.draws += 1;
    // SAFETY: the job's BCL is live; the draw's slices are as long as it says.
    unsafe {
        (*j.bcl).used += 120;
        seen(|s| s.draws += 1);
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
    seen(|s| s.tilings.push((rts, msaa, max_bpp)));
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
    yakogl::gl::framebuffer::HOST_R32F_RT.store(true, Ordering::Relaxed);
    Rig(g)
}

/// The rig's lock; the four-target and R32F stand-ins go back when it drops.
struct Rig(#[allow(dead_code)] MutexGuard<'static, ()>);

impl Drop for Rig {
    fn drop(&mut self) {
        yakogl::backend::HOST_DRAW_TARGETS.store(0, Ordering::Relaxed);
        yakogl::gl::framebuffer::HOST_R32F_RT.store(false, Ordering::Relaxed);
    }
}

/* ---- the scene: RGBA8 on 0-2, one format on 3 ------------------------------ */

const W: GLsizei = 24;
const H: GLsizei = 16;
/// Raw clear words for integer targets, and a float colour.
const WORDS: [u32; 4] = [0x7fff_fff1, 0x0000_0002, 0xffff_fffd, 0x0000_0004];
const FLOAT: [f32; 4] = [0.25, 0.5, 0.75, 1.0];

/// What the tile buffer is cleared from, by hand from Mesa's packing
/// (`v3d_tlb_clear`, v3dx_draw.c:1706-1769) of `FLOAT` and `WORDS`:
/// R8G8B8A8_UNORM of 0.25, 0.5, 0.75, 1.0 (64, 128, 191, 255);
const CLEAR_RGBA8: [u32; 4] = [0xffbf_8040, 0, 0, 0];
/// halves 0x3400, 0x3800, 0x3a00, 0x3c00, two a word;
const CLEAR_16F: [u32; 4] = [0x3800_3400, 0x3c00_3a00, 0, 0];
/// the low byte of each raw word, one word;
const CLEAR_8I: [u32; 4] = [0x04fd_02f1, 0, 0, 0];
/// the low 16 bits of each raw word, two a word;
const CLEAR_16I: [u32; 4] = [0x0002_fff1, 0x0004_fffd, 0, 0];
/// the raw words; and the float's bits.
const CLEAR_32I: [u32; 4] = WORDS;
const CLEAR_32F: [u32; 4] = [0x3e80_0000, 0x3f00_0000, 0x3f40_0000, 0x3f80_0000];

#[derive(Clone, Copy, PartialEq, Debug)]
enum Class {
    Float,
    Int,
    Uint,
}

/// One colour-renderable ES 3.0 internal format as the GL core takes it:
/// what it must become on render target 3 (Mesa 26.2.1's
/// `v3dX(get_internal_type_bpp_for_output_format)` and
/// `clamp_for_format_and_type`, docs/notes/mrt4-v3d-pvr.md section 1).
struct Fmt {
    name: &'static str,
    gl: GLenum,
    class: Class,
    /// The surface the back end must see; `None`: the GL core refuses the format.
    surf: Option<u8>,
    /// `V3D_OUTPUT_IMAGE_FORMAT_*` in `rt_format[3]` when the job is typed.
    output: u32,
    /// `V3D_INTERNAL_TYPE_*`, `V3D_INTERNAL_BPP_*`, `V3D_RENDER_TARGET_CLAMP_*`.
    internal: (u32, u32, u32),
    clear: [u32; 4],
}

const fn f(
    name: &'static str,
    gl: GLenum,
    class: Class,
    surf: u8,
    output: u32,
    internal: (u32, u32, u32),
    clear: [u32; 4],
) -> Fmt {
    Fmt { name, gl, class, surf: Some(surf), output, internal, clear }
}

/// A format the GL core refuses as a renderbuffer today.
const fn refused(name: &'static str, gl: GLenum, class: Class) -> Fmt {
    Fmt { name, gl, class, surf: None, output: 0, internal: (0, 0, 0), clear: [0; 4] }
}

use Class::{Float as Fl, Int as In, Uint as Ui};

const NONE: u32 = V3D_RENDER_TARGET_CLAMP_NONE;
const NORM: u32 = V3D_RENDER_TARGET_CLAMP_NORM;
const INT: u32 = V3D_RENDER_TARGET_CLAMP_INT;
const T8: (u32, u32, u32) = (V3D_INTERNAL_TYPE_8, V3D_INTERNAL_BPP_32, NONE);
const R8: u32 = V3D_OUTPUT_IMAGE_FORMAT_RGBA8;

/// RGBA8 held as one: the ES 2.0 targets, which the job keeps untyped
/// (`rt_format` is not read; Mesa's internal type 8 at 32 bpp).
const fn held8(name: &'static str, gl: GLenum) -> Fmt {
    f(name, gl, Fl, GLES_SURF_RGBA8888, R8, T8, CLEAR_RGBA8)
}

fn formats() -> Vec<Fmt> {
    vec![
        held8("RGBA8", GL_RGBA8),
        f(
            "SRGB8_ALPHA8",
            GL_SRGB8_ALPHA8,
            Fl,
            GLES_SURF_SRGB8_A8,
            V3D_OUTPUT_IMAGE_FORMAT_SRGB8_ALPHA8,
            (V3D_INTERNAL_TYPE_16F, V3D_INTERNAL_BPP_64, NORM),
            CLEAR_16F,
        ),
        held8("RGB565", GL_RGB565),
        held8("RGBA4", GL_RGBA4),
        held8("RGB5_A1", GL_RGB5_A1),
        f(
            "RGB10_A2",
            GL_RGB10_A2,
            Fl,
            GLES_SURF_RGB10_A2,
            V3D_OUTPUT_IMAGE_FORMAT_RGB10_A2,
            (V3D_INTERNAL_TYPE_16F, V3D_INTERNAL_BPP_64, NONE),
            CLEAR_16F,
        ),
        f(
            "RGB10_A2UI",
            GL_RGB10_A2UI,
            Ui,
            GLES_SURF_RGB10_A2UI,
            V3D_OUTPUT_IMAGE_FORMAT_RGB10_A2UI,
            (V3D_INTERNAL_TYPE_16UI, V3D_INTERNAL_BPP_64, INT),
            CLEAR_16I,
        ),
        held8("R8", GL_R8),
        held8("RG8", GL_RG8),
        f("R8I", GL_R8I, In, GLES_SURF_R8I, V3D_OUTPUT_IMAGE_FORMAT_R8I, (V3D_INTERNAL_TYPE_8I, 0, INT), CLEAR_8I),
        f("R8UI", GL_R8UI, Ui, GLES_SURF_R8UI, V3D_OUTPUT_IMAGE_FORMAT_R8UI, (V3D_INTERNAL_TYPE_8UI, 0, INT), CLEAR_8I),
        f("RG8I", GL_RG8I, In, GLES_SURF_RG8I, V3D_OUTPUT_IMAGE_FORMAT_RG8I, (V3D_INTERNAL_TYPE_8I, 0, INT), CLEAR_8I),
        f(
            "RG8UI",
            GL_RG8UI,
            Ui,
            GLES_SURF_RG8UI,
            V3D_OUTPUT_IMAGE_FORMAT_RG8UI,
            (V3D_INTERNAL_TYPE_8UI, 0, INT),
            CLEAR_8I,
        ),
        f(
            "RGBA8I",
            GL_RGBA8I,
            In,
            GLES_SURF_RGBA8I,
            V3D_OUTPUT_IMAGE_FORMAT_RGBA8I,
            (V3D_INTERNAL_TYPE_8I, 0, INT),
            CLEAR_8I,
        ),
        f(
            "RGBA8UI",
            GL_RGBA8UI,
            Ui,
            GLES_SURF_RGBA8UI,
            V3D_OUTPUT_IMAGE_FORMAT_RGBA8UI,
            (V3D_INTERNAL_TYPE_8UI, 0, INT),
            CLEAR_8I,
        ),
        f(
            "R16I",
            GL_R16I,
            In,
            GLES_SURF_R16I,
            V3D_OUTPUT_IMAGE_FORMAT_R16I,
            (V3D_INTERNAL_TYPE_16I, 0, INT),
            CLEAR_16I,
        ),
        f(
            "R16UI",
            GL_R16UI,
            Ui,
            GLES_SURF_R16UI,
            V3D_OUTPUT_IMAGE_FORMAT_R16UI,
            (V3D_INTERNAL_TYPE_16UI, 0, INT),
            CLEAR_16I,
        ),
        f(
            "RG16I",
            GL_RG16I,
            In,
            GLES_SURF_RG16I,
            V3D_OUTPUT_IMAGE_FORMAT_RG16I,
            (V3D_INTERNAL_TYPE_16I, 0, INT),
            CLEAR_16I,
        ),
        f(
            "RG16UI",
            GL_RG16UI,
            Ui,
            GLES_SURF_RG16UI,
            V3D_OUTPUT_IMAGE_FORMAT_RG16UI,
            (V3D_INTERNAL_TYPE_16UI, 0, INT),
            CLEAR_16I,
        ),
        f(
            "RGBA16I",
            GL_RGBA16I,
            In,
            GLES_SURF_RGBA16I,
            V3D_OUTPUT_IMAGE_FORMAT_RGBA16I,
            (V3D_INTERNAL_TYPE_16I, 1, INT),
            CLEAR_16I,
        ),
        f(
            "RGBA16UI",
            GL_RGBA16UI,
            Ui,
            GLES_SURF_RGBA16UI,
            V3D_OUTPUT_IMAGE_FORMAT_RGBA16UI,
            (V3D_INTERNAL_TYPE_16UI, 1, INT),
            CLEAR_16I,
        ),
        f(
            "R32I",
            GL_R32I,
            In,
            GLES_SURF_R32I,
            V3D_OUTPUT_IMAGE_FORMAT_R32I,
            (V3D_INTERNAL_TYPE_32I, 0, INT),
            CLEAR_32I,
        ),
        f(
            "R32UI",
            GL_R32UI,
            Ui,
            GLES_SURF_R32UI,
            V3D_OUTPUT_IMAGE_FORMAT_R32UI,
            (V3D_INTERNAL_TYPE_32UI, 0, INT),
            CLEAR_32I,
        ),
        f(
            "RG32I",
            GL_RG32I,
            In,
            GLES_SURF_RG32I,
            V3D_OUTPUT_IMAGE_FORMAT_RG32I,
            (V3D_INTERNAL_TYPE_32I, 1, INT),
            CLEAR_32I,
        ),
        f(
            "RG32UI",
            GL_RG32UI,
            Ui,
            GLES_SURF_RG32UI,
            V3D_OUTPUT_IMAGE_FORMAT_RG32UI,
            (V3D_INTERNAL_TYPE_32UI, 1, INT),
            CLEAR_32I,
        ),
        f(
            "RGBA32I",
            GL_RGBA32I,
            In,
            GLES_SURF_RGBA32I,
            V3D_OUTPUT_IMAGE_FORMAT_RGBA32I,
            (V3D_INTERNAL_TYPE_32I, 2, INT),
            CLEAR_32I,
        ),
        f(
            "RGBA32UI",
            GL_RGBA32UI,
            Ui,
            GLES_SURF_RGBA32UI,
            V3D_OUTPUT_IMAGE_FORMAT_RGBA32UI,
            (V3D_INTERNAL_TYPE_32UI, 2, INT),
            CLEAR_32I,
        ),
        refused("R11F_G11F_B10F", GL_R11F_G11F_B10F, Fl),
        f(
            "R16F",
            GL_R16F,
            Fl,
            GLES_SURF_R16F,
            V3D_OUTPUT_IMAGE_FORMAT_R16F,
            (V3D_INTERNAL_TYPE_16F, 1, NONE),
            CLEAR_16F,
        ),
        /* the GL core holds RG16F as RGBA16F (`renderbuffer_surf_format`),
         * on attachment 3 as on attachment 0 */
        f(
            "RG16F",
            GL_RG16F,
            Fl,
            GLES_SURF_RGBA16F,
            V3D_OUTPUT_IMAGE_FORMAT_RGBA16F,
            (V3D_INTERNAL_TYPE_16F, 1, NONE),
            CLEAR_16F,
        ),
        f(
            "RGBA16F",
            GL_RGBA16F,
            Fl,
            GLES_SURF_RGBA16F,
            V3D_OUTPUT_IMAGE_FORMAT_RGBA16F,
            (V3D_INTERNAL_TYPE_16F, 1, NONE),
            CLEAR_16F,
        ),
        f(
            "R32F",
            GL_R32F,
            Fl,
            GLES_SURF_R32F,
            V3D_OUTPUT_IMAGE_FORMAT_R32F,
            (V3D_INTERNAL_TYPE_32F, 0, NONE),
            CLEAR_32F,
        ),
        refused("RG32F", GL_RG32F, Fl),
        refused("RGBA32F", GL_RGBA32F, Fl),
    ]
}

fn logs() -> Vec<String> {
    seen(|s| s.log.clone())
}

/// What one scene left.
struct Run {
    storage: GLenum,
    status: GLenum,
    error: GLenum,
    read_error: GLenum,
    /// The read-back of attachment 3, `W * H` pixels of the class's read type.
    read: Vec<u8>,
    /// The bytes written into RT 3's memory before the read (the pattern).
    seeded: Vec<u8>,
    seen: Seen,
}

/// Four renderbuffers, RGBA8 on 0-2 and `format` on 3, all four draw buffers;
/// glClearBuffer of draw buffer 3 when the framebuffer is complete, then a
/// read-back of attachment 3 over RT 3's memory seeded with a pattern (the
/// job builder here records and draws nothing, so the memory holds only what
/// this puts there).
fn run(format: GLenum, class: Class) -> Run {
    let mut rbs = [0; 4];
    let mut fb = 0;
    let mut read = vec![0xa5u8; W as usize * H as usize * 16];
    let (mut storage, status, error, read_error);
    let mut seeded = Vec::new();
    // SAFETY: plain GL calls with room for the names, the clear values and the read-back.
    unsafe {
        glGenRenderbuffers(4, rbs.as_mut_ptr());
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        while glGetError() != GL_NO_ERROR {}
        storage = GL_NO_ERROR;
        for (i, &rb) in rbs.iter().enumerate() {
            glBindRenderbuffer(GL_RENDERBUFFER, rb);
            glRenderbufferStorage(GL_RENDERBUFFER, if i == 3 { format } else { GL_RGBA8 }, W, H);
            storage = storage.max(glGetError());
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0 + i as GLenum, GL_RENDERBUFFER, rb);
        }
        let bufs: [GLenum; 4] =
            [GL_COLOR_ATTACHMENT0, GL_COLOR_ATTACHMENT1, GL_COLOR_ATTACHMENT2, GL_COLOR_ATTACHMENT3];
        glDrawBuffers(4, bufs.as_ptr());
        status = glCheckFramebufferStatus(GL_FRAMEBUFFER);
        glViewport(0, 0, W, H);
        while glGetError() != GL_NO_ERROR {}
        if status == GL_FRAMEBUFFER_COMPLETE {
            match class {
                Class::Float => glClearBufferfv(GL_COLOR, 3, FLOAT.as_ptr()),
                Class::Int => glClearBufferiv(GL_COLOR, 3, WORDS.as_ptr() as *const GLint),
                Class::Uint => glClearBufferuiv(GL_COLOR, 3, WORDS.as_ptr()),
            }
        }
        glFinish();
        error = glGetError();
        if status == GL_FRAMEBUFFER_COMPLETE {
            let rt3 = seen(|s| s.jobs.first().map(|t| t.rt[2]));
            if let Some(rt3) = rt3.filter(|r| r.address != 0) {
                seeded = seed(u64::from(rt3.address), rt3.stride as usize * H as usize);
            }
        }
        glReadBuffer(GL_COLOR_ATTACHMENT3);
        let (fmt, ty) = match class {
            Class::Float => (GL_RGBA, GL_UNSIGNED_BYTE),
            Class::Int => (GL_RGBA_INTEGER, GL_INT),
            Class::Uint => (GL_RGBA_INTEGER, GL_UNSIGNED_INT),
        };
        glReadPixels(0, 0, W, H, fmt, ty, read.as_mut_ptr() as *mut c_void);
        read_error = glGetError();
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fb);
        glDeleteRenderbuffers(4, rbs.as_ptr());
    }
    Run { storage, status, error, read_error, read, seeded, seen: seen(core::mem::take) }
}

/// Put a pattern into `n` bytes of the platform memory at `gpu`; hands it back.
fn seed(gpu: u64, n: usize) -> Vec<u8> {
    let pat: Vec<u8> = (0..n).map(|i| (i as u32).wrapping_mul(37).wrapping_add(11) as u8).collect();
    with_platform(|p| {
        let (base, a) = p.allocs.range_mut(..=gpu).next_back().expect("RT 3's memory");
        let off = (gpu - base) as usize;
        // SAFETY: the allocation's own bytes, a u64 slice viewed as bytes.
        let bytes = unsafe { core::slice::from_raw_parts_mut(a.mem.as_mut_ptr() as *mut u8, a.size) };
        bytes[off..off + n].copy_from_slice(&pat);
    });
    pat
}

/// The TLB's clear of RT n in a job: loaded (`true`) or cleared.
fn loaded(c: &V3dJobClear, n: usize) -> bool {
    if n == 0 {
        c.load_rt0
    } else {
        c.load_rt[n - 1]
    }
}

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
    // Read under the lock with no rig alive: a rig stores 0 before its guard
    // releases the lock, so this is what the last rig left. Without the lock
    // the next test's rig may have stored its 4 already.
    let _l = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    assert_eq!(yakogl::backend::HOST_DRAW_TARGETS.load(Ordering::Relaxed), 0);
}

/// Whether `name` is in glGetString(GL_EXTENSIONS).
fn extension_listed(name: &str) -> bool {
    // SAFETY: glGetString returns a NUL-terminated static string.
    let s = unsafe { CStr::from_ptr(glGetString(GL_EXTENSIONS).cast::<c_char>()) }.to_string_lossy().into_owned();
    s.split_whitespace().any(|e| e == name)
}

/// The formats the GL core takes as a colour renderbuffer on attachment 3.
fn accepted() -> Vec<Fmt> {
    formats().into_iter().filter(|fm| fm.surf.is_some()).collect()
}

/// The pixels `glReadPixels` must give for a signed or unsigned integer
/// format whose memory holds `mem` (little-endian components, `n` of them,
/// `cs` bytes each), read as RGBA_INTEGER: missing channels 0, alpha 1.
fn integer_pixels(mem: &[u8], n: usize, cs: usize, signed: bool) -> Vec<u8> {
    let mut out = Vec::new();
    for p in 0..(W * H) as usize {
        for ch in 0..4 {
            let v: u32 = if ch < n {
                let b = &mem[(p * n + ch) * cs..][..cs];
                let raw = b.iter().rev().fold(0u32, |a, &x| (a << 8) | u32::from(x));
                if signed && cs < 4 && raw >> (8 * cs - 1) != 0 {
                    raw | (!0u32 << (8 * cs))
                } else {
                    raw
                }
            } else {
                u32::from(ch == 3)
            };
            out.extend_from_slice(&v.to_le_bytes());
        }
    }
    out
}

/// Components a pixel and bytes a component of an integer format by name.
fn integer_shape(name: &str) -> (usize, usize) {
    let n = if name.starts_with("RGBA") {
        4
    } else if name.starts_with("RG") {
        2
    } else {
        1
    };
    let bits: usize =
        name.trim_start_matches(char::is_alphabetic).trim_end_matches(char::is_alphabetic).parse().unwrap();
    (n, bits / 8)
}

/* ---- completeness ----------------------------------------------------------- */

/// RGBA8 on 0-2 and each format on 3: complete, the storage call without
/// error, for every format the GL core takes; the three it refuses today are
/// `float_formats_beyond_r32f_are_refused_today`.
#[test]
fn every_accepted_format_on_attachment_3_is_complete() {
    let _g = rig();
    for fm in accepted() {
        let r = run(fm.gl, fm.class);
        assert_eq!(r.storage, GL_NO_ERROR, "{}: storage", fm.name);
        assert_eq!(r.status, GL_FRAMEBUFFER_COMPLETE, "{}: {:?}", fm.name, logs());
        assert_eq!(r.error, GL_NO_ERROR, "{}: glClearBuffer of draw buffer 3: {:?}", fm.name, logs());
    }
}

/// GL_RGBA32F, GL_RG32F and GL_R11F_G11F_B10F are not accepted as renderbuffer
/// storage (GL_INVALID_ENUM, the framebuffer's attachment 3 incomplete, no
/// job, GL_INVALID_FRAMEBUFFER_OPERATION from the read), where GL_R32F is.
#[test]
fn float_formats_beyond_r32f_are_refused_today() {
    let _g = rig();
    for fm in formats().iter().filter(|fm| fm.surf.is_none()) {
        let r = run(fm.gl, fm.class);
        assert_eq!(r.storage, GL_INVALID_ENUM, "{}: storage", fm.name);
        assert_eq!(r.status, GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT, "{}", fm.name);
        assert!(r.seen.jobs.is_empty() && r.seen.clears.is_empty(), "{}: no job: {:?}", fm.name, r.seen.log);
        assert_eq!(r.read_error, GL_INVALID_FRAMEBUFFER_OPERATION, "{}: read of an incomplete framebuffer", fm.name);
    }
}

/* ---- the words of render target 3 ------------------------------------------ */

/// One job of four render targets whose RT 3 is the format: Mesa's output
/// image format in `rt_format[3]` (RGBA8 in 0-2) when the job is typed, the
/// surface's internal type, bpp and clamp, the tile size's bpp the four-RT
/// frame asks `v3d_frame_tiling_bpp` for, RT 3's raster address and stride,
/// and the TLB clear of RT 3 alone (RTs 0-2 loaded).
fn assert_rt3_words(fm: &Fmt, r: &Run) {
    let (name, s) = (fm.name, &r.seen);
    assert_eq!((s.jobs.len(), s.clears.len(), s.draws), (1, 1, 0), "{name}: one job, a TLB clear: {:?}", s.log);
    let t = &s.jobs[0];
    assert_eq!(t.rts, 4, "{name}: four render targets");
    let typed = fm.output != R8 || fm.internal != T8;
    assert_eq!(t.typed, typed, "{name}: a job with an ES 3.0 format on RT 3 is typed");
    if typed {
        assert_eq!(t.rt_format, [R8, R8, R8, fm.output], "{name}: output image formats of RT 0-3");
    }
    let surf = fm.surf.unwrap();
    let row = yakogl::v3d::state::rt_row(surf);
    assert_eq!(
        (row.output, (row.internal_type, row.internal_bpp, row.clamp)),
        (fm.output, fm.internal),
        "{name}: Mesa's row"
    );
    assert!(!s.tilings.is_empty(), "{name}: the tile size was asked for");
    for &(rts, msaa, bpp) in &s.tilings {
        assert_eq!((rts, msaa, bpp), (4, false, fm.internal.1), "{name}: four RTs, the largest bpp is RT 3's");
    }
    let bytes = if t.rt[2].address != 0 { t.rt[2].stride / W as u32 } else { 0 };
    assert_eq!(bytes, row.bytes, "{name}: RT 3's raster row is W pixels of the format");
    let c = &s.clears[0];
    for n in 0..3 {
        assert!(loaded(c, n), "{name}: RT {n} is loaded, only draw buffer 3 was cleared");
    }
    assert!(!loaded(c, 3), "{name}: RT 3 is cleared by the TLB");
    assert_eq!(c.clear_rt[3], fm.clear, "{name}: RT 3's clear words");
}

#[test]
fn rt3_words_of_every_format() {
    let _g = rig();
    for fm in &accepted() {
        assert_rt3_words(fm, &run(fm.gl, fm.class));
    }
}

/// The tile bpp: RGBA8 on 0-2 is 32 bpp, so RT 3 alone sets the largest and
/// with it the tile size's step (docs/notes/mrt4-v3d-pvr.md section 1).
#[test]
fn rt3_alone_sets_the_tile_bpp() {
    let _g = rig();
    let want = |name: &str| match name {
        "SRGB8_ALPHA8" | "RGB10_A2" | "RGB10_A2UI" | "RGBA16I" | "RGBA16UI" | "RG32I" | "RG32UI" | "R16F" | "RG16F"
        | "RGBA16F" => 1,
        "RGBA32I" | "RGBA32UI" => 2,
        _ => 0,
    };
    for fm in &accepted() {
        let r = run(fm.gl, fm.class);
        let bpp = r.seen.tilings.last().map(|t| t.2);
        assert_eq!(bpp, Some(want(fm.name)), "{}", fm.name);
    }
}

/* ---- the read-back of attachment 3 ------------------------------------------ */

/// glReadBuffer(GL_COLOR_ATTACHMENT3) + glReadPixels reads RT 3's own
/// memory, at its address and stride, whatever the format: with the memory
/// seeded after the clear (this job builder draws nothing), the bytes of an
/// RGBA8-held and sRGB target come back as they are.
#[test]
fn readback_of_attachment_3_reads_rt3s_memory_as_bytes() {
    let _g = rig();
    for fm in accepted()
        .iter()
        .filter(|fm| matches!(fm.name, "RGBA8" | "SRGB8_ALPHA8" | "RGB565" | "RGBA4" | "RGB5_A1" | "R8" | "RG8"))
    {
        let r = run(fm.gl, fm.class);
        assert_eq!(r.read_error, GL_NO_ERROR, "{}", fm.name);
        assert_eq!(r.read[..r.seeded.len()], r.seeded[..], "{}: the bytes of RT 3", fm.name);
    }
}

/// The integer formats read through RGBA_INTEGER: each component widened
/// (sign-extended for the signed ones), missing channels 0 and alpha 1.
#[test]
fn readback_of_attachment_3_integer_formats() {
    let _g = rig();
    for fm in accepted().iter().filter(|fm| fm.class != Class::Float && fm.name != "RGB10_A2UI") {
        let r = run(fm.gl, fm.class);
        assert_eq!(r.read_error, GL_NO_ERROR, "{}", fm.name);
        let (n, cs) = integer_shape(fm.name);
        assert_eq!(r.seeded.len(), n * cs * (W * H) as usize, "{}: RT 3's memory", fm.name);
        let want = integer_pixels(&r.seeded, n, cs, fm.class == Class::Int);
        assert_eq!(r.read[..want.len()], want[..], "{}", fm.name);
    }
}

/// The float formats read without error; R16F and R32F, which lack G, B and
/// A, give G = B = 0 and A = 1 (255 as bytes).
#[test]
fn readback_of_attachment_3_float_formats() {
    let _g = rig();
    for fm in accepted().iter().filter(|fm| matches!(fm.name, "R16F" | "RGBA16F" | "R32F" | "RG16F")) {
        let r = run(fm.gl, fm.class);
        assert_eq!(r.read_error, GL_NO_ERROR, "{}", fm.name);
        if matches!(fm.name, "R16F" | "R32F") {
            for px in r.read[..(W * H) as usize * 4].chunks(4) {
                assert_eq!(px[1..], [0, 0, 255], "{}", fm.name);
            }
        }
    }
}

/// RGB10_A2 read as RGBA/UNSIGNED_BYTE returns each field scaled to a byte
/// (ES 3.0 4.3.1), not the packed word.
#[test]
fn readback_of_attachment_3_rgb10_a2_converts_the_fields() {
    let _g = rig();
    let fm = accepted().into_iter().find(|fm| fm.name == "RGB10_A2").unwrap();
    let r = run(fm.gl, fm.class);
    assert_eq!(r.read_error, GL_NO_ERROR);
    let f = |v: u32, max: u32| ((v * 255 + max / 2) / max) as u8;
    for (p, px) in r.read[..(W * H) as usize * 4].chunks(4).enumerate() {
        let w = u32::from_le_bytes(r.seeded[p * 4..p * 4 + 4].try_into().unwrap());
        let want = [f(w & 0x3ff, 1023), f((w >> 10) & 0x3ff, 1023), f((w >> 20) & 0x3ff, 1023), f(w >> 30, 3)];
        assert_eq!(px, want, "pixel {p}");
    }
}

/// RGB10_A2UI reads as RGBA_INTEGER / UNSIGNED_INT: the packed fields of
/// RT 3's words, widened.
#[test]
fn readback_of_attachment_3_rgb10_a2ui() {
    let _g = rig();
    let r = run(GL_RGB10_A2UI, Class::Uint);
    assert_eq!(r.read_error, GL_NO_ERROR);
    for (p, px) in r.read[..(W * H) as usize * 16].chunks(16).enumerate() {
        let w = u32::from_le_bytes(r.seeded[p * 4..p * 4 + 4].try_into().unwrap());
        let want = [w & 0x3ff, (w >> 10) & 0x3ff, (w >> 20) & 0x3ff, w >> 30];
        let got: Vec<u32> = px.chunks(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect();
        assert_eq!(got, want, "pixel {p}");
    }
}

/* ---- the stand-in ---------------------------------------------------------- */

/// With the default of 3 (no stand-in) attachment 3 is not a colour
/// attachment: the framebuffer call is refused, whatever the format.
#[test]
fn without_the_stand_in_attachment_3_is_refused() {
    let _g = rig();
    yakogl::backend::HOST_DRAW_TARGETS.store(0, Ordering::Relaxed);
    let mut rb = 0;
    // SAFETY: plain GL calls.
    let e = unsafe {
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorage(GL_RENDERBUFFER, GL_RGBA8, W, H);
        let mut fb = 0;
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        while glGetError() != GL_NO_ERROR {}
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT3, GL_RENDERBUFFER, rb);
        let e = glGetError();
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fb);
        glDeleteRenderbuffers(1, &rb);
        e
    };
    assert_eq!(e, GL_INVALID_ENUM);
}

/// GL_EXT_color_buffer_float is not listed, which is what makes those three
/// refusals ES 3.0's (core ES 3.0 renders no 32-bit float and no packed
/// float format; GL_R32F is accepted here only through `rig`'s stand-in).
#[test]
fn the_float_extension_is_not_listed_today() {
    let _g = rig();
    assert!(!extension_listed("GL_EXT_color_buffer_float"));
}

/// The three formats the task names as colour-renderable with
/// GL_EXT_color_buffer_float (RGBA32F, RG32F, R11F_G11F_B10F) should complete
/// on attachment 3 once the build lists the extension. The V3D side then needs
/// rows too: `state::rt_format` has none for them, V3D 4.2 has the output
/// formats RGBA32F 9, RG32F 10 and R11F_G11F_B10F 8.
#[test]
#[ignore = "gl-core + v3d state: RGBA32F, RG32F and R11F_G11F_B10F are refused as renderbuffer storage (GL_EXT_color_buffer_float unlisted) and state::rt_format has no V3D row for them"]
fn float_formats_beyond_r32f_are_renderable() {
    let _g = rig();
    for fm in formats().iter().filter(|fm| fm.surf.is_none()) {
        let r = run(fm.gl, fm.class);
        assert_eq!((r.storage, r.status), (GL_NO_ERROR, GL_FRAMEBUFFER_COMPLETE), "{}", fm.name);
    }
}
