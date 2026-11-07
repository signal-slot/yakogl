// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! glClearBuffer of every ES 3.0 sized colour format, as the V3D job's clear
//! words (t-clearbuffer-formats), on the host.
//!
//! tests/es3_int_rt_v3d.rs checks `state::clear_words` per internal type; this
//! file goes one level up: glClearBuffer{fv,iv,uiv} through the real GL core
//! and the real V3D back end, with the job builder stood in for by a recorder
//! (the same fakes as that file's `gl_api`), and the clear colour the back end
//! hands `v3d_job_end` (`V3dJobClear`) checked against Mesa 26.2.1's packing
//! of the same value, written out here from the sources and not taken from
//! `state::clear_words`:
//!
//! * internal type 8 (the ES 2.0 targets: R8, RG8, RGB8, RGB565, RGBA4,
//!   RGB5_A1, RGBA8 are all held as RGBA8): `v3d_tlb_clear`
//!   (src/gallium/drivers/v3d/v3dx_draw.c:1706-1769) packs with
//!   `util_pack_color(.., PIPE_FORMAT_R8G8B8A8_UNORM)`, i.e. `float_to_ubyte`
//!   per channel (NaN and negatives 0, 1 and above 255), one word;
//! * 8I / 8UI: the low byte of each of the four raw words in one word (mask
//!   0xff, no clamp, Mesa's own comment), 16I / 16UI the low 16 bits of each,
//!   two a word; 32I / 32UI / 32F the four words as they are;
//! * 16F (RGBA16F, R16F, and the fixed-point sRGB and RGB10_A2 whose tile
//!   buffer is 16F, `v3dx_format_table.c` get_internal_type_bpp:249-262):
//!   each channel a half (`_mesa_float_to_half`, round to nearest even, NaN
//!   the quiet pattern, overflow infinity), two a word.
//!
//! The full, unscissored, unmasked clear is the TLB's: a clear word and no
//! draw (`load_rt0` false). What the V3D refuses is pinned by name. The
//! scissored and masked clears (a quad) are in tests/es3_int_rt_v3d.rs and
//! tests/es3_fragops_scissor_v3d.rs.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_clearbuffer_formats_v3d
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
    if !(x > 0.0) {
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

/* ---- the fixed-point formats held as RGBA8 ---- */

/// R8, RG8, RGB8, RGB565, RGBA4, RGB5_A1 and RGBA8 are one surface for the
/// V3D (RGBA8888, an untyped job), cleared with the one R8G8B8A8 word
/// `util_pack_color` makes of the clamped float colour. A format without
/// alpha reads it as 1 (the GL core hands 1.0), and the formats with fewer
/// alpha bits get the alpha the store keeps (the GL core quantises it: 4 bits
/// 0x33 steps, 1 bit 0 or 0xff).
#[test]
fn fixed_point_formats_clear_as_the_rgba8_word() {
    let _g = rig();
    let formats: [(GLenum, &str, fn(f32) -> u32); 7] = [
        (GL_R8, "R8", |_| 255),
        (GL_RG8, "RG8", |_| 255),
        (GL_RGB8, "RGB8", |_| 255),
        (GL_RGB565, "RGB565", |_| 255),
        (GL_RGBA4, "RGBA4", |a| (ubyte(a) * 15 + 127) / 255 * 17),
        (GL_RGB5_A1, "RGB5_A1", |a| if ubyte(a) >= 128 { 255 } else { 0 }),
        (GL_RGBA8, "RGBA8", ubyte),
    ];
    for (f, name, alpha) in formats {
        for v in [EDGE, MID] {
            let s = clear(f, Call::Fv, bits(v)).unwrap_or_else(|e| panic!("{name}: {e:?}"));
            assert_tlb(&s, name, false);
            let mut want = pack(Tile::U8, bits(v))[0] & 0x00ff_ffff;
            want |= alpha(v[3]) << 24;
            assert_eq!(s.clears[0].rgba8, want, "{name} {v:?}");
            assert_eq!(s.clears[0].clear_rt[0][0], want, "{name} {v:?}: RT 0's word");
        }
    }
}

/* ---- the 16F tile buffer: sRGB, RGB10_A2 and the half-float formats ---- */

/// SRGB8_ALPHA8 and RGB10_A2 are fixed point over a 16F tile buffer (Mesa
/// v3dx_format_table.c:249-262): the clear is clamped to [0, 1] (NaN to 0,
/// ES 3.0 4.2.3) and packed as four halves. The sRGB encoding happens at the
/// store, so the clear colour is the linear one.
#[test]
fn srgb_and_rgb10_a2_clear_as_clamped_halves() {
    let _g = rig();
    for (f, name, out) in [
        (GL_SRGB8_ALPHA8, "SRGB8_ALPHA8", V3D_OUTPUT_IMAGE_FORMAT_SRGB8_ALPHA8),
        (GL_RGB10_A2, "RGB10_A2", V3D_OUTPUT_IMAGE_FORMAT_RGB10_A2),
    ] {
        for v in [EDGE, MID, [0.5, 0.25, 0.75, 1.5]] {
            let s = clear(f, Call::Fv, bits(v)).unwrap_or_else(|e| panic!("{name}: {e:?}"));
            assert_tlb(&s, name, true);
            assert_eq!(s.jobs[0].rt_format[0], out, "{name}");
            let clamped = v.map(|x| if x > 0.0 { x.min(1.0) } else { 0.0 });
            let want = pack(Tile::F16, bits(clamped));
            assert_eq!(s.clears[0].clear_rt[0], want, "{name} {v:?}");
        }
    }
}

/// The float formats are not clamped (`v3d_tlb_clear`: "clamping is not
/// performed on the clear color"): R16F, RG16F, RGB16F and RGBA16F as
/// halves, R32F as the word. A format without alpha reads it as 1.
#[test]
fn float_formats_clear_unclamped() {
    let _g = rig();
    let formats = [
        (GL_R16F, "R16F", V3D_OUTPUT_IMAGE_FORMAT_R16F, false),
        (GL_RG16F, "RG16F", V3D_OUTPUT_IMAGE_FORMAT_RGBA16F, false),
        (GL_RGB16F, "RGB16F", V3D_OUTPUT_IMAGE_FORMAT_RGBA16F, false),
        (GL_RGBA16F, "RGBA16F", V3D_OUTPUT_IMAGE_FORMAT_RGBA16F, true),
        (GL_R32F, "R32F", V3D_OUTPUT_IMAGE_FORMAT_R32F, false),
    ];
    for (f, name, out, has_alpha) in formats {
        let tile = if f == GL_R32F { Tile::W32 } else { Tile::F16 };
        for mut v in [EDGE, MID, HALF_EDGE, HALF_MORE, [-1.5, 3.0e4, 0.1, -7.25]] {
            let s = clear(f, Call::Fv, bits(v)).unwrap_or_else(|e| panic!("{name}: {e:?}"));
            assert_tlb(&s, name, true);
            assert_eq!(s.jobs[0].rt_format[0], out, "{name}");
            if !has_alpha {
                v[3] = 1.0;
            }
            assert_eq!(s.clears[0].clear_rt[0], pack(tile, bits(v)), "{name} {v:?}");
        }
    }
}

/* ---- the integer formats ---- */

/// All nineteen integer formats of table 3.13, with the width of the
/// tile-buffer type each is held as (RGB10_A2UI as 16UI) and its signedness.
const INTEGERS: [(GLenum, &str, u32, bool); 19] = [
    (GL_R8I, "R8I", 8, true),
    (GL_R8UI, "R8UI", 8, false),
    (GL_R16I, "R16I", 16, true),
    (GL_R16UI, "R16UI", 16, false),
    (GL_R32I, "R32I", 32, true),
    (GL_R32UI, "R32UI", 32, false),
    (GL_RG8I, "RG8I", 8, true),
    (GL_RG8UI, "RG8UI", 8, false),
    (GL_RG16I, "RG16I", 16, true),
    (GL_RG16UI, "RG16UI", 16, false),
    (GL_RG32I, "RG32I", 32, true),
    (GL_RG32UI, "RG32UI", 32, false),
    (GL_RGBA8I, "RGBA8I", 8, true),
    (GL_RGBA8UI, "RGBA8UI", 8, false),
    (GL_RGBA16I, "RGBA16I", 16, true),
    (GL_RGBA16UI, "RGBA16UI", 16, false),
    (GL_RGBA32I, "RGBA32I", 32, true),
    (GL_RGBA32UI, "RGBA32UI", 32, false),
    (GL_RGB10_A2UI, "RGB10_A2UI", 16, false),
];

/// The raw words of glClearBufferiv: negative, the extremes, and the
/// value that overflows 8 and 16 bits.
const IV: [[i32; 4]; 4] =
    [[-1, i32::MAX, i32::MIN, 300], [127, -128, 128, -129], [32767, -32768, 32768, 65536], [0, 1, -2, 0x1_0102]];
/// The same for glClearBufferuiv: all ones, the top bit, a value past 8 bits
/// and past 10 (the RGB10_A2UI's 1023 and 3), and the byte and half edges.
const UIV: [[u32; 4]; 4] =
    [[0xffff_ffff, 0x8000_0000, 0x1ff, 1025], [255, 256, 65535, 65536], [1023, 1024, 3, 4], [0, 1, 0x7f, 0x80]];

/// Mesa copies the low bits of each word and clamps nothing, so a value past
/// the format's width wraps (v3dx_draw.c:1735-1769); the unused channels of an
/// R or RG format go along with the four words. Every integer format, its
/// own call, the edge words.
#[test]
fn integer_formats_clear_the_low_bits_of_the_raw_words() {
    let _g = rig();
    for (f, name, width, signed) in INTEGERS {
        let tile = match width {
            8 => Tile::I8,
            16 => Tile::I16,
            _ => Tile::W32,
        };
        let sets: Vec<[u32; 4]> = if signed { IV.iter().map(|v| v.map(|x| x as u32)).collect() } else { UIV.to_vec() };
        for v in sets {
            let s = clear(f, if signed { Call::Iv } else { Call::Uiv }, v).unwrap_or_else(|e| panic!("{name}: {e:?}"));
            assert_tlb(&s, name, true);
            assert_eq!(s.clears[0].clear_rt[0], pack(tile, v), "{name} {v:x?}");
        }
    }
}

/// The wrong call for the format is GL_INVALID_OPERATION (ES 3.0 4.2.3: the
/// float form clears a fixed or float buffer, iv a signed and uiv an unsigned
/// integer one) and no job is built for it.
#[test]
fn the_wrong_clear_call_is_invalid_operation() {
    let _g = rig();
    for (f, name, _, signed) in INTEGERS {
        let wrong = if signed { [Call::Fv, Call::Uiv] } else { [Call::Fv, Call::Iv] };
        for call in wrong {
            assert_eq!(clear(f, call, [1; 4]).err(), Some(Refused::Clear(GL_INVALID_OPERATION)), "{name} {call:?}");
        }
    }
    for (f, name) in
        [(GL_RGBA8, "RGBA8"), (GL_SRGB8_ALPHA8, "SRGB8_ALPHA8"), (GL_RGBA16F, "RGBA16F"), (GL_R32F, "R32F")]
    {
        for call in [Call::Iv, Call::Uiv] {
            assert_eq!(clear(f, call, [1; 4]).err(), Some(Refused::Clear(GL_INVALID_OPERATION)), "{name} {call:?}");
        }
    }
}

/* ---- what is refused ---- */

/// ES 3.0 makes RG32F, RGBA32F and R11F_G11F_B10F colour-renderable only with
/// GL_EXT_color_buffer_float, which this context does not report: the GL core
/// refuses them as renderbuffer storage by name (GL_INVALID_ENUM) before the
/// V3D back end sees a job. (Mesa's v3d renders all three: RG32F, RGBA32F
/// and R11G11B10_FLOAT are in its format table, rows 32F/32F/16F; a GL-core
/// step, not this back end's.) Any clear call answers the same.
#[test]
fn formats_the_gl_core_does_not_render_are_refused_by_name() {
    let _g = rig();
    for (f, name) in [(GL_RG32F, "RG32F"), (GL_RGBA32F, "RGBA32F"), (GL_R11F_G11F_B10F, "R11F_G11F_B10F")] {
        for call in [Call::Fv, Call::Iv, Call::Uiv] {
            assert_eq!(clear(f, call, [0; 4]).err(), Some(Refused::Storage(GL_INVALID_ENUM)), "{name} {call:?}");
        }
    }
}
