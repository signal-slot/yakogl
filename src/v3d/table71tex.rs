// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The texture half of the V3D 7.1 table (`src/v3d/table71.rs`; P10 of
//! docs/notes/pre-study-rpi5.md, docs/notes/v3d71-gl-table.md section 3):
//! the tiled copies the TMU samples, and `glGenerateMipmap` through the
//! 7.1 TFU.
//!
//! # Tiled copies
//!
//! The GL core keeps every texture raster in its own storage
//! (`GlesTextureBind::base_gpu`, its CPU view `cpu`, the levels at
//! `texture::gl_level_layered`'s offsets). The 7.1 TMU samples the layout
//! Mesa's `v3d_setup_slices` gives (`texture71::layout`: level 0 last and on
//! 4 KiB, UIF, UB-linear or LT per level), so a sampled texture is copied
//! into memory of the table's own, tiled level by level with Mesa's
//! `v3d_store_tiled_image` (broadcom/common/v3d_tiling.c, compiled by
//! driver/src/v3d/v3d_tiling_mesa.c), as the 4.2 table does (`Tiled`). A
//! copy is kept across draws and jobs under its key: the storage, size,
//! levels, layers, type and texel class, the texels' write stamp
//! (`content_gen`) and the render-into stamp (`target_kick`). An unstamped
//! binding (`content_gen` 0, an EGLImage) is copied for the one draw.
//!
//! At most [`MAX_COPIES`] copies are live. When a draw needs one more, the
//! least recently used live copy that this draw has not bound already is
//! **marked dead, not freed**: a draw queued earlier, or the job in flight,
//! may still sample it. Dead copies are freed by [`Textures::free_dead`],
//! which the table calls only when its queue is empty and no job is in
//! flight (the start of a new queue, `glFinish`, `glGenerateMipmap`). So a
//! copy is never freed while a draw that names its address can still run.
//!
//! # Mip chains (P10, `IOC`)
//!
//! `glGenerateMipmap` on a 2D RGBA8 texture (the GL core's offer,
//! `YakoglBackend::generate_mipmap`) makes a copy of level 0 with room for
//! the chain and has the TFU write levels 1.. into it: `mip71::plan`'s
//! `V3dTfu71Args`, packed by `v3d_tfu71_pack` (driver/src/v3d/v3d_tfu71.c,
//! the `IOC` word Mesa's 7.1 `gl_mipgen` job has) and run by the OSAL's
//! `osal_v3d71_tfu` (src/osal/zephyr_v3d71.h), which waits for it. The
//! answer is 1 (`MipChain::Whole`): the chain is the copy's until the GL
//! core asks for it back (`return_mipmap_chain`), and a copy holding it
//! (`chain`) passes it to the next copy of the same storage (a render into
//! level 0 moves `target_kick`, GL keeps levels 1..). A chain copy that has
//! to go is written back into the storage first. A refusal or a failed job
//! answers 0: the GL core filters on the CPU.
//!
//! # The incomplete texture
//!
//! A binding the GL core marks incomplete samples (0, 0, 0, 1) (GL ES 2.0
//! 3.7.10): the table binds a 1x1 black stand-in of its own (a cube map's
//! six faces for a cube binding), copied once and kept.
//!
//! `host-test` only, as the table's recording path (src/v3d/table71.rs).

use core::ffi::{c_int, c_void};

use crate::backend::{gles_texfmt_bytes, GlesTextureBind, GLES_TEXTYPE_CUBE};
use crate::osal::{self, OsalGpuMem, OSAL_MEM_DATA};
use crate::v3d::ffi71::{self, V3dTfu71Job};
use crate::v3d::fill71::Refusal;
use crate::v3d::texture::{gl_level_layered, layers as gl_layers};
use crate::v3d::texture71::{self, Layout};
use crate::v3d::{mip71, texfill71, texglue71};

/// Live copies kept at once (the 4.2 table keeps its tiled copies by a
/// byte budget; 64 covers glmark2's scenes and gl_test's texture cases).
pub const MAX_COPIES: usize = 64;
/// How long one TFU job may take (the OSAL's own bound is 4 s).
pub const TFU_TIMEOUT_US: u32 = 1_000_000;

/// This module's refusals (module `table71tex`).
pub const TABLE71TEX_REFUSALS: [&str; 6] = [REF_NO_CPU, REF_MEMORY, REF_ADDRESS, REF_CPP, REF_DEPTH_ID, REF_STORAGE];

pub const REF_NO_CPU: &str = "V3D 7.1: a sampled texture has no CPU view of its texels to copy";
pub const REF_MEMORY: &str = "V3D 7.1: out of GPU memory for a texture's tiled copy";
pub const REF_ADDRESS: &str =
    "V3D 7.1: a texture's tiled copy is not 4 KiB aligned inside the V3D's 32-bit address space";
pub const REF_CPP: &str = "V3D 7.1: a texture's storage texel size is not the TMU format's";
pub const REF_DEPTH_ID: &str = "V3D 7.1: a depth texture sampled from a rendered depth buffer";
pub const REF_STORAGE: &str = "V3D 7.1: a texture's storage rows are not at the pitch the copy reads them at";

fn refuse(reason: &str) -> Refusal {
    Refusal { module: "table71tex", reason: reason.into() }
}

fn glue_refusal(r: texglue71::Refusal) -> Refusal {
    Refusal { module: "texglue71", reason: r.to_string() }
}

/// `struct pipe_box` (Mesa util/box.h).
#[repr(C)]
struct PipeBox {
    x: i32,
    width: i32,
    y: i32,
    height: i32,
    z: i16,
    depth: i16,
}

/// `struct osal_v3d71_tfu_status` (src/osal/zephyr_v3d71.h).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct OsalV3d71TfuStatus {
    pub cs_before: u32,
    pub cs_after: u32,
    pub hub_int_sts: u32,
    pub su: u32,
    pub us: u32,
}

extern "C" {
    /// Mesa's v3d_tiling.h: tile the `box` of raster rows at `src` into the
    /// level at `dst` (`tiling` is `enum v3d_tiling_mode`, `image_h` the
    /// level's padded height).
    fn v3d_store_tiled_image(
        dst: *mut c_void,
        dst_stride: u32,
        src: *mut c_void,
        src_stride: u32,
        tiling: c_int,
        cpp: c_int,
        image_h: u32,
        b: *const PipeBox,
    );
    /// The same box back out into raster rows.
    fn v3d_load_tiled_image(
        dst: *mut c_void,
        dst_stride: u32,
        src: *mut c_void,
        src_stride: u32,
        tiling: c_int,
        cpp: c_int,
        image_h: u32,
        b: *const PipeBox,
    );
    /// src/osal/zephyr_v3d71.h: run one 7.1 TFU job and wait for it.
    fn osal_v3d71_tfu(j: *const V3dTfu71Job, timeout_us: u32, st: *mut OsalV3d71TfuStatus) -> i32;
}

/// What identifies the bytes of a copy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Key {
    base_gpu: u64,
    width: u32,
    height: u32,
    levels: u32,
    layers: u32,
    type_: u8,
    format: u8,
    content_gen: u32,
    target_kick: u32,
    /// the table's black stand-in (`base_gpu` 0)
    stand_in: bool,
}

impl Key {
    /// The storage a chain belongs to: the key without the stamps.
    fn same_storage(&self, o: &Key) -> bool {
        !self.stand_in
            && !o.stand_in
            && (self.base_gpu, self.width, self.height, self.levels, self.layers, self.type_, self.format)
                == (o.base_gpu, o.width, o.height, o.levels, o.layers, o.type_, o.format)
    }
}

struct Copy71 {
    key: Key,
    mem: OsalGpuMem,
    gpu: u32,
    layout: Layout,
    /// The GL core's storage it was copied from (its CPU view), for a chain
    /// written back.
    src: *mut u8,
    last_use: u32,
    /// Levels 1.. are the TFU's chain, not the storage's (module docs).
    chain: bool,
    /// Replaced or evicted; freed by [`Textures::free_dead`].
    dead: bool,
}

/// One sampled unit, bound: the copy it samples and the two records' values.
#[derive(Clone, Debug)]
pub struct Bound71 {
    pub copy: usize,
    /// The copy's GPU address (the record's buffer).
    pub gpu: u32,
    pub filled: texfill71::Filled71,
}

/// What a copy is, for a test (`table71::texture_copies`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CopyInfo {
    pub gpu: u32,
    pub bytes: usize,
    pub base_gpu: u64,
    pub chain: bool,
    pub dead: bool,
}

/// The table's copies.
pub struct Textures {
    copies: Vec<Copy71>,
    clock: u32,
    /// TFU jobs run and failed since init.
    pub tfu_jobs: u32,
    pub tfu_failed: u32,
}

/// The 1x1 black (0, 0, 0, 1) RGBA8 texels of the stand-in, six faces.
static BLACK: [u8; 24] = [0, 0, 0, 255, 0, 0, 0, 255, 0, 0, 0, 255, 0, 0, 0, 255, 0, 0, 0, 255, 0, 0, 0, 255];

fn gpu32(m: &OsalGpuMem) -> Option<u32> {
    u32::try_from(m.gpu).ok().filter(|&a| a != 0 && a % 4096 == 0)
}

/// The levels a binding's copy holds: the chain when the min filter uses
/// mipmaps, else up to the base level.
fn levels_of(b: &GlesTextureBind) -> u32 {
    let stored = u32::from(b.mip_levels).max(1);
    if b.mip_filter != 0 {
        stored
    } else {
        (u32::from(b.base_level) + 1).min(stored)
    }
}

/// Tile (`store`) or untile one level of one layer between the GL core's
/// storage `cpu` and the copy at `image`.
///
/// # Safety
/// `cpu` holds the GL core's levels of `key` (`gl_level_layered`), `image`
/// the copy's `layout.size` bytes.
unsafe fn move_level(key: &Key, layout: &Layout, image: *mut u8, cpu: *mut u8, level: u32, layer: u32, store: bool) {
    let cpp = layout.cpp;
    let (off, pitch, rows) = gl_level_layered(key.width, key.height, key.layers, cpp, level);
    let s = layout.slices[level as usize];
    let Some(dst_off) = layout.offset(level, layer) else { return };
    let (lw, lh) = ((key.width >> level).max(1), (key.height >> level).max(1));
    let b = PipeBox { x: 0, width: lw as i32, y: 0, height: lh as i32, z: 0, depth: 1 };
    let raster = off as usize + (layer * rows * pitch * cpp) as usize;
    // SAFETY: the caller's contract: the level's rows and the slice lie in
    // the two buffers; Mesa's tiler reads one side and writes the other.
    unsafe {
        let (tiled, flat) = (image.add(dst_off as usize) as *mut c_void, cpu.add(raster) as *mut c_void);
        if store {
            v3d_store_tiled_image(
                tiled,
                s.stride,
                flat,
                pitch * cpp,
                s.tiling as c_int,
                cpp as c_int,
                s.padded_height,
                &b,
            );
        } else {
            v3d_load_tiled_image(
                flat,
                pitch * cpp,
                tiled,
                s.stride,
                s.tiling as c_int,
                cpp as c_int,
                s.padded_height,
                &b,
            );
        }
    }
}

impl Textures {
    pub const NEW: Textures = Textures { copies: Vec::new(), clock: 0, tfu_jobs: 0, tfu_failed: 0 };

    pub fn info(&self) -> Vec<CopyInfo> {
        self.copies
            .iter()
            .map(|c| CopyInfo { gpu: c.gpu, bytes: c.mem.size, base_gpu: c.key.base_gpu, chain: c.chain, dead: c.dead })
            .collect()
    }

    fn live(&self) -> usize {
        self.copies.iter().filter(|c| !c.dead).count()
    }

    /// Free every dead copy. Only when nothing queued or in flight can name
    /// one (the module docs): the caller's contract.
    pub fn free_dead(&mut self) {
        for c in self.copies.iter_mut().filter(|c| c.dead) {
            osal::gpu_free(&mut c.mem);
        }
        self.copies.retain(|c| !c.dead);
    }

    /// Free everything (shutdown: nothing runs any more).
    pub fn free_all(&mut self) {
        for c in &mut self.copies {
            osal::gpu_free(&mut c.mem);
        }
        self.copies.clear();
    }

    /// Write a chain copy's levels 1.. back into the GL core's storage and
    /// drop the flag.
    fn chain_back(c: &mut Copy71) {
        if !c.chain || c.src.is_null() {
            c.chain = false;
            return;
        }
        // SAFETY: the copy's own memory; the TFU job that wrote it has
        // completed (osal_v3d71_tfu waits).
        unsafe { osal::cache_invd(c.mem.cpu, c.mem.size) };
        for level in 1..c.key.levels {
            // SAFETY: `src` is the storage the copy was made from, still
            // allocated (`released` drops the flag before it goes), holding
            // every level of the key.
            unsafe { move_level(&c.key, &c.layout, c.mem.cpu as *mut u8, c.src, level, 0, false) };
        }
        c.chain = false;
    }

    /// Make room for one more live copy: the least recently used live copy
    /// not in `pinned` (the copies this draw bound already), a copy without
    /// a GPU chain first, is marked dead. Never freed here (module docs).
    fn evict(&mut self, pinned: &[usize]) {
        while self.live() >= MAX_COPIES {
            let pick = |chain: bool| {
                self.copies
                    .iter()
                    .enumerate()
                    .filter(|(i, c)| !c.dead && c.chain == chain && !pinned.contains(i))
                    .min_by_key(|(_, c)| c.last_use)
                    .map(|(i, _)| i)
            };
            let Some(i) = pick(false).or_else(|| pick(true)) else { return };
            let c = &mut self.copies[i];
            Self::chain_back(c);
            c.dead = true;
        }
    }

    /// The live copy of `key`, or a new one tiled from `cpu`.
    fn copy(&mut self, key: Key, layout: &Layout, cpu: *mut u8, pinned: &[usize]) -> Result<usize, Refusal> {
        self.clock = self.clock.wrapping_add(1);
        if key.content_gen != 0 || key.stand_in {
            if let Some(i) = self.copies.iter().position(|c| !c.dead && c.key == key) {
                self.copies[i].last_use = self.clock;
                return Ok(i);
            }
        }
        self.evict(pinned);
        let mut mem = OsalGpuMem::ZERO;
        if osal::gpu_alloc(0, layout.size as usize, OSAL_MEM_DATA, &mut mem) != 0 || mem.cpu.is_null() {
            if !mem.impl_.is_null() || mem.gpu != 0 {
                osal::gpu_free(&mut mem);
            }
            return Err(refuse(REF_MEMORY));
        }
        let Some(gpu) = gpu32(&mem) else {
            osal::gpu_free(&mut mem);
            return Err(refuse(REF_ADDRESS));
        };
        let image = mem.cpu as *mut u8;
        let layers = if key.type_ == GLES_TEXTYPE_CUBE { 6 } else { 1 };
        /* a GPU chain of the same storage: its levels 1.. are GL's, not the
         * storage's (module docs) */
        let chain_from = self.copies.iter().position(|c| !c.dead && c.chain && c.key.same_storage(&key));
        for layer in 0..layers {
            for level in 0..key.levels {
                if level > 0 && chain_from.is_some() {
                    continue;
                }
                // SAFETY: `cpu` holds the key's levels (checked by the
                // caller), `image` is the layout's size.
                unsafe { move_level(&key, layout, image, cpu, level, layer, true) };
            }
        }
        let mut chain = false;
        if let Some(j) = chain_from {
            let old = &mut self.copies[j];
            let below = layout.slices[0].offset as usize;
            // SAFETY: both copies have this layout; levels 1.. are the bytes
            // below level 0 (`texture71::layout`), one layer.
            unsafe { core::ptr::copy_nonoverlapping(old.mem.cpu as *const u8, image, below) };
            old.chain = false;
            old.dead = true;
            chain = true;
        }
        // SAFETY: the copy's own bytes, written above.
        unsafe { osal::cache_flush(mem.cpu, layout.size as usize) };
        let src = if key.stand_in { core::ptr::null_mut() } else { cpu };
        let c = Copy71 { key, mem, gpu, layout: layout.clone(), src, last_use: self.clock, chain, dead: false };
        let i = self.copies.len();
        self.copies.push(c);
        if key.content_gen == 0 && !key.stand_in {
            /* unstamped: for this draw only (freed once it has run) */
            self.copies[i].dead = true;
        }
        Ok(i)
    }

    /// Bind `b` on its unit: the copy and the two records' values, or a
    /// named refusal. `pinned` are the copies this draw bound already;
    /// `p1` the program's TMU Config Parameter 1 data for the unit, when it
    /// reads one (its return size is checked).
    pub fn bind(
        &mut self,
        b: &GlesTextureBind,
        pinned: &[usize],
        p1: Option<u32>,
        ver: u32,
        iprev: u32,
    ) -> Result<Bound71, Refusal> {
        if b.depth_id != 0 {
            return Err(refuse(REF_DEPTH_ID));
        }
        let smp = texglue71::GlSampler71 {
            wrap_s: b.wrap_s,
            wrap_t: b.wrap_t,
            wrap_r: b.wrap_r,
            compare_op: b.compare_op,
            ..texglue71::GlSampler71::filters(b.linear_filter != 0, b.min_linear != 0, b.mip_filter)
        };
        let stand_in = b.incomplete != 0;
        let cube = b.type_ == GLES_TEXTYPE_CUBE;
        let mut t = if stand_in {
            let mut t = if cube {
                texglue71::GlTexBind71::cube(texglue71::GLES_TEXFMT_RGBA8, 1, 1, 0)
            } else {
                texglue71::GlTexBind71::tex2d(texglue71::GLES_TEXFMT_RGBA8, 1, 1, 1, 0)
            };
            t.unit = u32::from(b.unit);
            t
        } else {
            texglue71::GlTexBind71 {
                unit: u32::from(b.unit),
                tex_type: b.type_,
                texfmt: b.format,
                layers: gl_layers(b),
                width: b.width,
                height: b.height,
                levels: levels_of(b),
                base_level: u32::from(b.base_level),
                max_level: texture71::GL_MAX_LEVEL_DEFAULT,
                swizzle: b.swizzle,
                no_alpha: b.no_alpha != 0,
                bgra: b.bgra != 0,
                depth_luminance: b.depth_luminance != 0,
                srgb_decode: true,
                incomplete: false,
                bo_address: 0,
            }
        };
        /* the layout, and every refusal, before anything is copied */
        let probe = texglue71::glue(&t, &smp, ver, iprev).map_err(glue_refusal)?;
        if let Some(p1) = p1 {
            probe.check_return_size(p1).map_err(glue_refusal)?;
        }
        let layout = probe.composed.layout.clone();
        let (key, cpu) = if stand_in {
            let key = Key {
                base_gpu: 0,
                width: 1,
                height: 1,
                levels: 1,
                layers: t.layers,
                type_: t.tex_type,
                format: t.texfmt,
                content_gen: 0,
                target_kick: 0,
                stand_in: true,
            };
            (key, BLACK.as_ptr() as *mut u8)
        } else {
            if b.cpu.is_null() {
                return Err(refuse(REF_NO_CPU));
            }
            if layout.cpp != gles_texfmt_bytes(b.format) {
                return Err(refuse(REF_CPP));
            }
            let key = Key {
                base_gpu: b.base_gpu,
                width: b.width,
                height: b.height,
                levels: t.levels,
                layers: t.layers,
                type_: b.type_,
                format: b.format,
                content_gen: b.content_gen,
                target_kick: b.target_kick,
                stand_in: false,
            };
            /* the levels are read at `gl_level_layered`'s offsets: level 0
             * must be the pitch the GL core gives it */
            let (_, pitch0, _) = gl_level_layered(b.width, b.height, t.layers, layout.cpp, 0);
            if b.stride_texels != 0 && b.stride_texels != pitch0 {
                return Err(refuse(REF_STORAGE));
            }
            (key, b.cpu as *mut u8)
        };
        let copy = self.copy(key, &layout, cpu, pinned)?;
        let gpu = self.copies[copy].gpu;
        t.bo_address = gpu;
        let glued = texglue71::glue(&t, &smp, ver, iprev).map_err(glue_refusal)?;
        let filled = texfill71::fill(&glued.tex, &glued.sampler)
            .map_err(|r| Refusal { module: "texfill71", reason: r.to_string() })?;
        Ok(Bound71 { copy, gpu, filled })
    }

    /// Texel storage `[base, base + bytes)` is released: its copies are
    /// dead, and a chain they hold goes with the storage.
    pub fn released(&mut self, base: u64, bytes: u64) {
        let end = base.saturating_add(bytes.max(1));
        for c in self.copies.iter_mut().filter(|c| !c.key.stand_in && c.key.base_gpu >= base && c.key.base_gpu < end) {
            c.chain = false;
            c.src = core::ptr::null_mut();
            c.dead = true;
        }
    }

    /// `glGenerateMipmap` of the 2D texture `b` with a chain of `levels`
    /// levels: 1 when the TFU made it in a copy, 0 when the GL core filters
    /// (logged by name). The caller has emptied its queue and waited for
    /// the job in flight.
    pub fn generate(&mut self, b: &GlesTextureBind, levels: u32, ver: u32, iprev: u32) -> i32 {
        match self.try_generate(b, levels, ver, iprev) {
            Ok(answer) => answer,
            Err(why) => {
                glog!("yakogl/v3d71: glGenerateMipmap left to the CPU: {}", why);
                0
            }
        }
    }

    fn try_generate(&mut self, b: &GlesTextureBind, levels: u32, ver: u32, iprev: u32) -> Result<i32, String> {
        if b.cpu.is_null() {
            return Err(REF_NO_CPU.into());
        }
        let mut t = texglue71::GlTexBind71::tex2d(b.format, b.width, b.height, levels, 0);
        t.base_level = u32::from(b.base_level);
        t.no_alpha = b.no_alpha != 0;
        t.bgra = b.bgra != 0;
        let gl = texglue71::gl_tex(&t, ver, iprev).map_err(|r| r.to_string())?;
        let plan = mip71::plan(&gl).map_err(|r| r.to_string())?;
        if plan.jobs.is_empty() {
            return Ok(0); /* nothing after the base level */
        }
        let layout = texture71::compose(&gl).map_err(|r| r.to_string())?.layout;
        if layout.cpp != gles_texfmt_bytes(b.format) {
            return Err(REF_CPP.into());
        }
        /* every copy of this storage is older than the chain: dead (the
         * caller waited, nothing runs) */
        let key = Key {
            base_gpu: b.base_gpu,
            width: b.width,
            height: b.height,
            levels,
            layers: 1,
            type_: b.type_,
            format: b.format,
            content_gen: b.content_gen,
            target_kick: b.target_kick,
            stand_in: false,
        };
        for c in self.copies.iter_mut().filter(|c| !c.dead && c.key.base_gpu == b.base_gpu) {
            c.chain = false;
            c.dead = true;
        }
        self.free_dead();
        /* level 0 from the storage, levels 1.. by the TFU: a one-level key
         * tiles level 0 alone into the chain's layout */
        let level0 = Key { levels: 1, ..key };
        let i = self.copy(level0, &layout, b.cpu as *mut u8, &[]).map_err(|r| r.reason)?;
        self.copies[i].key = key;
        let gpu = self.copies[i].gpu;
        let plan = mip71::plan(&texture71::GlTex71 { bo_address: gpu, ..gl }).map_err(|r| r.to_string())?;
        for j in &plan.jobs {
            let mut job = V3dTfu71Job::default();
            // SAFETY: plain structs of the sizes the header pins.
            if !unsafe { ffi71::v3d_tfu71_pack(&j.args, &mut job) } {
                self.copies[i].dead = true;
                return Err("v3d_tfu71_pack declined the job".into());
            }
            let mut st = OsalV3d71TfuStatus::default();
            // SAFETY: a packed job over the copy's memory, flushed by `copy`.
            let rc = unsafe { osal_v3d71_tfu(&job, TFU_TIMEOUT_US, &mut st) };
            self.tfu_jobs += 1;
            if rc != 0 {
                self.tfu_failed += 1;
                self.copies[i].dead = true;
                return Err(format!("the TFU job failed ({rc}, TFU_CS {:#x})", st.cs_after));
            }
        }
        self.copies[i].chain = true;
        Ok(1)
    }

    /// The GL core reads or writes levels 1.. of `b`'s storage: a chain the
    /// TFU made goes back into it first.
    pub fn return_chain(&mut self, b: &GlesTextureBind) {
        for c in self.copies.iter_mut().filter(|c| c.chain && c.key.base_gpu == b.base_gpu) {
            Self::chain_back(c);
        }
    }
}
