// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! glmark2 `shadow` (and `refract`) on the Pi 4: the depth texture the depth
//! pass renders is sampled by the ground from the D24S8 buffer the job
//! stored, not from the texels (src/v3d/backend.rs `depth_sample`,
//! `GlesTextureBind::depth_id`; docs/online-shaderc/v3d-backend.md "Depth
//! textures sampled from their depth buffer"). Before, every frame detiled
//! the buffer into the R32F texels on the CPU and tiled the texels again for
//! the texture unit: 7.68 MB twice, all of shadow's frame on the host
//! (docs/notes/pi4-glmark2-cpu.md).
//!
//! The GL calls are glmark2's `SceneShadow` frame in its order
//! (scene-shadow.cpp at the pinned 22c527cb: `DepthRenderTarget::enable`
//! binds the framebuffer whose only attachment is the depth texture, masks
//! colour and clears depth, the horse is drawn, `disable` binds the canvas
//! back, and the ground samples the texture). They run through the GL core
//! against the REAL V3D backend (`yakogl_backend_v3d`) and the REAL layout
//! code the board runs, `driver/src/v3d/v3d_tex.c` (Mesa's v3d_setup_slices
//! and tiling, built here with `cc` and loaded with `dlopen`, as
//! tests/v3d_rt_sample.rs builds it). The job builder (v3d_job.c) is stood
//! in for by fakes that record each job's target, clear and texture
//! records, as the backend's own unit tests do; the platform is an OSAL
//! written here whose "GPU" stores a known depth word into the depth
//! buffer of every job that stores one.
//!
//! What is pinned:
//! * no CPU resolve a frame: the backend's `depth_resolves` counter stays 0
//!   over four frames while its `depth_direct` counter rises one a frame,
//!   and nothing is tiled (no copy of the texels for the texture unit);
//! * the same sampled values as before: the ground's texture state names
//!   the depth pass's buffer as DEPTH24_X8, one level, with the layout the
//!   TLB stored it in, and every texel read through that state's layout
//!   (bits 31..8 of the word, over 0xffffff) equals what the CPU resolve
//!   (`v3d_depth_resolve`, the path every frame took before) writes for
//!   the same buffer, which is the depth the "GPU" stored;
//! * the CPU's own path is unchanged: a glTexSubImage2D into the texture
//!   resolves the texels first (one resolve), the draw after it samples
//!   the texels through a copy, and the next render is read from the
//!   buffer again;
//! * the other way: a depth pass with GL_LESS and no clear after a
//!   glTexSubImage2D starts every tile from the texels as the CPU holds
//!   them (`GlesDraw::depth_texels`, the backend's `seed_depth`), read back
//!   through the real tiling, not from the buffer the earlier pass stored.
//!
//! Run: `cargo test --no-default-features --features
//! host-test,gpu-v3d,shaderc-online,no-shader-table --test depth_resolve_v3d`.

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

use yakogl::backend::GLES_TEXFMT_R32F;
use yakogl::gl::consts::*;
use yakogl::gl::context::{self, GlContext};
use yakogl::gl::draw::glDrawArrays;
use yakogl::gl::framebuffer::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::*;
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
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("depth_resolve_v3d");
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
    /// What each job that loads its depth/stencil buffer loads: (fence,
    /// the level's words row by row, read before the job stores).
    loads: Vec<(usize, Vec<u32>)>,
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

/// The depth value a texel of word `w` reads as (Mesa's S8_UINT_Z24_UNORM:
/// bits 31..8 over 0xffffff), the conversion of `v3d_depth_resolve`.
fn depth_of(w: u32) -> f32 {
    (w >> 8) as f32 / 16_777_215.0
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
    if c.zs_load && c.zs_address != 0 {
        /* the tiles start from the buffer: what they load, untiled */
        let (w, h) = (job.target.width, job.target.height);
        let l = zs_layout(w, h);
        let mut words = vec![0u32; (w * h) as usize];
        with_platform(|p| {
            let image = platform_cpu(p, u64::from(c.zs_address), l.size as usize);
            // SAFETY: the buffer's live mapping holds the laid-out level;
            // `words` is h rows of w words.
            assert!(unsafe { v3d_tex_load_box(image, &l, 0, 0, 0, w, h, words.as_mut_ptr() as *mut u8, w * 4) });
            p.loads.push((fence, words));
        });
    }
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

/// glmark2's canvas is 800x600 and its depth target twice that; a smaller
/// pair of the same shape keeps the test quick.
const CANVAS: (i32, i32) = (40, 30);
const DEPTH: (i32, i32) = (80, 60);

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

fn ctx() -> &'static mut GlContext {
    // SAFETY: set once in `rig`, never freed; used under the lock.
    unsafe { &mut *CTX }
}

fn logs() -> Vec<String> {
    with_platform(|p| p.log.clone())
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

fn program(vs: &str, fs: &str) -> GLuint {
    let (v, f) = (shader(GL_VERTEX_SHADER, vs), shader(GL_FRAGMENT_SHADER, fs));
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

/// glmark2's depth pass draws no texture (depth.vert / depth.frag write
/// position and a constant); its ground samples unit 0 (shadow.frag's
/// ShadowMap).
const DEPTH_VS: &str = "attribute vec4 a_position;\nvoid main() { gl_Position = a_position; }\n";
const DEPTH_FS: &str = "precision mediump float;\nvoid main() { gl_FragColor = vec4(1.0); }\n";
const GROUND_VS: &str = "attribute vec4 a_position;\nvarying vec2 v_coord;\n\
    void main() { v_coord = a_position.xy * 0.5 + 0.5; gl_Position = a_position; }\n";
const GROUND_FS: &str = "precision mediump float;\nuniform sampler2D ShadowMap;\nvarying vec2 v_coord;\n\
    void main() { gl_FragColor = texture2D(ShadowMap, v_coord); }\n";

/* a quad over the whole target at z = 0, (x, y, z) per vertex */
static QUAD: [f32; 18] = [
    -1.0, -1.0, 0.0, 1.0, -1.0, 0.0, -1.0, 1.0, 0.0, //
    -1.0, 1.0, 0.0, 1.0, -1.0, 0.0, 1.0, 1.0, 0.0,
];

fn draw_quad() {
    // SAFETY: plain GL calls; the array is a static.
    unsafe {
        glVertexAttribPointer(0, 3, GL_FLOAT, 0, 12, QUAD.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glDrawArrays(GL_TRIANGLES, 0, 6);
        assert_eq!(glGetError(), GL_NO_ERROR, "{:?}", logs());
    }
}

struct Scene {
    depth_prog: GLuint,
    ground_prog: GLuint,
    tex: GLuint,
    fbo: GLuint,
    /// The depth object's id, as the GL core names it to the backend
    /// (`GlesDraw::depth_id`, `v3d_depth_resolve`).
    id: u32,
}

/// `DepthRenderTarget::setup` and `GroundRenderer::setup`: the depth
/// texture (GL_DEPTH_COMPONENT / GL_UNSIGNED_INT, nearest, clamped), the
/// framebuffer with it as the only attachment, the two programs.
fn setup() -> Scene {
    let depth_prog = program(DEPTH_VS, DEPTH_FS);
    let ground_prog = program(GROUND_VS, GROUND_FS);
    let (mut tex, mut fbo) = (0u32, 0u32);
    let name = CString::new("ShadowMap").unwrap();
    // SAFETY: plain GL calls with live out pointers.
    unsafe {
        glGenTextures(1, &mut tex);
        glBindTexture(GL_TEXTURE_2D, tex);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE as GLint);
        glTexImage2D(
            GL_TEXTURE_2D,
            0,
            (if cfg!(feature = "report-es2") { GL_DEPTH_COMPONENT } else { GL_DEPTH_COMPONENT24 }) as GLint,
            DEPTH.0,
            DEPTH.1,
            0,
            GL_DEPTH_COMPONENT,
            GL_UNSIGNED_INT,
            core::ptr::null(),
        );
        glBindTexture(GL_TEXTURE_2D, 0);
        glGenFramebuffers(1, &mut fbo);
        glBindFramebuffer(GL_FRAMEBUFFER, fbo);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_TEXTURE_2D, tex, 0);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "{:?}", logs());
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glUseProgram(ground_prog);
        glUniform1i(glGetUniformLocation(ground_prog, name.as_ptr()), 0);
        glEnable(GL_DEPTH_TEST);
        assert_eq!(glGetError(), GL_NO_ERROR, "{:?}", logs());
    }
    let t = &ctx().objs().textures[tex as usize];
    assert_eq!(t.texfmt, GLES_TEXFMT_R32F, "the GL core keeps a depth texture as R32F texels");
    let id = context::ds_object_id(t.serial, true);
    Scene { depth_prog, ground_prog, tex, fbo, id }
}

/// `SceneShadow::draw`: `DepthRenderTarget::enable`, the horse,
/// `disable`, the ground; then the swap (a finish here).
fn frame(s: &Scene) {
    // SAFETY: plain GL calls on live names.
    unsafe {
        glUseProgram(s.depth_prog);
        glBindFramebuffer(GL_FRAMEBUFFER, s.fbo);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_TEXTURE_2D, s.tex, 0);
        glViewport(0, 0, DEPTH.0, DEPTH.1);
        glColorMask(0, 0, 0, 0);
        glClear(GL_DEPTH_BUFFER_BIT);
        draw_quad();
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glViewport(0, 0, CANVAS.0, CANVAS.1);
        glColorMask(1, 1, 1, 1);
        glUseProgram(s.ground_prog);
        glActiveTexture(GL_TEXTURE0);
        glBindTexture(GL_TEXTURE_2D, s.tex);
        draw_quad();
        glFinish();
        assert_eq!(glGetError(), GL_NO_ERROR, "{:?}", logs());
    }
}

/// The ground's draw alone (no render since the last one).
fn ground(s: &Scene) {
    // SAFETY: plain GL calls on live names.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glUseProgram(s.ground_prog);
        glBindTexture(GL_TEXTURE_2D, s.tex);
        draw_quad();
        glFinish();
        assert_eq!(glGetError(), GL_NO_ERROR, "{:?}", logs());
    }
}

/* ---- what the backend did -------------------------------------------------- */

/// (bindings sampled from a depth buffer, CPU resolves into texels, draws
/// refused) since init.
fn depth_samples() -> (u32, u32, u32) {
    let (mut d, mut r, mut f) = (0u32, 0u32, 0u32);
    // SAFETY: live out-parameters.
    unsafe { v3d_depth_samples(&mut d, &mut r, &mut f) };
    (d, r, f)
}

/// Textures tiled since init (a copy made for the texture unit), and the
/// kept copies live now.
fn tilings_and_copies() -> (u32, u32) {
    let (mut t, mut live) = (0u32, 0u32);
    // SAFETY: live out-parameters.
    unsafe {
        v3d_texture_counters(&mut t, core::ptr::null_mut());
        v3d_tiled_copies(&mut live, core::ptr::null_mut(), core::ptr::null_mut());
    }
    (t, live)
}

/// The last depth pass: the fence of its kick and its clear. The job into
/// the depth texture's framebuffer has no colour target (address 0,
/// `DRAWS_WITHOUT_COLOUR`) and stores its depth; the ground's job on the
/// canvas stores the canvas's own depth, which is not it.
fn last_depth_pass() -> (usize, V3dJobClear) {
    let jobs = jobs();
    let (i, j) = jobs
        .iter()
        .enumerate()
        .rev()
        .find(|(_, j)| j.target.address == 0 && j.clear.is_some_and(|c| c.zs_store && c.zs_address != 0))
        .expect("a job into the depth texture that stored its depth");
    (i + 1, j.clear.unwrap())
}

/// The texture state the last draw of the last job sampled unit 0 with.
fn ground_state() -> V3dTexState {
    let gpu = {
        let jobs = jobs();
        let job = jobs.last().expect("a job");
        let draw = job.tmu.last().expect("a draw in the ground's job");
        assert_eq!(draw.len(), 1, "the ground samples one texture: {draw:?}");
        draw[0].tex_state
    };
    let cpu = with_platform(|p| platform_cpu(p, u64::from(gpu), V3D_TEX_STATE_BYTES)) as usize;
    let states = STATES.lock().unwrap_or_else(|e| e.into_inner());
    states.iter().rev().find(|(at, _)| *at == cpu).map(|(_, s)| *s).expect("the record was packed")
}

/// Texel (x, y) of the depth buffer at `address` as the texture unit reads
/// it through texture state `s`: the layout the state's level-0 fields
/// describe (the real C's, UIF without the XOR as the state says), the word
/// at (x, y), its bits 31..8 over 0xffffff (DEPTH24_X8).
fn tmu_depth(s: &V3dTexState, address: u32, x: u32, y: u32) -> f32 {
    assert_eq!(s.type_, V3D_TEXTURE_DATA_FORMAT_DEPTH24_X8, "the record's type");
    let mut l = zs_layout(s.width, s.height);
    assert!(!s.level_0_xor_enable, "the TLB stored the buffer without the XOR, and so is it read");
    assert_eq!(
        s.level_0_is_strictly_uif,
        l.slices[0].tiling == V3D_TEX_TILING_UIF_NO_XOR,
        "the state's tiling is the buffer's"
    );
    if s.level_0_is_strictly_uif {
        assert_eq!(s.level_0_ub_pad, l.slices[0].ub_pad, "the state's UIF padding is the buffer's");
    }
    l.slices[0].tiling = if s.level_0_is_strictly_uif { V3D_TEX_TILING_UIF_NO_XOR } else { l.slices[0].tiling };
    assert_eq!(s.base, address + l.slices[0].offset, "the record's base is the buffer");
    let mut word = 0u32;
    with_platform(|p| {
        let image = platform_cpu(p, u64::from(address), l.size as usize);
        // SAFETY: the buffer's live mapping; one word out.
        assert!(unsafe { v3d_tex_load_box(image, &l, 0, x, y, 1, 1, &mut word as *mut u32 as *mut u8, 4) });
    });
    depth_of(word)
}

/// The depth the CPU resolve (`v3d_depth_resolve`, the path every frame
/// took before) writes for object `id`: `h` rows of `w` floats.
fn resolved(id: u32, w: u32, h: u32) -> Vec<f32> {
    let mut t = vec![-1.0f32; (w * h) as usize];
    // SAFETY: h rows of w floats.
    assert_eq!(unsafe { v3d_depth_resolve(id, t.as_mut_ptr(), w, h, w) }, 0, "the resolve reads the buffer");
    t
}

/* ---- the tests ----------------------------------------------------------------- */

/// Four shadow frames: each samples the depth pass's buffer (one more
/// direct sample a frame, never a resolve, nothing tiled), through a
/// DEPTH24_X8 record of one level at the buffer's address, and every texel
/// the texture unit reads through the record's layout is the texel the CPU
/// resolve gives and the depth the GPU stored. Then the CPU's path: a
/// glTexSubImage2D resolves once, the draw after it samples the texels
/// through a copy (R32F), and the next frame is the buffer's again.
#[test]
fn shadows_depth_texture_is_sampled_from_its_buffer_with_no_cpu_resolve_a_frame() {
    let _g = rig();
    let s = setup();
    let (w, h) = (DEPTH.0 as u32, DEPTH.1 as u32);
    let (tilings0, copies0) = tilings_and_copies();
    let (direct0, resolves0, refused0) = depth_samples();
    for n in 1..=4u32 {
        frame(&s);
        /* the resolves counted so far are this test's own reads of the
         * buffer (`resolved`, one a frame below), none the frame's */
        assert_eq!(
            depth_samples(),
            (direct0 + n, resolves0 + n - 1, refused0),
            "frame {n}: one direct sample, no CPU resolve, no refusal\n{:?}",
            logs()
        );
        assert_eq!(tilings_and_copies(), (tilings0, copies0), "frame {n}: nothing tiled, no copy kept");
        let (fence, pass) = last_depth_pass();
        let st = ground_state();
        assert_eq!((st.width, st.height, st.max_level, st.base_level), (w, h, 0, 0), "frame {n}: one level");
        assert_eq!(st.swizzle, [V3D_SWIZZLE_RED, V3D_SWIZZLE_ZERO, V3D_SWIZZLE_ZERO, V3D_SWIZZLE_ONE], "(d, 0, 0, 1)");
        let cpu = resolved(s.id, w, h);
        for y in 0..h {
            for x in 0..w {
                let tmu = tmu_depth(&st, pass.zs_address, x, y);
                let want = depth_of(depth_word(fence, x, y));
                assert_eq!(tmu, want, "frame {n}: ({x}, {y}): the texture unit reads the depth the GPU stored");
                assert_eq!(cpu[(y * w + x) as usize], want, "frame {n}: ({x}, {y}): the resolve reads the same");
            }
        }
    }
    /* the resolves above were this test's own reads of the buffer */
    let (direct, resolves, refused) = depth_samples();
    assert_eq!((direct, resolves, refused), (direct0 + 4, resolves0 + 4, refused0));
    assert_eq!(logs().iter().filter(|l| l.contains("sampled from its depth buffer (DEPTH24_X8)")).count(), 1);

    /* the CPU writes a texel: the texels are resolved first (the rendered
     * rest is kept), and the draw after it, with no render since, samples
     * the texels through a copy */
    let one = [0xffff_ffffu32];
    // SAFETY: plain GL calls; `one` is a 1x1 GL_UNSIGNED_INT depth image.
    unsafe {
        glBindTexture(GL_TEXTURE_2D, s.tex);
        glTexSubImage2D(
            GL_TEXTURE_2D,
            0,
            0,
            0,
            1,
            1,
            GL_DEPTH_COMPONENT,
            GL_UNSIGNED_INT,
            one.as_ptr() as *const c_void,
        );
        assert_eq!(glGetError(), GL_NO_ERROR, "{:?}", logs());
    }
    assert_eq!(depth_samples(), (direct, resolves + 1, refused), "the write resolved the texels once");
    ground(&s);
    assert_eq!(depth_samples(), (direct, resolves + 1, refused), "no render since: the texels, no resolve");
    assert_eq!(tilings_and_copies(), (tilings0 + 1, copies0 + 1), "through one tiled copy of the texels");
    let st = ground_state();
    assert_eq!(st.type_, V3D_TEXTURE_DATA_FORMAT_R32F, "the texels' own record");
    /* the next frame renders again: the buffer again, and the stale copy goes */
    frame(&s);
    assert_eq!(depth_samples(), (direct + 1, resolves + 1, refused));
    assert_eq!(ground_state().type_, V3D_TEXTURE_DATA_FORMAT_DEPTH24_X8);
    assert_eq!(tilings_and_copies().0, tilings0 + 1, "nothing more tiled");
}

/// A depth pass renders the texture, glTexSubImage2D writes a box of it,
/// and the next depth pass draws with GL_LESS and no clear: its job loads
/// the depth buffer at every tile's start, and what it loads (read here
/// through Mesa's tiling, as the TLB reads it) is the texture's texels as
/// the CPU holds them after the write -- the rendered depth resolved in
/// and the uploaded box over it -- each as the 24-bit depth in bits 31..8.
/// So the draw tests against the uploaded depth, and the job's store
/// (whatever the "GPU" writes) is what the next resolve reads, not the
/// buffer from before the write. The draw after it in the same pass loads
/// nothing new.
///
/// Revert check: without the texels the job loads the depth the first pass
/// stored (`depth_word` of its fence), not the uploaded box.
#[test]
fn a_gl_less_draw_after_gltexsubimage2d_starts_from_the_uploaded_depth() {
    let _g = rig();
    let s = setup();
    let (w, h) = (DEPTH.0 as u32, DEPTH.1 as u32);
    frame(&s);
    let (first, _) = last_depth_pass();
    /* a 3x2 box of GL_UNSIGNED_INT depth at (2, 1) */
    let boxed = [0u32, 0x4000_0000, 0x8000_0000, 0xc000_0000, 0xffff_ffff, 0x1234_5678];
    // SAFETY: plain GL calls; `boxed` is a 3x2 GL_UNSIGNED_INT depth image.
    unsafe {
        glBindTexture(GL_TEXTURE_2D, s.tex);
        glTexSubImage2D(
            GL_TEXTURE_2D,
            0,
            2,
            1,
            3,
            2,
            GL_DEPTH_COMPONENT,
            GL_UNSIGNED_INT,
            boxed.as_ptr() as *const c_void,
        );
        glBindTexture(GL_TEXTURE_2D, 0);
        assert_eq!(glGetError(), GL_NO_ERROR, "{:?}", logs());
    }
    let texels: Vec<f32> = {
        let t = &ctx().objs().textures[s.tex as usize];
        let (off, pitch) = (t.level_off[0] as usize, t.level_pitch_at(0) as usize);
        let base = (t.mem.cpu as *const u8).wrapping_add(off) as *const f32;
        // SAFETY: level 0's h rows of `pitch` floats, inside the texture's mapping.
        (0..h as usize).flat_map(|y| (0..w as usize).map(move |x| unsafe { base.add(y * pitch + x).read() })).collect()
    };
    assert_eq!((texels[(w + 4) as usize], texels[(2 * w + 3) as usize]), (0.5, 1.0), "the box is in the texels");
    assert_eq!(texels[0], depth_of(depth_word(first, 0, 0)), "and the rendered depth around it");
    /* the next depth pass, GL_LESS, no clear: two draws */
    // SAFETY: plain GL calls on live names.
    unsafe {
        glUseProgram(s.depth_prog);
        glBindFramebuffer(GL_FRAMEBUFFER, s.fbo);
        glViewport(0, 0, DEPTH.0, DEPTH.1);
        glColorMask(0, 0, 0, 0);
        glDepthFunc(GL_LESS);
    }
    draw_quad();
    draw_quad();
    // SAFETY: plain GL calls.
    unsafe {
        glFinish();
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glViewport(0, 0, CANVAS.0, CANVAS.1);
        glColorMask(1, 1, 1, 1);
        assert_eq!(glGetError(), GL_NO_ERROR, "{:?}", logs());
    }
    let (fence, pass) = last_depth_pass();
    assert!(fence > first && pass.zs_load, "the pass's job loads the buffer: {pass:?}");
    assert_eq!(jobs()[fence - 1].tmu.len(), 2, "both draws in that one job");
    let loaded = with_platform(|p| p.loads.iter().rev().find(|(f, _)| *f == fence).map(|(_, l)| l.clone()));
    let loaded = loaded.expect("the job's load was seen");
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) as usize;
            let want = yakogl::ds_retention::depth_unorm24(texels[i]);
            assert_eq!(loaded[i] >> 8, want, "({x}, {y}): the job starts from the texels");
        }
    }
    /* the store after it is what the resolve reads now */
    assert_eq!(resolved(s.id, 1, 1)[0], depth_of(depth_word(fence, 0, 0)));
}
