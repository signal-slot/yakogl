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
//! `carveout.rs` replays the same sequence with the NPOT mip chains at
//! their power-of-two extent on carve-outs of other sizes, the freelists
//! sized as `driver/src/pvr_render.c` sizes them per carve-out
//! (docs/notes/am62p-render-pool-glmark2.md).

mod carveout;

use super::*;
use crate::gpu::pvr::fwif::{PDSCODEDATA_HEAP_BASE, PMMETA_PROTECT_HEAP_BASE, USCCODE_HEAP_BASE};
use crate::gpu::pvr::mem::{bo_alloc, bo_alloc_pm, pool_align};
use std::eprintln;

const PDS: usize = 1;
const K: u32 = 1024;
/// The AM62P's carveout and render pool: 64 MiB, the pool after the
/// firmware heap's 4 MiB + 192 KiB and the 2 MiB of page tables.
const CARVEOUT: usize = 64 << 20;
const POOL_AM62P: u32 = (4 << 20) + (192 << 10) + (2 << 20);
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
}
use Op::*;

/// A block of the replay: its scene, what it is, heap, size.
struct Blk {
    scene: u32,
    what: &'static str,
    heap: usize,
    size: u32,
    bo: PvrGpuBo,
}

/// The render service's blocks before the first scene, as
/// `driver/src/pvr_render.c` (`pvr_render_init`) sizes them from the
/// carve-out: the shared freelist grow pool (pages; 0: none) and the three
/// freelists (backing pages, PFN-array bytes) in their build order: the two
/// global lists, then the local one.
struct Cfg {
    grow_pool_pages: u32,
    lists: [(u32, u32); 3],
}

impl Cfg {
    /// `pvr_render.c` lines 632-676 for a carve-out of `bytes`. Below
    /// 128 MiB each global list backs 448 current + 64 ready pages and a
    /// reserve of 4 grows of 256 (`freelist::build` with the default
    /// reserve multiple), 1536 pages, and its PFN array has the 4096
    /// `max` entries. From 128 MiB on, a grow pool of a quarter of the
    /// carve-out (in pages, rounded down to 1024) comes first; each global
    /// list then backs 2048 pages with no reserve, and its array has
    /// 2048 + the pool's entries. The local list is 26 pages with a 32-entry
    /// array at every size (`bo_alloc_pm` rounds it to 16 KiB).
    fn for_carveout(bytes: usize) -> Cfg {
        let big_pb = bytes >= 128 << 20;
        let pool = if big_pb { ((bytes / 4 / 4096) as u32) & !1023 } else { 0 };
        let (cur, ready, grow, reserve_mult) = if big_pb { (2048, 0, 1024, 0) } else { (448, 64, 256, 4) };
        let max = if big_pb { cur + pool } else { 4096 };
        let global = (cur + ready + grow * reserve_mult, max * 4);
        Cfg { grow_pool_pages: pool, lists: [global, global, (26, 32 * 4)] }
    }
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
    /// GENERAL at each refusal of `refused`, taken before the request.
    refused_at: Vec<Shot>,
    /// Each scene's live high-water, KiB: (scene, live), in the order the
    /// scenes were first seen; `LIFE` is the boot and the blocks between
    /// scenes.
    live_hw: Vec<(u32, u32)>,
    /// Every served `a` block: (scene, what, device address).
    placed: Vec<(u32, &'static str, u64)>,
}

impl Replay {
    fn new(top_above: u32) -> Replay {
        Replay::with_carveout(CARVEOUT, top_above)
    }

    /// A carve-out of `bytes` with the AM62P's render-pool offset: the pool
    /// is `bytes` less `POOL_AM62P` (59200 KiB at 64 MiB, 255808 at 256).
    fn with_carveout(bytes: usize, top_above: u32) -> Replay {
        Replay {
            r: Rig::with_layout(bytes, POOL_AM62P, top_above),
            live: Vec::new(),
            pm: 0,
            scene: LIFE,
            refused: Vec::new(),
            refused_at: Vec::new(),
            live_hw: Vec::new(),
            placed: Vec::new(),
        }
    }

    fn shot(&mut self) -> Shot {
        let f = self.r.free_of(GEN);
        Shot { largest: f.largest / K, free: (f.bytes / 1024) as u32, gap: (self.r.k.pool_gap() / 1024) as u32 }
    }

    /// The bytes the pool has handed out (bump plus top region) less the
    /// bytes on every heap's free list, KiB: the board's "live"
    /// (`osal_mem_line`, `yd_pvr_pool_usage`). Frees the board holds for a
    /// fence are live there too; here a scene's frees are given back at its
    /// teardown, so a scene's high-water is the same in both.
    fn live(&mut self) -> u32 {
        let out = u64::from(self.r.pool_used()) + u64::from(self.r.k.pool_top);
        let free: u64 = (0..POOL_HEAPS).map(|h| self.r.free_of(h).bytes).sum();
        ((out - free) / 1024) as u32
    }

    /// Record the live bytes against the current scene's high-water.
    fn note_live(&mut self) {
        let live = self.live();
        let scene = self.scene;
        match self.live_hw.iter_mut().find(|(s, _)| *s == scene) {
            Some((_, hw)) => *hw = (*hw).max(live),
            None => self.live_hw.push((scene, live)),
        }
    }

    /// A refusal: the request's scene, name and KiB, with GENERAL's state.
    fn refuse(&mut self, what: &'static str, bytes: u32) {
        let s = self.shot();
        self.refused.push((self.scene, what, bytes.div_ceil(K)));
        self.refused_at.push(s);
    }

    /// A pool allocation (`pvr_render_alloc`) for `scene`; false when
    /// refused (and recorded).
    fn a(&mut self, scene: u32, what: &'static str, heap: usize, bytes: u32) -> bool {
        match self.r.try_alloc(heap, bytes) {
            Some(bo) => {
                self.placed.push((scene, what, bo.gpu));
                self.live.push(Blk { scene, what, heap, size: bytes, bo });
                self.note_live();
                true
            }
            None => {
                self.refuse(what, bytes);
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
            self.refuse("a driver block", bytes);
        } else {
            self.note_live();
        }
    }

    /// [`Replay::drv`], false when refused.
    fn drv_ok(&mut self, heap: usize, bytes: u32) -> bool {
        let n = self.refused.len();
        self.drv(heap, bytes);
        self.refused.len() == n
    }

    /// A PM-protected block (a PFN array, an MList) from the bump.
    fn pm(&mut self, bytes: u32) {
        let mut pm = self.pm;
        let bo = bo_alloc_pm(&mut self.r.k, PMMETA_PROTECT_HEAP_BASE, &mut pm, bytes);
        self.pm = pm;
        if bo.cpu.is_null() {
            self.refuse("a PM block", bytes);
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
    /// context, the backend's frame sets and statics. The freelists are the
    /// 64 MiB board's.
    fn boot(&mut self) {
        self.boot_for(&Cfg::for_carveout(CARVEOUT));
    }

    /// [`Replay::boot`] with the freelists of `cfg`.
    fn boot_for(&mut self, cfg: &Cfg) {
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
        /* the freelists' shared grow pool (freelist::pool_init; none below
         * 128 MiB), then the lists: their backing pages and PFN arrays */
        if cfg.grow_pool_pages != 0 {
            pool_align(&mut self.r.k, 0x4000);
            self.drv(GEN, cfg.grow_pool_pages * 4 * K);
        }
        for (pages, arr_bytes) in cfg.lists {
            pool_align(&mut self.r.k, 0x4000);
            self.drv(GEN, pages * 4 * K);
            self.pm(arr_bytes);
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
            }
        }
    }

    /// Scene `n`'s texture `what` grows from its one-level storage to a
    /// mip chain of `chain` bytes, as the GL core does on the first
    /// `glGenerateMipmap` of an NPOT texture (`src/gl/texture.rs`
    /// `grow_npot_chain`, `texture_alloc_laid_out`): the old storage is
    /// released first, then the chain is asked for. False when the chain is
    /// refused: the texture then has no storage at all.
    fn grow(&mut self, n: u32, what: &'static str, chain: (&'static str, u32)) -> bool {
        /* a texture whose level 0 was itself refused has no storage to give back */
        if let Some(i) = self.live.iter().position(|b| b.scene == n && b.what == what) {
            let old = self.live.remove(i);
            self.r.free(&old.bo, old.size);
        }
        self.a(n, chain.0, GEN, chain.1)
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
/// buffer, whether that buffer was served, and every refusal.
struct Glmark2 {
    before_refract: Shot,
    at_depth_buffer: Shot,
    depth_buffer_served: bool,
    refused: Vec<(u32, &'static str, u32)>,
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

/// glmark2's entries 0-24 (to refract's first draw), with `unnamed` 256 KiB
/// blocks kept for the rest of the run from shadow's frames on.
fn glmark2(top_above: u32, unnamed: u32) -> Glmark2 {
    let mut rp = Replay::new(top_above);
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
    rp.scene(22, TERRAIN);
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
    rp.teardown();
    Glmark2 { before_refract, at_depth_buffer, depth_buffer_served, refused: rp.refused }
}

/// Refract's three large blocks (depth and colour textures, the depth
/// buffer), KiB.
const REFRACT_NEEDS: u32 = (2 * TEX_1600 + DS_1600) / 1024;
const DS_KIB: u32 = DS_1600 / 1024;

fn say(arm: &str, unnamed: u32, g: &Glmark2) {
    eprintln!(
        "glmark2 replay, {arm}, {unnamed} unnamed: before refract {:?} (contiguous {} KiB, refract needs {} KiB); at its depth buffer {:?}, served {}; refused {:?}",
        g.before_refract,
        g.before_refract.contiguous(),
        REFRACT_NEEDS,
        g.at_depth_buffer,
        g.depth_buffer_served,
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
