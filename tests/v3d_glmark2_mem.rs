// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! glmark2's V3D memory on the Pi 4, replayed on the host: does every large
//! allocation of the default scene order find its range in the carve-out?
//!
//! Item pi4-mem raised glmark2's carve-out from 48 MiB to 96 MiB
//! (`CONFIG_YAKOGL_V3D_CARVEOUT_BYTES`, driver/samples/zephyr/glmark2/boards/
//! rpi_4b.conf) for refract's 56.2 MB peak. Its open point: terrain's
//! 800x600 render targets have non-power-of-two mip chains under
//! GL_OES_texture_npot (about +33% a texture), which no board run measured.
//!
//! The replay uses the code the board runs, not a model of it. A small C
//! shim is built at test time (`cc`, as tools/check.sh builds its C checks)
//! from `src/osal/v3d_granules.h` and `v3d_frees.h` (the carve-out
//! allocator and its free-run count, the OSAL's `gpu_alloc` rule from
//! zephyr_v3d.c: 256-byte granules, 4 KiB alignment from 4 KiB up, first
//! fit) and `driver/src/v3d/v3d_tex.c` (Mesa's UIF layout, what
//! `v3d_tex_layout` gives the backend's tiled copies), and loaded with
//! `dlopen`. Nothing under src/ or driver/ changes.
//!
//! What is reconstructed, and from where (every size is a number in
//! tools/cts/RESULTS.md, "Pi 4: the stale-copy fix in glmark2 and gl_test"
//! and "Pi 4: the OSAL looks at the job in flight", or a sample's source as
//! driver/docs/glmark2.md quotes it):
//!
//! * The carve-out: 96 MiB or 48 MiB, a 512 KiB page table and a 4 KiB
//!   scratch page below the allocator's first granule (zephyr_v3d.c). The
//!   48 MiB heap is 49,803,264 bytes, the "49.8 MB carve-out" there.
//! * 17,817,856 bytes always in use (RESULTS: "used minus held is 17817856
//!   bytes ... the permanent set"), one block at the bottom: first fit packs
//!   it there (the board's longest free run at a boundary, 31,960,832 bytes,
//!   is the free 31,985,408 less 24,576 bytes).
//! * Scene boundaries: the OSAL polls the job in flight before a free and
//!   before an allocation (a528c8a3), so a torn-down scene's memory is free
//!   at the next scene's setup ("frees held 0" at every boundary, longest
//!   free run 31,960,832 bytes). Each scene's frees are replayed at its
//!   end. Within a scene a copy is freed when the texture is rendered into
//!   again, at once: a free the job in flight may still read is held on the
//!   board until the job completes, so a frame here sees at most what the
//!   board sees.
//! * The scenes. Each is its largest live set, in the entries RESULTS lists.
//! * The tiled copy of a sampled texture is `v3d_tex_layout`'s size plus
//!   4096 (`tiled_copy`, src/v3d/backend.rs: "room to align level 0"): the
//!   recorded 7,684,096, 1,404,928, 356,352 and 266,240 byte requests are
//!   1600x1200, a 512x512 chain, a 256x256 chain and a 256x256 level, all
//!   RGBA8 (checked below against the real layout).
//! * The GL core's storage of a chain is the levels packed one after the
//!   other, each at its pair-padded pitch (`v3d::texture::gl_level_layered`'s
//!   rule, which the GL core's `chain_layout` follows for the V3D).
//!
//! Not modelled: the job arenas and programs a scene adds to the permanent
//! set (RESULTS: +16,384 bytes in terrain), the copies' eviction budget
//! (`tiled_budget`), and the time a free is held inside a frame.

#![cfg(all(feature = "shaderc-online", feature = "gpu-v3d"))]

use std::ffi::{c_char, c_int, c_void, CString};
use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;

extern "C" {
    fn dlopen(path: *const c_char, flags: c_int) -> *mut c_void;
    fn dlsym(h: *mut c_void, name: *const c_char) -> *mut c_void;
}

/// The shim over the real C: one pool per replay.
const SHIM: &str = r#"
#include "v3d_tex.h"
#include "v3d_granules.h"
#include "v3d_frees.h"
#include <stdlib.h>

#define GRANULE 256u
#define PAGE 4096u

struct pool { struct v3d_granules gm; uint32_t *map; };

uint32_t shim_layout(uint32_t w, uint32_t h, uint32_t levels, uint32_t cpp)
{
	struct v3d_tex_layout l;
	return v3d_tex_layout(w, h, levels, cpp, &l) ? l.size : 0;
}

void *shim_new(uint32_t first, uint32_t n)
{
	struct pool *p = calloc(1, sizeof(*p));
	p->map = calloc(n / 32, sizeof(uint32_t));
	v3d_granules_init(&p->gm, p->map, first, n);
	return p;
}

void shim_delete(struct pool *p) { free(p->map); free(p); }

/* zephyr_v3d.c gpu_alloc: whole granules, a page's alignment from a page up */
int64_t shim_alloc(struct pool *p, uint64_t size)
{
	uint32_t n = (uint32_t)((size + GRANULE - 1) / GRANULE);
	uint32_t align = size >= PAGE ? PAGE / GRANULE : 1;
	int64_t g = v3d_granules_place(&p->gm, n, align);
	if (g < 0)
		return -1;
	v3d_granules_mark(&p->gm, (uint32_t)g, n, true);
	return g * GRANULE;
}

void shim_free(struct pool *p, uint64_t off, uint64_t size)
{
	v3d_granules_mark(&p->gm, (uint32_t)(off / GRANULE), (uint32_t)((size + GRANULE - 1) / GRANULE), false);
}

uint64_t shim_longest(struct pool *p)
{
	uint32_t longest = 0;
	v3d_free_runs(p->map, p->gm.first, p->gm.n, &longest, NULL);
	return (uint64_t)longest * GRANULE;
}

uint64_t shim_free_total(struct pool *p)
{
	return (uint64_t)(p->gm.n - p->gm.first - p->gm.used) * GRANULE;
}
"#;

fn lib() -> *mut c_void {
    static L: OnceLock<usize> = OnceLock::new();
    *L.get_or_init(|| {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("v3d_glmark2_mem");
        std::fs::create_dir_all(&dir).expect("the scratch directory");
        let src = dir.join(format!("shim-{}.c", std::process::id()));
        let so = dir.join(format!("shim-{}.so", std::process::id()));
        std::fs::write(&src, SHIM).expect("the shim source");
        let st = Command::new("cc")
            .current_dir(&root)
            .args(["-std=c11", "-O1", "-shared", "-fPIC", "-Wall", "-Wextra", "-Werror"])
            .args(["-Idriver/src/v3d", "-Idriver/src/v3d/generated", "-Isrc/osal"])
            .args(["-Ithird_party/mesa-26.2.1/src/broadcom", "-Ithird_party/mesa-26.2.1/src"])
            .arg("-Ithird_party/mesa-26.2.1/include")
            .arg(&src)
            .args(["driver/src/v3d/v3d_tex.c", "driver/src/v3d/v3d_tiling_mesa.c", "-lm", "-o"])
            .arg(&so)
            .status()
            .expect("cc runs");
        assert!(st.success(), "the C shim over v3d_granules.h and v3d_tex.c builds");
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

/// `v3d_tex_layout(w, h, levels, cpp)`'s size: the tiled image's bytes.
fn layout(w: u32, h: u32, levels: u32, cpp: u32) -> u64 {
    // SAFETY: shim_layout's C signature.
    let f: extern "C" fn(u32, u32, u32, u32) -> u32 = unsafe { std::mem::transmute(sym("shim_layout")) };
    let n = f(w, h, levels, cpp);
    assert_ne!(n, 0, "v3d_tex_layout refused {w}x{h} x{levels}");
    u64::from(n)
}

/// A level count of a full chain: down to 1x1.
fn chain_levels(w: u32, h: u32) -> u32 {
    32 - w.max(h).leading_zeros()
}

/// The GL core's storage of the first `levels` levels of a one-layer RGBA8
/// texture: packed, each at its pair-padded pitch.
fn gl_storage(w: u32, h: u32, levels: u32) -> u64 {
    u64::from(yakogl::v3d::texture::gl_level_layered(w, h, 1, 4, levels).0)
}

/// A tiled copy's request: the image plus the page the backend adds.
fn copy_request(w: u32, h: u32, levels: u32) -> u64 {
    layout(w, h, levels, 4) + 4096
}

/* ---- the OSAL's carve-out (zephyr_v3d.c) -------------------------------------- */

const MIB: u64 = 1 << 20;
const GRANULE: u64 = 256;
/// The page table of the GPU's 512 MiB address space, then the scratch page.
const PAGE_TABLE: u64 = (0x2000_0000 / 4096) * 4;
const HEAP_START: u64 = PAGE_TABLE + 4096;
/// What every entry from 1 on holds permanently (RESULTS, the memory lines).
const PERMANENT: u64 = 17_817_856;
/// A request of at least this is "large" for the fit check.
const LARGE: u64 = MIB;

/// One allocation request of the replay and what the pool had for it.
#[derive(Clone, Debug)]
struct Request {
    scene: &'static str,
    what: &'static str,
    size: u64,
    /// The longest free range before it, and the free bytes in all.
    longest: u64,
    free: u64,
    served: bool,
}

struct Pool {
    h: *mut c_void,
    log: Vec<Request>,
    scene: &'static str,
    /// The live blocks of the scene: (offset, size).
    live: Vec<(u64, u64)>,
}

impl Pool {
    fn new(carve: u64) -> Pool {
        // SAFETY: shim_new's C signature.
        let new: extern "C" fn(u32, u32) -> *mut c_void = unsafe { std::mem::transmute(sym("shim_new")) };
        let h = new((HEAP_START / GRANULE) as u32, (carve / GRANULE) as u32);
        let mut p = Pool { h, log: Vec::new(), scene: "permanent", live: Vec::new() };
        p.alloc("permanent set", PERMANENT).expect("the permanent set fits");
        p.live.clear();
        p
    }

    fn call_u64(&self, name: &str) -> u64 {
        // SAFETY: the shim's `uint64_t (struct pool *)` functions.
        let f: extern "C" fn(*mut c_void) -> u64 = unsafe { std::mem::transmute(sym(name)) };
        f(self.h)
    }

    fn longest(&self) -> u64 {
        self.call_u64("shim_longest")
    }

    fn free_total(&self) -> u64 {
        self.call_u64("shim_free_total")
    }

    /// `gpu_alloc`: the offset, or `None` (the OSAL's -ENOMEM).
    fn alloc(&mut self, what: &'static str, size: u64) -> Option<u64> {
        let (longest, free) = (self.longest(), self.free_total());
        // SAFETY: shim_alloc's C signature.
        let f: extern "C" fn(*mut c_void, u64) -> i64 = unsafe { std::mem::transmute(sym("shim_alloc")) };
        let off = f(self.h, size);
        self.log.push(Request { scene: self.scene, what, size, longest, free, served: off >= 0 });
        (off >= 0).then(|| {
            self.live.push((off as u64, size));
            off as u64
        })
    }

    fn free(&mut self, off: u64) {
        let at = self.live.iter().position(|b| b.0 == off).expect("a live block");
        let (off, size) = self.live.swap_remove(at);
        // SAFETY: shim_free's C signature.
        let f: extern "C" fn(*mut c_void, u64, u64) = unsafe { std::mem::transmute(sym("shim_free")) };
        f(self.h, off, size);
    }

    /// The scene is torn down: every block it still holds goes back.
    fn end_scene(&mut self) {
        while let Some(&(off, _)) = self.live.last() {
            self.free(off);
        }
    }

    fn refused(&self) -> Vec<(&'static str, u64)> {
        self.log.iter().filter(|r| !r.served).map(|r| (r.scene, r.size)).collect()
    }
}

impl Drop for Pool {
    fn drop(&mut self) {
        // SAFETY: shim_delete's C signature; the pool is not used again.
        let f: extern "C" fn(*mut c_void) = unsafe { std::mem::transmute(sym("shim_delete")) };
        f(self.h);
    }
}

/* ---- the scenes --------------------------------------------------------------- */

/// glmark2's scenes before terrain, as the largest live set each held at its
/// teardown (RESULTS, entries 2-21: "the large holds: entry 10 9.8 MB, 11 9.9
/// MB, 15 12.3 MB, 16 11.5 MB, 21 20.0 MB ...; smaller ones of 2.3-3.9 MB at
/// entries 2-4, 9, 12 and 17-19"). One request each, the contiguous worst case.
const BASE: [(&str, u64); 12] = [
    ("entry 2-4", 3_900_000),
    ("entry 9", 3_900_000),
    ("entry 10", 9_800_000),
    ("entry 11", 9_900_000),
    ("entry 12", 3_900_000),
    ("entry 15", 12_300_000),
    ("entry 16", 11_500_000),
    ("entry 17-19", 3_900_000),
    ("entry 21", 20_000_000),
    ("entry 5-8", 2_300_000),
    ("entry 13-14", 2_300_000),
    ("entry 20", 2_300_000),
];

fn base(p: &mut Pool) {
    p.scene = "base";
    for (what, size) in BASE {
        p.alloc(what, size).expect("a base scene fits");
        p.end_scene();
    }
}

/// shadow (entry 23): a 1600x1200 depth target (R32F storage 7,680,000 and a
/// D24S8 buffer of the same size), the depth texture's tiled copy (7,684,096,
/// RESULTS: "shadow's own 1600x1200 depth copy"); its hold of 23.6 MB leaves
/// 0.56 MB for its models and textures.
fn shadow(p: &mut Pool) {
    p.scene = "shadow";
    p.alloc("depth R32F storage 1600x1200", 7_680_000);
    p.alloc("depth D24S8 1600x1200", 7_680_000);
    p.alloc("depth tiled copy", copy_request(1600, 1200, 1));
    p.alloc("models and textures", 23_600_000 - 7_680_000 * 2 - (copy_request(1600, 1200, 1)));
    p.end_scene();
}

/// refract (entry 24), RESULTS: the 1600x1200 R32F depth storage (7,680,000),
/// the RGBA colour storage (7,680,000), the depth texture's D24S8 buffer
/// (about 7.7 MB) and two tiled copies (7,684,096 each), a peak of about
/// 56.2 MB with the permanent set.
fn refract(p: &mut Pool) {
    p.scene = "refract";
    p.alloc("depth R32F storage 1600x1200", 7_680_000);
    p.alloc("colour storage 1600x1200", 7_680_000);
    p.alloc("depth D24S8 1600x1200", 7_700_000);
    p.alloc("depth tiled copy", copy_request(1600, 1200, 1));
    p.alloc("colour tiled copy", copy_request(1600, 1200, 1));
    p.end_scene();
}

/// One render target of terrain: its storage in the GL core and its tiled copy
/// while it has one.
struct Target {
    name: &'static str,
    w: u32,
    h: u32,
    /// The levels the pass that samples it filters (the whole chain, or 1).
    levels: u32,
    copy: Option<u64>,
}

/// terrain (entry 22), scene-terrain.cpp:62-120 as driver/docs/glmark2.md
/// lists its passes: height, normal and specular maps (512x512 with chains),
/// the terrain pass at 800x600 with a depth target, bloom at 256x256 (one
/// target with a chain, one a single level), the overlay and the two tilt
/// passes at 800x600. The mesh buffers are three of 4608 KiB and one of
/// 3072 KiB (driver/docs/pvr-render-pool.md; glmark2.md: "terrain asks 3,072
/// KiB (a buffer), 1,368 KiB (512x512 mip chain), 2 x 1,876 KiB (800x600)").
///
/// `npot_chains`: the 800x600 targets have a full chain (GL_OES_texture_npot,
/// item npot) and are sampled with it; off, they have one level, as the
/// board run RESULTS records (mipmaps refused, no copy sampled). The copies
/// of a frame are made at the draws that sample them, in pass order, and a
/// pass that renders into a texture drops its copy first. Four copies of the
/// 512x512 chain, two of the 256x256 chain and one of the 256x256 level are
/// asked for each frame: RESULTS' refusals, 228 x 1,404,928, 114 x 356,352
/// and 57 x 266,240 bytes in 57 frames, are exactly 4, 2 and 1 a frame.
fn terrain(p: &mut Pool, npot_chains: bool, frames: u32) {
    p.scene = "terrain";
    for (what, size) in [
        ("mesh 4608 KiB", 4608 * 1024),
        ("mesh 4608 KiB", 4608 * 1024),
        ("mesh 4608 KiB", 4608 * 1024),
        ("mesh 3072 KiB", 3072 * 1024),
    ] {
        p.alloc(what, size).expect("terrain's buffers fit");
    }
    let big = if npot_chains { chain_levels(800, 600) } else { 1 };
    let t = |name, w, h, levels, p: &mut Pool| {
        p.alloc(name, gl_storage(w, h, levels));
        Target { name, w, h, levels, copy: None }
    };
    let chain512 = chain_levels(512, 512);
    let chain256 = chain_levels(256, 256);
    let mut height = t("height map 512x512", 512, 512, chain512, p);
    let mut normal = t("normal map 512x512", 512, 512, chain512, p);
    let mut specular = t("specular map 512x512", 512, 512, chain512, p);
    let mut scene800 = t("terrain colour 800x600", 800, 600, big, p);
    p.alloc("terrain depth 800x600", 800 * 600 * 4);
    let mut bloom_b = t("bloom 256x256 level", 256, 256, 1, p);
    let mut bloom_a = t("bloom 256x256 chain", 256, 256, chain256, p);
    let mut overlay = t("overlay 800x600", 800, 600, big, p);
    let mut tilt_h = t("tilt h 800x600", 800, 600, big, p);
    let mut tilt_v = t("tilt v 800x600", 800, 600, big, p);

    fn render(t: &mut Target, p: &mut Pool) {
        if let Some(c) = t.copy.take() {
            p.free(c);
        }
    }
    fn sample(t: &mut Target, p: &mut Pool) {
        if t.copy.is_none() {
            t.copy = p.alloc(t.name, copy_request(t.w, t.h, t.levels));
        }
    }
    for _ in 0..frames {
        render(&mut height, p);
        render(&mut normal, p);
        sample(&mut height, p);
        render(&mut specular, p);
        render(&mut scene800, p);
        sample(&mut height, p);
        sample(&mut normal, p);
        sample(&mut specular, p);
        render(&mut bloom_b, p);
        if npot_chains {
            sample(&mut scene800, p);
        }
        render(&mut bloom_a, p);
        sample(&mut bloom_b, p);
        render(&mut overlay, p);
        sample(&mut bloom_a, p);
        if npot_chains {
            sample(&mut scene800, p);
        }
        render(&mut tilt_h, p);
        if npot_chains {
            sample(&mut overlay, p);
        }
        sample(&mut bloom_a, p);
        render(&mut tilt_v, p);
        if npot_chains {
            sample(&mut tilt_h, p);
        }
    }
    p.end_scene();
}

/* ---- the replay ---------------------------------------------------------------- */

#[derive(Clone, Copy, PartialEq)]
enum Order {
    /// The order RESULTS records (entries 22, 23, 24): terrain, shadow, refract.
    Recorded,
    /// shadow before terrain, as the item's task states the default order.
    ShadowFirst,
}

fn replay(carve: u64, npot_chains: bool, order: Order) -> Pool {
    let mut p = Pool::new(carve);
    base(&mut p);
    match order {
        Order::Recorded => {
            terrain(&mut p, npot_chains, 4);
            shadow(&mut p);
        }
        Order::ShadowFirst => {
            shadow(&mut p);
            terrain(&mut p, npot_chains, 4);
        }
    }
    refract(&mut p);
    p
}

fn report(tag: &str, p: &Pool) {
    let large: Vec<&Request> = p.log.iter().filter(|r| r.size >= LARGE && r.scene != "permanent").collect();
    let tightest = large.iter().filter(|r| r.served).map(|r| r.longest - r.size).min();
    let least_free = p.log.iter().map(|r| r.free).min();
    println!(
        "{tag}: {} requests, {} large, refused {}, the longest free range beyond a large request at least {:?} bytes, \
         free bytes at the fullest request {:?}",
        p.log.len(),
        large.len(),
        p.refused().len(),
        tightest,
        least_free
    );
}

#[test]
fn the_recorded_request_sizes_are_the_real_layouts() {
    assert_eq!(copy_request(1600, 1200, 1), 7_684_096, "refract's and shadow's tiled copy");
    assert_eq!(copy_request(512, 512, chain_levels(512, 512)), 1_404_928, "terrain's 512x512 chain");
    assert_eq!(copy_request(256, 256, chain_levels(256, 256)), 356_352, "terrain's 256x256 chain");
    assert_eq!(copy_request(256, 256, 1), 266_240, "terrain's 256x256 level");
    assert_eq!(layout(800, 600, 1, 4), 800 * 600 * 4, "one level of 800x600");
    assert_eq!(PAGE_TABLE, 524_288);
    let heap_48 = 48 * MIB - HEAP_START;
    assert_eq!(heap_48 - PERMANENT, 31_985_408, "RESULTS: 31985408 bytes free at refract's start");
}

#[test]
fn the_48_mib_control_reproduces_the_recorded_refusals() {
    let p = replay(48 * MIB, false, Order::Recorded);
    report("48 MiB, one-level 800x600 targets", &p);
    let refused = p.refused();
    let count = |s: u64| refused.iter().filter(|r| r.1 == s).count();
    // terrain: 4, 2 and 1 a frame (57 frames recorded: 228, 114 and 57)
    assert_eq!(count(1_404_928), 4 * 4, "512x512 chain copies refused");
    assert_eq!(count(356_352), 2 * 4, "256x256 chain copies refused");
    assert_eq!(count(266_240), 4, "256x256 level copies refused");
    // refract: its second 7,684,096-byte copy (39 refusals recorded: every
    // frame; the replay asks once)
    assert_eq!(count(7_684_096), 1, "refract's second copy refused");
    assert!(
        refused.iter().all(|r| [1_404_928, 356_352, 266_240, 7_684_096].contains(&r.1)),
        "nothing else: {refused:?}"
    );
    assert!(refused.iter().filter(|r| r.1 == 7_684_096).all(|r| r.0 == "refract"));
}

/// refract after npot: the colour render target has its whole 11-level chain, so
/// its tiled copy is 11,005,952 bytes (tools/cts/log/hostharness/glmark2/FINDINGS-refract-budget.md section 2).
/// `budget` is CONFIG_YAKOGL_V3D_TILED_COPIES_BYTES, an LRU cap on the copies
/// kept across jobs: when the depth copy and the colour copy together exceed
/// it, the depth copy is evicted before the colour copy is made. Returns the
/// peak use of the carve-out, permanent set included.
fn refract_npot(p: &mut Pool, budget: u64) -> u64 {
    p.scene = "refract";
    p.alloc("depth R32F storage 1600x1200", 7_680_000);
    p.alloc("colour storage 1600x1200", 7_680_000);
    p.alloc("depth D24S8 1600x1200", 7_700_000);
    let depth = copy_request(1600, 1200, 1);
    let colour = copy_request(1600, 1200, chain_levels(1600, 1200));
    assert_eq!(colour, 11_005_952, "the colour chain's tiled copy");
    assert!(colour <= budget, "the budget keeps the colour copy");
    let d = p.alloc("depth tiled copy", depth).expect("the depth copy fits");
    if depth + colour > budget {
        p.free(d);
    }
    p.alloc("colour tiled copy", colour).expect("the colour copy fits");
    let used = p.live.iter().map(|b| b.1).sum::<u64>() + PERMANENT;
    p.end_scene();
    used
}

#[test]
fn refract_with_the_colour_chain_copy_fits_the_96_mib_carve_out() {
    let carve = 96 * MIB;
    assert_eq!(carve, 100_663_296);
    for budget in [12 * MIB, 16 * MIB, 24 * MIB] {
        for order in [Order::Recorded, Order::ShadowFirst] {
            let mut p = Pool::new(carve);
            base(&mut p);
            match order {
                Order::Recorded => {
                    terrain(&mut p, true, 4);
                    shadow(&mut p);
                }
                Order::ShadowFirst => {
                    shadow(&mut p);
                    terrain(&mut p, true, 4);
                }
            }
            let peak = refract_npot(&mut p, budget);
            println!("refract, budget {budget}: peak {peak} of {carve}");
            assert!(peak <= carve, "refract's peak {peak} fits {carve}");
            assert!(p.refused().is_empty(), "nothing refused: {:?}", p.refused());
        }
    }
}

#[test]
fn every_large_allocation_fits_the_96_mib_carve_out() {
    for order in [Order::Recorded, Order::ShadowFirst] {
        for npot in [false, true] {
            let p = replay(96 * MIB, npot, order);
            report(&format!("96 MiB, npot chains {npot}, shadow first {}", order == Order::ShadowFirst), &p);
            assert!(p.refused().is_empty(), "no request is refused: {:?}", p.refused());
            for r in p.log.iter().filter(|r| r.size >= LARGE && r.scene != "permanent") {
                assert!(
                    r.longest >= r.size,
                    "{} {}: {} bytes, longest free range {}",
                    r.scene,
                    r.what,
                    r.size,
                    r.longest
                );
            }
        }
    }
}
