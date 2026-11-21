// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The Pi 4's dEQP-GLES2 image T5 on the host: do the seven
//! texture.size.2d.2048x2048_* cases fit the V3D memory that
//! driver/samples/zephyr/deqp/texture-2048.conf gives that image?
//!
//! T5 is one image running the seven cases one after the other in one
//! context (tools/cts/pi4-full-tree.txt, image T5): l8, l8_mipmap, rgb888,
//! rgb888_mipmap, rgba4444_mipmap, rgba8888, rgba8888_mipmap, the mustpass
//! order. Its configuration is boards/rpi_4b.conf plus texture-2048.conf:
//! `CONFIG_YAKOGL_V3D_CARVEOUT_BYTES` and
//! `CONFIG_YAKOGL_V3D_TILED_COPIES_BYTES` (the budget of the tiled copies
//! the back end keeps across jobs), which this test reads from that file.
//! The whole Pi 4 tree at 077ce85c failed
//! in T5.
//!
//! The rig is the real GL core (a `report-es2` build: an ES 2.0 context,
//! GLSL ES 1.00) against the real V3D back end. The job builder is stood in
//! for (it records nothing; as in tests/v3d_es2ctx_refusals.rs), while the
//! texture layout and tiling code (driver/src/v3d/v3d_tex.c) and the frame
//! sizes that size the tile memory (driver/src/v3d/v3d_frame.c
//! `v3d_frame_tiling_*` and `v3d_frame_alloc_sizes`) are the real C, built
//! at test time and reached through trampolines.
//!
//! The platform is the Zephyr OSAL's carve-out (src/osal/zephyr_v3d.c), with
//! its own allocator: one block of the configured size, its first 512 KiB the page
//! table of the 512 MiB GPU address space and then a scratch page; the heap
//! after them is handed out by `v3d_granules_place` (src/osal/v3d_granules.h:
//! 256-byte granules, 4 KiB alignment from 4 KiB up, first fit), the two
//! 256 KiB binner overflow blocks taken first (`v3d_granules_find`, the
//! Kconfig default of 2). A free made while the back end records a job
//! (OSAL_QUERY_V3D_JOB_OPEN) is held until that job is submitted, as
//! `v3d_frees_free` fences it at `submitted + 1`; a job completes when it is
//! submitted, so this is the least the board holds. The queries are the
//! Zephyr OSAL's answers for this image: split depth/stencil on, S8 off, 1024
//! draws a job, the TFU off (rpi_4b.conf has no CONFIG_YAKOGL_V3D_TFU).
//!
//! Each case is what es2fTextureSizeTests.cpp's Texture2DSizeCase does: a
//! 2048x2048 texture of the case's format, each level given with
//! glTexImage2D at UNPACK_ALIGNMENT 1 (glu::Texture2D::upload; with
//! `_mipmap`, all 12 levels down to 1x1), GL_CLAMP_TO_EDGE, min filter
//! GL_NEAREST_MIPMAP_NEAREST (with mipmaps) or GL_NEAREST, mag GL_NEAREST,
//! one quad drawn with glDrawElements over a 128x128 viewport of the
//! 256x256 pbuffer (CONFIG_DEQP_SURFACE_SIZE), read back with glReadPixels,
//! the texture deleted (deinit).
//!
//! What is asserted: no draw refused and no GL error in any case; no
//! allocation refused; the carve-out's high-water (granule-rounded bytes
//! in use, the page table and scratch page not counted) leaves at least
//! `CARVE_MARGIN` of the heap free; the tiled copies' high-water
//! (`v3d_tiled_copies`) leaves at least `TILED_MARGIN` of the budget.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table,report-es2 --test v3d_es2ctx_t5_mem
//! ```
#![cfg(all(
    feature = "shaderc-online",
    feature = "gpu-v3d",
    feature = "host-test",
    feature = "report-es2",
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]

use core::ffi::{c_char, c_int, c_void, CStr};
use std::collections::BTreeMap;
use std::ffi::CString;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use yakogl::gl::consts::*;
use yakogl::gl::context;
use yakogl::gl::draw::glDrawElements;
use yakogl::gl::framebuffer::glReadPixels;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::{glEnableVertexAttribArray, glVertexAttribPointer};
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::v3d::backend::{v3d_refused_draws, v3d_tiled_copies, yakogl_backend_v3d};
use yakogl::v3d::ffi::*;
use yakogl::v3d::kick::V3dKick;

/* ---- the image's memory (texture-2048.conf) ---------------------------------- */

const MIB: u64 = 1 << 20;
/// The image's own configuration file, read as the build reads it.
const T5_CONF: &str = include_str!("../driver/samples/zephyr/deqp/texture-2048.conf");

/// `CONFIG_<key>=<bytes>` of texture-2048.conf.
fn conf(key: &str) -> u64 {
    let want = format!("CONFIG_{key}=");
    let v = T5_CONF.lines().find_map(|l| l.trim().strip_prefix(want.as_str()));
    v.and_then(|v| v.trim().parse().ok()).unwrap_or_else(|| panic!("texture-2048.conf sets CONFIG_{key}"))
}

/// CONFIG_YAKOGL_V3D_CARVEOUT_BYTES of driver/samples/zephyr/deqp/texture-2048.conf.
fn carve() -> u64 {
    conf("YAKOGL_V3D_CARVEOUT_BYTES")
}

/// CONFIG_YAKOGL_V3D_TILED_COPIES_BYTES of the same file.
fn tiled_budget() -> u64 {
    conf("YAKOGL_V3D_TILED_COPIES_BYTES")
}
/// The heap's free bytes at the image's peak must be at least this: room
/// for what the stand-in job builder and the host do not model (the
/// pbuffer's depth/stencil buffers, dEQP's own programs, a free held a
/// little longer by a job still running on the board).
const CARVE_MARGIN: u64 = 4 * MIB;
/// And the tiled copies' high-water must stay this far under the budget.
const TILED_MARGIN: u64 = MIB;

const GRANULE: u64 = 256;
const PAGE: u64 = 4096;
/// The page table of the 512 MiB GPU address space (zephyr_v3d.c PT_BYTES).
const PT_BYTES: u64 = (0x2000_0000 / PAGE) * 4;
/// The allocator's first byte: past the page table and the scratch page.
const HEAP_OFF: u64 = PT_BYTES + PAGE;
/// The heap: what the allocator hands out.
fn heap() -> u64 {
    carve() - HEAP_OFF
}
/// The carve-out's first GPU address (zephyr_v3d.c VA_BASE).
const VA_BASE: u64 = 0x0010_0000;
/// The binner's overflow blocks, taken at init (CONFIG_YAKOGL_V3D_BIN_OVERFLOW_BLOCKS).
const OVERFLOW_BLOCKS: u64 = 2;
const OVERFLOW_BLOCK: u64 = 256 * 1024;

/* ---- the real C: layout, tiling, frame sizes and the granule allocator ------- */

extern "C" {
    fn dlopen(path: *const c_char, flags: c_int) -> *mut c_void;
    fn dlsym(h: *mut c_void, name: *const c_char) -> *mut c_void;
}

/// The OSAL's allocator over one map (zephyr_v3d.c `z_gpu_alloc`'s rule).
const SHIM: &str = r#"
#include "v3d_granules.h"
#include "v3d_frees.h"
#include <stdlib.h>

struct pool { struct v3d_granules gm; uint32_t *map; };

void *shim_new(uint32_t first, uint32_t n)
{
	struct pool *p = calloc(1, sizeof(*p));
	p->map = calloc(n / 32, sizeof(uint32_t));
	v3d_granules_init(&p->gm, p->map, first, n);
	return p;
}

/* the binner overflow blocks: v3d_granules_find, a page's alignment */
int64_t shim_reserve(struct pool *p, uint64_t size)
{
	uint32_t n = (uint32_t)(size / 256u);
	int64_t g = v3d_granules_find(&p->gm, n, 4096u / 256u);
	if (g < 0)
		return -1;
	v3d_granules_mark(&p->gm, (uint32_t)g, n, true);
	return g * 256;
}

/* z_gpu_alloc: whole granules, a page's alignment from a page up */
int64_t shim_alloc(struct pool *p, uint64_t size)
{
	uint32_t n = (uint32_t)((size + 255u) / 256u);
	uint32_t align = size >= 4096u ? 4096u / 256u : 1;
	int64_t g = v3d_granules_place(&p->gm, n, align);
	if (g < 0)
		return -1;
	v3d_granules_mark(&p->gm, (uint32_t)g, n, true);
	return g * 256;
}

void shim_free(struct pool *p, uint64_t off, uint64_t size)
{
	v3d_granules_mark(&p->gm, (uint32_t)(off / 256u), (uint32_t)((size + 255u) / 256u), false);
}

uint64_t shim_longest(struct pool *p)
{
	uint32_t longest = 0;
	v3d_free_runs(p->map, p->gm.first, p->gm.n, &longest, NULL);
	return (uint64_t)longest * 256u;
}
"#;

fn lib() -> *mut c_void {
    static L: OnceLock<usize> = OnceLock::new();
    *L.get_or_init(|| {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("v3d_es2ctx_t5_mem");
        std::fs::create_dir_all(&dir).expect("the scratch directory");
        let src = dir.join(format!("shim-{}.c", std::process::id()));
        let so = dir.join(format!("shim-{}.so", std::process::id()));
        std::fs::write(&src, SHIM).expect("the shim source");
        /* the compiler's own temporary files next to the object */
        let st = Command::new("cc")
            .current_dir(&root)
            .env("TMPDIR", &dir)
            .args(["-std=c11", "-O1", "-shared", "-fPIC", "-Wall", "-Wextra", "-Werror"])
            .args(["-Idriver/src/v3d", "-Idriver/src/v3d/generated", "-Isrc/osal"])
            .args(["-Ithird_party/mesa-26.2.1/src/broadcom", "-Ithird_party/mesa-26.2.1/src"])
            .arg("-Ithird_party/mesa-26.2.1/include")
            .arg(&src)
            .args(["driver/src/v3d/v3d_frame.c", "driver/src/v3d/v3d_tex.c", "driver/src/v3d/v3d_tiling_mesa.c"])
            .args(["-lm", "-o"])
            .arg(&so)
            .status()
            .expect("cc runs");
        assert!(st.success(), "the shim, v3d_frame.c and v3d_tex.c build");
        let path = CString::new(so.to_str().expect("a utf-8 path")).unwrap();
        // SAFETY: a path of a library this test just built; RTLD_NOW.
        let h = unsafe { dlopen(path.as_ptr(), 2) };
        assert!(!h.is_null(), "the shim loads");
        let _ = std::fs::remove_file(&src);
        let _ = std::fs::remove_file(&so);
        h as usize
    }) as *mut c_void
}

fn sym(name: &str) -> *mut c_void {
    let c = CString::new(name).unwrap();
    // SAFETY: a handle `lib` made and a NUL-terminated name.
    let p = unsafe { dlsym(lib(), c.as_ptr()) };
    assert!(!p.is_null(), "{name} in the shim");
    p
}

/// The trampolines: the library's `extern "C"` functions (src/v3d/ffi.rs),
/// each handed to the real C.
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

    trampoline!(v3d_frame_tiling(width: u32, height: u32, t: *mut V3dTiling));
    trampoline!(v3d_frame_tiling_rts(width: u32, height: u32, rts: u32, t: *mut V3dTiling));
    trampoline!(v3d_frame_tiling_ms(width: u32, height: u32, rts: u32, msaa: bool, t: *mut V3dTiling));
    trampoline!(v3d_frame_tiling_bpp(width: u32, height: u32, rts: u32, msaa: bool, max_bpp: u32, t: *mut V3dTiling));
    trampoline!(v3d_frame_alloc_sizes(t: *const V3dTiling, draws: u32, page_size: u32, tile_alloc_size: *mut u32,
                                      tile_state_size: *mut u32));
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
    trampoline!(v3d_tex_pack_state(out: *mut u8, s: *const V3dTexState) -> bool);
    trampoline!(v3d_tex_pack_sampler(out: *mut u8, s: *const V3dSampler) -> bool);
    trampoline!(v3d_tex_pack_sampler_compare(out: *mut u8, s: *const V3dSampler, compare_op: u32) -> bool);
}

/* ---- the job builder: records, draws nothing ---------------------------------- */

static LOG: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn log_lines() -> Vec<String> {
    LOG.lock().unwrap_or_else(|e| e.into_inner()).clone()
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
    t.width != 0 && t.height != 0
}

#[no_mangle]
unsafe extern "C" fn v3d_job_draw(j: *mut V3dJob, _d: *const V3dJobDraw, _layout: *mut c_void) -> i32 {
    // SAFETY: the backend passes its live job.
    unsafe {
        (*j).draws += 1;
        (*(*j).bcl).used += 120;
    }
    V3D_JOB_OK
}

#[no_mangle]
unsafe extern "C" fn v3d_job_end(
    j: *mut V3dJob,
    _c: *const V3dJobClear,
    _tile_list_start: *mut u32,
    _tile_list_end: *mut u32,
) -> bool {
    // SAFETY: the backend passes its live job; its lists are live.
    unsafe {
        (*(*j).bcl).used += 1;
        (*(*j).rcl).used = 200;
    }
    true
}

/* ---- the platform: the carve-out, its allocator, jobs done at submit --------- */

struct Platform {
    /// The carve-out's bytes (u64s for the alignment of what is handed out).
    carve: Box<[u64]>,
    pool: usize,
    /// Live allocations: offset into the carve-out -> requested bytes.
    live: BTreeMap<u64, u64>,
    /// Frees fenced at a job not submitted yet: (fence, offset, bytes).
    held: Vec<(u64, u64, u64)>,
    job_open: bool,
    submitted: u64,
    /// Granule-rounded bytes in use (live and held), and their high-water.
    used: u64,
    peak: u64,
    /// The longest free run when the high-water was reached.
    longest_at_peak: u64,
    /// Allocations the allocator could not place, and the largest of them.
    refused: u32,
    refused_largest: u64,
}

static PLATFORM: Mutex<Option<Platform>> = Mutex::new(None);

fn with_platform<R>(f: impl FnOnce(&mut Platform) -> R) -> R {
    let mut g = PLATFORM.lock().unwrap_or_else(|e| e.into_inner());
    f(g.as_mut().expect("the platform is set up"))
}

fn granules(bytes: u64) -> u64 {
    bytes.div_ceil(GRANULE) * GRANULE
}

impl Platform {
    fn new() -> Platform {
        // SAFETY: shim_new's C signature.
        let new: extern "C" fn(u32, u32) -> *mut c_void = unsafe { std::mem::transmute(sym("shim_new")) };
        let pool = new((HEAP_OFF / GRANULE) as u32, (carve() / GRANULE) as u32) as usize;
        let mut p = Platform {
            carve: vec![0u64; (carve() / 8) as usize].into_boxed_slice(),
            pool,
            live: BTreeMap::new(),
            held: Vec::new(),
            job_open: false,
            submitted: 0,
            used: 0,
            peak: 0,
            longest_at_peak: 0,
            refused: 0,
            refused_largest: 0,
        };
        // SAFETY: shim_reserve's C signature.
        let reserve: extern "C" fn(usize, u64) -> i64 = unsafe { std::mem::transmute(sym("shim_reserve")) };
        for _ in 0..OVERFLOW_BLOCKS {
            assert!(reserve(pool, OVERFLOW_BLOCK) >= 0, "the binner overflow blocks fit");
            p.used += OVERFLOW_BLOCK;
        }
        p.peak = p.used;
        p
    }

    fn release(&mut self, off: u64, bytes: u64) {
        // SAFETY: shim_free's C signature.
        let free: extern "C" fn(usize, u64, u64) = unsafe { std::mem::transmute(sym("shim_free")) };
        free(self.pool, off, bytes);
        self.used -= granules(bytes);
    }

    /// The held frees whose job has completed (every submitted job has).
    fn drain(&mut self) {
        let done = self.submitted;
        let (ready, kept): (Vec<_>, Vec<_>) = self.held.drain(..).partition(|h| h.0 <= done);
        self.held = kept;
        for (_, off, bytes) in ready {
            self.release(off, bytes);
        }
    }

    fn longest(&self) -> u64 {
        // SAFETY: shim_longest's C signature.
        let f: extern "C" fn(usize) -> u64 = unsafe { std::mem::transmute(sym("shim_longest")) };
        f(self.pool)
    }

    fn alloc(&mut self, size: u64) -> Option<u64> {
        // SAFETY: shim_alloc's C signature.
        let place: extern "C" fn(usize, u64) -> i64 = unsafe { std::mem::transmute(sym("shim_alloc")) };
        self.drain();
        let off = place(self.pool, size);
        if off < 0 {
            self.refused += 1;
            self.refused_largest = self.refused_largest.max(size);
            return None;
        }
        let off = off as u64;
        self.live.insert(off, size);
        self.used += granules(size);
        if self.used > self.peak {
            self.peak = self.used;
            self.longest_at_peak = self.longest();
        }
        Some(off)
    }

    fn cpu(&mut self, off: u64) -> *mut u8 {
        // SAFETY: `off` is inside the carve-out (the allocator's range).
        unsafe { (self.carve.as_mut_ptr() as *mut u8).add(off as usize) }
    }
}

unsafe extern "C" fn p_gpu_alloc(_heap: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
    if size == 0 || size as u64 > carve() {
        return -22;
    }
    with_platform(|p| match p.alloc(size as u64) {
        None => -12,
        Some(off) => {
            let cpu = p.cpu(off) as *mut c_void;
            let gpu = VA_BASE + off;
            // SAFETY: the caller passes a writable descriptor.
            unsafe { *out = OsalGpuMem { cpu, gpu, phys: gpu, size, impl_: gpu as *mut c_void } };
            0
        }
    })
}

unsafe extern "C" fn p_gpu_free(mem: *mut OsalGpuMem) -> i32 {
    // SAFETY: the caller passes its descriptor.
    let m = unsafe { &mut *mem };
    if !m.impl_.is_null() {
        with_platform(|p| {
            let off = m.gpu - VA_BASE;
            let bytes = p.live.remove(&off).expect("a free of a live allocation");
            p.drain();
            /* v3d_frees_free: fenced at the job being recorded */
            let fence = if p.job_open { p.submitted + 1 } else { p.submitted };
            if fence <= p.submitted {
                p.release(off, bytes);
            } else {
                p.held.push((fence, off, bytes));
            }
        });
    }
    *m = OsalGpuMem { cpu: core::ptr::null_mut(), gpu: 0, phys: 0, size: 0, impl_: core::ptr::null_mut() };
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
        p.drain();
        p.submitted as i64
    })
}

unsafe extern "C" fn p_wait(_fence: i64, _timeout_us: i64) -> i32 {
    0
}

unsafe extern "C" fn p_completed() -> i64 {
    with_platform(|p| p.submitted as i64)
}

fn answer(out: *mut c_void, out_sz: usize, v: u32) -> i32 {
    if out_sz != 4 {
        return -22;
    }
    // SAFETY: a u32 out-parameter of the size passed.
    unsafe { *(out as *mut u32) = v };
    0
}

unsafe extern "C" fn p_query(what: u32, out: *mut c_void, out_sz: usize) -> i32 {
    use yakogl::osal::*;
    match what {
        OSAL_QUERY_V3D_TILED_BUDGET => answer(out, out_sz, tiled_budget() as u32),
        OSAL_QUERY_V3D_VPM_BYTES => answer(out, out_sz, 65536),
        OSAL_QUERY_V3D_SPLIT_ZS => answer(out, out_sz, 1),
        OSAL_QUERY_V3D_S8_STENCIL => answer(out, out_sz, 0),
        OSAL_QUERY_V3D_DRAWS_PER_JOB => answer(out, out_sz, 1024),
        OSAL_QUERY_V3D_JOB_OPEN if out_sz == 4 => {
            // SAFETY: the backend's u32 in-parameter.
            let open = unsafe { *(out as *const u32) } != 0;
            with_platform(|p| {
                p.job_open = open;
                if !open {
                    p.drain();
                }
            });
            0
        }
        OSAL_QUERY_V3D_CPU_OF_GPU if out_sz == size_of::<OsalV3dCpuOf>() => {
            // SAFETY: the backend's in/out struct of that size.
            let q = unsafe { &mut *(out as *mut OsalV3dCpuOf) };
            with_platform(|p| {
                let off = q.gpu.wrapping_sub(VA_BASE);
                if q.gpu < VA_BASE + HEAP_OFF || off.checked_add(q.bytes).is_none_or(|e| e > carve()) {
                    return -22;
                }
                q.cpu = p.cpu(off) as *mut c_void;
                0
            })
        }
        OSAL_QUERY_V3D_BIN_POOL => -61,
        _ => -22,
    }
}

/// The library logs every line as `log("%s", line)` (`osal::log_str`).
unsafe extern "C" fn p_log2(_fmt: *const c_char, line: *const c_char) {
    // SAFETY: the library passes a NUL-terminated line.
    let s = unsafe { CStr::from_ptr(line) }.to_string_lossy().into_owned();
    LOG.lock().unwrap_or_else(|e| e.into_inner()).push(s);
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

/* ---- the seven cases ---------------------------------------------------------- */

/// CONFIG_DEQP_SURFACE_SIZE: the pbuffer's side.
const SURFACE: u32 = 256;
/// RandomViewport(renderTarget, 128, 128, ...): the case's viewport.
const VIEWPORT: GLsizei = 128;
const SIDE: GLsizei = 2048;

/// dEQP's 2D texture renderer, reduced: a position and a texture coordinate.
const VS: &str = "attribute highp vec4 a_position;\nattribute highp vec2 a_texCoord;\n\
                  varying mediump vec2 v_texCoord;\n\
                  void main() { gl_Position = a_position; v_texCoord = a_texCoord; }\n";
const FS: &str = "uniform lowp sampler2D u_sampler;\nvarying mediump vec2 v_texCoord;\n\
                  void main() { gl_FragColor = texture2D(u_sampler, v_texCoord); }\n";

/// The case name's format, its glTexImage2D format and type, and its bytes a texel.
const CASES: [(&str, GLenum, GLenum, usize, bool); 7] = [
    ("2048x2048_l8", GL_LUMINANCE, GL_UNSIGNED_BYTE, 1, false),
    ("2048x2048_l8_mipmap", GL_LUMINANCE, GL_UNSIGNED_BYTE, 1, true),
    ("2048x2048_rgb888", GL_RGB, GL_UNSIGNED_BYTE, 3, false),
    ("2048x2048_rgb888_mipmap", GL_RGB, GL_UNSIGNED_BYTE, 3, true),
    ("2048x2048_rgba4444_mipmap", GL_RGBA, GL_UNSIGNED_SHORT_4_4_4_4, 2, true),
    ("2048x2048_rgba8888", GL_RGBA, GL_UNSIGNED_BYTE, 4, false),
    ("2048x2048_rgba8888_mipmap", GL_RGBA, GL_UNSIGNED_BYTE, 4, true),
];

fn err() -> GLenum {
    // SAFETY: plain GL calls.
    unsafe {
        let e = glGetError();
        while glGetError() != GL_NO_ERROR {}
        e
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

fn program() -> GLuint {
    let (v, f) = (shader(GL_VERTEX_SHADER, VS), shader(GL_FRAGMENT_SHADER, FS));
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glBindAttribLocation(p, 0, c"a_position".as_ptr());
        glBindAttribLocation(p, 1, c"a_texCoord".as_ptr());
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "the program links");
        p
    }
}

static POS: [f32; 16] = [-1.0, -1.0, 0.0, 1.0, -1.0, 1.0, 0.0, 1.0, 1.0, -1.0, 0.0, 1.0, 1.0, 1.0, 0.0, 1.0];
static TEXC: [f32; 8] = [0.0, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0, 1.0];
static QUAD: [u16; 6] = [0, 1, 2, 2, 1, 3];

/// What one case left behind.
#[derive(Debug)]
struct CaseUse {
    name: &'static str,
    /// The carve-out's high-water since the image started, in bytes.
    carve_peak: u64,
    /// The tiled copies' high-water since the image started.
    tiled_peak: u64,
    refused_draws: u32,
    error: GLenum,
}

/// One Texture2DSizeCase: upload, draw, read back, delete.
fn run_case(prog: GLuint, name: &'static str, format: GLenum, type_: GLenum, cpp: usize, mipmaps: bool) -> CaseUse {
    let refused0 = v3d_refused_draws();
    let levels = if mipmaps { 32 - (SIDE as u32).leading_zeros() } else { 1 };
    let texels = vec![0x5au8; SIDE as usize * SIDE as usize * cpp];
    let mut pixels = vec![0u8; (VIEWPORT * VIEWPORT * 4) as usize];
    // SAFETY: plain GL calls; the client arrays are static or outlive the
    // calls that read them.
    let error = unsafe {
        let mut t = 0;
        glGenTextures(1, &mut t);
        glViewport(0, 0, VIEWPORT, VIEWPORT);
        /* glu::Texture2D::upload */
        glBindTexture(GL_TEXTURE_2D, t);
        glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
        for l in 0..levels {
            let side = (SIDE >> l).max(1);
            glTexImage2D(
                GL_TEXTURE_2D,
                l as GLint,
                format as GLint,
                side,
                side,
                0,
                format,
                type_,
                texels.as_ptr() as *const c_void,
            );
        }
        glActiveTexture(GL_TEXTURE0);
        glBindTexture(GL_TEXTURE_2D, t);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE as GLint);
        let min = if mipmaps { GL_NEAREST_MIPMAP_NEAREST } else { GL_NEAREST };
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, min as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
        /* TextureRenderer::renderQuad */
        glUseProgram(prog);
        glUniform1i(glGetUniformLocation(prog, c"u_sampler".as_ptr()), 0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, POS.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glVertexAttribPointer(1, 2, GL_FLOAT, 0, 0, TEXC.as_ptr() as *const c_void);
        glEnableVertexAttribArray(1);
        glDrawElements(GL_TRIANGLES, 6, GL_UNSIGNED_SHORT, QUAD.as_ptr() as *const c_void);
        let e = err();
        glReadPixels(0, 0, VIEWPORT, VIEWPORT, GL_RGBA, GL_UNSIGNED_BYTE, pixels.as_mut_ptr() as *mut c_void);
        let e = if e == GL_NO_ERROR { err() } else { e };
        /* deinit */
        glDeleteTextures(1, &t);
        glFinish();
        if e == GL_NO_ERROR {
            err()
        } else {
            e
        }
    };
    let (mut live, mut bytes, mut hw) = (0u32, 0u64, 0u64);
    // SAFETY: three out-parameters.
    unsafe { v3d_tiled_copies(&mut live, &mut bytes, &mut hw) };
    CaseUse {
        name,
        carve_peak: with_platform(|p| p.peak),
        tiled_peak: hw,
        refused_draws: v3d_refused_draws().wrapping_sub(refused0),
        error,
    }
}

/// The whole image, once: the platform, the back end, the 256x256 pbuffer,
/// then the seven cases in the mustpass order.
fn image() -> &'static Vec<CaseUse> {
    static RUN: OnceLock<Vec<CaseUse>> = OnceLock::new();
    RUN.get_or_init(|| {
        *PLATFORM.lock().unwrap_or_else(|e| e.into_inner()) = Some(Platform::new());
        // SAFETY: statics; this runs once, before any other GL call.
        unsafe {
            osal_set_ops(osal());
            yakogl::backend::g_backend = &yakogl_backend_v3d;
            assert_eq!(yakogl::backend::init(), 0, "{:?}", log_lines());
            assert!(!context::gl_context_create(SURFACE, SURFACE).is_null(), "{:?}", log_lines());
        }
        let prog = program();
        let init_used = with_platform(|p| p.used);
        eprintln!("v3d_es2ctx_t5_mem: in use after init, the pbuffer and the program: {init_used} bytes");
        CASES.iter().map(|&(name, format, type_, cpp, mip)| run_case(prog, name, format, type_, cpp, mip)).collect()
    })
}

/* ---- the asserts -------------------------------------------------------------- */

#[test]
fn every_t5_case_draws() {
    let run = image();
    for c in run {
        eprintln!("{c:?}");
    }
    for c in run {
        assert_eq!((c.error, c.refused_draws), (GL_NO_ERROR, 0), "{}: error, refused draws; {:?}", c.name, log_lines());
    }
    let refused = with_platform(|p| (p.refused, p.refused_largest));
    assert_eq!(refused, (0, 0), "allocations refused (count, largest bytes); {:?}", log_lines());
}

#[test]
fn the_carve_out_keeps_its_margin() {
    let run = image();
    let (heap, peak, longest, refused) = with_platform(|p| (heap(), p.peak, p.longest_at_peak, p.refused_largest));
    eprintln!(
        "v3d_es2ctx_t5_mem: carve-out high-water {peak} of {heap} heap bytes ({} free, longest free run {longest}); \
         per case {:?}",
        heap - peak,
        run.iter().map(|c| (c.name, c.carve_peak)).collect::<Vec<_>>()
    );
    /* a refused allocation keeps the high-water low: the margin is only
     * read when everything was placed */
    assert_eq!(refused, 0, "an allocation of {refused} bytes found no room in the {heap}-byte heap");
    assert!(peak + CARVE_MARGIN <= heap, "carve-out high-water {peak} leaves less than {CARVE_MARGIN} of {heap}");
}

/// The tiled copy of a 2048x2048 RGBA8 chain: `v3d_tex_layout`'s size
/// (texture-2048.conf's header, tests/v3d_es2ctx_refusals.rs
/// `a_large_texture_samples`).
const CHAIN_COPY: u64 = 22_376_448;

#[test]
fn the_tiled_copies_keep_their_margin() {
    let run = image();
    let (budget, hw) = (tiled_budget(), run.last().map_or(0, |c| c.tiled_peak));
    eprintln!("v3d_es2ctx_t5_mem: tiled copies high-water {hw} of {budget} bytes");
    assert!(hw >= CHAIN_COPY, "the mipmap cases keep the {CHAIN_COPY}-byte copy of their chain (high-water {hw})");
    assert!(hw + TILED_MARGIN <= budget, "tiled copies high-water {hw} leaves less than {TILED_MARGIN} of {budget}");
}
