// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! What the V3D back end hands the hardware today for the float colour
//! targets it draws (test-v3d-half), on the host: R16F, RG16F, RGB16F,
//! RGBA16F and R32F. The baseline of plan #360 (float targets): its v3d-arm
//! item adds RG32F, RGBA32F and R11F_G11F_B10F behind a cargo feature and
//! must not change any of these in a default build (the Pi 4's behaviour
//! must not move).
//!
//! Four things are pinned, each through the GL API where there is one:
//!
//! * the render-target config words of the job (`state::rt_format`: output
//!   image format, internal type, internal bpp, clamp), and the output format
//!   the GL core's renderbuffer puts in `V3dJobTarget::rt_format`;
//! * the bytes a pixel (`state::rt_format().bytes`, the GL core's
//!   `gles_surf_bytes_per_pixel`, and the job target's row stride);
//! * the TLB clear words of glClearBufferfv (`V3dJobClear::clear_rt`);
//! * the fragment shader's TLB writes of a `float`, `vec2` and `vec4` output
//!   (the compiler on the QPU interpreter, as tests/es3_int_rt_v3d.rs): the
//!   V3D program is not keyed by the target's format, so every float output is
//!   written as f16 pairs, for the 32F target too.
//!
//! Where today's value differs from Mesa 26.2.1 a comment states Mesa's, with
//! the reason (no `#[ignore]`d test: the gate's rows run none).
//!
//! The rig is tests/es3_clearbuffer_formats_v3d.rs': the job builder and the
//! platform stood in for, the real GL core and V3D back end.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_half_float_rt_v3d
//! ```
#![cfg(all(feature = "host-test", feature = "gpu-v3d", feature = "shaderc-online", not(feature = "report-es2")))]
#![allow(dead_code)]

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

#[derive(Default)]
struct Seen {
    jobs: Vec<V3dJobTarget>,
    clears: Vec<V3dJobClear>,
    draws: usize,
    log: Vec<String>,
}

static SEEN: Mutex<Seen> = Mutex::new(Seen { jobs: Vec::new(), clears: Vec::new(), draws: 0, log: Vec::new() });

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
    let (j, _d) = unsafe { (&mut *j, &*d) };
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

/* ---- one clear ---- */

const W: GLsizei = 24;
const H: GLsizei = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Call {
    Fv,
    Iv,
    Uiv,
}

/// What an unclearable format answers: the first GL error or the
/// framebuffer status of the attempt, by name.
#[derive(Debug, PartialEq, Eq)]
enum Refused {
    Storage(GLenum),
    Status(GLenum),
    Clear(GLenum),
}

/// A renderbuffer of `format` as the only attachment, `values` (four 32-bit
/// words: floats as bits for `Fv`) cleared with glClearBuffer*v on draw
/// buffer 0, flushed; what the job builder recorded.
fn clear(format: GLenum, call: Call, values: [u32; 4]) -> Result<Seen, Refused> {
    let (mut rb, mut fb) = (0, 0);
    // SAFETY: plain GL calls with room for one name each; the clear values are four words.
    let r = unsafe {
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorage(GL_RENDERBUFFER, format, W, H);
        let e = glGetError();
        let r = if e != GL_NO_ERROR {
            Err(Refused::Storage(e))
        } else {
            glGenFramebuffers(1, &mut fb);
            glBindFramebuffer(GL_FRAMEBUFFER, fb);
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb);
            let st = glCheckFramebufferStatus(GL_FRAMEBUFFER);
            if st != GL_FRAMEBUFFER_COMPLETE {
                Err(Refused::Status(st))
            } else {
                glViewport(0, 0, W, H);
                match call {
                    Call::Fv => glClearBufferfv(GL_COLOR, 0, values.as_ptr() as *const GLfloat),
                    Call::Iv => glClearBufferiv(GL_COLOR, 0, values.as_ptr() as *const GLint),
                    Call::Uiv => glClearBufferuiv(GL_COLOR, 0, values.as_ptr()),
                }
                let e = glGetError();
                if e != GL_NO_ERROR {
                    Err(Refused::Clear(e))
                } else {
                    glFinish();
                    Ok(())
                }
            }
        };
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fb);
        glDeleteRenderbuffers(1, &rb);
        while glGetError() != GL_NO_ERROR {}
        r
    };
    let s = seen(core::mem::take);
    r.map(|()| s)
}

/* ---- Mesa's packing, written out ---- */

/// `float_to_ubyte` (util/format_r11g11b10f.h's neighbour, u_math.h): what
/// `util_pack_color(.., PIPE_FORMAT_R8G8B8A8_UNORM)` does to one channel.
fn ubyte(x: f32) -> u32 {
    if x.is_nan() || x <= 0.0 {
        0
    } else if x >= 1.0 {
        255
    } else {
        (x * 255.0 + 0.5) as u32
    }
}

/// `_mesa_float_to_half`: round to nearest even, overflow to infinity, the
/// quiet NaN, denormals kept.
fn half(x: f32) -> u32 {
    let b = x.to_bits();
    let sign = (b >> 16) & 0x8000;
    let exp = ((b >> 23) & 0xff) as i32;
    let man = b & 0x7f_ffff;
    if exp == 0xff {
        return sign | if man != 0 { 0x7e00 } else { 0x7c00 };
    }
    let e = exp - 127 + 15;
    if e >= 31 {
        return sign | 0x7c00;
    }
    if e <= 0 {
        /* a denormal half (or zero): the 24-bit significand shifted down */
        if e < -10 {
            return sign;
        }
        let sig = man | 0x80_0000;
        let shift = (14 - e) as u32;
        let (q, rem) = (sig >> shift, sig & ((1 << shift) - 1));
        let half_way = 1 << (shift - 1);
        let q = if rem > half_way || (rem == half_way && q & 1 == 1) { q + 1 } else { q };
        return sign | q;
    }
    let (q, rem) = (((e as u32) << 10) | (man >> 13), man & 0x1fff);
    let q = if rem > 0x1000 || (rem == 0x1000 && q & 1 == 1) { q + 1 } else { q };
    sign | q
}

/// `v3d_tlb_clear`'s packing of the clear colour per internal type
/// (v3dx_draw.c:1706-1769), for a colour whose four channels arrive as the
/// words the application gave (floats as bits).
#[derive(Clone, Copy)]
enum Tile {
    /// 8-bit unorm: one R8G8B8A8 word.
    U8,
    /// 16F: four halves, two a word.
    F16,
    /// 8I / 8UI: the low byte of each word.
    I8,
    /// 16I / 16UI: the low half of each word, two a word.
    I16,
    /// 32I / 32UI / 32F: the words as they are.
    W32,
}

fn pack(t: Tile, c: [u32; 4]) -> [u32; 4] {
    match t {
        Tile::U8 => {
            let u = c.map(|w| ubyte(f32::from_bits(w)));
            [u[0] | u[1] << 8 | u[2] << 16 | u[3] << 24, 0, 0, 0]
        }
        Tile::F16 => {
            let h = c.map(|w| half(f32::from_bits(w)));
            [h[0] | h[1] << 16, h[2] | h[3] << 16, 0, 0]
        }
        Tile::I8 => [(c[0] & 0xff) | (c[1] & 0xff) << 8 | (c[2] & 0xff) << 16 | (c[3] & 0xff) << 24, 0, 0, 0],
        Tile::I16 => [(c[0] & 0xffff) | c[1] << 16, (c[2] & 0xffff) | c[3] << 16, 0, 0],
        Tile::W32 => c,
    }
}

/// The record of a clear of the whole target: the TLB's, never a quad.
fn assert_tlb(s: &Seen, name: &str, typed: bool) {
    assert_eq!(s.draws, 0, "{name}: no clear quad, the TLB clears: {:?}", s.log);
    assert_eq!((s.jobs.len(), s.clears.len()), (1, 1), "{name}: one job, one end: {:?}", s.log);
    assert_eq!(s.jobs[0].typed, typed, "{name}: typed job");
    assert!(!s.clears[0].load_rt0, "{name}: the colour is cleared, not loaded");
}

/// Floats: edge values of a fixed-point or float colour.
fn bits(v: [f32; 4]) -> [u32; 4] {
    v.map(f32::to_bits)
}

/// Negative, above one, NaN, in range.
const EDGE: [f32; 4] = [-1.0, 2.0, f32::NAN, 0.2];
/// In range, away from a rounding tie, alpha 0.6 (the same 8, 4 and 1 bit way).
const MID: [f32; 4] = [0.2, 0.8, 1.0, 0.6];
/// Half-float edges: the largest half, one past it, a value below the
/// smallest denormal's half, and the smallest normal.
const HALF_EDGE: [f32; 4] = [65504.0, 70000.0, 1.0e-8, 6.103_515_6e-5];
/// More half-float values: a value that rounds to even, a denormal, negative
/// zero, and minus infinity.
const HALF_MORE: [f32; 4] = [1.000_488_3, 3.0e-6, -0.0, f32::NEG_INFINITY];

/* ---- the compiler: float outputs to the TLB ---- */

use yakogl::backend::{gles_surf_bytes_per_pixel, gles_surf_name, GLES_SURF_R16F, GLES_SURF_R32F, GLES_SURF_RGBA16F};
use yakogl::shaderc::online::arena::Arena;
use yakogl::shaderc::online::v3d::chain::{compile_program, Prim, Target};
use yakogl::shaderc::online::v3d::select::tlb_config_f16;
use yakogl::shaderc::online::v3d::sim::{self, Env, FsInputs, Inputs, LANES};
use yakogl::shaderc::online::{lower, sema};
use yakogl::v3d::state;

/// The compiler keeps its output in one arena that every compile resets.
static COMPILING: Mutex<()> = Mutex::new(());

const VS: &str = "#version 300 es\nin vec4 p;\nvoid main() { gl_Position = p; }\n";

/// What the fragment shader wrote to the TLB for one pixel (lane 0): each
/// write's config (`Some` for a TLBU write) and value.
fn run_fs(fs: &str) -> Vec<(Option<u32>, u32)> {
    let _g = COMPILING.lock().unwrap_or_else(|e| e.into_inner());
    let mut mem = vec![0u8; 16 << 20];
    let a = Arena::new(&mut mem);
    let vs = a.alloc_str(VS).expect("arena");
    let fs = a.alloc_str(fs).expect("arena");
    let v = lower::compile_to_ir(&a, sema::Stage::Vertex, vs).map_err(|e| e.msg.to_string()).expect("the VS lowers");
    let f = lower::compile_to_ir(&a, sema::Stage::Fragment, fs).map_err(|e| e.msg.to_string()).expect("the FS lowers");
    let p = compile_program(&v.module, &f.module, Prim::Triangles, Target { vpm_size: 65536 }, &a)
        .unwrap_or_else(|e| panic!("the program compiles: {e:?}"));
    let uniforms: Vec<u32> =
        p.fs.code
            .uniforms
            .iter()
            .map(|u| match u.contents {
                0 => u.data,
                c => panic!("uniform contents {c} not modelled by this test"),
            })
            .collect();
    let inputs = FsInputs {
        w: [1.0f32.to_bits(); LANES],
        w_centroid: [1.0f32.to_bits(); LANES],
        z: [0; LANES],
        x: [0; LANES],
        y: [0; LANES],
        back_facing: [false; LANES],
        varyings: Vec::new(),
    };
    assert_eq!(p.fs.num_inputs, 0, "the shaders here read no varying");
    let env = Env { threads: p.fs.threads, ..Env::default() };
    let o = sim::run(p.fs.code.words, &uniforms, &Inputs::Fragment(Box::new(inputs)), &env)
        .unwrap_or_else(|e| panic!("the fragment program runs: {e}"));
    o.tlb.iter().map(|w| (w.config, w.value[0].expect("a defined colour word"))).collect()
}

fn fs_of(ty: &str, expr: &str) -> String {
    format!("#version 300 es\nprecision highp float;\nout {ty} o;\nvoid main() {{ o = {expr}; }}\n")
}

/* ---- the formats ---- */

/// A GL format, the surface the GL core draws it as, the output image format
/// of that surface and its bytes a pixel.
struct Row {
    gl: GLenum,
    name: &'static str,
    surf: u8,
    output: u32,
    bytes: u32,
}

const ROWS: [Row; 5] = [
    Row { gl: GL_R16F, name: "R16F", surf: GLES_SURF_R16F, output: V3D_OUTPUT_IMAGE_FORMAT_R16F, bytes: 2 },
    /* RG16F and RGB16F keep four halves (src/gl/framebuffer.rs:735-745) */
    Row { gl: GL_RG16F, name: "RG16F", surf: GLES_SURF_RGBA16F, output: V3D_OUTPUT_IMAGE_FORMAT_RGBA16F, bytes: 8 },
    Row { gl: GL_RGB16F, name: "RGB16F", surf: GLES_SURF_RGBA16F, output: V3D_OUTPUT_IMAGE_FORMAT_RGBA16F, bytes: 8 },
    Row { gl: GL_RGBA16F, name: "RGBA16F", surf: GLES_SURF_RGBA16F, output: V3D_OUTPUT_IMAGE_FORMAT_RGBA16F, bytes: 8 },
    Row { gl: GL_R32F, name: "R32F", surf: GLES_SURF_R32F, output: V3D_OUTPUT_IMAGE_FORMAT_R32F, bytes: 4 },
];

/// The numbers of v3d_packet.xml pinned in `ffi.rs`, spelled out so a moved
/// constant is seen here too.
#[test]
fn the_enum_values_are_the_packets() {
    assert_eq!(
        (V3D_OUTPUT_IMAGE_FORMAT_RGBA16F, V3D_OUTPUT_IMAGE_FORMAT_R16F, V3D_OUTPUT_IMAGE_FORMAT_R32F),
        (18, 20, 11)
    );
    assert_eq!((V3D_INTERNAL_TYPE_16F, V3D_INTERNAL_TYPE_32F), (6, 10));
    assert_eq!((V3D_INTERNAL_BPP_32, V3D_INTERNAL_BPP_64, V3D_INTERNAL_BPP_128), (0, 1, 2));
    assert_eq!(V3D_RENDER_TARGET_CLAMP_NONE, 0);
}

/// The render-target config words per surface: 16F at 64 bpp for the half
/// formats (R16F too), 32F at 32 bpp for R32F, never clamped, always a typed
/// job.
#[test]
fn the_render_target_words() {
    let want = [
        (GLES_SURF_RGBA16F, V3D_OUTPUT_IMAGE_FORMAT_RGBA16F, V3D_INTERNAL_TYPE_16F, V3D_INTERNAL_BPP_64, 8),
        (GLES_SURF_R16F, V3D_OUTPUT_IMAGE_FORMAT_R16F, V3D_INTERNAL_TYPE_16F, V3D_INTERNAL_BPP_64, 2),
        (GLES_SURF_R32F, V3D_OUTPUT_IMAGE_FORMAT_R32F, V3D_INTERNAL_TYPE_32F, V3D_INTERNAL_BPP_32, 4),
    ];
    for (s, output, ty, bpp, bytes) in want {
        let name = gles_surf_name(s);
        let r = state::rt_format(s).expect(name);
        assert_eq!(
            (r.output, r.internal_type, r.internal_bpp, r.clamp, r.bytes),
            (output, ty, bpp, V3D_RENDER_TARGET_CLAMP_NONE, bytes),
            "{name}"
        );
        assert_eq!(gles_surf_bytes_per_pixel(s), bytes, "{name}: the GL core's bytes a pixel");
        assert!(state::typed(s), "{name}: a typed job");
        assert!(yakogl::backend::rt_format_drawable(s), "{name}: drawable");
    }
}

/// The 32-bit RG and RGBA float and the packed R11F_G11F_B10F are refused as
/// renderbuffer storage today (the GL core, without GL_EXT_color_buffer_float;
/// v3d-arm's, behind its feature).
#[test]
fn the_wide_float_formats_are_refused_today() {
    let _g = rig();
    for (gl, name) in [(GL_RG32F, "RG32F"), (GL_RGBA32F, "RGBA32F"), (GL_R11F_G11F_B10F, "R11F_G11F_B10F")] {
        assert_eq!(clear(gl, Call::Fv, [0; 4]).err(), Some(Refused::Storage(GL_INVALID_ENUM)), "{name}");
    }
}

/// Through the GL API, per format: the job's RT 0 output format, the typed
/// job, the row stride (the bytes a pixel of the surface), and the clear
/// words of glClearBufferfv, halves two a word, R32F's whole; a format
/// without alpha reads it as 1.
#[test]
fn the_job_target_and_the_clear_words_per_format() {
    let _g = rig();
    for r in &ROWS {
        let row = state::rt_format(r.surf).expect(r.name);
        assert_eq!(row.output, r.output, "{}", r.name);
        assert_eq!(row.bytes, r.bytes, "{}", r.name);
        for mut v in [[0.25, -2.0, 1.0e4, 0.5], [1.0, 0.0, -0.0, 65504.0], [-1.5, 3.0e4, 0.1, -7.25]] {
            let s = clear(r.gl, Call::Fv, bits(v)).unwrap_or_else(|e| panic!("{}: {e:?}", r.name));
            assert_tlb(&s, r.name, true);
            let t = &s.jobs[0];
            assert_eq!(t.rt_format[0], r.output, "{}: the job's RT 0 format", r.name);
            assert_eq!(t.stride, t.width * r.bytes, "{}: stride is width x bytes", r.name);
            if r.gl != GL_RGBA16F {
                v[3] = 1.0;
            }
            let tile = if r.gl == GL_R32F { Tile::W32 } else { Tile::F16 };
            assert_eq!(s.clears[0].clear_rt[0], pack(tile, bits(v)), "{} {v:?}", r.name);
            assert_eq!(state::clear_words(r.surf, v), s.clears[0].clear_rt[0], "{}: state::clear_words", r.name);
        }
    }
}

/// Known words, spelled out: 1.0 is 0x3c00, -2.0 0xc000, 0.5 0x3800, 4.0
/// 0x4400; R32F's 0.25 is 0x3e800000.
#[test]
fn clear_words_by_value() {
    let _g = rig();
    let v = bits([1.0, -2.0, 0.5, 4.0]);
    let s = clear(GL_RGBA16F, Call::Fv, v).unwrap();
    assert_eq!(s.clears[0].clear_rt[0], [0xc000_3c00, 0x4400_3800, 0, 0]);
    let s = clear(GL_R16F, Call::Fv, v).unwrap();
    assert_eq!(s.clears[0].clear_rt[0], [0xc000_3c00, 0x3c00_3800, 0, 0], "R16F: green, blue as given, alpha 1");
    let s = clear(GL_R32F, Call::Fv, bits([0.25, 9.0, 9.0, 9.0])).unwrap();
    assert_eq!(s.clears[0].clear_rt[0], [0x3e80_0000, 0x4110_0000, 0x4110_0000, 0x3f80_0000]);
}

/* ---- the TLB writes ---- */

/// A `vec4` output is two f16 pairs through TLB, no config (RT 0's f16 vec4
/// is the TLB's default), whatever the target: the program is not keyed by
/// the target's format.
#[test]
fn a_vec4_output_is_two_f16_pairs() {
    assert_eq!(tlb_config_f16(0, 4), !0);
    let w = run_fs(&fs_of("vec4", "vec4(1.0, 0.5, -2.0, 4.0)"));
    assert_eq!(w, [(None, 0x3800_3c00), (None, 0x4400_c000)]);
}

/// A `vec2` or `float` output is written as a vec4 today: the same two f16
/// pairs through TLB with the TLB's default config (no TLBU), the missing
/// components 0, whatever the target (R16F and R32F included). Mesa writes a
/// shorter vector with a size in the config (`tlb_config_f16(0, 1 or 2)`).
#[test]
fn a_vec2_and_a_float_output_are_padded_to_two_f16_pairs() {
    assert_eq!(tlb_config_f16(0, 2), 0xffff_fffe, "Mesa's f16 config of a 1 or 2 component output");
    assert_eq!(run_fs(&fs_of("vec2", "vec2(1.0, 0.5)")), [(None, 0x3800_3c00), (None, 0)]);
    assert_eq!(run_fs(&fs_of("float", "-2.0")), [(None, 0x0000_c000), (None, 0)]);
}

/// The f16 config names its render target (7 - rt in bits 3..5).
#[test]
fn the_f16_config_names_the_render_target() {
    assert_eq!(tlb_config_f16(1, 2), 0xffff_fff6);
    assert_eq!(tlb_config_f16(1, 4), 0xffff_fff7);
}

/* ---- Mesa's value where it differs ---- */

// Mesa 26.2.1 (nir_to_vir.c:1974-1977, vir_emit_tlb_color_write) writes a
// colour output bound for a 32F target (`f32_color_rb`) with the 32-bit
// config, one word a component, because its compiler key carries the
// target's format. Ours is not keyed: an R32F target gets f16 pairs today,
// so a value that does not survive a half (here 65536.0, past 65504) is
// lost. v3d-arm's work.
