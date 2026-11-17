// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! glBlitFramebuffer depth from and into a cube face or a 2D array layer of
//! a depth texture, and stencil to and from a texture, through the GL API
//! over the V3D host back end (plan item layers-test-v3d;
//! docs/notes/blit-depth-layers.md).
//!
//! tests/es3_blit_ds_layers.rs is the GL core's half on a stand-in back end.
//! This file is the real V3D's: the real GL core and the real
//! `v3d_depth_resolve` / `v3d_stencil_resolve` / `v3d_ds_write`
//! (src/v3d/backend.rs, with src/v3d/ds_plane.rs), the job builder faked (the
//! rig of tests/es3_blit_ds_window_v3d.rs, copied). Every shape has a
//! renderbuffer on the other side (4.3.3: formats match, so D24, D32F and
//! D24S8 textures pair with a renderbuffer of their format).
//!
//! Layout on the V3D (docs/notes/blit-depth-layers.md (a), "V3D"): ONE depth
//! buffer PER TEXTURE, not per face or layer. `Target::ds` is `d.ds_id` and
//! `ds_owners` is `[d.depth_id, d.stencil_id]` (src/v3d/backend.rs), keyed by
//! `zs_keys` / `zs_buf` on the id alone, and the id is
//! `ds_object_id(serial, true)` (src/gl/context.rs), which has no face, layer
//! or level. In the GL core the texels of a cube or array sit stacked in the
//! texture's linear R32F storage, layer k's row y at row k * height + y
//! (src/gl/context.rs `layers`), and `texture::resolve_depth` / `depth_seed`
//! touch only face/layer 0 of a level. So the texel store is what the tests
//! set and read for a face or layer (as tests/es3_blit_ds_layers.rs does),
//! and the planes are read through the V3D's own entries.
//!
//! Each shape is a pair. `..._is_refused_today` pins the answer of today's
//! code: exactly one GL_INVALID_OPERATION (src/gl/blit.rs REFUSED_DEPTH_SRC /
//! REFUSED_DEPTH_DST / REFUSED_STENCIL), the destination unchanged, the
//! context usable afterwards. The twin, `#[ignore = "needs gl-ds-layers"]`,
//! holds what a working blit leaves, from the nearest-sample model
//! (`model`, `model_axis`, copied from tests/es3_blit_ds_layers.rs, written
//! apart from the code); it fails until gl-ds-layers lands. Run it with
//! `-- --ignored`.
//!
//! The planes are not what the fake GPU stores: each renderbuffer's and the
//! stencil texture's planes are set through the back end's own `ds_write`
//! (by id) to a distinct value per pixel, the blit runs, and they are read
//! back through `depth_resolve` / `stencil_resolve`. The face/layer texels
//! are written and read in the GL core's texel store. Compared in 24-bit
//! depth units (the buffer is D24S8: `split_zs` off as in the other V3D
//! rigs; D32F is a 24-bit round trip).
//!
//! Left open: the stencil texture is a 2D D24S8; a cube or array
//! depth-stencil texture's stencil is not covered (the note: right only for
//! the image the plane holds). Nothing here ran on a Pi 4.
//!
//! Run: `cargo test --no-default-features --features
//! host-test,gpu-v3d,shaderc-online,no-shader-table --test
//! es3_blit_ds_layers_v3d`, and the ignored twins with `-- --ignored`.

#![cfg(all(
    feature = "host-test",
    feature = "gpu-v3d",
    feature = "shaderc-online",
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
/* the fakes and trampolines take the C functions' parameter lists as they are */
#![allow(clippy::too_many_arguments)]

use core::ffi::{c_char, c_int, c_void, CStr};
use std::collections::BTreeMap;
use std::ffi::CString;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Mutex, MutexGuard, Once, OnceLock};

use yakogl::gl::consts::*;
use yakogl::gl::context::{self, GlContext};
use yakogl::gl::framebuffer::*;
use yakogl::gl::names::{self, Kind};
use yakogl::gl::state::*;
use yakogl::gl::texture::{glBindTexture, glDeleteTextures, glGenTextures, glTexImage2D, glTexImage3D};
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps, OsalV3dCpuOf};
use yakogl::v3d::backend::*;
use yakogl::v3d::ffi::*;
use yakogl::v3d::kick::V3dKick;

/* ---- the real layout code, built from driver/src/v3d/v3d_tex.c ------------- */

extern "C" {
    fn dlopen(path: *const c_char, flags: c_int) -> *mut c_void;
    fn dlsym(h: *mut c_void, name: *const c_char) -> *mut c_void;
}

/// `driver/src/v3d/v3d_tex.c` and the Mesa tiling it includes, as a shared
/// object of this test's own; its symbols are reached through the handle
/// only (RTLD_LOCAL), so the trampolines below are the ones the library
/// links.
fn lib() -> *mut c_void {
    static L: OnceLock<usize> = OnceLock::new();
    *L.get_or_init(|| {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("es3_blit_ds_layers_v3d");
        std::fs::create_dir_all(&dir).expect("the scratch directory");
        let so = dir.join(format!("v3d_tex-{}.so", std::process::id()));
        /* the compiler's own temporary files next to the object: a host
         * whose /tmp is full must not fail the check */
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
/// (src/v3d/ffi.rs), each handed to the real C. `v3d_tex_pack_state` also
/// records the state it packed, by the record's address, for the checks.
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

/* The signatures are src/v3d/ffi.rs's. Kept as written: a macro call wider
 * than the width is wrapped by some rustfmt releases and left by others, so
 * the gate's `cargo fmt --check` must not depend on the host's release. */
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

/// Every texture state packed, by the record's CPU address.
static STATES: Mutex<Vec<(usize, V3dTexState)>> = Mutex::new(Vec::new());

#[no_mangle]
unsafe extern "C" fn v3d_tex_pack_state(out: *mut u8, s: *const V3dTexState) -> bool {
    // SAFETY: the C function's signature.
    let f: unsafe extern "C" fn(*mut u8, *const V3dTexState) -> bool =
        unsafe { std::mem::transmute(sym("v3d_tex_pack_state")) };
    // SAFETY: the library's call, passed on; `s` is its live state.
    let ok = unsafe { f(out, s) };
    if ok {
        // SAFETY: the backend's live state.
        STATES.lock().unwrap_or_else(|e| e.into_inner()).push((out as usize, unsafe { *s }));
    }
    ok
}

/* ---- the job builder, stood in for (driver/src/v3d/v3d_job.c) -------------- */

#[derive(Clone, Debug)]
struct SeenJob {
    target: V3dJobTarget,
    clear: Option<V3dJobClear>,
    /// The FS texture records of each draw, in order.
    tmu: Vec<Vec<V3dJobTmu>>,
}

static JOBS: Mutex<Vec<SeenJob>> = Mutex::new(Vec::new());

fn jobs() -> MutexGuard<'static, Vec<SeenJob>> {
    JOBS.lock().unwrap_or_else(|e| e.into_inner())
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
    jobs().push(SeenJob { target: t, clear: None, tmu: Vec::new() });
    t.width != 0 && t.height != 0
}

#[no_mangle]
unsafe extern "C" fn v3d_job_draw(j: *mut V3dJob, d: *const V3dJobDraw, _layout: *mut c_void) -> i32 {
    // SAFETY: the backend passes its live job and a draw whose pointers live for the call.
    let (j, d) = unsafe { (&mut *j, &*d) };
    let tmu = if d.tmu.is_null() || d.num_tmu == 0 {
        Vec::new()
    } else {
        // SAFETY: `num_tmu` records, live for the call.
        unsafe { std::slice::from_raw_parts(d.tmu, d.num_tmu as usize) }.to_vec()
    };
    j.draws += 1;
    // SAFETY: the job's BCL is live.
    unsafe { (*j.bcl).used += 120 };
    if let Some(job) = jobs().last_mut() {
        job.tmu.push(tmu);
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
    // SAFETY: the backend passes its live job and clear.
    let (j, c) = unsafe { (&mut *j, *c) };
    // SAFETY: the job's lists are live.
    unsafe {
        (*j.bcl).used += 1;
        (*j.rcl).used = 200;
    }
    if let Some(job) = jobs().last_mut() {
        job.clear = Some(c);
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

/// The tile-size table of v3d_frame.c (Mesa's v3d_choose_tile_size), as
/// the backend's unit tests fake it.
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
unsafe extern "C" fn v3d_frame_alloc_sizes(t: *const V3dTiling, _draws: u32, _page: u32, ta: *mut u32, ts: *mut u32) {
    // SAFETY: the backend's parameters.
    unsafe {
        let n = (*t).tiles_x * (*t).tiles_y;
        *ta = n * 128 + 8192;
        *ts = n * 256;
    }
}

/* ---- the platform ------------------------------------------------------------ */

/// GPU addresses are 32-bit (the V3D's MMU), from here up.
const GPU_BASE: u64 = 0x0100_0000;

struct Alloc {
    /// u64 backing: every u32 view of it is aligned.
    mem: Box<[u64]>,
    size: usize,
}

#[derive(Default)]
struct Platform {
    next: u64,
    allocs: BTreeMap<u64, Alloc>,
    kicks: Vec<V3dKick>,
    log: Vec<String>,
    /// Bytes of the live allocations and their high-water since the last
    /// `mark_peak`: the carve-out use as the GL core and the back end make it.
    live: usize,
    peak: usize,
}

static PLATFORM: Mutex<Option<Platform>> = Mutex::new(None);

fn with_platform<R>(f: impl FnOnce(&mut Platform) -> R) -> R {
    let mut g = PLATFORM.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(Platform::default))
}

/// The D24S8 word the "GPU" stores at (x, y) in the job of fence `fence`:
/// a depth that differs per texel and per frame in bits 31..8, and a
/// stencil byte the texture unit and the resolve must both ignore.
fn depth_word(fence: usize, x: u32, y: u32) -> u32 {
    let d = (x * 7 + y * 13 + fence as u32 * 1009) & 0x00ff_ffff;
    (d << 8) | 0x3c
}

/// The CPU view of `n` bytes at GPU address `gpu`.
fn platform_cpu(p: &mut Platform, gpu: u64, n: usize) -> *mut u8 {
    let (base, a) = p.allocs.range_mut(..=gpu).next_back().expect("an allocation below the address");
    let off = (gpu - base) as usize;
    assert!(off + n <= a.size, "0x{gpu:x}+{n} is past its allocation");
    // SAFETY: inside the allocation.
    unsafe { (a.mem.as_mut_ptr() as *mut u8).add(off) }
}

/// A one-level D24S8 layout as the backend lays out a depth/stencil buffer
/// (`zs_buf`: `v3d_tex_layout(w, h, 1, 4)` with the UIF XOR dropped), from
/// the real C.
fn zs_layout(w: u32, h: u32) -> V3dTexLayout {
    let mut l = V3dTexLayout::default();
    // SAFETY: a live layout to fill.
    assert!(unsafe { v3d_tex_layout(w, h, 1, 4, &mut l) }, "{w}x{h} lays out");
    if l.slices[0].tiling == V3D_TEX_TILING_UIF_XOR {
        l.slices[0].tiling = V3D_TEX_TILING_UIF_NO_XOR;
    }
    l
}

unsafe extern "C" fn p_gpu_alloc(_heap: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
    with_platform(|p| {
        if size == 0 || size > 256 << 20 {
            return -12;
        }
        let mut mem = vec![0u64; size.div_ceil(8)].into_boxed_slice();
        let gpu = GPU_BASE + p.next;
        p.next += (size as u64).div_ceil(4096) * 4096;
        let cpu = mem.as_mut_ptr() as *mut c_void;
        // SAFETY: the caller passes a writable descriptor.
        unsafe { *out = OsalGpuMem { cpu, gpu, phys: gpu, size, impl_: cpu } };
        p.allocs.insert(gpu, Alloc { mem, size });
        p.live += size;
        p.peak = p.peak.max(p.live);
        0
    })
}

unsafe extern "C" fn p_gpu_free(mem: *mut OsalGpuMem) -> i32 {
    // SAFETY: the caller passes its descriptor.
    let m = unsafe { *mem };
    if !m.impl_.is_null() {
        with_platform(|p| {
            if let Some(a) = p.allocs.remove(&m.gpu) {
                p.live -= a.size;
            }
        });
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

/// The "GPU": every job completes inside its submit (as src/osal/linux.c),
/// and a job that stores its depth/stencil buffer leaves `depth_word` in
/// every texel of it, tiled as the TLB tiles it (the layout `zs_buf` gave
/// the buffer, which the job's clear names).
unsafe extern "C" fn p_submit(kick: *const c_void, len: usize) -> i64 {
    if len != size_of::<V3dKick>() {
        return -22;
    }
    // SAFETY: the backend's live descriptor of that size.
    let k = unsafe { *(kick as *const V3dKick) };
    let fence = with_platform(|p| {
        p.kicks.push(k);
        p.kicks.len()
    });
    let job = jobs().get(fence - 1).cloned().expect("the job this kick runs was begun and ended");
    let c = job.clear.expect("the job was ended");
    if c.zs_store && c.zs_address != 0 {
        let (w, h) = (job.target.width, job.target.height);
        let l = zs_layout(w, h);
        assert_eq!(c.zs_memory_format, l.slices[0].tiling, "the store's memory format is the layout's tiling");
        let words: Vec<u32> = (0..w * h).map(|i| depth_word(fence, i % w, i / w)).collect();
        with_platform(|p| {
            let image = platform_cpu(p, u64::from(c.zs_address), l.size as usize);
            // SAFETY: the buffer's live mapping holds the laid-out level
            // (platform_cpu checked l.size bytes); `words` is h rows of w words.
            unsafe { v3d_tex_store_level(image, &l, 0, words.as_ptr() as *const u8, w * 4) };
        });
    }
    fence as i64
}

unsafe extern "C" fn p_wait(_fence: i64, _timeout_us: i64) -> i32 {
    0
}

unsafe extern "C" fn p_completed() -> i64 {
    with_platform(|p| p.kicks.len() as i64)
}

unsafe extern "C" fn p_query(what: u32, out: *mut c_void, out_sz: usize) -> i32 {
    use yakogl::osal::{
        OSAL_QUERY_V3D_BIN_POOL, OSAL_QUERY_V3D_CPU_OF_GPU, OSAL_QUERY_V3D_JOB_OPEN, OSAL_QUERY_V3D_VPM_BYTES,
    };
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
    if what == OSAL_QUERY_V3D_JOB_OPEN && out_sz == 4 {
        return 0;
    }
    if what == OSAL_QUERY_V3D_BIN_POOL {
        return -61; /* ENODATA, as zephyr_v3d.c answers before the first job */
    }
    -22 /* no TFU, the default budgets, no clock */
}

/// The library logs every line as `log("%s", line)` (`osal::log_str`), so a
/// two-pointer function takes it; on x86-64 and AArch64 Linux a variadic call
/// passes those two arguments as a plain one does.
unsafe extern "C" fn p_log2(_fmt: *const c_char, line: *const c_char) {
    // SAFETY: the library passes a NUL-terminated line.
    let s = unsafe { CStr::from_ptr(line) }.to_string_lossy().into_owned();
    with_platform(|p| p.log.push(s));
}

type LogFn = unsafe extern "C" fn(*const c_char, ...);

unsafe extern "C" fn p_init(_platform_arg: *mut c_void) -> i32 {
    0
}

/// The platform's table (made once, at run time: the log hook is a cast).
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

/* ---- the context ------------------------------------------------------------- */

/// The default framebuffer's size.
const CANVAS: (i32, i32) = (40, 30);

static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();
static mut CTX: *mut GlContext = core::ptr::null_mut();

/// The one context and backend of the file, set up once; the tests take
/// turns.
fn rig() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    ONCE.call_once(|| {
        // SAFETY: statics; the lock serialises every GL call of the file.
        unsafe {
            osal_set_ops(osal());
            yakogl::backend::g_backend = &yakogl_backend_v3d;
            assert_eq!(yakogl::backend::init(), 0, "{:?}", logs());
            CTX = context::gl_context_create(CANVAS.0 as u32, CANVAS.1 as u32);
            assert!(!CTX.is_null(), "{:?}", logs());
        }
    });
    // SAFETY: plain GL calls.
    unsafe {
        glFinish();
        while glGetError() != GL_NO_ERROR {}
    }
    g
}

fn logs() -> Vec<String> {
    with_platform(|p| p.log.clone())
}

/* ---- planes and objects ------------------------------------------------------- */

const D: u32 = yakogl::backend::DS_PLANE_DEPTH;
const S: u32 = yakogl::backend::DS_PLANE_STENCIL;
/// 24-bit depth: the unit the buffer holds.
const UNORM: f32 = 16_777_215.0;

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

/// The id the back end keeps renderbuffer `rb`'s depth and stencil under.
fn ds_id(rb: GLuint) -> u32 {
    let c = context::current().expect("a current context");
    let slot = names::slot_of(c, Kind::Renderbuffer, rb).expect("a renderbuffer name");
    context::ds_object_id(c.renderbuffer_object(slot).expect("a renderbuffer").serial, false)
}

/// The id the back end keeps texture `tex`'s depth and stencil under.
fn tex_ds_id(tex: GLuint) -> u32 {
    let c = context::current().expect("a current context");
    let slot = names::slot_of(c, Kind::Texture, tex).expect("a texture name");
    context::ds_object_id(c.texture_object(slot).expect("a texture object").serial, true)
}

/// Write a whole plane through the table's `ds_write` of object `id`.
fn write_depth(id: u32, w: u32, h: u32, v: &[f32]) {
    // SAFETY: `h` rows of `w` floats.
    let r = unsafe { v3d_ds_write(id, D, v.as_ptr() as *const c_void, 0, 0, w, h, w) };
    assert_eq!(r, 0, "ds_write of the depth of 0x{id:x}: {:?}", logs());
}
fn write_stencil(id: u32, w: u32, h: u32, v: &[u8]) {
    // SAFETY: `h` rows of `w` bytes.
    let r = unsafe { v3d_ds_write(id, S, v.as_ptr() as *const c_void, 0, 0, w, h, w) };
    assert_eq!(r, 0, "ds_write of the stencil of 0x{id:x}: {:?}", logs());
}

/// The planes as the back end resolves them (row 0 first).
fn read_depth(id: u32, w: u32, h: u32) -> Vec<f32> {
    let mut v = vec![-1.0f32; (w * h) as usize];
    // SAFETY: `h` rows of `w` floats.
    let r = unsafe { v3d_depth_resolve(id, v.as_mut_ptr(), w, h, w) };
    assert_eq!(r, 0, "depth_resolve of 0x{id:x}: {:?}", logs());
    v
}
fn read_stencil(id: u32, w: u32, h: u32) -> Vec<u8> {
    let f = yakogl_backend_v3d.stencil_resolve.expect("the V3D table has a stencil_resolve entry");
    let mut v = vec![0xffu8; (w * h) as usize];
    // SAFETY: `h` rows of `w` bytes.
    let r = unsafe { f(id, v.as_mut_ptr(), w, h, w) };
    assert_eq!(r, 0, "stencil_resolve of 0x{id:x}: {:?}", logs());
    v
}

/// A `w` x `h` plane (row-major) of `f(x, y)`.
fn plane<T>(w: u32, h: u32, f: impl Fn(u32, u32) -> T) -> Vec<T> {
    (0..h).flat_map(|y| (0..w).map(move |x| (x, y))).map(|(x, y)| f(x, y)).collect()
}

fn assert_close(got: &[f32], want: &[f32], what: &str) {
    assert_eq!(got.len(), want.len(), "{what}");
    for (i, (g, w)) in got.iter().zip(want).enumerate() {
        assert!((g - w).abs() <= 0.5 / UNORM, "{what}: pixel {i}: {g} want {w}");
    }
}

fn blit(s: [i32; 4], d: [i32; 4], mask: GLbitfield) {
    // SAFETY: plain GL call.
    unsafe { glBlitFramebuffer(s[0], s[1], s[2], s[3], d[0], d[1], d[2], d[3], mask, GL_NEAREST) };
}

fn bind(read: GLuint, draw: GLuint) {
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, read);
        glBindFramebuffer(GL_DRAW_FRAMEBUFFER, draw);
    }
}

/// The source texel destination pixel `o` takes on one axis, or None when
/// its sample point is outside the source cut to `[0, n)`
/// (tests/es3_blit_ds_layers.rs).
fn model_axis(o: i64, s0: i64, s1: i64, d0: i64, d1: i64, n: i64) -> Option<i64> {
    let p = s0 as f64 + (o as f64 + 0.5 - d0 as f64) * (s1 - s0) as f64 / (d1 - d0) as f64;
    let (lo, hi) = (s0.min(s1).max(0), s0.max(s1).min(n));
    let t = p.floor() as i64;
    (p >= lo as f64 && p < hi as f64).then_some(t)
}

/// The destination after the blit `s` -> `d` of a `sw` x `sh` source whose
/// value is `src` into a `dw` x `dh` destination that held `keep`.
fn model(
    src: &dyn Fn(u32, u32) -> f32,
    (sw, sh): (u32, u32),
    keep: &dyn Fn(u32, u32) -> f32,
    (dw, dh): (u32, u32),
    s: [i32; 4],
    d: [i32; 4],
) -> Vec<f32> {
    let mut out = Vec::new();
    for y in 0..dh {
        for x in 0..dw {
            let (xi, yi) = (x as i64, y as i64);
            let in_dst = xi >= d[0].min(d[2]) as i64
                && xi < d[0].max(d[2]) as i64
                && yi >= d[1].min(d[3]) as i64
                && yi < d[1].max(d[3]) as i64;
            let from = in_dst
                .then(|| {
                    let tx = model_axis(xi, s[0] as i64, s[2] as i64, d[0] as i64, d[2] as i64, sw as i64)?;
                    let ty = model_axis(yi, s[1] as i64, s[3] as i64, d[1] as i64, d[3] as i64, sh as i64)?;
                    Some((tx as u32, ty as u32))
                })
                .flatten();
            out.push(match from {
                Some((tx, ty)) => src(tx, ty),
                None => keep(x, y),
            });
        }
    }
    out
}

/* ---- texture storage: a cube or array keeps its layers stacked vertically ---- */

/// How a depth texture is specified, and the renderbuffer that matches it
/// (4.3.3: the formats must match): (internal format, format, type,
/// renderbuffer format, the attachment point of both).
#[derive(Clone, Copy, Debug)]
struct Fmt(GLenum, GLenum, GLenum, GLenum, GLenum);
const D32F: Fmt = Fmt(GL_DEPTH_COMPONENT32F, GL_DEPTH_COMPONENT, GL_FLOAT, GL_DEPTH_COMPONENT32F, GL_DEPTH_ATTACHMENT);
const D24: Fmt =
    Fmt(GL_DEPTH_COMPONENT24, GL_DEPTH_COMPONENT, GL_UNSIGNED_INT, GL_DEPTH_COMPONENT24, GL_DEPTH_ATTACHMENT);
const D24S8: Fmt =
    Fmt(GL_DEPTH24_STENCIL8, GL_DEPTH_STENCIL, GL_UNSIGNED_INT_24_8, GL_DEPTH24_STENCIL8, GL_DEPTH_STENCIL_ATTACHMENT);

const FACES: [GLenum; 6] = [
    GL_TEXTURE_CUBE_MAP_POSITIVE_X,
    GL_TEXTURE_CUBE_MAP_NEGATIVE_X,
    GL_TEXTURE_CUBE_MAP_POSITIVE_Y,
    GL_TEXTURE_CUBE_MAP_NEGATIVE_Y,
    GL_TEXTURE_CUBE_MAP_POSITIVE_Z,
    GL_TEXTURE_CUBE_MAP_NEGATIVE_Z,
];
const FACE_PX: u32 = 0;
const FACE_NZ: u32 = 5;

#[derive(Clone, Copy, Debug)]
enum Layered {
    Cube,
    Array,
}

fn texel_ptr(tex: GLuint, layer: u32, x: u32, y: u32) -> *mut f32 {
    let c = context::current().expect("a current context");
    let slot = names::slot_of(c, Kind::Texture, tex).expect("a texture name");
    let t = c.texture_object(slot).expect("a texture object");
    assert!(x < t.width && y < t.height && layer < t.layers.max(1) as u32);
    let row = layer as usize * t.height as usize + y as usize;
    let off = t.level_off[0] as usize + (row * t.level_pitch_at(0) as usize + x as usize) * 4;
    assert!(off + 4 <= t.mem.size);
    (t.mem.cpu as *mut u8).wrapping_add(off) as *mut f32
}

/// The R32F texel (x, y) of layer/face `layer` of level 0, as the GL core
/// stores it (src/gl/context.rs `layers`).
fn texel(tex: GLuint, layer: u32, x: u32, y: u32) -> f32 {
    // SAFETY: inside the texture's host allocation (texel_ptr checks).
    unsafe { texel_ptr(tex, layer, x, y).read() }
}

fn set_texel(tex: GLuint, layer: u32, x: u32, y: u32, v: f32) {
    // SAFETY: as in `texel`.
    unsafe { texel_ptr(tex, layer, x, y).write(v) }
}

/// A `side` x `side` cube map (six faces) or 2D array (`n` layers) of
/// depth, depth (x, y) of layer k = `f(k, x, y)`.
fn layered_texture(kind: Layered, fmt: Fmt, side: u32, n: u32, f: impl Fn(u32, u32, u32) -> f32) -> GLuint {
    let mut tex = 0;
    let s = side as i32;
    // SAFETY: room for one name; no pixels (the texels are written below).
    unsafe {
        glGenTextures(1, &mut tex);
        match kind {
            Layered::Cube => {
                glBindTexture(GL_TEXTURE_CUBE_MAP, tex);
                for face in FACES {
                    glTexImage2D(face, 0, fmt.0 as GLint, s, s, 0, fmt.1, fmt.2, core::ptr::null());
                }
            }
            Layered::Array => {
                glBindTexture(GL_TEXTURE_2D_ARRAY, tex);
                glTexImage3D(
                    GL_TEXTURE_2D_ARRAY,
                    0,
                    fmt.0 as GLint,
                    s,
                    s,
                    n as i32,
                    0,
                    fmt.1,
                    fmt.2,
                    core::ptr::null(),
                );
            }
        }
    }
    assert_eq!(err(), GL_NO_ERROR, "a {kind:?} depth texture of {:#x}: {:?}", fmt.0, logs());
    let layers = if matches!(kind, Layered::Cube) { 6 } else { n };
    for k in 0..layers {
        for y in 0..side {
            for x in 0..side {
                set_texel(tex, k, x, y, f(k, x, y));
            }
        }
    }
    tex
}

fn layer_picture(tex: GLuint, layer: u32, side: u32) -> Vec<f32> {
    plane(side, side, |x, y| texel(tex, layer, x, y))
}

/// A framebuffer with face/layer `k` of `tex` at `fmt`'s attachment point.
fn layer_fbo(kind: Layered, fmt: Fmt, tex: GLuint, k: u32) -> GLuint {
    let mut fb = 0;
    // SAFETY: room for one name; plain GL calls.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        match kind {
            Layered::Cube => glFramebufferTexture2D(GL_FRAMEBUFFER, fmt.4, FACES[k as usize], tex, 0),
            Layered::Array => glFramebufferTextureLayer(GL_FRAMEBUFFER, fmt.4, tex, 0, k as i32),
        }
        assert_eq!(err(), GL_NO_ERROR, "a {kind:?} image at the attachment: {:?}", logs());
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "{:?}", logs());
    }
    fb
}

/// A `w` x `h` renderbuffer of `format` at `point` of a new framebuffer,
/// cleared (so its buffer exists): (framebuffer, renderbuffer).
fn rb_fbo(format: GLenum, point: GLenum, w: u32, h: u32) -> (GLuint, GLuint) {
    let (mut fb, mut rb) = (0, 0);
    // SAFETY: room for one name each; plain GL calls.
    unsafe {
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorage(GL_RENDERBUFFER, format, w as i32, h as i32);
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, point, GL_RENDERBUFFER, rb);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "{:?}", logs());
        glClearDepthf(0.25);
        glClearStencil(0x5a);
        glClear(GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT);
        glFinish();
    }
    assert_eq!(err(), GL_NO_ERROR, "{:?}", logs());
    (fb, rb)
}

fn release(fbs: &[GLuint], texs: &[GLuint], rbs: &[GLuint]) {
    // SAFETY: names the test made.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(fbs.len() as i32, fbs.as_ptr());
        glDeleteTextures(texs.len() as i32, texs.as_ptr());
        glDeleteRenderbuffers(rbs.len() as i32, rbs.as_ptr());
    }
}

/// Blit once and require today's answer: exactly one GL_INVALID_OPERATION.
fn refused(s: [i32; 4], d: [i32; 4], mask: GLbitfield, what: &str) {
    while err() != GL_NO_ERROR {}
    blit(s, d, mask);
    assert_eq!(err(), GL_INVALID_OPERATION, "{what}: {:?}", logs());
    assert_eq!(err(), GL_NO_ERROR, "{what}: exactly one error");
}

/// The context still blits after a refusal: a renderbuffer's depth copies to
/// another's through the entries.
fn still_usable() {
    let (fa, ra) = rb_fbo(GL_DEPTH24_STENCIL8, GL_DEPTH_STENCIL_ATTACHMENT, 4, 4);
    let (fb, rb) = rb_fbo(GL_DEPTH24_STENCIL8, GL_DEPTH_STENCIL_ATTACHMENT, 4, 4);
    let pic = plane(4, 4, |x, y| (y * 4 + x + 1) as f32 * 1000.0 / UNORM);
    write_depth(ds_id(ra), 4, 4, &pic);
    write_depth(ds_id(rb), 4, 4, &plane(4, 4, |_, _| 0.5));
    bind(fa, fb);
    blit([0, 0, 4, 4], [0, 0, 4, 4], GL_DEPTH_BUFFER_BIT);
    assert_eq!(err(), GL_NO_ERROR, "the context after a refusal: {:?}", logs());
    assert_close(&read_depth(ds_id(rb), 4, 4), &pic, "a renderbuffer blit after the refusal");
    release(&[fa, fb], &[], &[ra, rb]);
}

/* ---- depth from and into a cube face or an array layer ------------------------ */

const SIDE: u32 = 8;
const WHOLE: [i32; 4] = [0, 0, SIDE as i32, SIDE as i32];

/// Depth of layer/face `k` at (x, y): a different ramp per layer, distinct
/// everywhere, exact in f32 and in 24 bits.
fn layer_depth(k: u32, x: u32, y: u32) -> f32 {
    (k * 256 + y * 16 + x) as f32 / 4096.0
}
fn dst_depth(x: u32, y: u32) -> f32 {
    0.75 + (y * 32 + x) as f32 / 8192.0
}
fn src_depth(x: u32, y: u32) -> f32 {
    (y * 16 + x) as f32 / 1024.0
}

fn layer_count(kind: Layered) -> u32 {
    if matches!(kind, Layered::Cube) {
        6
    } else {
        3
    }
}

fn assert_layers_untouched(tex: GLuint, n: u32, what: &str) {
    for l in 0..n {
        let want = plane(SIDE, SIDE, |x, y| layer_depth(l, x, y));
        assert_eq!(layer_picture(tex, l, SIDE), want, "{what}: layer {l} of the texture is untouched");
    }
}

/// Face/layer `k` of a layered depth texture blitted `s` -> `d` into a
/// `dz` x `dz` renderbuffer of the matching format, whose depth plane held
/// `dst_depth` (written through `ds_write`). `refused_today` pins the
/// refusal; otherwise the renderbuffer's plane is compared with the model.
fn from_layer(kind: Layered, fmt: Fmt, k: u32, dz: u32, s: [i32; 4], d: [i32; 4], refused_today: bool) {
    let n = layer_count(kind);
    let tex = layered_texture(kind, fmt, SIDE, n, layer_depth);
    let sfb = layer_fbo(kind, fmt, tex, k);
    let (dfb, rb) = rb_fbo(fmt.3, fmt.4, dz, dz);
    let rid = ds_id(rb);
    write_depth(rid, dz, dz, &plane(dz, dz, dst_depth));
    bind(sfb, dfb);
    let what = format!("{kind:?} {fmt:?} layer {k}: {s:?} -> {d:?}");
    let want = if refused_today {
        refused(s, d, GL_DEPTH_BUFFER_BIT, &what);
        plane(dz, dz, dst_depth)
    } else {
        blit(s, d, GL_DEPTH_BUFFER_BIT);
        assert_eq!(err(), GL_NO_ERROR, "{what}: {:?}", logs());
        let src = |x, y| layer_depth(k, x, y);
        model(&src, (SIDE, SIDE), &dst_depth, (dz, dz), s, d)
    };
    assert_close(&read_depth(rid, dz, dz), &want, &format!("{what}: the renderbuffer's depth"));
    assert_layers_untouched(tex, n, &what);
    release(&[sfb, dfb], &[tex], &[rb]);
}

/// A `sz` x `sz` renderbuffer's depth (`src_depth` in its plane) blitted
/// `s` -> `d` into face/layer `k` of a layered depth texture. The image takes
/// the model's picture when the blit works and stays as it was when it is
/// refused; every other image stays either way.
fn into_layer(kind: Layered, fmt: Fmt, k: u32, sz: u32, s: [i32; 4], d: [i32; 4], refused_today: bool) {
    let n = layer_count(kind);
    let tex = layered_texture(kind, fmt, SIDE, n, layer_depth);
    let (sfb, rb) = rb_fbo(fmt.3, fmt.4, sz, sz);
    write_depth(ds_id(rb), sz, sz, &plane(sz, sz, src_depth));
    let dfb = layer_fbo(kind, fmt, tex, k);
    bind(sfb, dfb);
    let what = format!("{kind:?} {fmt:?} into layer {k}: {s:?} -> {d:?}");
    let keep = |x, y| layer_depth(k, x, y);
    let want_k = if refused_today {
        refused(s, d, GL_DEPTH_BUFFER_BIT, &what);
        plane(SIDE, SIDE, keep)
    } else {
        blit(s, d, GL_DEPTH_BUFFER_BIT);
        assert_eq!(err(), GL_NO_ERROR, "{what}: {:?}", logs());
        model(&src_depth, (sz, sz), &keep, (SIDE, SIDE), s, d)
    };
    assert_close(&layer_picture(tex, k, SIDE), &want_k, &format!("{what}: the image"));
    for l in (0..n).filter(|&l| l != k) {
        let want = plane(SIDE, SIDE, |x, y| layer_depth(l, x, y));
        assert_eq!(layer_picture(tex, l, SIDE), want, "{what}: layer {l} must not change");
    }
    assert_close(&read_depth(ds_id(rb), sz, sz), &plane(sz, sz, src_depth), &format!("{what}: the source"));
    release(&[sfb, dfb], &[tex], &[rb]);
}

/// Faces +X and -Z of a D24, D32F and D24S8 cube, 1:1 and scaled up and down.
#[test]
fn depth_from_a_cube_face_is_refused_today() {
    let _g = rig();
    for fmt in [D24, D32F, D24S8] {
        for face in [FACE_PX, FACE_NZ] {
            from_layer(Layered::Cube, fmt, face, 8, WHOLE, WHOLE, true);
            from_layer(Layered::Cube, fmt, face, 16, WHOLE, [0, 0, 16, 16], true);
            from_layer(Layered::Cube, fmt, face, 4, WHOLE, [0, 0, 4, 4], true);
        }
    }
    still_usable();
}

#[test]
#[ignore = "needs gl-ds-layers"]
fn depth_from_a_cube_face_copies_that_face() {
    let _g = rig();
    for fmt in [D24, D32F, D24S8] {
        for face in [FACE_PX, FACE_NZ] {
            from_layer(Layered::Cube, fmt, face, 8, WHOLE, WHOLE, false);
            from_layer(Layered::Cube, fmt, face, 16, WHOLE, [0, 0, 16, 16], false);
            from_layer(Layered::Cube, fmt, face, 4, WHOLE, [0, 0, 4, 4], false);
            from_layer(Layered::Cube, fmt, face, 8, [1, 2, 6, 7], [2, 1, 7, 6], false);
        }
    }
}

/// Layers 0 and 2 of a depth array, 1:1 and scaled.
#[test]
fn depth_from_an_array_layer_is_refused_today() {
    let _g = rig();
    for fmt in [D24, D32F, D24S8] {
        for layer in [0, 2] {
            from_layer(Layered::Array, fmt, layer, 8, WHOLE, WHOLE, true);
            from_layer(Layered::Array, fmt, layer, 16, WHOLE, [0, 0, 16, 16], true);
            from_layer(Layered::Array, fmt, layer, 4, WHOLE, [0, 0, 4, 4], true);
        }
    }
    still_usable();
}

#[test]
#[ignore = "needs gl-ds-layers"]
fn depth_from_an_array_layer_copies_that_layer() {
    let _g = rig();
    for fmt in [D24, D32F, D24S8] {
        for layer in [0, 2] {
            from_layer(Layered::Array, fmt, layer, 8, WHOLE, WHOLE, false);
            from_layer(Layered::Array, fmt, layer, 16, WHOLE, [0, 0, 16, 16], false);
            from_layer(Layered::Array, fmt, layer, 4, WHOLE, [0, 0, 4, 4], false);
            from_layer(Layered::Array, fmt, layer, 8, [1, 2, 6, 7], [2, 1, 7, 6], false);
        }
    }
}

#[test]
fn depth_into_a_cube_face_is_refused_today() {
    let _g = rig();
    for fmt in [D24, D32F, D24S8] {
        into_layer(Layered::Cube, fmt, FACE_PX, 8, WHOLE, WHOLE, true);
        into_layer(Layered::Cube, fmt, FACE_NZ, 4, [0, 0, 4, 4], WHOLE, true);
    }
    still_usable();
}

#[test]
#[ignore = "needs gl-ds-layers"]
fn depth_into_a_cube_face_changes_only_that_face() {
    let _g = rig();
    for fmt in [D24, D32F, D24S8] {
        into_layer(Layered::Cube, fmt, FACE_PX, 8, WHOLE, WHOLE, false);
        into_layer(Layered::Cube, fmt, FACE_NZ, 4, [0, 0, 4, 4], WHOLE, false);
        into_layer(Layered::Cube, fmt, 2, 8, [0, 0, 8, 8], [2, 2, 6, 6], false);
    }
}

#[test]
fn depth_into_an_array_layer_is_refused_today() {
    let _g = rig();
    for fmt in [D24, D32F, D24S8] {
        into_layer(Layered::Array, fmt, 0, 8, WHOLE, WHOLE, true);
        into_layer(Layered::Array, fmt, 2, 4, [0, 0, 4, 4], WHOLE, true);
    }
    still_usable();
}

#[test]
#[ignore = "needs gl-ds-layers"]
fn depth_into_an_array_layer_changes_only_that_layer() {
    let _g = rig();
    for fmt in [D24, D32F, D24S8] {
        into_layer(Layered::Array, fmt, 0, 8, WHOLE, WHOLE, false);
        into_layer(Layered::Array, fmt, 2, 4, [0, 0, 4, 4], WHOLE, false);
        into_layer(Layered::Array, fmt, 1, 8, [0, 0, 8, 8], [2, 2, 6, 6], false);
    }
}

/* ---- stencil to and from a texture ---------------------------------------------- */

fn tex_stencil(x: u32, y: u32) -> f32 {
    (10 + y * 4 + x) as f32
}
fn rb_stencil(x: u32, y: u32) -> f32 {
    (100 + y * 5 + x * 2) as f32
}
fn tex_depth(x: u32, y: u32) -> f32 {
    ((y * 4 + x + 1) * 1009) as f32 / UNORM
}
fn rb_depth(x: u32, y: u32) -> f32 {
    ((y * 4 + x + 1) * 2003 + 300_000) as f32 / UNORM
}

/// A 4x4 D24S8 2D texture at the depth-stencil attachment of a framebuffer,
/// cleared through the GL API (so the back end holds its buffer under the
/// texture's id), and a 4x4 D24S8 renderbuffer; both planes of both set
/// through `ds_write`.
struct Pair {
    tex: GLuint,
    tex_fb: GLuint,
    rb: GLuint,
    rb_fb: GLuint,
    tex_id: u32,
    rb_id: u32,
}

fn stencil_pair() -> Pair {
    let (mut tex, mut tex_fb) = (0, 0);
    // SAFETY: room for one name each; plain GL calls.
    unsafe {
        glGenTextures(1, &mut tex);
        glBindTexture(GL_TEXTURE_2D, tex);
        let (f, t) = (GL_DEPTH_STENCIL, GL_UNSIGNED_INT_24_8);
        glTexImage2D(GL_TEXTURE_2D, 0, GL_DEPTH24_STENCIL8 as GLint, 4, 4, 0, f, t, core::ptr::null());
        glGenFramebuffers(1, &mut tex_fb);
        glBindFramebuffer(GL_FRAMEBUFFER, tex_fb);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_DEPTH_STENCIL_ATTACHMENT, GL_TEXTURE_2D, tex, 0);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "{:?}", logs());
        glClearDepthf(0.5);
        glClearStencil(0x21);
        glClear(GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT);
        glFinish();
    }
    assert_eq!(err(), GL_NO_ERROR, "{:?}", logs());
    let (rb_fb, rb) = rb_fbo(GL_DEPTH24_STENCIL8, GL_DEPTH_STENCIL_ATTACHMENT, 4, 4);
    let (tex_id, rb_id) = (tex_ds_id(tex), ds_id(rb));
    write_depth(tex_id, 4, 4, &plane(4, 4, tex_depth));
    write_stencil(tex_id, 4, 4, &plane(4, 4, |x, y| tex_stencil(x, y) as u8));
    write_depth(rb_id, 4, 4, &plane(4, 4, rb_depth));
    write_stencil(rb_id, 4, 4, &plane(4, 4, |x, y| rb_stencil(x, y) as u8));
    Pair { tex, tex_fb, rb, rb_fb, tex_id, rb_id }
}

impl Pair {
    fn release(self) {
        release(&[self.tex_fb, self.rb_fb], &[self.tex], &[self.rb]);
    }

    /// The planes of both: the stencils are the pictures given (`None`: as
    /// they were set), the depths are as they were set.
    fn check(&self, tex_stencil_is: Option<Vec<f32>>, rb_stencil_is: Option<Vec<f32>>, what: &str) {
        let bytes = |v: Vec<u8>| v.into_iter().map(f32::from).collect::<Vec<_>>();
        let (ts, rs) = (plane(4, 4, tex_stencil), plane(4, 4, rb_stencil));
        assert_eq!(
            bytes(read_stencil(self.tex_id, 4, 4)),
            tex_stencil_is.unwrap_or(ts),
            "{what}: the texture's stencil"
        );
        assert_eq!(
            bytes(read_stencil(self.rb_id, 4, 4)),
            rb_stencil_is.unwrap_or(rs),
            "{what}: the renderbuffer's stencil"
        );
        assert_close(&read_depth(self.tex_id, 4, 4), &plane(4, 4, tex_depth), &format!("{what}: the texture's depth"));
        assert_close(
            &read_depth(self.rb_id, 4, 4),
            &plane(4, 4, rb_depth),
            &format!("{what}: the renderbuffer's depth"),
        );
    }
}

const R: [i32; 4] = [0, 0, 4, 4];
const STENCIL: GLbitfield = GL_STENCIL_BUFFER_BIT;

/// Stencil from a renderbuffer into a D24S8 texture, and from the texture
/// into the renderbuffer, with and without depth.
#[test]
fn stencil_to_and_from_a_texture_is_refused_today() {
    let _g = rig();
    let r = stencil_pair();
    bind(r.rb_fb, r.tex_fb);
    refused(R, R, STENCIL, "stencil into a texture");
    refused(R, R, GL_DEPTH_BUFFER_BIT | STENCIL, "depth and stencil into a texture");
    bind(r.tex_fb, r.rb_fb);
    refused(R, R, STENCIL, "stencil from a texture");
    refused(R, R, GL_DEPTH_BUFFER_BIT | STENCIL, "depth and stencil from a texture");
    r.check(None, None, "after the refusals");
    r.release();
    still_usable();
}

/// The renderbuffer's stencil lands in the texture's stencil plane (the
/// texture's id), 1:1 and mirrored in x; the depth planes stay. Then the
/// texture's stencil lands in the renderbuffer's.
#[test]
#[ignore = "needs gl-ds-layers"]
fn stencil_goes_into_and_out_of_a_texture() {
    let _g = rig();
    let r = stencil_pair();
    for (d, name) in [(R, "1:1"), ([4, 0, 0, 4], "mirrored in x")] {
        bind(r.rb_fb, r.tex_fb);
        blit(R, d, STENCIL);
        assert_eq!(err(), GL_NO_ERROR, "stencil into a texture, {name}: {:?}", logs());
        let want = model(&rb_stencil, (4, 4), &tex_stencil, (4, 4), R, d);
        r.check(Some(want), None, &format!("into the texture, {name}"));
        write_stencil(r.tex_id, 4, 4, &plane(4, 4, |x, y| tex_stencil(x, y) as u8));
    }
    for (d, name) in [(R, "1:1"), ([4, 0, 0, 4], "mirrored in x")] {
        bind(r.tex_fb, r.rb_fb);
        blit(R, d, STENCIL);
        assert_eq!(err(), GL_NO_ERROR, "stencil from a texture, {name}: {:?}", logs());
        let want = model(&tex_stencil, (4, 4), &rb_stencil, (4, 4), R, d);
        r.check(None, Some(want), &format!("out of the texture, {name}"));
        write_stencil(r.rb_id, 4, 4, &plane(4, 4, |x, y| rb_stencil(x, y) as u8));
    }
    r.release();
}
