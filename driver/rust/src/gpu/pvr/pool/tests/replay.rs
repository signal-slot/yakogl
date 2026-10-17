// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! glmark2's render-pool sequence on the AM62P, and a Slint and an AGL
//! cluster one, replayed through the real pool and bump on the board's
//! layout: a 64 MiB carveout whose render pool is its last 59200 KiB
//! ("render pool summary: 59200 KiB",
//! docs/notes/pvr-render-pool.md).
//!
//! The board prints no per-allocation trace, so the sequence is a
//! reconstruction, and each number says where it comes from:
//! - the fixed part, from the owner report of the frame-set run (the doc's
//!   "The frame-set run", `build/frames-am62p-20261004-012818`): render
//!   statics 2144 KiB, freelists 12440 (general 12392, pm 48), render
//!   context 4, frame sets 7488 (general 4400, pds 3088), backend statics
//!   with the 128 KiB index buffer 280, GL leftovers 520; HWRT members by
//!   `render.rs`'s formulas, 10 at glmark2's close;
//! - jellyfish's 45 frees of 11576 KiB (held at terrain's set-up in the
//!   knob-off runs);
//! - terrain's requests named by its refusals: three 4608 KiB and one
//!   3072 KiB vertex buffer, 512x512 mip chains of 1368 KiB, 800x600
//!   targets of 1876 KiB, the 800x608 depth buffer (1900 KiB): about 30 MiB
//!   in all (`scene-terrain.cpp`);
//! - shadow's 1600x1200 depth texture (7500 KiB, fmt 7) and its 1600x1216
//!   depth buffer (7600 KiB: "ds bytes peak 9728000" with the window's);
//! - refract's 1600x1200 depth then colour texture ("fmt 7", then "fmt 0"),
//!   then its 7600 KiB depth buffer at the first draw;
//! - the scenes before jellyfish in glmark2's default order, from their
//!   meshes and textures, rounded. What matters is what outlives a scene
//!   and lands between two large blocks: HWRT members, the index buffer's
//!   growth, the window's depth buffer, GL leftovers.
//!
//! Frees are given back at the scene's teardown, as with
//! `CONFIG_YAKOGL_PVR_POOL_RECLAIM` (the default since 8e7257f8): at refract
//! the board printed "pending frees now 0".
//!
//! The board held about 6 MiB more than the reconstruction names at
//! refract. "Unnamed" blocks stand for it: 256 KiB blocks kept from shadow
//! on, swept from none to past what either pool arm survives
//! (docs/notes/pvr-render-pool.md, "The host replay").
//!
//! The tables above are the board's reconstruction of 2026-10-04, when a
//! non-power-of-two texture had one level on the PowerVR. Since
//! pvr-npot-mips the GL core stores the whole chain of a mipmapped NPOT
//! texture, laid out as its power-of-two extent's chain (src/gl/texture.rs
//! `pot_chain_layout`): [`NPOT_800X600`] and [`NPOT_1600X1200`] are what
//! the real path asks for terrain's 800x600 targets and refract's 1600x1200
//! colour texture, pinned by the GL crate's tests/pvr_pool_npot.rs, and
//! [`Regrow`] is how it asks (level 0 freed, then the chain). The AM62P's
//! glmark2 image runs on a 256 MiB carveout since 504d4fae
//! ([`glmark2_carveout`], read from its overlay); the board's own 64 MiB
//! region ([`CARVEOUT`]) is the driver's minimum and what every other
//! image has. The freelists are sized by the carveout as pvr_render.c
//! sizes them ([`Replay::boot`]), and each scene's live high-water is
//! kept as the board's `osal_mem_line` counts live bytes
//! ([`Replay::live_kib`]). The findings and the board records this is
//! checked against: docs/notes/am62p-render-pool-glmark2.md.

use super::*;
use crate::gpu::pvr::fwif::{PDSCODEDATA_HEAP_BASE, PMMETA_PROTECT_HEAP_BASE, USCCODE_HEAP_BASE};
use crate::gpu::pvr::mem::{bo_alloc, bo_alloc_pm, pool_align};
use std::collections::BTreeMap;
use std::eprintln;

const PDS: usize = 1;
const K: u32 = 1024;
/// The AM62P's carveout and render pool: 64 MiB, the pool after the
/// firmware heap's 4 MiB + 192 KiB and the 2 MiB of page tables.
const CARVEOUT: usize = 64 << 20;
/// The glmark2 image's AM62P overlay: its `gpu_carveout_glmark2` node's
/// `reg` is the carveout the image runs on (256 MiB at 0xa0000000 since
/// 504d4fae). Read at compile time so that the replay's verdict follows
/// the overlay: shrinking the carveout below what glmark2 needs fails
/// `the_glmark2_images_carveout_serves_terrains_and_refracts_chains`.
const GLMARK2_OVERLAY: &str =
    include_str!("../../../../../../samples/zephyr/glmark2/boards/verdin_am62p_am62p54_a53.overlay");
const POOL_AM62P: u32 = (4 << 20) + (192 << 10) + (2 << 20);

/// The glmark2 image's carveout size, bytes, from [`GLMARK2_OVERLAY`]: the
/// second cell of the `reg = <base size>;` after `gpu_carveout_glmark2`.
fn glmark2_carveout() -> usize {
    let node = GLMARK2_OVERLAY.find("gpu_carveout_glmark2:").expect("the overlay's gpu_carveout_glmark2 node");
    let rest = &GLMARK2_OVERLAY[node..];
    let reg = rest.find("reg = <").expect("the node's reg") + "reg = <".len();
    let cells = &rest[reg..rest[reg..].find('>').expect("reg's closing bracket") + reg];
    let size = cells.split_whitespace().nth(1).expect("reg's size cell");
    usize::from_str_radix(size.trim_start_matches("0x"), 16).expect("reg's size cell is hex")
}
/// A mipmapped 800x600 RGBA8 texture on the PowerVR (terrain's render
/// targets): the 1024x1024 chain, 4 * (1024^2 + 512^2 + ... + 1) bytes,
/// 5462 KiB where level 0 alone is 1875. Pinned by tests/pvr_pool_npot.rs
/// (`terrains_800x600_target_frees_level_0_then_takes_the_chain`).
const NPOT_800X600: u32 = 5_592_404;
/// A mipmapped 1600x1200 RGBA8 texture (refract's colour texture): the
/// 2048x2048 chain, 21846 KiB where level 0 is 7500. Pinned by
/// tests/pvr_pool_npot.rs (`refracts_colour_texture_frees_level_0_then_
/// takes_the_chain_at_its_first_frame`).
const NPOT_1600X1200: u32 = 22_369_620;
/// Blocks that live for the whole run, and the index buffer (which grows).
const LIFE: u32 = u32::MAX;
const INDEX: u32 = u32::MAX - 1;

/// 1600x1200 R32F / RGBA8 (refract's and shadow's 2x-window targets), and
/// the backend's depth/stencil buffer for it: 1600 x 1216 x 4.
const TEX_1600: u32 = 1600 * 1200 * 4;
const DS_1600: u32 = 1600 * 1216 * 4;

/// What a scene does, in its order.
#[derive(Clone, Copy)]
enum Op {
    /// A GL object or backend block in GENERAL (`gpu_alloc`), freed at the
    /// scene's teardown.
    Gen(&'static str, u32),
    /// The same, kept for the whole run.
    Keep(&'static str, u32),
    /// A program: USC code and its uniforms.
    Prog,
    /// One HWRT member for a w x h target (the driver's, from the bump).
    Hwrt(u32, u32),
    /// The backend's sequential index buffer grows to this many bytes (the
    /// old one is freed).
    Index(u32),
    /// The scene's first draw: what follows is allocated there.
    Draw,
    /// A mipmapped NPOT texture's first `glGenerateMipmap`: the scene's most
    /// recent block of the first size (its level 0, a `Gen` before this) is
    /// freed, then its chain, the second size, is allocated
    /// (`texture_alloc_laid_out` releases the old storage before it asks for
    /// the new one; tests/pvr_pool_npot.rs). The free counts at once: the
    /// texture was settled first (its kick submitted and waited), so with
    /// the reclaim (the default) a refused chain drains the held level 0
    /// without a wait and asks again, and the chain can take its place.
    Regrow(&'static str, u32, u32),
}
use Op::*;

/// A block of the replay: its scene, heap, size.
struct Blk {
    scene: u32,
    heap: usize,
    size: u32,
    bo: PvrGpuBo,
}

/// GENERAL's state, KiB.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct Shot {
    /// The largest free range on the lists.
    largest: u32,
    /// Free bytes on the lists.
    free: u32,
    /// The gap between the bump and the top region's floor.
    gap: u32,
}

impl Shot {
    /// The largest contiguous range one request can get.
    fn contiguous(&self) -> u32 {
        self.largest.max(self.gap)
    }
}

struct Replay {
    r: Rig,
    live: Vec<Blk>,
    pm: u32,
    scene: u32,
    refused: Vec<(u32, &'static str, u32)>,
    /// Each scene's live high-water, KiB ([`Replay::live_kib`]).
    peak: BTreeMap<u32, u32>,
}

impl Replay {
    fn new(top_above: u32) -> Replay {
        Replay::with_carveout(top_above, CARVEOUT)
    }

    /// The AM62P's layout on a carveout of `carveout` bytes ([`CARVEOUT`],
    /// [`glmark2_carveout`]).
    fn with_carveout(top_above: u32, carveout: usize) -> Replay {
        Replay {
            r: Rig::with_layout(carveout, POOL_AM62P, top_above),
            live: Vec::new(),
            pm: 0,
            scene: LIFE,
            refused: Vec::new(),
            peak: BTreeMap::new(),
        }
    }

    fn shot(&mut self) -> Shot {
        let f = self.r.free_of(GEN);
        Shot { largest: f.largest / K, free: (f.bytes / 1024) as u32, gap: (self.r.k.pool_gap() / 1024) as u32 }
    }

    /// The pool's size, KiB (the carveout after the pool's offset).
    fn pool_kib(&self) -> u32 {
        ((self.r.k.pool_end() - u64::from(POOL_AM62P)) / 1024) as u32
    }

    /// Live bytes as the board's `osal_mem_line` counts them, KiB: what
    /// the pool handed out (the bump and the top region) minus the bytes
    /// on every heap's free lists (src/osal/zephyr.c).
    fn live_kib(&mut self) -> u32 {
        let handed = u64::from(self.r.pool_used()) + u64::from(self.r.k.pool_top);
        let listed: u64 = (0..POOL_HEAPS).map(|h| self.r.free_of(h).bytes).sum();
        ((handed - listed) / 1024) as u32
    }

    /// The scene's live high-water, after a block was served.
    fn note_live(&mut self) {
        let live = self.live_kib();
        let p = self.peak.entry(self.scene).or_insert(0);
        *p = (*p).max(live);
    }

    /// A pool allocation (`pvr_render_alloc`) for `scene`; false when
    /// refused (and recorded).
    fn a(&mut self, scene: u32, what: &'static str, heap: usize, bytes: u32) -> bool {
        match self.r.try_alloc(heap, bytes) {
            Some(bo) => {
                self.live.push(Blk { scene, heap, size: bytes, bo });
                self.note_live();
                true
            }
            None => {
                self.refused.push((self.scene, what, bytes.div_ceil(K)));
                false
            }
        }
    }

    /// A block of the driver's own from the bump (`gpu_bo_alloc`), never
    /// freed in the run.
    fn drv(&mut self, heap: usize, bytes: u32) {
        let base = [GENERAL_HEAP_BASE, PDSCODEDATA_HEAP_BASE, USCCODE_HEAP_BASE][heap];
        let bo = bo_alloc(&mut self.r.k, base, &mut self.r.bumps[heap], bytes, L0_FLAGS_DATA);
        if bo.cpu.is_null() {
            self.refused.push((self.scene, "a driver block", bytes.div_ceil(K)));
        } else {
            self.note_live();
        }
    }

    /// A PM-protected block (a PFN array, an MList) from the bump.
    fn pm(&mut self, bytes: u32) {
        let mut pm = self.pm;
        let bo = bo_alloc_pm(&mut self.r.k, PMMETA_PROTECT_HEAP_BASE, &mut pm, bytes);
        self.pm = pm;
        if bo.cpu.is_null() {
            self.refused.push((self.scene, "a PM block", bytes.div_ceil(K)));
        } else {
            self.note_live();
        }
    }

    /// One HWRT member for a w x h target (`render.rs` hwrt_get's sizes).
    fn hwrt_member(&mut self, w: u32, h: u32) {
        let mtx1 = w.div_ceil(16).div_ceil(8) * 2;
        let mty1 = h.div_ceil(16).div_ceil(8) * 2;
        let (tx, ty) = (4 * mtx1, 4 * mty1);
        let rgn = round_up(u64::from((tx / 2) * (ty / 2) * 6), 64) as u32;
        let m = tx.next_power_of_two().max(ty.next_power_of_two());
        let tpc = m * m / 4 * 8;
        let tpc_alloc = if tpc * 2 > 0x4000 { round_up(u64::from(tpc * 2), 4096) as u32 } else { 0x4000 };
        let rgn_alloc = if rgn * 4 > 0x10000 { round_up(u64::from(rgn * 4), 4096) as u32 } else { 0x10000 };
        self.drv(GEN, tpc_alloc);
        self.drv(GEN, 0x2000);
        self.drv(GEN, rgn_alloc);
        self.drv(GEN, 0x1000);
        self.pm(0x1000);
    }

    /// What every run has before its first scene: the render service's
    /// statics and freelists (pvr_render.c, freelist::build), the render
    /// context, the backend's frame sets and statics.
    fn boot(&mut self) {
        /* static_areas_init: VDM_SYNC, end-of-tile programs, tile buffers */
        self.drv(USC, 4 * K);
        self.drv(PDS, 4 * K);
        for _ in 0..3 {
            self.drv(USC, 4 * K);
        }
        for _ in 0..2 {
            self.drv(GEN, 32 * K);
        }
        for _ in 0..3 {
            self.drv(USC, 4 * K);
        }
        /* the parameter-buffer reserve, the border colours */
        self.drv(GEN, 2048 * K);
        self.drv(GEN, 4 * K);
        /* The freelists, sized by the carveout as pvr_render.c sizes them
         * (its parameter-buffer block). Below 128 MiB: two global lists of
         * 448 current + 64 ready + 4 grows of 256 pages, 1536 backed, with
         * a 4096-entry PFN array each. From 128 MiB: a quarter of the
         * carveout as the shared grow pool first (freelist::pool_init, 16
         * KiB aligned), then two lists of 2048 pages with no reserve and a
         * PFN array of 2048 + the pool's pages. The local list is 26 pages
         * either way. */
        let carveout = self.r.k.size;
        let grow_pool = if carveout >= 128 << 20 { ((carveout / 4 / 4096) as u32) & !1023 } else { 0 };
        if grow_pool != 0 {
            pool_align(&mut self.r.k, 0x4000);
            self.drv(GEN, grow_pool * 4 * K);
        }
        let global = if grow_pool != 0 { 2048 } else { 1536 };
        for pages in [global, global, 26] {
            pool_align(&mut self.r.k, 0x4000);
            self.drv(GEN, pages * 4 * K);
            self.pm(if pages == 26 { 16 * K } else { (if grow_pool != 0 { 2048 + grow_pool } else { 4096 }) * 4 });
        }
        self.drv(GEN, 4 * K); /* the render context's VDM stream */
        /* the backend: 4 frame sets of 1024 + 768 KiB arenas and their tables */
        for _ in 0..4 {
            assert!(self.a(LIFE, "pds arena", PDS, 768 * K));
            assert!(self.a(LIFE, "pds static page", PDS, 4 * K));
            assert!(self.a(LIFE, "general arena", GEN, 1024 * K));
            assert!(self.a(LIFE, "vdm stream", GEN, 64 * K));
            for what in ["depth-bias table", "scissor table", "background colour"] {
                assert!(self.a(LIFE, what, GEN, 4 * K));
            }
        }
        for i in 0..38 {
            assert!(self.a(LIFE, "backend static", if i % 3 == 0 { GEN } else { USC }, 4 * K));
        }
        assert!(self.a(INDEX, "index buffer", GEN, 4 * K));
    }

    /// One scene: its ops in order, then its teardown.
    fn scene(&mut self, n: u32, ops: &[Op]) {
        self.scene = n;
        for &op in ops {
            match op {
                Gen(what, bytes) => {
                    self.a(n, what, GEN, bytes);
                }
                Keep(what, bytes) => {
                    self.a(LIFE, what, GEN, bytes);
                }
                Prog => {
                    self.a(n, "program code", USC, 8 * K);
                    self.a(n, "program uniforms", GEN, 4 * K);
                }
                Hwrt(w, h) => self.hwrt_member(w, h),
                Index(bytes) => {
                    let i = self.live.iter().position(|b| b.scene == INDEX).expect("the index buffer");
                    let old = self.live.remove(i);
                    self.r.free(&old.bo, old.size);
                    self.a(INDEX, "index buffer", GEN, bytes);
                }
                Draw => {}
                Regrow(what, level0, chain) => {
                    self.regrow(n, what, level0, chain);
                }
            }
        }
    }

    /// [`Regrow`]: free the scene's most recent block of `level0` bytes (if
    /// level 0 itself was served), then allocate `chain`; whether the chain
    /// was served.
    fn regrow(&mut self, n: u32, what: &'static str, level0: u32, chain: u32) -> bool {
        if let Some(i) = self.live.iter().rposition(|b| b.scene == n && b.heap == GEN && b.size == level0) {
            let old = self.live.remove(i);
            self.r.free(&old.bo, old.size);
        }
        self.a(n, what, GEN, chain)
    }

    fn teardown(&mut self) {
        let n = self.scene;
        let (gone, keep): (Vec<Blk>, Vec<Blk>) = self.live.drain(..).partition(|b| b.scene == n);
        self.live = keep;
        for b in gone {
            self.r.free(&b.bo, b.size);
        }
    }
}

const JELLYFISH: &[Op] = &[
    Prog,
    Gen("jellyfish positions", 64 * K),
    Gen("jellyfish normals", 64 * K),
    Gen("jellyfish texcoords", 48 * K),
    Gen("jellyfish indices", 32 * K),
    Gen("background 256x256", 256 * K),
    Gen("uniforms", 16 * K),
    Gen("uniforms", 16 * K),
    Gen("uniforms", 16 * K),
    Gen("uniforms", 16 * K),
    Gen("uniforms", 16 * K),
    Gen("staging", 12 * K),
];

const TERRAIN: &[Op] = &[
    Prog,
    Gen("terrain positions", 4608 * K),
    Prog,
    Gen("terrain normals", 4608 * K),
    Gen("terrain texcoords", 4608 * K),
    Gen("terrain indices", 3072 * K),
    Prog,
    Gen("512x512 mip chain", 1_398_101),
    Gen("512x512 mip chain", 1_398_101),
    Gen("512x512 mip chain", 1_398_101),
    Prog,
    Gen("256x256 target", 256 * K),
    Gen("256x256 target", 256 * K),
    Prog,
    Gen("800x600 target", 800 * 600 * 4),
    Gen("800x600 target", 800 * 600 * 4),
    Gen("800x600 target", 800 * 600 * 4),
    Gen("800x600 target", 800 * 600 * 4),
    Prog,
    Prog,
    Draw,
    Gen("800x608 depth buffer", 800 * 608 * 4),
    Hwrt(256, 256),
    Hwrt(256, 256),
    Hwrt(512, 512),
    Hwrt(512, 512),
];

/// [`TERRAIN`] on the PowerVR since pvr-npot-mips: its first `chains`
/// 800x600 targets are mipmapped, so each is regrown from level 0 to its
/// NPOT chain ([`Regrow`], [`NPOT_800X600`]) right after `glTexImage2D`
/// (`BaseRenderer::setup_texture` calls `update_mipmap` at set-up). At
/// glmark2's default options (bloom and tilt-shift on) the source has two
/// such targets, `terrain_renderer` and `tilt_h_renderer`
/// (scene-terrain.cpp `init_renderers`); the table's other two 800x600
/// targets are the board's reconstruction and stay at level 0. `chains` is
/// swept past two for margin. 0 is the table as it was.
fn terrain(chains: u32) -> Vec<Op> {
    let mut left = chains;
    let mut ops = Vec::new();
    for &op in TERRAIN {
        ops.push(op);
        if let Gen("800x600 target", level0) = op {
            if left > 0 {
                left -= 1;
                ops.push(Regrow("800x600 mip chain", level0, NPOT_800X600));
            }
        }
    }
    ops
}

const SHADOW: &[Op] = &[
    Prog,
    Prog,
    Gen("horse vertices", 192 * K),
    Gen("horse indices", 192 * K),
    Gen("ground", 16 * K),
    Gen("1600x1200 depth texture", TEX_1600),
    Draw,
    Gen("1600x1216 depth buffer", DS_1600),
    Hwrt(1600, 1200),
    Hwrt(1600, 1200),
];

const REFRACT_SETUP: &[Op] = &[
    Prog,
    Prog,
    Gen("bunny vertices", 256 * K),
    Gen("bunny indices", 256 * K),
    Gen("1600x1200 depth texture (fmt 7)", TEX_1600),
    Gen("1600x1200 colour texture (fmt 0)", TEX_1600),
];

/// glmark2's entries 0-20, before jellyfish (build, texture, shading, bump,
/// effect2d, pulsar, desktop, buffer, ideas).
fn before_jellyfish(rp: &mut Replay) {
    let tex512 = 512 * 512 * 4;
    let scenes: [&[Op]; 21] = [
        &[Prog, Draw, Keep("window depth buffer 800x608", 800 * 608 * 4), Hwrt(800, 600), Hwrt(800, 600)],
        &[Prog, Gen("horse vbo", 192 * K), Gen("horse vbo", 192 * K)],
        &[Prog, Gen("cube", 16 * K), Gen("crate 512x512", tex512)],
        &[Prog, Gen("cube", 16 * K), Gen("crate 512x512", tex512)],
        &[Prog, Gen("cube", 16 * K), Gen("crate 512x512 mip chain", 1_398_101)],
        &[Prog, Gen("cat vbo", 320 * K), Gen("cat vbo", 320 * K)],
        &[Prog, Gen("cat vbo", 320 * K), Gen("cat vbo", 320 * K)],
        &[Prog, Gen("cat vbo", 320 * K), Gen("cat vbo", 320 * K)],
        &[Prog, Gen("cat vbo", 320 * K), Gen("cat vbo", 320 * K)],
        &[Prog, Gen("asteroid-high", 2800 * K), Gen("asteroid-high", 2800 * K), Draw, Index(128 * K)],
        &[Prog, Gen("asteroid-low", 40 * K), Gen("normal map 1024x1024", 4096 * K)],
        &[Prog, Gen("asteroid-low", 40 * K), Gen("height map 1024x1024", 4096 * K)],
        &[Prog, Gen("effect2d 512x512", tex512)],
        &[Prog, Gen("effect2d 512x512", tex512)],
        &[Prog, Gen("quads", 16 * K), Gen("pulsar 256x256", 256 * K)],
        &[
            Prog,
            Prog,
            Gen("window 256x256", 256 * K),
            Gen("window 256x256", 256 * K),
            Gen("blur 800x600", 800 * 600 * 4),
            Gen("blur 800x600", 800 * 600 * 4),
            Draw,
            Hwrt(300, 300),
            Hwrt(300, 300),
        ],
        &[Prog, Gen("window 256x256", 256 * K), Gen("window 256x256", 256 * K), Gen("shadow", 64 * K)],
        &[Prog, Gen("grid vbo", 2048 * K), Gen("grid vbo", 2048 * K)],
        &[Prog, Gen("grid vbo", 2048 * K), Gen("grid vbo", 2048 * K)],
        &[Prog, Gen("grid vbo", 2048 * K), Gen("grid vbo", 2048 * K)],
        &[Prog, Prog, Prog, Prog, Gen("ideas", 512 * K), Gen("ideas", 256 * K), Gen("ideas", 128 * K)],
    ];
    rp.scene = LIFE;
    rp.a(LIFE, "gl leftovers", GEN, 512 * K);
    rp.a(LIFE, "gl leftovers", GEN, 4 * K);
    rp.a(LIFE, "gl leftovers", GEN, 4 * K);
    for (n, ops) in scenes.iter().enumerate() {
        rp.scene(n as u32, ops);
        rp.teardown();
    }
}

/// What the replay shows: GENERAL before refract's set-up and at its depth
/// buffer, whether that buffer was served, GENERAL at its colour chain and
/// whether that was served (`None`: the run stopped at the depth buffer,
/// the table as the board measured it), and every refusal.
struct Glmark2 {
    before_refract: Shot,
    at_depth_buffer: Shot,
    depth_buffer_served: bool,
    at_colour_chain: Shot,
    colour_chain_served: Option<bool>,
    refused: Vec<(u32, &'static str, u32)>,
    /// Each scene's live high-water, KiB ([`Replay::live_kib`]; the boot
    /// and the blocks kept for the run are under [`LIFE`]).
    peak: BTreeMap<u32, u32>,
    /// The bump after refract's teardown, KiB (the board's "bump N KiB
    /// now" of the summary).
    end_bump: u32,
    /// The pool's size, KiB.
    pool: u32,
}

/// One glmark2 replay: the pool arm, the unnamed blocks, and what
/// pvr-npot-mips added (`chains` of terrain's targets regrown, refract's
/// colour texture regrown at its first frame) on a carveout.
#[derive(Clone, Copy)]
struct Run {
    top_above: u32,
    unnamed: u32,
    chains: u32,
    refract_chain: bool,
    carveout: usize,
}

impl Run {
    /// The board's reconstruction: level-0 targets on the 64 MiB carveout.
    const fn measured(top_above: u32, unnamed: u32) -> Run {
        Run { top_above, unnamed, chains: 0, refract_chain: false, carveout: CARVEOUT }
    }

    /// The PowerVR since pvr-npot-mips: terrain's two mipmapped targets and
    /// refract's colour texture as chains, on `carveout`.
    const fn with_chains(top_above: u32, unnamed: u32, carveout: usize) -> Run {
        Run { top_above, unnamed, chains: 2, refract_chain: true, carveout }
    }
}

/// The board's GENERAL at refract's depth buffer (the reclaim round,
/// 8e7257f8, knob off): "free 9712 KiB in 8 ranges, largest 3936 KiB",
/// "1940 KiB left" above the bump.
const BOARD_FREE: u32 = 9712;
const BOARD_LEFT: u32 = 1940;

/// The reconstruction alone holds about 6 MiB less there than the board
/// did (its GENERAL free at refract's depth buffer is 15872 KiB, the
/// board's 9712). The blocks the reconstruction does not name are modelled
/// as 256 KiB blocks kept from shadow on; this many bring its free bytes
/// and its bump to the board's.
const UNNAMED_AT_THE_BOARD: u32 = 24;

/// The glmark2 host harness on the PowerVR build measured refract's scene
/// at 45160 KiB, 7700 KiB above the blocks the reconstruction names
/// (docs/notes/am62p-render-pool-glmark2.md, "Harness"): the unnamed
/// blocks measured, 30 of 256 KiB (7680 KiB).
const UNNAMED_AT_THE_HARNESS: u32 = 30;

/// glmark2's entries 0-24 (to refract's first draw), with `unnamed` 256 KiB
/// blocks kept for the rest of the run from shadow's frames on: the board's
/// reconstruction ([`Run::measured`]).
fn glmark2(top_above: u32, unnamed: u32) -> Glmark2 {
    replay(Run::measured(top_above, unnamed))
}

/// [`glmark2`] for any [`Run`]: with `refract_chain` the run goes on to the
/// end of refract's first frame, where its colour texture is regrown.
fn replay(run: Run) -> Glmark2 {
    let Run { top_above, unnamed, chains, refract_chain, carveout } = run;
    let mut rp = Replay::with_carveout(top_above, carveout);
    rp.boot();
    before_jellyfish(&mut rp);
    /* 21 jellyfish: 32 caustic frames of 256x256 with mips (344 KiB), the
     * rest: 45 blocks, 11576 KiB */
    rp.scene = 21;
    for _ in 0..32 {
        rp.a(21, "caustic 256x256 mip chain", GEN, 349_525);
    }
    rp.scene(21, JELLYFISH);
    let jelly: u32 = rp.live.iter().filter(|b| b.scene == 21).map(|b| b.size.div_ceil(4096) * 4).sum();
    assert_eq!((rp.live.iter().filter(|b| b.scene == 21).count(), jelly), (45, 11576), "jellyfish's frees");
    rp.teardown();
    rp.scene(22, &terrain(chains));
    rp.teardown();
    rp.scene(23, SHADOW);
    for _ in 0..unnamed {
        rp.a(LIFE, "unnamed", GEN, 256 * K);
    }
    rp.teardown();
    let before_refract = rp.shot();
    rp.scene(24, REFRACT_SETUP);
    let at_depth_buffer = rp.shot();
    let depth_buffer_served = rp.a(24, "1600x1216 depth buffer", GEN, DS_1600);
    let at_colour_chain = rp.shot();
    /* the first frame's glGenerateMipmap of the colour texture, after the
     * depth pass (scene-refract.cpp) */
    let colour_chain_served =
        refract_chain.then(|| rp.regrow(24, "1600x1200 colour mip chain", TEX_1600, NPOT_1600X1200));
    rp.teardown();
    let (end_bump, pool) = (rp.r.pool_used() / K, rp.pool_kib());
    Glmark2 {
        before_refract,
        at_depth_buffer,
        depth_buffer_served,
        at_colour_chain,
        colour_chain_served,
        refused: rp.refused,
        peak: rp.peak,
        end_bump,
        pool,
    }
}

/// Refract's three large blocks (depth and colour textures, the depth
/// buffer), KiB.
const REFRACT_NEEDS: u32 = (2 * TEX_1600 + DS_1600) / 1024;
const DS_KIB: u32 = DS_1600 / 1024;

fn say(arm: &str, unnamed: u32, g: &Glmark2) {
    eprintln!(
        "glmark2 replay, {arm}, {unnamed} unnamed: before refract {:?} (contiguous {} KiB, refract needs {} KiB); at its depth buffer {:?}, served {}; at its colour chain {:?}, served {:?}; refused {:?}",
        g.before_refract,
        g.before_refract.contiguous(),
        REFRACT_NEEDS,
        g.at_depth_buffer,
        g.depth_buffer_served,
        g.at_colour_chain,
        g.colour_chain_served,
        g.refused
    );
}

/// Without the top region: before refract no range holds its three blocks
/// (shadow's two freed blocks merged are the largest, 15100 KiB). With the
/// unnamed blocks that bring the reconstruction to the board's bytes, its
/// GENERAL free and its bump at refract's depth buffer are the board's to
/// within 256 KiB. That buffer is still served there: the board's free space
/// was in smaller pieces (largest 3936 KiB) than the reconstruction's
/// (7600). From 31 unnamed blocks on (7.75 MiB) it is refused as on the
/// board, with the bytes free in pieces; nothing else is refused.
#[test]
fn glmark2_without_the_top_region_has_no_range_for_refract_and_refuses_its_depth_buffer() {
    let g = glmark2(0, 0);
    say("top region off", 0, &g);
    assert_eq!(g.before_refract.contiguous(), 15100);
    let g = glmark2(0, UNNAMED_AT_THE_BOARD);
    say("top region off", UNNAMED_AT_THE_BOARD, &g);
    assert!(g.at_depth_buffer.free.abs_diff(BOARD_FREE) < 256 && g.at_depth_buffer.gap.abs_diff(BOARD_LEFT) < 256);
    assert!(g.before_refract.contiguous() < REFRACT_NEEDS);
    for n in 0..=38 {
        let g = glmark2(0, n);
        assert_eq!(g.depth_buffer_served, n < 31, "{n} unnamed");
        if n >= 31 {
            assert_eq!(g.refused, [(24, "1600x1216 depth buffer", DS_KIB)], "{n} unnamed: only refract's depth buffer");
            let s = g.at_depth_buffer;
            assert!(s.largest < DS_KIB && s.gap < DS_KIB && s.free + s.gap >= DS_KIB, "{n} unnamed: {s:?}");
        }
    }
    say("top region off", 31, &glmark2(0, 31));
}

/// With the top region at the driver's threshold, the same sequence leaves
/// one range of at least refract's three blocks (22600 KiB) before its
/// set-up, and nothing in the run is refused: alone, at the board's bytes,
/// and up to 38 unnamed blocks (9.5 MiB), seven more than the arm without it
/// takes. From 39 on, the gap is short of refract's blocks too.
#[test]
fn glmark2_with_the_top_region_fits_refract_after_shadow_and_terrain() {
    for n in 0..=39 {
        let g = glmark2(YD_PVR_POOL_TOP_ABOVE, n);
        if n == 0 || n == UNNAMED_AT_THE_BOARD || n >= 38 {
            say("top region on", n, &g);
        }
        if n == 39 {
            assert!(!g.depth_buffer_served && g.before_refract.contiguous() < REFRACT_NEEDS);
            continue;
        }
        assert!(g.before_refract.contiguous() >= REFRACT_NEEDS, "{n} unnamed: {:?}", g.before_refract);
        assert!(g.depth_buffer_served, "{n} unnamed");
        assert_eq!(g.refused, [], "{n} unnamed");
    }
}

/// Refract's colour chain, KiB.
const COLOUR_CHAIN_KIB: u32 = NPOT_1600X1200.div_ceil(K);

/// Terrain's NPOT mip chains (pvr-pool-npot) on the board's 64 MiB
/// carveout: with one to four of its 800x600 targets regrown to chains
/// (5462 KiB instead of 1875; the source has two), refract's three blocks
/// (22600 KiB) still fit after shadow and terrain with the top region at
/// the driver's threshold, alone and at the board's bytes, and its depth
/// buffer is served. Terrain's blocks all go back at its teardown, so
/// GENERAL before refract is what it is with level-0 targets.
///
/// What the chains strain is terrain's own set-up: from one chain on the
/// 59200 KiB pool refuses some of it in either arm (capacity: 3587 KiB more
/// per chain on top of its 30 MiB). With the region the refusals are
/// terrain's alone; without it, from three chains on, the old
/// fragmentation is back and refract's depth buffer is refused again. The
/// lists are the same alone and at the board's bytes.
#[test]
fn glmark2_with_terrains_npot_mip_chains_still_fits_refract_in_the_64_mib_carveout() {
    let target = (22, "800x600 target", 800 * 600 * 4 / 1024);
    let chain = (22, "800x600 mip chain", NPOT_800X600.div_ceil(K));
    let depth = (22, "800x608 depth buffer", 800 * 608 * 4 / 1024);
    let refract_ds = (24, "1600x1216 depth buffer", DS_KIB);
    let on_refuses: [&[(u32, &str, u32)]; 5] = [
        &[],
        &[depth],
        &[target, target, depth],
        &[target, chain, target, depth],
        &[target, chain, target, chain, depth],
    ];
    let off_refuses: [&[(u32, &str, u32)]; 5] =
        [&[], &[depth], &[chain], &[chain, chain, refract_ds], &[chain, chain, chain, refract_ds]];
    for chains in 0..=4 {
        for n in [0, UNNAMED_AT_THE_BOARD] {
            let run =
                Run { top_above: YD_PVR_POOL_TOP_ABOVE, unnamed: n, chains, refract_chain: false, carveout: CARVEOUT };
            let on = replay(run);
            let off = replay(Run { top_above: 0, ..run });
            say(&std::format!("top region on, {chains} npot chains"), n, &on);
            say(&std::format!("top region off, {chains} npot chains"), n, &off);
            let level0 = glmark2(YD_PVR_POOL_TOP_ABOVE, n);
            assert_eq!(
                on.before_refract, level0.before_refract,
                "{chains} chains, {n} unnamed: GENERAL before refract"
            );
            assert!(on.before_refract.contiguous() >= REFRACT_NEEDS, "{chains} chains, {n} unnamed");
            assert!(on.depth_buffer_served, "{chains} chains, {n} unnamed");
            assert_eq!(on.refused, on_refuses[chains as usize], "{chains} chains, {n} unnamed, top region on");
            assert_eq!(off.refused, off_refuses[chains as usize], "{chains} chains, {n} unnamed, top region off");
            assert_eq!(off.depth_buffer_served, chains < 3, "{chains} chains, {n} unnamed, top region off");
        }
    }
}

/// Refract's own colour texture is a 2048x2048 chain since pvr-npot-mips
/// (21846 KiB, [`NPOT_1600X1200`]), regrown at the end of its first frame.
/// On the board's 64 MiB carveout its three blocks are served (the source's
/// two terrain chains) and the colour chain is refused in both arms, with
/// the free bytes and the gap together short of it: capacity, not
/// placement. With it refract needs 7500 + 21846 + 7600 = 36946 KiB, and
/// the pool holds about 26 MiB before refract whatever the arm.
#[test]
fn refracts_own_1600x1200_chain_is_beyond_the_64_mib_carveouts_capacity() {
    for n in [0, UNNAMED_AT_THE_BOARD] {
        let on = replay(Run::with_chains(YD_PVR_POOL_TOP_ABOVE, n, CARVEOUT));
        let off = replay(Run::with_chains(0, n, CARVEOUT));
        say("top region on, refract's chain", n, &on);
        say("top region off, refract's chain", n, &off);
        for (arm, g) in [("on", &on), ("off", &off)] {
            assert!(g.depth_buffer_served, "{n} unnamed, top region {arm}");
            assert_eq!(g.colour_chain_served, Some(false), "{n} unnamed, top region {arm}");
            let s = g.at_colour_chain;
            assert!(s.free + s.gap < COLOUR_CHAIN_KIB, "{n} unnamed, top region {arm}: {s:?} is capacity");
            assert_eq!(g.refused.last(), Some(&(24, "1600x1200 colour mip chain", COLOUR_CHAIN_KIB)));
        }
    }
}

/// The glmark2 image's carveout, read from its overlay (256 MiB, 504d4fae):
/// terrain's chains, refract's three blocks and its colour chain are all
/// served, in both arms, alone, at the board's and at the harness's extra
/// bytes and with 38 unnamed blocks; nothing in the run is refused, every
/// scene's live high-water stays under 3/4 of the pool (the rule the
/// GC7000's glmark2 row uses), and the threshold stays 256 KiB. The pool
/// is 255808 KiB there, and the persistent set about 95 MiB: from 128 MiB
/// up the render service takes a quarter of the carveout as the freelist
/// grow pool. Shrinking the overlay's carveout below what glmark2 needs
/// fails here.
#[test]
fn the_glmark2_images_carveout_serves_terrains_and_refracts_chains() {
    assert_eq!(YD_PVR_POOL_TOP_ABOVE, 256 * K);
    let carveout = glmark2_carveout();
    assert_eq!(carveout, 256 << 20, "the overlay's gpu_carveout_glmark2 reg size");
    for n in [0, UNNAMED_AT_THE_BOARD, UNNAMED_AT_THE_HARNESS, 38] {
        for top_above in [YD_PVR_POOL_TOP_ABOVE, 0] {
            let run = Run { chains: 4, ..Run::with_chains(top_above, n, carveout) };
            let g = replay(run);
            say(
                &std::format!("{} MiB, top region {}, 4 npot chains, refract's chain", carveout >> 20, top_above / K),
                n,
                &g,
            );
            eprintln!("  pool {} KiB, end bump {} KiB, peaks {:?}", g.pool, g.end_bump, g.peak);
            assert_eq!(g.pool, 255_808, "{n} unnamed, top region {top_above}");
            assert_eq!(g.refused, [], "{n} unnamed, top region {top_above}");
            assert!(g.depth_buffer_served, "{n} unnamed, top region {top_above}");
            assert_eq!(g.colour_chain_served, Some(true), "{n} unnamed, top region {top_above}");
            assert!(g.before_refract.contiguous() >= REFRACT_NEEDS + COLOUR_CHAIN_KIB);
            for (scene, peak) in &g.peak {
                assert!(
                    *peak < g.pool / 4 * 3,
                    "{n} unnamed, top region {top_above}: scene {scene} peaks at {peak} KiB"
                );
            }
        }
    }
}

/// The 256 MiB replay against the board run at e3749a25
/// (docs/notes/am62p-render-pool-glmark2.md, "How well the model
/// matches the board"): the bump after refract, with no unnamed blocks (on
/// the board they are a scene's and go back; here they are kept for the
/// run), is within 1 MiB of the board's end-of-run 91356 KiB, and
/// refract's live high-water at the board's extra bytes is within 1 MiB of
/// its 139212 KiB (142553088 bytes live at refract's end). At the
/// harness's extra bytes the peak is 1.5 MiB over the board's. This keeps
/// the reconstruction honest against a measured run.
#[test]
fn the_replay_matches_the_256_mib_board_run() {
    let carveout = glmark2_carveout();
    let alone = replay(Run::with_chains(YD_PVR_POOL_TOP_ABOVE, 0, carveout));
    let board = replay(Run::with_chains(YD_PVR_POOL_TOP_ABOVE, UNNAMED_AT_THE_BOARD, carveout));
    let harness = replay(Run::with_chains(YD_PVR_POOL_TOP_ABOVE, UNNAMED_AT_THE_HARNESS, carveout));
    eprintln!(
        "256 MiB board match: end bump {} KiB alone (board 91356); refract's peak {} KiB at the board's bytes, {} at the harness's (board 139212)",
        alone.end_bump, board.peak[&24], harness.peak[&24]
    );
    assert!(alone.end_bump.abs_diff(91_356) <= 1024, "end bump {}", alone.end_bump);
    assert!(board.peak[&24].abs_diff(139_212) <= 1024, "refract's peak {}", board.peak[&24]);
    assert!(harness.peak[&24].abs_diff(139_212) <= 2048, "refract's peak {}", harness.peak[&24]);
}

/// A UI run: the boot, then the application's objects, which live for the
/// run, then `frames` frames whose small buffers come and go, and a glyph
/// atlas that grows once. Every refusal, and GENERAL at the end.
fn ui(
    top_above: u32,
    objects: &[(&'static str, u32)],
    atlas: (u32, u32),
    frames: u32,
) -> (Vec<(u32, &'static str, u32)>, Shot) {
    let mut rp = Replay::new(top_above);
    rp.boot();
    rp.scene = LIFE;
    for &(what, bytes) in objects {
        rp.a(LIFE, what, GEN, bytes);
    }
    rp.a(1, "glyph atlas", GEN, atlas.0);
    for f in 0..frames {
        rp.scene = 2 + f;
        for i in 0..8 {
            rp.a(2 + f, "vertices", GEN, (4 + 4 * (i % 4)) * K);
        }
        rp.a(2 + f, "uniforms", GEN, 64 * K);
        rp.a(2 + f, "image upload", GEN, 192 * K);
        if f == frames / 2 {
            /* the atlas grows: the new one first, then the old one goes */
            rp.a(LIFE, "glyph atlas", GEN, atlas.1);
            rp.scene = 1;
            rp.teardown();
            rp.scene = 2 + f;
        }
        rp.teardown();
    }
    let s = rp.shot();
    (rp.refused, s)
}

/// gl_slint's dashboard (800x480) and the AGL cluster (1280x720): with the
/// top region, every allocation is served, as without it, the frame sets
/// (4 x (1 MiB + 768 KiB)) included, and the largest range left is no
/// smaller.
#[test]
fn the_slint_and_cluster_sequences_are_served_with_the_top_region_as_without() {
    let slint: &[(&str, u32)] = &[
        ("window depth buffer 800x480", 800 * 480 * 4),
        ("background 800x480", 800 * 480 * 4),
        ("image 400x240", 400 * 240 * 4),
        ("image 400x240", 400 * 240 * 4),
        ("icon 128x128", 64 * K),
        ("icon 128x128", 64 * K),
        ("icon 128x128", 64 * K),
        ("icon 128x128", 64 * K),
        ("icon 128x128", 64 * K),
        ("icon 128x128", 64 * K),
    ];
    let cluster: &[(&str, u32)] = &[
        ("window depth buffer 1280x736", 1280 * 736 * 4),
        ("background 1280x720", 1280 * 720 * 4),
        ("gauge 512x512", 512 * 512 * 4),
        ("gauge 512x512", 512 * 512 * 4),
        ("needle 64x256", 64 * K),
        ("needle 64x256", 64 * K),
        ("needle 64x256", 64 * K),
        ("needle 64x256", 64 * K),
        ("map tile 512x512", 512 * 512 * 4),
        ("map tile 512x512", 512 * 512 * 4),
    ];
    for (name, objects, atlas) in
        [("slint", slint, (512 * 512 * 4, 1024 * 1024 * 4)), ("cluster", cluster, (1024 * 1024 * 4, 2048 * 1024 * 4))]
    {
        let (off_refused, off) = ui(0, objects, atlas, 200);
        let (on_refused, on) = ui(YD_PVR_POOL_TOP_ABOVE, objects, atlas, 200);
        eprintln!("{name} replay: top region off {off:?}, on {on:?}");
        assert_eq!((off_refused, on_refused), (std::vec![], std::vec![]), "{name}: nothing refused in either arm");
        assert!(on.contiguous() >= off.contiguous(), "{name}: the largest range left shrank");
    }
}

/// The frame sets with the top region: their four 1 MiB GENERAL arenas are
/// the region's first blocks, at the carveout's end, one after another; the
/// rest of each set (PDS arenas, tables) comes from the bump as before.
#[test]
fn the_frame_sets_general_arenas_are_the_top_regions_first_blocks() {
    let mut rp = Replay::new(YD_PVR_POOL_TOP_ABOVE);
    rp.boot();
    let end = CARVEOUT as u32;
    let arenas: Vec<u32> =
        rp.live.iter().filter(|b| b.size == 1024 * K && b.heap == GEN).map(|b| b.bo.heap_off).collect();
    assert_eq!(arenas, (1..=4).map(|i| TOP_VA + end - i * 1024 * K).collect::<Vec<u32>>());
    assert!(rp.live.iter().filter(|b| b.heap == PDS).all(|b| b.bo.heap_off < TOP_VA));
    assert_eq!(rp.r.k.pool_top, 4096 * K);
}
