// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The Pi 4's dEQP-GLES3 images T14 and T14a on the host: do their
//! texture.units cases with cube maps or many units fit the V3D memory of
//! boards/rpi_4b.conf, and of driver/samples/zephyr/deqp/texture-2048.conf?
//!
//! At b1b5b845, T14a aborted on texture.units.8_units.only_cube.2: the kept
//! tiled copies found no GPU memory (`osal(v3d): out of GPU memory`), no
//! empty job held the draw, and the draw answered `-E2BIG`
//! (docs/notes/v3d-refusals.md, T14a). The cases replayed here are those of
//! T14 and T14a (tools/cts/pi4-gles3-tree.txt) that use cube maps or many
//! units, in the mustpass order of tools/cts/gles3-mustpass.txt and in one
//! context, as the images run them: 4_units.mixed.7, 8_units.mixed.0-9,
//! 8_units.only_cube.0-9, all_units.mixed.0 and all_units.mixed.8.
//! all_units.only_cube.0 (also in T14a) is replayed alone, in a fresh
//! context. Each replay is a child process of this test binary (one GL
//! context and one back end a process) with one configuration's
//! `CONFIG_YAKOGL_V3D_CARVEOUT_BYTES` and
//! `CONFIG_YAKOGL_V3D_TILED_COPIES_BYTES`, read from
//! driver/samples/zephyr/deqp/boards/rpi_4b.conf or from texture-2048.conf.
//!
//! The rig is tests/v3d_es2ctx_t5_mem.rs's with an ES 3.0 context: the real
//! GL core against the real V3D back end, the job builder stood in for (it
//! records nothing), the texture layout and tiling (driver/src/v3d/v3d_tex.c)
//! and the frame sizes (driver/src/v3d/v3d_frame.c) the real C, and the
//! Zephyr OSAL's carve-out (src/osal/zephyr_v3d.c) with its allocator
//! (`v3d_granules_place`, src/osal/v3d_granules.h), the binner's two
//! overflow blocks taken first, a free made while a job is recorded held
//! until it is submitted (see that file for the details).
//!
//! Each case is what es3fTextureUnitTests.cpp's TextureUnitCase does
//! (VK-GL-CTS 93bca01): `de::Random` seeded with `deUint32Hash(testNdx)`
//! draws the number of textures (one a unit, or 30% of the time
//! `getInt(max(1, units - 2), units)`), each texture's type (mixed cases)
//! and sized internal format out of 49, its wraps, filters and grid
//! colours, the extra units' textures and the shuffle. Textures are 2D
//! 128x128, cube 256x256, 2D array 64x64x4 and 3D 32x32x32 with every
//! level, given with glTexImage2D/3D at UNPACK_ALIGNMENT 1 in the
//! format's transfer format (glu::getTransferFormat) when a unit first
//! binds them; one quad is drawn with a program sampling every unit over a
//! 128x128 viewport of the 256x256 pbuffer and read back; the textures and
//! the program are deleted. all_units is the context's
//! GL_MAX_TEXTURE_IMAGE_UNITS.
//!
//! A case "allocates" when during it no allocation was refused, no draw
//! was refused and no GL error was raised. Asserted:
//! (a) with rpi_4b.conf's carve-out and tiled budget, at least one
//!     8_units.only_cube case does not allocate (the board at b1b5b845);
//! (b) with texture-2048.conf's, every replayed case allocates. Ignored:
//!     8_units.only_cube.3, .4 and .8 do not; the smallest carve-out that
//!     holds them all is 105 MiB (110,100,480 bytes) at the 24 MiB tiled
//!     budget (`the_smallest_numbers_hold_every_case` pins it);
//! (c) all_units.only_cube.0 alone in a fresh context: its peak under
//!     both, and that it allocates under neither (`ALL_CUBE_FITS`). It
//!     needs a tiled budget of at least 30,146,560 bytes and, with 32 MiB,
//!     a carve-out of 95,420,416 (91 MiB); 105 MiB and 32 MiB hold it after
//!     the other cases in one context.
//! Every case's peak (granule-rounded bytes in use, the page table and the
//! scratch page not counted) is printed with `--nocapture`.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d_units_cube_mem -- --nocapture
//! ```
#![cfg(all(
    feature = "shaderc-online",
    feature = "gpu-v3d",
    feature = "host-test",
    not(feature = "report-es2"),
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

/* ---- the images' memory (rpi_4b.conf, texture-2048.conf) ---------------------- */

/// The deqp sample's board file: every Pi 4 image's configuration.
const BOARD_CONF: &str = include_str!("../driver/samples/zephyr/deqp/boards/rpi_4b.conf");
/// The extra configuration file the large-texture images add.
const T2048_CONF: &str = include_str!("../driver/samples/zephyr/deqp/texture-2048.conf");

/// `CONFIG_<key>=<bytes>` of a configuration file.
fn conf_in(file: &str, text: &str, key: &str) -> u64 {
    let want = format!("CONFIG_{key}=");
    let v = text.lines().find_map(|l| l.trim().strip_prefix(want.as_str()));
    v.and_then(|v| v.trim().parse().ok()).unwrap_or_else(|| panic!("{file} sets CONFIG_{key}"))
}

/// A configuration's carve-out and tiled-copy budget.
#[derive(Clone, Copy, Debug)]
struct Mem {
    carve: u64,
    tiled: u64,
}

/// "rpi_4b" (the board file alone) or "texture-2048" (with texture-2048.conf).
fn mem_of(name: &str) -> Mem {
    let (file, text) = match name {
        "rpi_4b" => ("rpi_4b.conf", BOARD_CONF),
        "texture-2048" => ("texture-2048.conf", T2048_CONF),
        _ => panic!("no configuration {name}"),
    };
    Mem {
        carve: conf_in(file, text, "YAKOGL_V3D_CARVEOUT_BYTES"),
        tiled: conf_in(file, text, "YAKOGL_V3D_TILED_COPIES_BYTES"),
    }
}

/// The child's carve-out and tiled budget: `V3D_UNITS_CARVE` and
/// `V3D_UNITS_TILED`, in bytes.
fn mem() -> Mem {
    static M: OnceLock<Mem> = OnceLock::new();
    *M.get_or_init(|| {
        let var =
            |k: &str| std::env::var(k).ok().and_then(|v| v.parse().ok()).unwrap_or_else(|| panic!("{k} in the child"));
        Mem { carve: var("V3D_UNITS_CARVE"), tiled: var("V3D_UNITS_TILED") }
    })
}

/// CONFIG_YAKOGL_V3D_CARVEOUT_BYTES of the child's configuration.
fn carve() -> u64 {
    mem().carve
}

/// CONFIG_YAKOGL_V3D_TILED_COPIES_BYTES of the same file.
fn tiled_budget() -> u64 {
    mem().tiled
}

const GRANULE: u64 = 256;
const PAGE: u64 = 4096;
/// The page table of the 512 MiB GPU address space (zephyr_v3d.c PT_BYTES).
const PT_BYTES: u64 = (0x2000_0000 / PAGE) * 4;
/// The allocator's first byte: past the page table and the scratch page.
const HEAP_OFF: u64 = PT_BYTES + PAGE;
/// The heap a configuration's allocator hands out.
fn heap_of(m: Mem) -> u64 {
    m.carve - HEAP_OFF
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
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("v3d_units_cube_mem");
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
    /// The high-water and the refused allocations since the case began.
    case_peak: u64,
    case_refused: u32,
    case_refused_largest: u64,
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
            case_peak: 0,
            case_refused: 0,
            case_refused_largest: 0,
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
            self.case_refused += 1;
            self.case_refused_largest = self.case_refused_largest.max(size);
            return None;
        }
        let off = off as u64;
        self.live.insert(off, size);
        self.used += granules(size);
        self.case_peak = self.case_peak.max(self.used);
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

/// all_units.only_cube.0 alone allocates with (rpi_4b.conf, texture-2048.conf).
const ALL_CUBE_FITS: (bool, bool) = (false, false);

/* ---- de::Random and deUint32Hash (VK-GL-CTS 93bca01, framework/delibs) -------- */

/// `deUint32Hash` (deInt32.h `deInt32Hash`): the cases' seeds.
fn de_hash(a: u32) -> u32 {
    let mut k = a;
    k = (k ^ 61) ^ (k >> 16);
    k = k.wrapping_add(k << 3);
    k ^= k >> 4;
    k = k.wrapping_mul(0x27d4_eb2d);
    k ^ (k >> 15)
}

/// `de::Random` (deRandom.c, deRandom.hpp): xorshift128.
struct Rnd {
    x: u32,
    y: u32,
    z: u32,
    w: u32,
}

impl Rnd {
    fn new(seed: u32) -> Rnd {
        Rnd {
            x: (seed as i32).wrapping_neg() as u32 ^ 123_456_789,
            y: 362_436_069u32.wrapping_mul(seed),
            z: 521_288_629 ^ (seed >> 7),
            w: 88_675_123 ^ (seed << 3),
        }
    }

    fn u32(&mut self) -> u32 {
        let w = self.w;
        let t = self.x ^ (self.x << 11);
        self.x = self.y;
        self.y = self.z;
        self.z = w;
        self.w = (w ^ (w >> 19)) ^ (t ^ (t >> 8));
        self.w
    }

    fn float(&mut self) -> f32 {
        (self.u32() & 0x0fff_ffff) as f32 / (0x0fff_ffffu32 + 1) as f32
    }

    /// `getInt(min, max)`, both inclusive.
    fn int(&mut self, min: i32, max: i32) -> i32 {
        min + (self.u32() % ((max as u32).wrapping_sub(min as u32) + 1)) as i32
    }

    /// `shuffle`: Fisher-Yates from the back, `j = getInt(0, i)`.
    fn shuffle<T>(&mut self, v: &mut [T]) {
        for i in (1..v.len()).rev() {
            let j = self.int(0, i as i32) as usize;
            v.swap(i, j);
        }
    }
}

/* ---- the cases (es3fTextureUnitTests.cpp) -------------------------------------- */

/// A texture's type: CASE_ONLY_2D, CASE_ONLY_CUBE, CASE_ONLY_2D_ARRAY,
/// CASE_ONLY_3D, the order a mixed case's `getInt(0, CASE_MIXED - 1)` picks.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    Tex2d,
    Cube,
    Array,
    Tex3d,
}

const KINDS: [Kind; 4] = [Kind::Tex2d, Kind::Cube, Kind::Array, Kind::Tex3d];

/// s_testSizedInternalFormats in order, each with its transfer format and
/// type (glu::getTransferFormat), its client bytes a texel, whether
/// glu::isGLInternalColorFormatFilterable holds, and its sampler prefix.
#[rustfmt::skip]
const FORMATS: [(GLenum, GLenum, GLenum, usize, bool, &str); 49] = [
    (GL_RGBA32F, GL_RGBA, GL_FLOAT, 16, false, ""),
    (GL_RGBA32I, GL_RGBA_INTEGER, GL_INT, 16, false, "i"),
    (GL_RGBA32UI, GL_RGBA_INTEGER, GL_UNSIGNED_INT, 16, false, "u"),
    (GL_RGBA16F, GL_RGBA, GL_HALF_FLOAT, 8, true, ""),
    (GL_RGBA16I, GL_RGBA_INTEGER, GL_SHORT, 8, false, "i"),
    (GL_RGBA16UI, GL_RGBA_INTEGER, GL_UNSIGNED_SHORT, 8, false, "u"),
    (GL_RGBA8, GL_RGBA, GL_UNSIGNED_BYTE, 4, true, ""),
    (GL_RGBA8I, GL_RGBA_INTEGER, GL_BYTE, 4, false, "i"),
    (GL_RGBA8UI, GL_RGBA_INTEGER, GL_UNSIGNED_BYTE, 4, false, "u"),
    (GL_SRGB8_ALPHA8, GL_RGBA, GL_UNSIGNED_BYTE, 4, true, ""),
    (GL_RGB10_A2, GL_RGBA, GL_UNSIGNED_INT_2_10_10_10_REV, 4, true, ""),
    (GL_RGB10_A2UI, GL_RGBA_INTEGER, GL_UNSIGNED_INT_2_10_10_10_REV, 4, false, "u"),
    (GL_RGBA4, GL_RGBA, GL_UNSIGNED_SHORT_4_4_4_4, 2, true, ""),
    (GL_RGB5_A1, GL_RGBA, GL_UNSIGNED_SHORT_5_5_5_1, 2, true, ""),
    (GL_RGBA8_SNORM, GL_RGBA, GL_BYTE, 4, true, ""),
    (GL_RGB8, GL_RGB, GL_UNSIGNED_BYTE, 3, true, ""),
    (GL_RGB565, GL_RGB, GL_UNSIGNED_SHORT_5_6_5, 2, true, ""),
    (GL_R11F_G11F_B10F, GL_RGB, GL_UNSIGNED_INT_10F_11F_11F_REV, 4, true, ""),
    (GL_RGB32F, GL_RGB, GL_FLOAT, 12, false, ""),
    (GL_RGB32I, GL_RGB_INTEGER, GL_INT, 12, false, "i"),
    (GL_RGB32UI, GL_RGB_INTEGER, GL_UNSIGNED_INT, 12, false, "u"),
    (GL_RGB16F, GL_RGB, GL_HALF_FLOAT, 6, true, ""),
    (GL_RGB16I, GL_RGB_INTEGER, GL_SHORT, 6, false, "i"),
    (GL_RGB16UI, GL_RGB_INTEGER, GL_UNSIGNED_SHORT, 6, false, "u"),
    (GL_RGB8_SNORM, GL_RGB, GL_BYTE, 3, true, ""),
    (GL_RGB8I, GL_RGB_INTEGER, GL_BYTE, 3, false, "i"),
    (GL_RGB8UI, GL_RGB_INTEGER, GL_UNSIGNED_BYTE, 3, false, "u"),
    (GL_SRGB8, GL_RGB, GL_UNSIGNED_BYTE, 3, true, ""),
    (GL_RGB9_E5, GL_RGB, GL_UNSIGNED_INT_5_9_9_9_REV, 4, true, ""),
    (GL_RG32F, GL_RG, GL_FLOAT, 8, false, ""),
    (GL_RG32I, GL_RG_INTEGER, GL_INT, 8, false, "i"),
    (GL_RG32UI, GL_RG_INTEGER, GL_UNSIGNED_INT, 8, false, "u"),
    (GL_RG16F, GL_RG, GL_HALF_FLOAT, 4, true, ""),
    (GL_RG16I, GL_RG_INTEGER, GL_SHORT, 4, false, "i"),
    (GL_RG16UI, GL_RG_INTEGER, GL_UNSIGNED_SHORT, 4, false, "u"),
    (GL_RG8, GL_RG, GL_UNSIGNED_BYTE, 2, true, ""),
    (GL_RG8I, GL_RG_INTEGER, GL_BYTE, 2, false, "i"),
    (GL_RG8UI, GL_RG_INTEGER, GL_UNSIGNED_BYTE, 2, false, "u"),
    (GL_RG8_SNORM, GL_RG, GL_BYTE, 2, true, ""),
    (GL_R32F, GL_RED, GL_FLOAT, 4, false, ""),
    (GL_R32I, GL_RED_INTEGER, GL_INT, 4, false, "i"),
    (GL_R32UI, GL_RED_INTEGER, GL_UNSIGNED_INT, 4, false, "u"),
    (GL_R16F, GL_RED, GL_HALF_FLOAT, 2, true, ""),
    (GL_R16I, GL_RED_INTEGER, GL_SHORT, 2, false, "i"),
    (GL_R16UI, GL_RED_INTEGER, GL_UNSIGNED_SHORT, 2, false, "u"),
    (GL_R8, GL_RED, GL_UNSIGNED_BYTE, 1, true, ""),
    (GL_R8I, GL_RED_INTEGER, GL_BYTE, 1, false, "i"),
    (GL_R8UI, GL_RED_INTEGER, GL_UNSIGNED_BYTE, 1, false, "u"),
    (GL_R8_SNORM, GL_RED, GL_BYTE, 1, true, ""),
];

const WRAPS: [GLenum; 3] = [GL_CLAMP_TO_EDGE, GL_REPEAT, GL_MIRRORED_REPEAT];
const MIN_FILTERS: [GLenum; 6] = [
    GL_NEAREST,
    GL_LINEAR,
    GL_NEAREST_MIPMAP_NEAREST,
    GL_LINEAR_MIPMAP_NEAREST,
    GL_NEAREST_MIPMAP_LINEAR,
    GL_LINEAR_MIPMAP_LINEAR,
];
/// s_testNearestMinFilters: a mipmapped texture that is not filterable.
const NEAREST_MIN_FILTERS: [GLenum; 2] = [GL_NEAREST, GL_NEAREST_MIPMAP_NEAREST];
const MAG_FILTERS: [GLenum; 2] = [GL_NEAREST, GL_LINEAR];

/// One texture of a case.
#[derive(Clone, Copy, Debug)]
struct Tex {
    kind: Kind,
    format: usize,
    wrap: [GLenum; 3],
    min: GLenum,
    mag: GLenum,
}

impl Tex {
    /// (width, height, depth or layers, levels).
    fn dims(&self) -> (i32, i32, i32, i32) {
        match self.kind {
            Kind::Tex2d => (128, 128, 1, 8),
            Kind::Cube => (256, 256, 1, 9),
            Kind::Array => (64, 64, 4, 7),
            Kind::Tex3d => (32, 32, 32, 6),
        }
    }

    fn target(&self) -> GLenum {
        match self.kind {
            Kind::Tex2d => GL_TEXTURE_2D,
            Kind::Cube => GL_TEXTURE_CUBE_MAP,
            Kind::Array => GL_TEXTURE_2D_ARRAY,
            Kind::Tex3d => GL_TEXTURE_3D,
        }
    }
}

/// What TextureUnitCase::init draws: the textures and each unit's texture.
#[derive(Debug)]
struct Plan {
    textures: Vec<Tex>,
    units: Vec<usize>,
}

/// TextureUnitCase::init for `units` units, the case type (`None`:
/// mixed) and the case's index in its group.
fn plan(units: usize, only: Option<Kind>, idx: u32) -> Plan {
    let mut rnd = Rnd::new(de_hash(idx));
    let n = units as i32;
    let num = if rnd.float() < 0.7 { n } else { rnd.int((n - 2).max(1), n) } as usize;
    let mut textures = Vec::new();
    for _ in 0..num {
        let kind = only.unwrap_or_else(|| KINDS[rnd.int(0, 3) as usize]);
        let format = rnd.int(0, FORMATS.len() as i32 - 1) as usize;
        let wrap = [0; 3].map(|_| WRAPS[rnd.int(0, 2) as usize]);
        let filterable = FORMATS[format].4;
        /* every size is a power of two: always mipmapped */
        let mag = if filterable { MAG_FILTERS[rnd.int(0, 1) as usize] } else { GL_NEAREST };
        let min =
            if filterable { MIN_FILTERS[rnd.int(0, 5) as usize] } else { NEAREST_MIN_FILTERS[rnd.int(0, 1) as usize] };
        /* the grid colour, once a face */
        for _ in 0..if kind == Kind::Cube { 6 } else { 1 } {
            let _rgb = rnd.u32() & 0x00ff_ffff;
        }
        textures.push(Tex { kind, format, wrap, min, mag });
    }
    let mut unit_tex: Vec<usize> = (0..num).collect();
    while unit_tex.len() < units {
        unit_tex.push(rnd.int(0, num as i32 - 1) as usize);
    }
    rnd.shuffle(&mut unit_tex);
    let _shader_seed = rnd.u32();
    Plan { textures, units: unit_tex }
}

/// The replayed cases of T14 and T14a in the mustpass order (the group
/// is `units`: 0 for all_units; `None` for mixed), and all_units.only_cube.0.
const T14_CASES: [&str; 23] = [
    "4_units.mixed.7",
    "8_units.mixed.0",
    "8_units.mixed.1",
    "8_units.mixed.2",
    "8_units.mixed.3",
    "8_units.mixed.4",
    "8_units.mixed.5",
    "8_units.mixed.6",
    "8_units.mixed.7",
    "8_units.mixed.8",
    "8_units.mixed.9",
    "8_units.only_cube.0",
    "8_units.only_cube.1",
    "8_units.only_cube.2",
    "8_units.only_cube.3",
    "8_units.only_cube.4",
    "8_units.only_cube.5",
    "8_units.only_cube.6",
    "8_units.only_cube.7",
    "8_units.only_cube.8",
    "8_units.only_cube.9",
    "all_units.mixed.0",
    "all_units.mixed.8",
];
const ALL_CUBE: &str = "all_units.only_cube.0";

/// A case name's units (0: GL_MAX_TEXTURE_IMAGE_UNITS), type and index.
fn parse_case(name: &str) -> (usize, Option<Kind>, u32) {
    let mut it = name.split('.');
    let (g, t, i) = (it.next().unwrap(), it.next().unwrap(), it.next().unwrap());
    let units = if g == "all_units" { 0 } else { g.trim_end_matches("_units").parse().unwrap() };
    let only = match t {
        "mixed" => None,
        "only_2d" => Some(Kind::Tex2d),
        "only_cube" => Some(Kind::Cube),
        "only_2d_array" => Some(Kind::Array),
        "only_3d" => Some(Kind::Tex3d),
        _ => panic!("no case type {t}"),
    };
    (units, only, i.parse().unwrap())
}

/* ---- one case on the GL core --------------------------------------------------- */

/// CONFIG_DEQP_SURFACE_SIZE: the pbuffer's side.
const SURFACE: u32 = 256;
/// The case's viewport (VIEWPORT_WIDTH, VIEWPORT_HEIGHT).
const VIEWPORT: GLsizei = 128;

fn err() -> GLenum {
    // SAFETY: plain GL calls.
    unsafe {
        let e = glGetError();
        while glGetError() != GL_NO_ERROR {}
        e
    }
}

fn shader(ty: GLenum, text: &str) -> GLuint {
    let src = CString::new(text).unwrap();
    // SAFETY: plain GL calls; one NUL-terminated source.
    unsafe {
        let s = glCreateShader(ty);
        let p: [*const c_char; 1] = [src.as_ptr()];
        glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
        glCompileShader(s);
        let mut ok: GLint = 0;
        glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
        let mut log = vec![0u8; 4096];
        let mut len: GLsizei = 0;
        glGetShaderInfoLog(s, log.len() as GLsizei, &mut len, log.as_mut_ptr() as *mut c_char);
        assert_eq!(ok, 1, "{text} did not compile: {}", String::from_utf8_lossy(&log[..len.max(0) as usize]));
        s
    }
}

/// The MultiTexShader reduced: every unit sampled with its texture's
/// sampler type, the results summed.
fn program(p: &Plan) -> GLuint {
    let vs = "#version 300 es\nin highp vec4 a_position;\nin highp vec2 a_coord;\nout highp vec2 v_coord;\n\
              void main() { gl_Position = a_position; v_coord = a_coord; }\n";
    let mut fs =
        String::from("#version 300 es\nin highp vec2 v_coord;\nlayout(location = 0) out mediump vec4 o_color;\n");
    let mut body = String::from("void main() {\n  highp vec4 c = vec4(0.0);\n");
    for (u, &t) in p.units.iter().enumerate() {
        let t = p.textures[t];
        let (ty, coord) = match t.kind {
            Kind::Tex2d => ("sampler2D", "v_coord"),
            Kind::Cube => ("samplerCube", "vec3(v_coord * 2.0 - 1.0, 1.0)"),
            Kind::Array => ("sampler2DArray", "vec3(v_coord, 1.0)"),
            Kind::Tex3d => ("sampler3D", "vec3(v_coord, 0.5)"),
        };
        fs += &format!("uniform highp {}{ty} u_sampler{u};\n", FORMATS[t.format].5);
        body += &format!("  c += vec4(texture(u_sampler{u}, {coord}));\n");
    }
    fs += &body;
    fs += "  o_color = c;\n}\n";
    let (v, f) = (shader(GL_VERTEX_SHADER, vs), shader(GL_FRAGMENT_SHADER, &fs));
    // SAFETY: plain GL calls.
    unsafe {
        let prog = glCreateProgram();
        glAttachShader(prog, v);
        glAttachShader(prog, f);
        glBindAttribLocation(prog, 0, c"a_position".as_ptr());
        glBindAttribLocation(prog, 1, c"a_coord".as_ptr());
        glLinkProgram(prog);
        let mut ok: GLint = 0;
        glGetProgramiv(prog, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "the program links");
        glDeleteShader(v);
        glDeleteShader(f);
        prog
    }
}

static POS: [f32; 16] = [-1.0, -1.0, 0.0, 1.0, -1.0, 1.0, 0.0, 1.0, 1.0, -1.0, 0.0, 1.0, 1.0, 1.0, 0.0, 1.0];
static TEXC: [f32; 8] = [0.0, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0, 1.0];
static QUAD: [u16; 6] = [0, 1, 2, 2, 1, 3];

/// s_cubeFaceTargets: uploadCubeTexture gives each face's levels in this order.
const CUBE_FACES: [GLenum; 6] = [
    GL_TEXTURE_CUBE_MAP_POSITIVE_X,
    GL_TEXTURE_CUBE_MAP_NEGATIVE_X,
    GL_TEXTURE_CUBE_MAP_POSITIVE_Y,
    GL_TEXTURE_CUBE_MAP_NEGATIVE_Y,
    GL_TEXTURE_CUBE_MAP_POSITIVE_Z,
    GL_TEXTURE_CUBE_MAP_NEGATIVE_Z,
];

/// Every level (and face) of one texture, bound to the active unit.
fn upload(t: &Tex, data: &[u8]) {
    let (w, h, d, levels) = t.dims();
    let (internal, format, type_, _, _, _) = FORMATS[t.format];
    // SAFETY: plain GL calls; `data` holds level 0 of the largest texture.
    unsafe {
        let tgt = t.target();
        glTexParameteri(tgt, GL_TEXTURE_WRAP_S, t.wrap[0] as GLint);
        glTexParameteri(tgt, GL_TEXTURE_WRAP_T, t.wrap[1] as GLint);
        if t.kind == Kind::Tex3d {
            glTexParameteri(tgt, GL_TEXTURE_WRAP_R, t.wrap[2] as GLint);
        }
        glTexParameteri(tgt, GL_TEXTURE_MIN_FILTER, t.min as GLint);
        glTexParameteri(tgt, GL_TEXTURE_MAG_FILTER, t.mag as GLint);
        let ptr = data.as_ptr() as *const c_void;
        let faces: &[GLenum] = if t.kind == Kind::Cube { &CUBE_FACES } else { &[GL_TEXTURE_2D] };
        for &face in faces {
            for l in 0..levels {
                let (lw, lh) = ((w >> l).max(1), (h >> l).max(1));
                match t.kind {
                    Kind::Tex2d | Kind::Cube => {
                        glTexImage2D(face, l, internal as GLint, lw, lh, 0, format, type_, ptr);
                    }
                    Kind::Array => glTexImage3D(tgt, l, internal as GLint, lw, lh, d, 0, format, type_, ptr),
                    Kind::Tex3d => {
                        glTexImage3D(tgt, l, internal as GLint, lw, lh, (d >> l).max(1), 0, format, type_, ptr)
                    }
                }
            }
        }
    }
}

/// What one case did.
#[derive(Debug)]
struct CaseUse {
    name: String,
    /// The carve-out's high-water during the case, in bytes.
    peak: u64,
    /// Allocations refused during the case, and the largest of them.
    refused: u32,
    refused_largest: u64,
    /// The tiled copies' high-water since the context was made.
    tiled_hw: u64,
    refused_draws: u32,
    error: GLenum,
    /// The textures: count, and how many of them are cubes.
    textures: usize,
    cubes: usize,
}

impl CaseUse {
    fn allocates(&self) -> bool {
        self.refused == 0 && self.refused_draws == 0 && self.error == GL_NO_ERROR
    }
}

/// One TextureUnitCase: init, render, deinit.
fn run_case(name: &str, max_units: usize) -> CaseUse {
    let (units, only, idx) = parse_case(name);
    let units = if units == 0 { max_units } else { units };
    let p = plan(units, only, idx);
    with_platform(|pl| {
        pl.case_peak = pl.used;
        pl.case_refused = 0;
        pl.case_refused_largest = 0;
    });
    let refused0 = v3d_refused_draws();
    let data = vec![0x3cu8; 256 * 256 * 16];
    let mut pixels = vec![0u8; (VIEWPORT * VIEWPORT * 4) as usize];
    // SAFETY: plain GL calls; the client arrays are static or outlive the
    // calls that read them.
    let error = unsafe {
        let mut names = vec![0u32; p.textures.len()];
        glGenTextures(names.len() as GLsizei, names.as_mut_ptr());
        glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
        let mut given = vec![false; p.textures.len()];
        for (u, &t) in p.units.iter().enumerate() {
            glActiveTexture(GL_TEXTURE0 + u as GLenum);
            glBindTexture(p.textures[t].target(), names[t]);
            if !given[t] {
                given[t] = true;
                upload(&p.textures[t], &data);
            }
        }
        let e = err();
        let prog = program(&p);
        glViewport(0, 0, VIEWPORT, VIEWPORT);
        glClearColor(0.125, 0.25, 0.5, 1.0);
        glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT);
        glUseProgram(prog);
        for u in 0..p.units.len() {
            let n = CString::new(format!("u_sampler{u}")).unwrap();
            glUniform1i(glGetUniformLocation(prog, n.as_ptr()), u as GLint);
        }
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, POS.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glVertexAttribPointer(1, 2, GL_FLOAT, 0, 0, TEXC.as_ptr() as *const c_void);
        glEnableVertexAttribArray(1);
        glDrawElements(GL_TRIANGLES, 6, GL_UNSIGNED_SHORT, QUAD.as_ptr() as *const c_void);
        let e = if e == GL_NO_ERROR { err() } else { e };
        /* render() deletes the textures; the case then reads the frame
         * back, and sglr::GLContext's destructor deletes the program */
        glDeleteTextures(names.len() as GLsizei, names.as_ptr());
        glReadPixels(0, 0, VIEWPORT, VIEWPORT, GL_RGBA, GL_UNSIGNED_BYTE, pixels.as_mut_ptr() as *mut c_void);
        let e = if e == GL_NO_ERROR { err() } else { e };
        glUseProgram(0);
        glDeleteProgram(prog);
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
    let (peak, refused, refused_largest) = with_platform(|pl| (pl.case_peak, pl.case_refused, pl.case_refused_largest));
    CaseUse {
        name: name.to_string(),
        peak,
        refused,
        refused_largest,
        tiled_hw: hw,
        refused_draws: v3d_refused_draws().wrapping_sub(refused0),
        error,
        textures: p.textures.len(),
        cubes: p.textures.iter().filter(|t| t.kind == Kind::Cube).count(),
    }
}

/* ---- the replays: one child process each --------------------------------------- */

/// The child: `V3D_UNITS_CARVE` and `V3D_UNITS_TILED` give the memory, `V3D_UNITS_CASES`
/// the cases (comma-separated), run in one context; one `UNITS-CASE` line
/// a case on stdout. Without them (the plain test run), nothing.
#[test]
fn replay_child() {
    let Ok(cases) = std::env::var("V3D_UNITS_CASES") else {
        return;
    };
    *PLATFORM.lock().unwrap_or_else(|e| e.into_inner()) = Some(Platform::new());
    // SAFETY: statics; this runs once a process, before any other GL call.
    unsafe {
        osal_set_ops(osal());
        yakogl::backend::g_backend = &yakogl_backend_v3d;
        assert_eq!(yakogl::backend::init(), 0, "{:?}", log_lines());
        assert!(!context::gl_context_create(SURFACE, SURFACE).is_null(), "{:?}", log_lines());
    }
    let mut max_units: GLint = 0;
    // SAFETY: one GLint out-parameter.
    unsafe { glGetIntegerv(GL_MAX_TEXTURE_IMAGE_UNITS, &mut max_units) };
    let init_used = with_platform(|p| p.used);
    println!("UNITS-INIT {init_used} {max_units}");
    for name in cases.split(',') {
        let c = run_case(name, max_units as usize);
        println!(
            "UNITS-CASE {} {} {} {} {} {} {} {} {}",
            c.name, c.peak, c.refused, c.refused_largest, c.tiled_hw, c.refused_draws, c.error, c.textures, c.cubes
        );
    }
    if std::env::var_os("V3D_UNITS_LOG").is_some() {
        for l in log_lines() {
            println!("UNITS-LOG {l}");
        }
    }
}

/// One replay's result: bytes in use after init, the context's units, the cases.
struct Replay {
    conf: String,
    mem: Mem,
    init_used: u64,
    max_units: u32,
    cases: Vec<CaseUse>,
}

fn replay(conf: &'static str, cases: &[&str]) -> Replay {
    replay_mem(conf, mem_of(conf), cases)
}

/// A replay with the given carve-out and tiled budget, `conf` its label.
fn replay_mem(conf: &str, mem: Mem, cases: &[&str]) -> Replay {
    let out = Command::new(std::env::current_exe().expect("the test binary"))
        .args(["replay_child", "--exact", "--nocapture", "--test-threads=1"])
        .env("V3D_UNITS_CARVE", mem.carve.to_string())
        .env("V3D_UNITS_TILED", mem.tiled.to_string())
        .env("V3D_UNITS_CASES", cases.join(","))
        .output()
        .expect("the child runs");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "the {conf} replay: {stdout}\n{}", String::from_utf8_lossy(&out.stderr));
    let mut r = Replay { conf: conf.to_string(), mem, init_used: 0, max_units: 0, cases: Vec::new() };
    /* libtest's "test replay_child ... " heads the first line */
    for l in stdout.lines().filter_map(|l| l.find("UNITS-").map(|i| &l[i..])) {
        let f: Vec<&str> = l.split_whitespace().collect();
        match f.first() {
            Some(&"UNITS-INIT") => (r.init_used, r.max_units) = (f[1].parse().unwrap(), f[2].parse().unwrap()),
            Some(&"UNITS-CASE") => r.cases.push(CaseUse {
                name: f[1].to_string(),
                peak: f[2].parse().unwrap(),
                refused: f[3].parse().unwrap(),
                refused_largest: f[4].parse().unwrap(),
                tiled_hw: f[5].parse().unwrap(),
                refused_draws: f[6].parse().unwrap(),
                error: f[7].parse().unwrap(),
                textures: f[8].parse().unwrap(),
                cubes: f[9].parse().unwrap(),
            }),
            _ => {}
        }
    }
    assert_eq!(r.cases.len(), cases.len(), "the {conf} replay ran every case: {stdout}");
    r.print();
    r
}

impl Replay {
    fn print(&self) {
        let heap = heap_of(self.mem);
        eprintln!(
            "v3d_units_cube_mem: {} (carve-out {}, heap {heap}, tiled budget {}): in use after init {}, \
             GL_MAX_TEXTURE_IMAGE_UNITS {}",
            self.conf, self.mem.carve, self.mem.tiled, self.init_used, self.max_units
        );
        for c in &self.cases {
            eprintln!(
                "  {:<22} {:>2} textures ({:>2} cube) peak {:>9} ({:>9} free) tiled hw {:>9} refused allocs {} \
                 (largest {}) refused draws {} error {:#x} -> {}",
                c.name,
                c.textures,
                c.cubes,
                c.peak,
                heap.saturating_sub(c.peak),
                c.tiled_hw,
                c.refused,
                c.refused_largest,
                c.refused_draws,
                c.error,
                if c.allocates() { "allocates" } else { "does NOT allocate" }
            );
        }
    }
}

fn t14_board() -> &'static Replay {
    static R: OnceLock<Replay> = OnceLock::new();
    R.get_or_init(|| replay("rpi_4b", &T14_CASES))
}

fn t14_2048() -> &'static Replay {
    static R: OnceLock<Replay> = OnceLock::new();
    R.get_or_init(|| replay("texture-2048", &T14_CASES))
}

/* ---- the asserts -------------------------------------------------------------- */

/// (a) The board file's 48 MiB runs out in 8_units.only_cube, as T14a did
/// at b1b5b845.
#[test]
fn the_board_conf_runs_out_in_8_units_only_cube() {
    let r = t14_board();
    let short: Vec<&str> = r
        .cases
        .iter()
        .filter(|c| c.name.starts_with("8_units.only_cube.") && !c.allocates())
        .map(|c| &*c.name)
        .collect();
    assert!(!short.is_empty(), "every 8_units.only_cube case allocated with rpi_4b.conf's memory");
    eprintln!("v3d_units_cube_mem: rpi_4b.conf: 8_units.only_cube cases that do not allocate: {short:?}");
}

/// (b) texture-2048.conf's carve-out and tiled budget hold every replayed
/// case of T14 and T14a but all_units.only_cube.0.
///
/// Ignored: with texture-2048.conf's current numbers (a 75,497,472-byte
/// carve-out, a 25,165,824-byte tiled budget) 8_units.only_cube.3, .4 and
/// .8 do not allocate (an allocation of 4,206,592, 6,295,552 and 8,409,088
/// bytes finds no room; their draws are refused, GL_OUT_OF_MEMORY).
/// Stepping the carve-out up by 1 MiB at that tiled budget, the smallest
/// that holds every case is 110,100,480 bytes (105 MiB): .3 needs 76 MiB,
/// .4 79 MiB and .8 105 MiB (six RGBA32-class cubes, 8,409,088 bytes of
/// tiled copy each). A 16 MiB tiled budget fails at any carve-out up to
/// 250 MiB; a 32 MiB one needs the same 105 MiB.
/// `the_smallest_numbers_hold_every_case` pins those numbers.
#[test]
#[ignore = "texture-2048.conf's 72 MiB carve-out is too small: 105 MiB (110100480) holds every case"]
fn texture_2048_conf_holds_every_t14_cube_and_mixed_case() {
    let r = t14_2048();
    for c in &r.cases {
        assert!(c.allocates(), "{} does not allocate with texture-2048.conf: {c:?}", c.name);
    }
}

/// The smallest carve-out that holds every replayed case of T14 and T14a
/// at texture-2048.conf's tiled budget (1 MiB steps).
const T14_CARVE: u64 = 110_100_480;
/// The smallest tiled budget that holds all_units.only_cube.0 alone
/// (256 KiB steps, with a 256 MiB carve-out).
const ALL_CUBE_TILED: u64 = 30_146_560;
/// The smallest carve-out that holds it alone with a 32 MiB budget (1 MiB steps).
const ALL_CUBE_CARVE: u64 = 95_420_416;

/// The numbers (b) and (c) name: the replay holds every case with them.
/// With a 105 MiB carve-out and a 32 MiB tiled budget, the T14 cases and
/// all_units.only_cube.0 after them, in one context, all allocate.
#[test]
fn the_smallest_numbers_hold_every_case() {
    let at = |carve: u64, tiled: u64, cases: &[&str]| {
        let r = replay_mem(&format!("carve-out {carve}, tiled {tiled}"), Mem { carve, tiled }, cases);
        for c in &r.cases {
            assert!(c.allocates(), "{} does not allocate with ({carve}, {tiled}): {c:?}", c.name);
        }
    };
    let t2048 = mem_of("texture-2048");
    at(T14_CARVE, t2048.tiled, &T14_CASES);
    at(1 << 28, ALL_CUBE_TILED, &[ALL_CUBE]);
    at(ALL_CUBE_CARVE, 32 << 20, &[ALL_CUBE]);
    let mut both = T14_CASES.to_vec();
    both.push(ALL_CUBE);
    at(T14_CARVE, 32 << 20, &both);
}

/// (c) all_units.only_cube.0 alone in a fresh context, under both. It fits
/// neither: with rpi_4b.conf allocations find no room; with
/// texture-2048.conf an allocation of 3,149,824 bytes finds no room, and
/// at any carve-out a tiled budget under `ALL_CUBE_TILED` leaves a draw no
/// empty job holds (16 cubes; the per-job texture arena cannot take the
/// copies past the budget).
#[test]
fn all_units_only_cube_0_alone() {
    let board = replay("rpi_4b", &[ALL_CUBE]);
    let t2048 = replay("texture-2048", &[ALL_CUBE]);
    let (b, t) = (&board.cases[0], &t2048.cases[0]);
    eprintln!(
        "v3d_units_cube_mem: {ALL_CUBE} alone: rpi_4b.conf peak {} ({}), texture-2048.conf peak {} ({})",
        b.peak,
        if b.allocates() { "allocates" } else { "does not allocate" },
        t.peak,
        if t.allocates() { "allocates" } else { "does not allocate" }
    );
    assert_eq!((b.allocates(), t.allocates()), ALL_CUBE_FITS, "{ALL_CUBE}: (rpi_4b.conf, texture-2048.conf) fits");
}
