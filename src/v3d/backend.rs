// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The V3D backend's entry points (`struct yakogl_backend`): jobs, their
//! memory, and the draw.
//!
//! One job is one frame of one render target: `draw` records into the open
//! job through the C job builder (`ffi`, `driver/src/v3d/v3d_job.c`), and the
//! job is ended and submitted when the target changes, on `flush`/`finish`,
//! when a clear arrives after draws, or when a list is full. The submit is
//! waited for before the job's memory is used again, so there is one job in
//! flight at most (the M6 first cut: simple and correct; double buffering is
//! an optimisation for later).
//!
//! Depth and stencil live in the tile buffer only (no buffer in memory yet),
//! so a job cannot see what an earlier job left in them. The backend keeps,
//! per target, what the depth and the stencil are known to hold after the
//! last job ([`ZsKnown`]); a draw that would read a value no job can
//! reproduce is refused, never drawn against a wrong depth
//! (docs/online-shaderc/v3d-backend.md, "M6").

use core::ffi::c_void;

use super::ffi::*;
use super::kick::{V3dKick, V3D_KICK_MAGIC};
use super::state;
use super::texture;
use crate::backend::*;
use crate::ds_retention::{self, Parked};
use crate::osal::{
    self, OsalGpuMem, OsalV3dBinPool, OsalV3dCpuOf, OsalV3dTfu, OsalV3dTfuQueue, OSAL_MEM_DATA,
    OSAL_QUERY_V3D_BIN_POOL, OSAL_QUERY_V3D_CPU_OF_GPU, OSAL_QUERY_V3D_DRAWS_PER_JOB, OSAL_QUERY_V3D_JOB_OPEN,
    OSAL_QUERY_V3D_S8_STENCIL, OSAL_QUERY_V3D_SPLIT_ZS, OSAL_QUERY_V3D_TFU_MIN_BYTES, OSAL_QUERY_V3D_TFU_QUEUE,
    OSAL_QUERY_V3D_TFU_RUN, OSAL_QUERY_V3D_TILED_BUDGET, OSAL_QUERY_V3D_VPM_BYTES,
};
use crate::shaderc::online::v3d::decode::decode as qpu_decode;
use crate::shaderc::online::v3d::isa::Instr as QpuInstr;
use crate::shaderc::online::v3d::select::{QUNIFORM_TEXTURE_HEIGHT, QUNIFORM_TEXTURE_WIDTH};
use crate::shaderc::{ShcShader, ShcV3dVs, SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX, SHC_V3D_NO_RECORD};
use crate::util::{Global, EBUSY, EINVAL, EIO, ENODEV, ENOMEM, ENOTSUP};

/* ---- memory ---------------------------------------------------------------- */

/// This backend draws into a framebuffer with no colour attachment
/// (`backend::DEPTH_ONLY_TARGETS`): `target_gpu` 0 opens a job with target
/// address 0, whose tiles load and store no render target (v3d_job.h
/// `v3d_job_target::address`), as Mesa's v3d does for a job with no colour
/// buffer (v3dx_rcl.c:315-321, nr_cbufs 0). The depth/stencil buffer is
/// kept and stored as for any target.
pub const DRAWS_WITHOUT_COLOUR: bool = true;

/// The draws one job is sized for when the OSAL gives no budget
/// (OSAL_QUERY_V3D_DRAWS_PER_JOB; CONFIG_YAKOGL_V3D_DRAWS_PER_JOB on Zephyr).
pub const DEFAULT_DRAWS_PER_JOB: u32 = 1024;
/// A draw's binning-list and indirect-buffer bytes, 15 % over what Slint's
/// dashboard measured on the board (55,395 and 188,557 bytes for 493 draws:
/// 112 and 382 a draw, the prologues included), and room for the prologues
/// (the BCL's binning setup; the indirect buffer's default attribute values).
pub const BCL_BYTES_PER_DRAW: u32 = 129;
pub const IND_BYTES_PER_DRAW: u32 = 440;
const BCL_FIXED: u32 = 4096;
const IND_FIXED: u32 = 8192;

/// The binning list and the indirect buffer one job of `draws` draws needs,
/// each rounded up to 4 KiB; `None` for a budget past 32 bits.
pub fn arena_sizes(draws: u32) -> Option<(u32, u32)> {
    let size = |fixed: u32, per: u32| draws.checked_mul(per)?.checked_add(fixed)?.checked_next_multiple_of(4096);
    Some((size(BCL_FIXED, BCL_BYTES_PER_DRAW)?, size(IND_FIXED, IND_BYTES_PER_DRAW)?))
}
/// The render control list: about 100 bytes plus 3 per supertile (at most
/// 256 supertiles, `v3d_frame_tiling`).
pub const RCL_BYTES: usize = 4 * 1024;
/// Vertex and index data the GL core staged for one draw only (`cpu`
/// attributes, `index_cpu`), copied here for the job's life.
pub const DATA_BYTES: usize = 1024 * 1024;
/// The texture arena: per job, the tiled images the TMU samples and their
/// texture and sampler state records (docs/online-shaderc/v3d-backend.md,
/// "M6 textures"). A texture is tiled once per job.
pub const TEX_BYTES: usize = 8 * 1024 * 1024;
/// The second arena set's texture arena. Jobs alternate between two arena
/// sets, so the CPU records job N+1 while the GPU runs job N (the async
/// swap). A steady frame of the Slint cluster uses 8,568 bytes of its texture
/// arena (DEMO11, records/pi4/record_DEMO11_cluster_demo11_25697075.txt), and
/// the carve-out has no second 8 MiB to spare (5.3 MiB longest run), so the
/// second set's is small. A tiling that does not fit it asks for the 8 MiB
/// set (`V3dState::want_big_arena`): the draw's retry lands there.
pub const SPARE_TEX_BYTES: usize = 1024 * 1024;
/// Textures a job tiles once and finds again (a draw after draws of the same
/// texture reuses its image and record).
pub const TEX_CACHE: usize = 16;
/// Sampler state records a job packs once and finds again: every draw used
/// to pack its own (`sampler_for`), one of the per-draw costs of the V3D's
/// draw call (the per-unit cost table, 2026-09-26).
pub const SAMPLER_CACHE: usize = 16;
/// Bytes after a tiled image that must stay as written (`canary_set`): a
/// tiling that writes past its image breaks them first.
const CANARY_BYTES: usize = 64;
const CANARY: u8 = 0xa5;

/// # Safety
/// `p` has `n` writable bytes.
unsafe fn canary_set(p: *mut u8, n: usize) {
    // SAFETY: the caller's contract.
    unsafe { core::ptr::write_bytes(p, CANARY, n) };
}

/// The first of `n` canary bytes at `p` that changed.
///
/// # Safety
/// `p` has `n` readable bytes.
unsafe fn canary_first_broken(p: *const u8, n: usize) -> Option<usize> {
    // SAFETY: the caller's contract.
    unsafe { core::slice::from_raw_parts(p, n) }.iter().position(|&c| c != CANARY)
}

/// A per-job arena image's canary checked after tiling; a broken one is
/// counted and logged with the texture's shape.
///
/// # Safety
/// `p` has `n` readable bytes.
unsafe fn canary_check(g: &mut V3dState, b: &GlesTextureBind, image_bytes: usize, p: *const u8, n: usize) {
    // SAFETY: the caller's contract.
    if let Some(k) = unsafe { canary_first_broken(p, n) } {
        g.overruns += 1;
        glog!(
            "yakogl/v3d: a {}x{} texture ({} levels, {} layers) tiled into the arena was written past its {} bytes \
             (from byte {} after it)",
            b.width,
            b.height,
            b.mip_levels,
            b.layers,
            image_bytes,
            k
        );
    }
}

/// Programs a job remembers for the MMU-fault record.
const JOB_PROGS: usize = 8;
/// Vertex arrays a job remembers for the MMU-fault record.
const JOB_ATTRS: usize = 16;
/// Draws' client data a job can hold outside its data arena (`stage`).
const JOB_SPILLS: usize = 8;
/// Uniform streams a job remembers for the MMU-fault record: the first two
/// draws' fragment, vertex and coordinate streams.
const JOB_STREAMS: usize = 6;
/// Words of one stream the MMU-fault record prints at most.
const STREAM_WORDS_SHOWN: u32 = 48;
/// errno for an MMU fault, as the OSAL reports a job that faulted.
const EFAULT: i32 = 14;
/// Tiled copies kept across jobs (docs/online-shaderc/v3d-backend.md, "M6
/// tiled-copy cache"): at most this many, and at most the tiled budget's
/// bytes of them (`V3dState::tiled_budget`: the OSAL's,
/// OSAL_QUERY_V3D_TILED_BUDGET, or this default); the least recently sampled
/// goes first. 256: the cluster's frame needs about 86 (DEMO4, 5dfc7954,
/// 2026-09-25: 64 live, all sampled by the last job, 6.5 MB, the backdrop
/// among the evicted, and 17-20 hot evictions a frame on top), so 64
/// thrashed every frame; the bytes (about 12.3 MB) fit the 16 MiB budget.
/// A slot is about 100 bytes, and the key scans stay short at this size.
pub const TILED_ENTRIES: usize = 256;
pub const DEFAULT_TILED_BUDGET: usize = 8 * 1024 * 1024;
/// Sampler bindings one stage of a draw binds (the fragment stage's is the
/// larger).
pub const MAX_UNITS: usize = GLES_MAX_TEX_BINDS;
/// The GL core's incomplete texture's stand-in: a 1x1 (0, 0, 0, 1) image at 0
/// and its state record at this offset, in memory of its own.
const BLACK_STATE_OFF: u32 = 1024;
/// The same for a samplerCube: a 1x1 cube, six black faces, at this offset
/// of the same memory, its record BLACK_STATE_OFF after it. A cube lookup
/// steps faces by the record's array stride, so a 2D record would send
/// faces 1..5 past its image; Mesa, too, samples a fallback texture of the
/// sampler's own target (main/texobj.c _mesa_get_fallback_texture).
const BLACK_CUBE_OFF: u32 = 4096;
/// The GL sampler types a unit is a cube for (GL ES 2.0 samplerCube; the
/// shadow one from ES 3.0), as the program's uniform table names them.
const GL_SAMPLER_CUBE: u16 = 0x8B60;
const GL_SAMPLER_CUBE_SHADOW: u16 = 0x8DC5;

/// What identifies a tiled image within a job: the GL core's storage and
/// what of it is sampled.
#[derive(Clone, Copy, PartialEq, Debug)]
struct TexKey {
    base_gpu: u64,
    width: u32,
    height: u32,
    levels: u32,
    bgra: u8,
    /// `GLES_TEXFMT_*`: the texel size and the TMU type the copy was tiled
    /// for (storage re-specified in another format also takes a new stamp).
    format: u8,
    content_gen: u32,
    /// 6 for a cube map: the same storage sampled as 2D is another image.
    layers: u32,
    /// A depth texture read from its depth buffer (`GlesTextureBind::depth_id`,
    /// `depth_sample`) is another record than the same texels read from a
    /// copy: never 0 for such a binding, 0 for every kept copy.
    depth_id: u32,
}

impl TexKey {
    const ZERO: TexKey = TexKey {
        base_gpu: 0,
        width: 0,
        height: 0,
        levels: 0,
        bgra: 0,
        format: 0,
        content_gen: 0,
        layers: 0,
        depth_id: 0,
    };

    fn of(b: &GlesTextureBind) -> TexKey {
        TexKey {
            base_gpu: b.base_gpu,
            width: b.width,
            height: b.height,
            levels: texture::levels_sampled(b),
            bgra: b.bgra,
            format: b.format,
            content_gen: b.content_gen,
            layers: texture::layers(b),
            depth_id: b.depth_id,
        }
    }
}

/// A tiled copy kept across jobs: the texels of `key` (a stamped binding,
/// `content_gen` != 0) as the TMU walks them, in memory of its own.
#[derive(Clone, Copy)]
struct Tiled {
    key: TexKey,
    /// The GL core's storage it was tiled from: `[key.base_gpu, + src_bytes)`.
    src_bytes: u64,
    mem: OsalGpuMem,
    /// The image inside `mem`, at a 4 KiB boundary.
    image_gpu: u32,
    bytes: usize,
    /// The kick that last sampled it (LRU).
    last_kick: u32,
}

impl Tiled {
    const NONE: Tiled =
        Tiled { key: TexKey::ZERO, src_bytes: 0, mem: OsalGpuMem::ZERO, image_gpu: 0, bytes: 0, last_kick: 0 };

    fn live(&self) -> bool {
        !self.mem.impl_.is_null()
    }

    fn overlaps(&self, base: u64, bytes: u64) -> bool {
        self.live() && self.key.base_gpu < base.saturating_add(bytes) && base < self.key.base_gpu + self.src_bytes
    }
}

/// RT 0 stores of submitted jobs: into the raster target (the window, or
/// the GL core's storage of a texture) and into a render image, each with
/// its bytes (width x height x bytes a pixel, without the row or UIF
/// padding).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ColourStores {
    pub raster: u32,
    pub raster_bytes: u64,
    pub image: u32,
    pub image_bytes: u64,
}

impl ColourStores {
    const ZERO: ColourStores = ColourStores { raster: 0, raster_bytes: 0, image: 0, image_bytes: 0 };
}

/// Render images kept at once (`RtImage`): the textures an application
/// renders into and then samples. glmark2's desktop blurs 4 windows into 2
/// textures, refract and shadow have 1 each, terrain 7 (its render-to-
/// texture chain); Slint and the cluster render their layers into textures
/// as femtovg's image render targets. The least recently used goes when the
/// table is full; a target without a slot is sampled through a tiled copy,
/// as before.
pub const RT_IMAGES: usize = 8;

/// A render target's second home, its render image (docs/online-shaderc/
/// v3d-backend.md, "Render targets sampled from their render image"): a
/// texture the application renders into (an FBO's colour attachment 0) and
/// samples, laid out as Mesa lays out every tiled resource
/// (`texture::rt_layout`, `levels` levels), so the TMU samples it as it is.
/// Mesa keeps every render target UIF and samples it directly; the GL core
/// here keeps every texture raster, so a job rendering into the texture
/// stores RT 0 twice (v3d_job.h `uif_*`): the raster store into the GL
/// core's storage, which every CPU path (glReadPixels, glTexSubImage2D,
/// glCopyTex*) keeps reading and writing unchanged, and the UIF store into
/// level 0 of this image. A draw that samples the texture then needs no
/// copy: no CPU tiling of the 7.68 MB of a 1600x1200 target per frame, and
/// no second 7.68 MB for the copy while the image stands (RESULTS "Pi 4:
/// the OSAL looks at the job in flight", option (a)).
///
/// When the image holds what the storage holds: a texture is registered
/// here the first time a draw samples it after a render into it
/// (`GlesTextureBind::target_kick` not 0, `rt_sample`), with the texels'
/// stamp of that moment (`seen_gen`) and the levels it is sampled with;
/// every job that renders into the target from then on stores level 0 of
/// the image, and `stored` says a submitted job has. The storage changes
/// under the image only when the CPU writes it, and every CPU write moves
/// `content_gen` (texture.rs `stamp_texels`), so a sample whose stamp is not
/// `seen_gen` takes the copy path, records the new stamp and waits for the
/// next render to store the image again. A sample whose stamp is `seen_gen`
/// with `stored` set reads the image, which the same jobs wrote that wrote
/// the storage. The first sample after a render into a texture is therefore
/// a copy; from the second render on, none.
///
/// The levels below 0 (`chain`): glGenerateMipmap on a registered target is
/// delegated to this backend (`v3d_generate_mipmap`), which has the TFU
/// make levels 1.. of the image from its level 0, as Mesa's
/// v3d_generate_mipmap does, instead of the GL core box-filtering 7.68 MB
/// of non-cacheable storage on the CPU; the GL core's levels 1.. are then
/// stale until it asks for them back (`v3d_return_mipmap_chain`, before any
/// CPU read or write of them), and the image's are GL's chain, kept across
/// renders into level 0 as GL keeps them. A mip-filtered sample reads the
/// image only while `chain` holds.
#[derive(Clone, Copy, Debug)]
struct RtImage {
    /// The target's identity (`Target`): the texture's level 0 at `gpu`,
    /// `w` x `h` RGBA8 texels, rows `pitch` texels apart. 0: a free slot.
    gpu: u64,
    w: u32,
    h: u32,
    pitch: u32,
    /// The levels the image is laid out with: 1, or the chain the texture
    /// is sampled with (`texture::levels_sampled`). A sample that needs more
    /// lays the image out again.
    levels: u32,
    /// The image: `texture::rt_layout`'s bytes plus 4 KiB of slack, `base`
    /// the layout's start at a 4 KiB boundary inside it and `address` level
    /// 0 (at its slice offset; the texture state's base pointer and the TLB
    /// store's address). Not allocated until the first job renders into the
    /// target after the registration (`rt_for_render`): a texture sampled
    /// once after a render costs nothing.
    mem: OsalGpuMem,
    base: u32,
    address: u32,
    /// The TLB store's memory format and "Height in UB" (`zs_layout`).
    memory_format: u32,
    height_in_ub: u32,
    seen_gen: u32,
    stored: bool,
    /// The lazy raster store (docs/notes/v3d-lazy-raster.md): a submitted
    /// job stored level 0 into the image alone (v3d_job.h `uif_only`), so
    /// the GL core's storage of level 0 is behind the image, which is the
    /// live copy. `rt_level0_back` untiles the image into the storage
    /// before any CPU reader (and the copy path) reads it, and clears the
    /// bit; while it is set, a job that loads the target loads the image
    /// (`uif_load`). Every CPU access of the storage goes through
    /// `rt_level0_back` first, so a set bit also says no CPU write is newer
    /// than the image.
    raster_stale: bool,
    /// A CPU reader has needed the storage (`rt_level0_back` untiled level
    /// 0): the target's later jobs store the raster as well, as before the
    /// lazy store, so a reader every frame (glGenerateMipmap's CPU filter
    /// with the TFU off, glReadPixels each frame) costs one untile, not one
    /// a frame. Kept while the slot is registered.
    raster_wanted: bool,
    /// Levels 1.. hold the chain the TFU made from level 0, GL's chain since
    /// (`v3d_generate_mipmap`); false once the GL core took it back, the
    /// storage was written under the image, or the image was made again.
    chain: bool,
    /// The fence the TFU job making the chain runs behind (the OSAL runs it
    /// when it finishes that job, `OSAL_QUERY_V3D_TFU_QUEUE`), until the CPU
    /// has seen it complete; 0 when the job ran within the query or since.
    /// A CPU read of the levels (`rt_chain_back`) waits for it first.
    tfu_fence: i64,
    /// `V3dState::kicks` + 1 when last registered, rendered or sampled: the
    /// LRU order when the table is full.
    used: u32,
}

impl RtImage {
    const NONE: RtImage = RtImage {
        gpu: 0,
        w: 0,
        h: 0,
        pitch: 0,
        levels: 1,
        mem: OsalGpuMem::ZERO,
        base: 0,
        address: 0,
        memory_format: 0,
        height_in_ub: 0,
        seen_gen: 0,
        stored: false,
        raster_stale: false,
        raster_wanted: false,
        chain: false,
        tfu_fence: 0,
        used: 0,
    };

    fn live(&self) -> bool {
        self.gpu != 0
    }

    fn has_image(&self) -> bool {
        !self.mem.impl_.is_null()
    }

    /// The storage bytes the image stands for.
    fn storage_bytes(&self) -> u64 {
        u64::from(self.pitch) * 4 * u64::from(self.h)
    }

    fn overlaps(&self, base: u64, bytes: u64) -> bool {
        self.live() && self.gpu < base.saturating_add(bytes) && base < self.gpu + self.storage_bytes()
    }

    /// The CPU view of the image's layout (`base`).
    fn image_cpu(&self) -> *mut u8 {
        // SAFETY: `base` is inside `mem` (rt_for_render placed it there).
        unsafe { (self.mem.cpu as *mut u8).add((u64::from(self.base) - self.mem.gpu) as usize) }
    }
}

/// The largest target the tile memory is sized for at init.
pub const MAX_TARGET: u32 = 2048;
/// The most tiles one job may bin: what the pools hold, sized at init for
/// `MAX_TARGET` square at four 32-bpp render targets (32x32 tiles). A typed
/// job (an ES 3.0 colour format at 64 or 128 bpp) bins smaller tiles
/// (`v3d_frame_tiling_bpp`); one with more tiles than this is refused by
/// name rather than run past the tile state.
const MAX_JOB_TILES: u32 = (MAX_TARGET / 32) * (MAX_TARGET / 32);

/// Device-VA heap labels (the OSAL maps its carve-out whole; the labels only
/// say which pool a request came from).
pub const V3D_HEAP_DATA: u64 = 0x1000_0000;
pub const V3D_HEAP_SHADER: u64 = 0x2000_0000;

/// How long `flush` waits for a job, in microseconds. The Linux driver's
/// hang timeout is 500 ms (v3d_sched.c); a gl_test job takes milliseconds.
pub const JOB_TIMEOUT_US: i64 = 1_000_000;

/* ---- what depth and stencil hold between jobs ------------------------------ */

/// What a target's depth (or stencil) is known to hold when no job is open.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ZsKnown {
    /// No job has cleared or written it: GL leaves it undefined, so a job
    /// may start it from anything.
    Untouched,
    /// The last job that touched it cleared it to this value and drew
    /// nothing into it: a new job starts from the same clear value.
    Cleared(f32),
    /// A job wrote it. Nothing stores it, so no later job can see it: a
    /// draw that reads it before a clear is refused. Only for a target with
    /// no depth/stencil buffer (`ds_persist` false, or none allocated).
    Written,
    /// A job wrote it and stored it into the target's depth/stencil buffer
    /// (M6 step C): the next job loads it.
    Stored,
}

/// The targets whose depth/stencil history is kept. A target not in the
/// table once it is full is taken as [`ZsKnown::Written`]: an unknown
/// history is never read as an empty one.
pub const ZS_TARGETS: usize = 8;

#[derive(Clone, Copy, Debug)]
struct ZsEntry {
    target: u64,
    z: ZsKnown,
    s: ZsKnown,
}

/// Depth/stencil buffers kept between jobs (M6 step C), one per
/// `GlesDraw::ds_id`, as the PowerVR's `ds_get`: a buffer is re-allocated
/// when its id comes back at another size. A buffer that holds stored
/// contents is never given to another id: with every slot holding some, a
/// draw that needs a new one is refused (`zs_buf`, src/ds_retention.rs).
///
/// [`ds_retention::DS_SLOTS`] (32), so that memory, not the slot count, is
/// what runs out. The cluster
/// (agl-slint-cluster through femtovg) keeps one stencil renderbuffer per
/// layer image and fills concave paths through it, so each holds stencil GL
/// keeps: frame 1 made 13 of them, and with 8 slots the 9th to 13th were
/// refused (records/pi4/record_DEMOG1_cluster_demog1.txt, 21 x
/// GL_OUT_OF_MEMORY). The largest D24S8 the cluster makes is the 1920x720
/// window's, 5,533,696 bytes + 4 KiB alignment; 32 of those (177 MB) would
/// not fit the image's 64 MiB carve-out (the cluster sample's
/// boards/rpi_4b.conf), so the carve-out is the real bound, and a buffer it
/// cannot give is refused by name (`ds_retention::refuse_no_memory`), never
/// kept on-chip.
pub const ZS_BUFS: usize = ds_retention::DS_SLOTS;
/// Buffer requests (`V3dState::zs_clock`, which `zs_buf` and `ms_buf`
/// advance) a refused depth/stencil allocation is not asked for again; they are refused meanwhile. Counted in
/// requests, not jobs or swaps: a conformance run may never swap, and with
/// every draw on a target refused no job runs, so a count of jobs would
/// never let the ask come again after memory is freed. Why not ask every
/// time:
/// DEMO12 asked the OSAL for the second window's 5.5 MB D24S8 at every job,
/// 423 times in 47 s, until a run freed by the tiled copies' churn let it
/// take the carve-out's last 5.5 MB and a 21 KB texture failed next
/// (records/pi4/record_DEMO12_cluster_demo12_1f68ba0a.txt).
pub const ZS_RETRY_JOBS: u32 = 64;

/// One depth/stencil buffer: D24S8, laid out as Mesa lays out a zsbuf
/// (`v3d_tex_layout`, one level, 4 bytes a pixel), or S8 (`s8`: 1 byte a
/// pixel, Mesa's separate stencil, see `V3dState::s8_stencil`), with the UIF XOR of
/// Mesa's page-cache optimisation dropped (docs/online-shaderc/v3d-backend.md,
/// "M6 step C": NO_XOR is a valid layout, and only the TLB reads the buffer).
#[derive(Clone, Copy, Debug)]
struct ZsBuf {
    id: u32,
    /// The target's size, and whether the buffer is a 4x one (twice the
    /// width and height, `v3d_tex_layout_msaa`).
    w: u32,
    h: u32,
    msaa: bool,
    /// An S8 buffer: the stencil alone, loaded and stored as buffer STENCIL
    /// with image format S8 (`V3dState::s8_stencil`); else D24S8.
    s8: bool,
    mem: OsalGpuMem,
    /// 4 KiB aligned inside `mem`.
    address: u32,
    memory_format: u32,
    height_in_ub: u32,
    /// What it holds between jobs.
    z: ZsKnown,
    s: ZsKnown,
    /// The colour target of the last job that used it (a swap of that
    /// surface drops what it holds).
    target: u64,
    /// `V3dState::zs_clock` when a job last asked for it: of the buffers
    /// that may be let go, the least recently used one is.
    used: u32,
    /// The GL object of its depth and of its stencil (`GlesDraw::depth_id`
    /// form; 0: none): `attachment_gone` of one makes that half Untouched,
    /// and a buffer left holding nothing is freed.
    owners: [u32; 2],
}

impl ZsBuf {
    const NONE: ZsBuf = ZsBuf {
        target: 0,
        id: 0,
        w: 0,
        h: 0,
        msaa: false,
        s8: false,
        mem: OsalGpuMem::ZERO,
        address: 0,
        memory_format: 0,
        height_in_ub: 0,
        z: ZsKnown::Untouched,
        s: ZsKnown::Untouched,
        used: 0,
        owners: [0; 2],
    };

    fn live(&self) -> bool {
        !self.mem.impl_.is_null()
    }

    /// Holds stored depth or stencil: contents GL keeps, which are nowhere
    /// else. Such a buffer is never let go for another id.
    fn holds_contents(&self) -> bool {
        self.live() && (self.z == ZsKnown::Stored || self.s == ZsKnown::Stored)
    }

    /// Holds a history without contents (a clear value, or "written, not
    /// stored"), which must be parked to let it go.
    fn holds_history(&self) -> bool {
        self.live() && (self.z, self.s) != (ZsKnown::Untouched, ZsKnown::Untouched)
    }
}

/// What one job used of each per-job arena, in bytes, and its draws; or,
/// field by field, the most any job used over a span (`max`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct ArenaUse {
    bcl: u32,
    rcl: u32,
    ind: u32,
    data: u32,
    tex: u32,
    draws: u32,
    /// The binner's tile-memory pool the job used (`bin_pool_used`), and the
    /// pool's size; `None` when it could not be read (no OSAL answer, or a
    /// position outside the pool), which is counted in `bin_unread`, never
    /// taken as 0.
    bin: Option<u32>,
    bin_size: u32,
    bin_unread: u32,
    /// Overflow blocks the binner took after the pool ran out (the most in
    /// one job), and the blocks there are and their size, as the OSAL said.
    bin_overflow: u32,
    bin_overflow_blocks: u32,
    bin_overflow_block: u32,
}

impl ArenaUse {
    const ZERO: ArenaUse = ArenaUse {
        bcl: 0,
        rcl: 0,
        ind: 0,
        data: 0,
        tex: 0,
        draws: 0,
        bin: None,
        bin_size: 0,
        bin_unread: 0,
        bin_overflow: 0,
        bin_overflow_blocks: 0,
        bin_overflow_block: 0,
    };

    fn max(self, o: ArenaUse) -> ArenaUse {
        ArenaUse {
            bcl: self.bcl.max(o.bcl),
            rcl: self.rcl.max(o.rcl),
            ind: self.ind.max(o.ind),
            data: self.data.max(o.data),
            tex: self.tex.max(o.tex),
            draws: self.draws.max(o.draws),
            bin: match (self.bin, o.bin) {
                (Some(a), Some(b)) => Some(a.max(b)),
                (a, b) => a.or(b),
            },
            bin_size: self.bin_size.max(o.bin_size),
            bin_unread: self.bin_unread.max(o.bin_unread),
            bin_overflow: self.bin_overflow.max(o.bin_overflow),
            bin_overflow_blocks: self.bin_overflow_blocks.max(o.bin_overflow_blocks),
            bin_overflow_block: self.bin_overflow_block.max(o.bin_overflow_block),
        }
    }
}

/// The arena line's binner pool: "N of S bytes", "not read", and how many
/// jobs' pools could not be read, so an unread pool never prints as 0; when a
/// job ran the pool out, "ran out" and the overflow blocks it took.
struct BinPool(ArenaUse);

impl core::fmt::Display for BinPool {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let u = &self.0;
        match u.bin {
            _ if u.bin_overflow != 0 => write!(
                f,
                "ran out of {} bytes: {} of {} overflow blocks of {} bytes taken",
                u.bin_size, u.bin_overflow, u.bin_overflow_blocks, u.bin_overflow_block
            )?,
            Some(n) => write!(f, "{} of {} bytes", n, u.bin_size)?,
            None => write!(f, "not read")?,
        }
        if self.0.bin_unread != 0 && self.0.bin.is_some() {
            write!(f, " (a job's pool not read)")?;
        }
        Ok(())
    }
}

/// The binner's tile-memory use after the last job's binning, from the raw
/// registers the OSAL read (`OSAL_QUERY_V3D_BIN_POOL`): the pool's current
/// address (V3D_PTB_0_BPCA) less the pool's start, when it lies inside the pool
/// the job was given; `None` otherwise (no answer, or a position the reading
/// "BPCA is the next free address in the pool" cannot explain). The first
/// answer is printed raw once, so the reading can be checked on the board.
/// A job that took overflow blocks has BPCA in the last block: its pool use
/// is `None` (the line says "ran out" from `bin_overflow` instead).
fn bin_pool_used(g: &mut V3dState) -> ArenaUse {
    let none = ArenaUse { bin_unread: 1, ..ArenaUse::ZERO };
    let Some(query) = osal::ops().and_then(|o| o.query) else { return none };
    let mut p = OsalV3dBinPool::default();
    // SAFETY: `p` is a live struct osal_v3d_bin_pool of the size passed.
    let rc = unsafe {
        query(
            OSAL_QUERY_V3D_BIN_POOL,
            &mut p as *mut OsalV3dBinPool as *mut c_void,
            core::mem::size_of::<OsalV3dBinPool>(),
        )
    };
    if rc != 0 {
        return none;
    }
    if !core::mem::replace(&mut g.bin_raw_logged, true) {
        glog!(
            "yakogl/v3d: binner pool after job {}: given 0x{:08x} + {}, BPCA 0x{:08x} BPCS {} CT0QMA 0x{:08x} CT0QMS {}, \
             overflow blocks {} of {} ({} bytes each)",
            p.jobs,
            p.pool,
            p.pool_size,
            p.bpca,
            p.bpcs,
            p.qma,
            p.qms,
            p.overflow_used,
            p.overflow_blocks,
            p.overflow_block_size
        );
    }
    let inside = p.overflow_used == 0 && p.bpca >= p.pool && p.bpca - p.pool <= p.pool_size;
    let bin = inside.then(|| p.bpca - p.pool);
    ArenaUse {
        bin,
        bin_size: p.pool_size,
        bin_unread: (bin.is_none() && p.overflow_used == 0) as u32,
        bin_overflow: p.overflow_used,
        bin_overflow_blocks: p.overflow_blocks,
        bin_overflow_block: p.overflow_block_size,
        ..ArenaUse::ZERO
    }
}

/// A job's target: the colour buffer, whether its format has no alpha
/// (`GlesDraw::target_alpha_bits` 0: GL_RGB, GL_RGB565, stored as RGBA8 with
/// alpha 1), and the depth/stencil buffer id kept for it (`GlesDraw::ds_id`
/// when `ds_persist`). Two draws with a different one are never in one job.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Target {
    gpu: u64,
    w: u32,
    h: u32,
    /// The row pitch in pixels ([`crate::backend::draw_pitch`]): the raster
    /// load's and store's stride (v3d_job.c, `height_in_ub_or_stride` =
    /// `t.stride`). Part of the job's identity.
    pitch: u32,
    no_alpha: bool,
    /// The target stores B,G,R,X (`GLES_SURF_XRGB8888`): loaded and stored
    /// with R and B swapped.
    bgr: bool,
    /// The target is 16-bit RGB565 (`GLES_SURF_RGB565`): two bytes a pixel,
    /// loaded and stored as bgr565.
    rgb565: bool,
    /// RT 0's surface format (`GLES_SURF_*`). An ES 3.0 colour format
    /// (`state::typed`) makes the job's render targets typed: each loaded
    /// and stored in `state::rt_format`'s image format at its own bytes a
    /// pixel, with its internal type and bpp (v3d_job.h).
    format: u8,
    ds: Option<u32>,
    /// The depth and stencil objects behind `ds` (`GlesDraw::depth_id` and
    /// `stencil_id`): the owners of its buffers.
    ds_owners: [u32; 2],
    /// Render targets the draw's framebuffer has (0 and 1: RT 0 alone), and
    /// the GPU address of RT 1..3 (0: a GL_NONE draw buffer), each at RT 0's
    /// size in its own format (`rt_format`). Part of the job's identity:
    /// another set is another job.
    rts: u8,
    rt: [u32; 3],
    /// RT 1..3's surface formats (`GLES_SURF_*`; RGBA8888 for a GL_NONE
    /// slot and past `rts`).
    rt_format: [u8; 3],
    /// 4x multisampled (the draw's `samples` 4: a 4x EGL surface): part of
    /// the job's identity, one job is either 4x or not.
    msaa: bool,
}

impl Target {
    const NONE: Target = Target {
        gpu: 0,
        w: 0,
        h: 0,
        pitch: 0,
        no_alpha: false,
        bgr: false,
        rgb565: false,
        format: GLES_SURF_RGBA8888,
        ds: None,
        ds_owners: [0; 2],
        rts: 0,
        rt: [0; 3],
        rt_format: [GLES_SURF_RGBA8888; 3],
        msaa: false,
    };

    /// Bytes a pixel of RT 0 (`state::rt_format`: 2 for RGB565, 4 for
    /// RGBA8, the ES 3.0 formats' own).
    fn bpp(&self) -> u32 {
        state::rt_row(self.format).bytes
    }

    /// The surface formats of RT 0..3.
    fn formats(&self) -> [u8; 4] {
        [self.format, self.rt_format[0], self.rt_format[1], self.rt_format[2]]
    }

    /// Is the job typed (`v3d_job_target::typed`): any RT in an ES 3.0
    /// colour format?
    fn typed(&self) -> bool {
        self.formats().iter().any(|&f| state::typed(f))
    }

    /// The largest internal bpp of the configured RTs (`V3D_INTERNAL_BPP_*`).
    fn max_bpp(&self) -> u32 {
        let n = usize::from(self.rts.max(1)).min(4);
        self.formats()[..n].iter().map(|&f| state::rt_row(f).internal_bpp).max().unwrap_or(0)
    }
}

/* ---- the clear program --------------------------------------------------- */

/// The clear quad's vertex shader: the position passes through (the quad's
/// vertices are in clip space, at the clear depth, `state::clear_quad_vertices`).
/// The same shader as the GC7000's `tools/shaders/internal/clear.vert`, in
/// GLSL ES 1.00, the language our compiler takes.
pub const CLEAR_VS: &[u8] = b"attribute vec4 a_pos;\nvoid main() { gl_Position = a_pos; }\n\0";
/// The clear quad's fragment shader: the clear colour, from its one uniform
/// (`tools/shaders/internal/clear.frag`, in GLSL ES 1.00).
pub const CLEAR_FS: &[u8] =
    b"precision mediump float;\nuniform vec4 u_color;\nvoid main() { gl_FragColor = u_color; }\n\0";
/// The integer clear quad's vertex shader: `CLEAR_VS` in GLSL ES 3.00, the
/// language of the fragment shader it links with.
pub const CLEAR_VS_INT: &[u8] = b"#version 300 es\nin vec4 a_pos;\nvoid main() { gl_Position = a_pos; }\n\0";
/// The integer clear quad's fragment shader: the four raw words of
/// glClearBuffer{i,ui}v (`GlesDraw::clear_rgba` holds them as f32 bit
/// patterns) as a `uvec4`, which the compiler writes as four 32-bit TLB words
/// (the TLBU config `tlb_config_32b(0, 4)`, then three TLB writes) that the
/// tile buffer narrows to the target's internal type. One program serves
/// every integer class, signed or not: the 32-bit write carries no
/// signedness (Mesa 26.2.1 writes every `f32_color_rb` target with
/// TLB_TYPE_F32_COLOR, nir_to_vir.c:1963-1977), as Mesa's clear shader
/// stores its one vec4 for every format (st_nir_builtins.c:138-156).
/// docs/notes/v3d-int-clear.md.
pub const CLEAR_FS_INT: &[u8] =
    b"#version 300 es\nprecision highp int;\nuniform uvec4 u_color;\nout uvec4 o;\nvoid main() { o = u_color; }\n\0";

/// Where the clear program stands.
#[derive(Clone, Copy, PartialEq, Debug)]
enum ClearStatus {
    NotBuilt,
    Built,
    /// It did not build; why was logged once, and every quad is refused.
    Failed,
}

/// The clear program, built by the product compiler on the first clear that
/// needs the quad, and kept for the backend's life. `fs`, `code` and
/// `location` mean something only while `status` is `Built`.
#[derive(Clone, Copy)]
struct ClearProgram {
    status: ClearStatus,
    /// The linked program (the fragment shader record).
    fs: ShcShader,
    /// Its code, uploaded where the V3D fetches it.
    code: OsalGpuMem,
    /// The GL location of `a_pos`.
    location: u8,
}

impl ClearProgram {
    const NOT_BUILT: ClearProgram =
        ClearProgram { status: ClearStatus::NotBuilt, fs: ShcShader::ZERO, code: OsalGpuMem::ZERO, location: 0 };
    const FAILED: ClearProgram = ClearProgram { status: ClearStatus::Failed, ..ClearProgram::NOT_BUILT };
}

/* ---- state ---------------------------------------------------------------- */

/// The open job and everything it writes into.
struct Job {
    open: bool,
    job: V3dJob,
    bcl: V3dCl,
    rcl: V3dCl,
    ind: V3dCl,
    target: Target,
    clear: V3dJobClear,
    /// The depth/stencil buffer's slot in `V3dState::zsbufs`, and whether it
    /// held stored depth / stencil when the job began (loaded unless the job
    /// clears them).
    zs: Option<usize>,
    /// With depth and stencil apart (`V3dState::split_zs`): the stencil's
    /// own buffer, `zs` then holding the depth only.
    ss: Option<usize>,
    z_stored: bool,
    s_stored: bool,
    /// The target was swapped (eglSwapBuffers) while this job was open: its
    /// depth/stencil are undefined afterwards, so nothing is stored and the
    /// next job starts Untouched (EGL 1.5 3.10.1).
    zs_discard: bool,
    /// A 4x job: its colour samples' slot in `V3dState::msbufs` (None: not
    /// kept, no memory for them).
    ms: Option<usize>,
    /// The render image this job stores RT 0 into (`V3dState::rt_images`;
    /// None: the target has none): `stored` once the job is submitted.
    rt: Option<usize>,
    /// The largest internal bpp of the job's render targets
    /// (`Target::max_bpp`): with the RT count and 4x, the job's tile size.
    max_bpp: u32,
    /// Whether this job's draws may read depth / stencil: the job cleared
    /// them, or it starts from a value it can reproduce.
    z_valid: bool,
    s_valid: bool,
    /// What the job's draws did to them.
    z_written: bool,
    s_written: bool,
    /// What the job cleared (for the history after it).
    z_cleared: bool,
    s_cleared: bool,
    data_used: usize,
    /// The texture arena's bytes this job used, and the images it holds.
    tex_used: usize,
    tex_keys: [TexKey; TEX_CACHE],
    tex_states: [u32; TEX_CACHE],
    /// For the MMU-fault record (`fault_record`): each cached record's image
    /// address and its bytes as packed, and the programs the job ran (the
    /// code's GPU address, its CPU copy, its words).
    tex_images: [u32; TEX_CACHE],
    tex_packed: [[u8; V3D_TEX_STATE_BYTES]; TEX_CACHE],
    progs: [(u64, usize, u32); JOB_PROGS],
    nprogs: usize,
    /// The vertex arrays the job's draws read: address, stride, and the
    /// bytes staged (0: a buffer's, its size not known here).
    attrs: [(u32, u32, u32); JOB_ATTRS],
    nattrs: usize,
    /// The uniform streams the job's first draws wrote: the stage ('F', 'V',
    /// 'C'), the stream's GPU address, the program's uniform list (contents
    /// and data per word; the GL core keeps it live with the program) and
    /// its length.
    streams: [(u8, u32, usize, u32); JOB_STREAMS],
    nstreams: usize,
    /// Client data of a draw that does not fit even an empty job's data
    /// arena, each piece in GPU memory of its own (`stage`), given back
    /// once the job has run (`free_spills`).
    spills: [OsalGpuMem; JOB_SPILLS],
    nspills: usize,
    ntex: usize,
    /// The job's packed sampler records by their state (`sampler_for`).
    sampler_keys: [V3dSampler; SAMPLER_CACHE],
    sampler_recs: [u32; SAMPLER_CACHE],
    nsamplers: usize,
}

impl Job {
    const ZERO: Job = Job {
        open: false,
        job: V3dJob::ZERO,
        bcl: V3dCl::ZERO,
        rcl: V3dCl::ZERO,
        ind: V3dCl::ZERO,
        target: Target::NONE,
        zs: None,
        ss: None,
        z_stored: false,
        s_stored: false,
        zs_discard: false,
        ms: None,
        rt: None,
        max_bpp: 0,
        clear: V3dJobClear {
            load_rt0: true,
            rgba8: 0,
            z: 1.0,
            stencil: 0,
            zs_address: 0,
            zs_memory_format: 0,
            zs_height_in_ub: 0,
            zs_load: false,
            zs_store: false,
            force_alpha_1: false,
            r_b_swap: false,
            s_address: 0,
            s_memory_format: 0,
            s_height_in_ub: 0,
            s_load: false,
            s_store: false,
            load_rt: [true; 3],
            rgba8_rt: [0; 3],
            s_s8: false,
            clear_rt: [[0; 4]; 4],
        },
        z_valid: false,
        s_valid: false,
        z_written: false,
        s_written: false,
        z_cleared: false,
        s_cleared: false,
        data_used: 0,
        tex_used: 0,
        tex_keys: [TexKey::ZERO; TEX_CACHE],
        tex_states: [0; TEX_CACHE],
        tex_images: [0; TEX_CACHE],
        tex_packed: [[0; V3D_TEX_STATE_BYTES]; TEX_CACHE],
        progs: [(0, 0, 0); JOB_PROGS],
        nprogs: 0,
        attrs: [(0, 0, 0); JOB_ATTRS],
        nattrs: 0,
        streams: [(0, 0, 0, 0); JOB_STREAMS],
        nstreams: 0,
        spills: [OsalGpuMem::ZERO; JOB_SPILLS],
        nspills: 0,
        ntex: 0,
        sampler_keys: [V3dSampler {
            wrap_s: 0,
            wrap_t: 0,
            wrap_r: 0,
            min_nearest: false,
            mag_nearest: false,
            mip_nearest: false,
            mip_none: false,
            min_lod: 0.0,
            max_lod: 0.0,
            lod_bias: 0.0,
            border_color_mode: 0,
        }; SAMPLER_CACHE],
        sampler_recs: [0; SAMPLER_CACHE],
        nsamplers: 0,
    };
}

/// One set of job arenas: the lists, the staged data and the texture arena
/// a job is recorded into, and the last kick recorded into them (0: none).
/// The GPU reads them until that kick has completed.
#[derive(Clone, Copy)]
struct ArenaSet {
    bcl: OsalGpuMem,
    rcl: OsalGpuMem,
    ind: OsalGpuMem,
    data: OsalGpuMem,
    tex: OsalGpuMem,
    tex_bytes: usize,
    kick: u32,
}

impl ArenaSet {
    const ZERO: ArenaSet = ArenaSet {
        bcl: OsalGpuMem::ZERO,
        rcl: OsalGpuMem::ZERO,
        ind: OsalGpuMem::ZERO,
        data: OsalGpuMem::ZERO,
        tex: OsalGpuMem::ZERO,
        tex_bytes: 0,
        kick: 0,
    };

    fn live(&self) -> bool {
        !self.tex.impl_.is_null()
    }
}

/// A job submitted and not yet waited for: its kick and the OSAL's fence.
/// At most two (`V3dState::queued` and `in_flight`): a job is submitted only
/// when at most one is outstanding (`flush_job`). The OSAL runs one job at
/// a time, in submit order, so waiting for the newer one waits for both.
#[derive(Clone, Copy)]
struct InFlight {
    kick: u32,
    fence: i64,
    /// The open job's `open_alias` when it was submitted (0: none): a draw
    /// stamped with the job it ended is in this one, so that stamp stays
    /// pending until this job completes (`v3d_kick_pending`,
    /// `v3d_wait_kick`). Without it the stamp named a completed job as soon
    /// as the job was submitted, and a texture rendered by that draw was
    /// tiled before the render ran (dEQP fbo.render resize.tex2d_*,
    /// FBOR5 1a61e2cef4bed60a).
    alias: u32,
}

/// What `fault_record` reads of a job, kept past the job's end: an MMU fault
/// is only seen when the job is waited for, which is after the next job has
/// begun (the async swap). The job's arenas are still as it left them then:
/// the next job records into the other set.
#[derive(Clone, Copy)]
struct JobRecord {
    kick: u32,
    ntex: usize,
    tex_states: [u32; TEX_CACHE],
    tex_images: [u32; TEX_CACHE],
    tex_packed: [[u8; V3D_TEX_STATE_BYTES]; TEX_CACHE],
    progs: [(u64, usize, u32); JOB_PROGS],
    nprogs: usize,
    attrs: [(u32, u32, u32); JOB_ATTRS],
    nattrs: usize,
    streams: [(u8, u32, usize, u32); JOB_STREAMS],
    nstreams: usize,
}

impl JobRecord {
    const ZERO: JobRecord = JobRecord {
        kick: 0,
        ntex: 0,
        tex_states: [0; TEX_CACHE],
        tex_images: [0; TEX_CACHE],
        tex_packed: [[0; V3D_TEX_STATE_BYTES]; TEX_CACHE],
        progs: [(0, 0, 0); JOB_PROGS],
        nprogs: 0,
        attrs: [(0, 0, 0); JOB_ATTRS],
        nattrs: 0,
        streams: [(0, 0, 0, 0); JOB_STREAMS],
        nstreams: 0,
    };

    fn of(j: &Job, kick: u32) -> JobRecord {
        JobRecord {
            kick,
            ntex: j.ntex,
            tex_states: j.tex_states,
            tex_images: j.tex_images,
            tex_packed: j.tex_packed,
            progs: j.progs,
            nprogs: j.nprogs,
            attrs: j.attrs,
            nattrs: j.nattrs,
            streams: j.streams,
            nstreams: j.nstreams,
        }
    }
}

struct V3dState {
    inited: bool,
    /// The arenas the open job records into (or the next job, when none is
    /// open), and the other set, which the job before holds until it has
    /// run. `spare` is not live when init could not allocate it: every job
    /// then waits for the one before (`choose_arenas`).
    arenas: ArenaSet,
    spare: ArenaSet,
    /// A tiling did not fit the small texture arena: the next job takes the
    /// 8 MiB set (`choose_arenas`).
    want_big_arena: bool,
    /// The newest job submitted and not yet waited for, and the fault record
    /// of the last job submitted.
    in_flight: Option<InFlight>,
    /// The job submitted before `in_flight` and not yet waited for: it runs
    /// while `in_flight` waits behind it in the OSAL (`flush_job` keeps at
    /// most these two).
    queued: Option<InFlight>,
    ended: JobRecord,
    /// The incomplete texture's image and state record, made at init.
    black: OsalGpuMem,
    tile_alloc: OsalGpuMem,
    tile_state: OsalGpuMem,
    tile_alloc_size: u32,
    tile_state_size: u32,
    j: Job,
    clear_program: ClearProgram,
    /// The integer clear program (`CLEAR_FS_INT`), built on first use.
    clear_program_int: ClearProgram,
    zs: [ZsEntry; ZS_TARGETS],
    nzs: usize,
    /// The depth/stencil buffers kept between jobs, the slot given up next
    /// when the table is full, and whether a failed allocation was logged.
    zsbufs: [ZsBuf; ZS_BUFS],
    /// The histories of buffers let go while they held no contents (a clear
    /// value, or "written, not stored"), given back when their id returns
    /// (src/ds_retention.rs).
    zs_parked: Parked<u32, ZsKnown, ZS_BUFS>,
    /// The colour samples of 4x targets, kept between jobs (`ms_buf`).
    msbufs: [MsBuf; MS_BUFS],
    /// The render images of the textures sampled after a render into them
    /// (`RtImage`), and their counters since init (`v3d_rt_images`): draw
    /// bindings sampled from an image (no copy), registrations, samples
    /// sent back to the copy path by a CPU write since the image was
    /// stored, images not allocated for want of memory, mip chains the TFU
    /// made in an image (glGenerateMipmap delegated) and those it failed,
    /// chains written back into the GL core's storage, and the most bytes
    /// the images held at once.
    rt_images: [RtImage; RT_IMAGES],
    rt_direct: u32,
    rt_registered: u32,
    rt_cpu_writes: u32,
    rt_no_memory: u32,
    rt_chains: u32,
    rt_chains_failed: u32,
    rt_chains_returned: u32,
    rt_bytes_peak: usize,
    /// Level 0 of a render image written back into the GL core's storage
    /// for a CPU reader (`rt_level0_back`), since init.
    rt_level0_backs: u32,
    /// `rt_storage_current`, set by `v3d_init`: the GL core's entry
    /// (`v3d_storage_current`) reaches the untile through it, so a GL test
    /// binary on the mock table links that entry without the C it calls.
    storage_current: Option<fn(&mut V3dState, &GlesTextureBind)>,
    /// RT 0 (colour) stores of the frame being drawn and of the last whole
    /// frame (`ColourStores`): logged at shutdown, read per glmark2 scene
    /// (tools/hostharness/glmark2/FINDINGS-v3d-stores.md).
    frame_stores: ColourStores,
    last_frame_stores: ColourStores,
    /// glGenerateMipmap delegated to the TFU (`v3d_generate_mipmap`), since
    /// init: the calls it answered 0 for, by reason (`MipRefusal`), the
    /// chains queued behind a job in flight (no CPU wait), and the calls
    /// that found the OSAL's queue full and waited for the job in flight.
    mip_refused: [u32; MIP_REFUSALS],
    /// Tiled copies and image stores made, by reason (`CopyReason`); the
    /// reason `rt_sample` left for the copy path of the binding being drawn.
    copy_counts: [u32; COPY_REASONS],
    copy_why: CopyReason,
    mip_queued: u32,
    mip_queue_full: u32,
    /// Depth textures sampled from their depth buffer (`depth_sample`,
    /// `v3d_depth_samples`), since init: the draw bindings that read a
    /// buffer (no CPU resolve, no copy), the CPU resolves that wrote texels
    /// (`v3d_depth_resolve`), and the draws refused because the buffer the
    /// GL core was told of was not there to sample.
    depth_direct: u32,
    depth_resolves: u32,
    depth_refused: u32,
    /// Counts `zs_buf` calls: the buffers' use stamps (`ZsBuf::used`).
    zs_clock: u32,
    /// Depth/stencil buffers lent to another id since init (`zs_buf`).
    zs_lent: u32,
    /// The last refused depth/stencil allocation: its bytes and `zs_clock`
    /// when it was refused; none of at least as many bytes is asked for
    /// again within ZS_RETRY_JOBS requests (the draw is refused meanwhile). And
    /// the asks so skipped, since init.
    zs_refused: Option<(usize, u32)>,
    zs_skipped: u32,
    /// The most depth/stencil buffers live at once since init, and the most
    /// bytes they held (`v3d_zs_usage`): the headroom a record shows.
    zs_slots_peak: u32,
    zs_bytes_peak: usize,
    /// Depth and stencil kept apart, each by its own renderbuffer's name
    /// (`v3d_split_zs`; default off until measured on the board): an FBO
    /// whose depth and stencil renderbuffers differ keeps its depth in the
    /// buffer of the depth's name and its stencil in the stencil's.
    split_zs: bool,
    /// Stencil halves in S8 buffers (1 byte a pixel) instead of D24S8 (4):
    /// with `split_zs` on, every stencil half (a stencil-only framebuffer's
    /// stencil, and the stencil of a framebuffer whose depth and stencil
    /// renderbuffers differ) lives under the stencil renderbuffer's own key
    /// and is loaded and stored as buffer STENCIL, format S8, as Mesa does for
    /// the separate stencil of a Z32F_S8X24 zsbuf (v3dx_rcl.c:72-160). Mesa
    /// itself never makes a stencil-only S8 zsbuf (v3d_screen.c:508-514: a
    /// GL_STENCIL_INDEX8 renderbuffer is a D24S8 there), so this shape is
    /// ours. Off without `split_zs` (a stencil-only buffer would then change
    /// key when a depth is attached, and its contents with it) and at 4x.
    /// OSAL_QUERY_V3D_S8_STENCIL (CONFIG_YAKOGL_V3D_S8_STENCIL), read at init.
    s8_stencil: bool,
    /// Programs whose code passed `check_program` (keyed by the code's CPU
    /// address, length and checksum), and the slot given up next.
    prog_ok: [ProgKey; PROG_OK],
    prog_next: usize,
    /// Tilings that wrote past their image (the canaries after each).
    overruns: u32,
    /// Draws whose kept tiled copy found no GPU memory with a job open, so
    /// the job was run and the draw tried again in a new one
    /// (`Attempt::NoMemory`), and of those, the ones then drawn.
    memory_retries: u32,
    memory_retries_drawn: u32,
    kicks: u32,
    /// Another id the open job answers to (0: none). The GL core stamps what
    /// a draw uses with `v3d_kick_id()` before the draw reaches the backend;
    /// a draw that ends the open job itself (another target, a full job, no
    /// memory for a kept copy) lands in the next one, so its stamps name the
    /// job it ended. `v3d_kick_pending` answers this id as pending while the
    /// job holding that draw is open; `flush_job` moves it into `InFlight`,
    /// so it stays pending until that job has run.
    open_alias: u32,
    waits: u32,
    /// Draws that waited for the job rendering into a texture they sample
    /// (`tex_state_for`), since init, and how many of those named that job
    /// only by its alias (`InFlight::alias`): `v3d_tex_waits`.
    tex_waits: u32,
    tex_waits_alias: u32,
    /// Textures tiled, and their bytes, since init (`v3d_texture_counters`):
    /// the cost of tiling on the CPU once per job.
    tilings: u32,
    /// Draws recorded whose VS writes the point size into the VPM (the
    /// record's point-size bit set), since init (`v3d_psiz_draws`).
    /// Set while a draw is recorded into a job begun for it alone (the
    /// second attempt of `draw_in_job`): client data the data arena cannot
    /// take then goes into GPU memory of its own (`stage`).
    spill_ok: bool,
    /// Pieces of client data staged outside the arena since init.
    spills_made: u32,
    psiz_draws: u32,
    /// Of `tilings`, those made at an upload from the caller's rows
    /// (`v3d_texture_uploaded`), not at a draw from the storage.
    eager_tilings: u32,
    tiled_bytes: u64,
    /// The smallest kept copy the TFU makes (OSAL_QUERY_V3D_TFU_MIN_BYTES,
    /// read at init; 0: the CPU tiles everything). Of the tilings, those the
    /// TFU made, and those it was asked for and did not make (the CPU tiled
    /// them), since init (`v3d_tfu_counters`).
    tfu_min_bytes: usize,
    tfu_jobs: u32,
    tfu_failed: u32,
    /// Sampler records packed, and found again in the job (`sampler_for`),
    /// since init (`v3d_sampler_counters`).
    samplers_packed: u32,
    samplers_reused: u32,
    /// The CPU time of the tilings, since init (`v3d_tiling_time`), in ns
    /// of the OSAL's clock (OSAL_QUERY_CLOCK_NS), by where it was spent
    /// (`TILE_AT_UPLOAD`, `TILE_KEPT_AT_DRAW`, `TILE_ARENA`); and the
    /// tilings the clock could not time (no answer), which have no time in
    /// `tile_ns` rather than a time of zero.
    tile_ns: [u64; TILE_WHERE],
    tile_untimed: u32,
    /// Draws refused since init (`v3d_refused_draws`), every one, where the
    /// log has one line per cause.
    refused: u32,
    /// The tiled copies kept across jobs, their bytes, and the most they held.
    tiled: [Tiled; TILED_ENTRIES],
    tiled_live_bytes: usize,
    tiled_high_water: usize,
    /// The occlusion query counters (`v3d_query_begin`): one 32-bit word per
    /// query handle, at `handle * OQ_STRIDE` (counter 0 of the handle's own
    /// 1024-byte counter group), allocated by the first query
    /// and kept until shutdown. Zero: none yet.
    oq: OsalGpuMem,
    /// The most bytes of tiled copies kept at once (set at init).
    tiled_budget: usize,
    /// The high-water last logged (a line each time it doubles).
    tiled_logged: usize,
    /// Tiled copies given up to make room for another, since init
    /// (`v3d_tiled_evictions`), by cause: [0] the entry count
    /// (`TILED_ENTRIES`), [1] the byte budget. `evicted_hot` counts those
    /// the previous or the open job had sampled: the working set itself,
    /// which the next job that samples it tiles again.
    evictions: [u32; 2],
    evicted_bytes: [u64; 2],
    evicted_hot: [u32; 2],
    /// Kept copies not made, since init (`v3d_tiled_misses`): the OSAL had no
    /// memory for one, or the table was full of the working set.
    kept_no_memory: u32,
    kept_refused: u32,
    /// Stale kept copies (`drop_stale_copies`) given back before a job
    /// opened, and their bytes: each one's free waited only for the job in
    /// flight, not for the job about to be recorded.
    stale_dropped: u32,
    stale_dropped_bytes: u64,
    /// The per-job arenas: the most each was used by a job of the frame
    /// being drawn, and since init; frames counted by swaps. A swap with a
    /// job open closes the frame at that job's flush (the swap's own).
    frame_arenas: ArenaUse,
    arenas_high_water: ArenaUse,
    /// The draw-time sampler variants `(built, refused)` the last variant
    /// line printed (`end_frame`).
    variants_logged: (u32, u32),
    frames: u32,
    frame_ends_at_flush: bool,
    /// The draw budget the job arenas were sized for at init, and their sizes.
    draws_per_job: u32,
    bcl_bytes: u32,
    ind_bytes: u32,
    /// The binner pool's raw registers were printed once (`bin_pool_used`).
    bin_raw_logged: bool,
    /// Refusal reasons already logged (each is one `&'static str`, compared
    /// by address), so the log has one line per cause, not one per draw.
    logged: [*const u8; 32],
    nlogged: usize,
}

impl V3dState {
    const ZERO: V3dState = V3dState {
        inited: false,
        arenas: ArenaSet::ZERO,
        spare: ArenaSet::ZERO,
        want_big_arena: false,
        in_flight: None,
        queued: None,
        ended: JobRecord::ZERO,
        black: OsalGpuMem::ZERO,
        tile_alloc: OsalGpuMem::ZERO,
        tile_state: OsalGpuMem::ZERO,
        tile_alloc_size: 0,
        tile_state_size: 0,
        j: Job::ZERO,
        clear_program: ClearProgram::NOT_BUILT,
        clear_program_int: ClearProgram::NOT_BUILT,
        zs: [ZsEntry { target: 0, z: ZsKnown::Untouched, s: ZsKnown::Untouched }; ZS_TARGETS],
        nzs: 0,
        zsbufs: [ZsBuf::NONE; ZS_BUFS],
        zs_parked: Parked::new(0, ZsKnown::Untouched),
        msbufs: [MsBuf::NONE; MS_BUFS],
        rt_images: [RtImage::NONE; RT_IMAGES],
        rt_direct: 0,
        rt_registered: 0,
        rt_cpu_writes: 0,
        rt_no_memory: 0,
        rt_chains: 0,
        rt_chains_failed: 0,
        rt_chains_returned: 0,
        mip_refused: [0; MIP_REFUSALS],
        copy_counts: [0; COPY_REASONS],
        copy_why: CopyReason::Plain,
        mip_queued: 0,
        mip_queue_full: 0,
        rt_bytes_peak: 0,
        rt_level0_backs: 0,
        storage_current: None,
        frame_stores: ColourStores::ZERO,
        last_frame_stores: ColourStores::ZERO,
        depth_direct: 0,
        depth_resolves: 0,
        depth_refused: 0,
        zs_clock: 0,
        zs_lent: 0,
        zs_refused: None,
        zs_skipped: 0,
        zs_slots_peak: 0,
        zs_bytes_peak: 0,
        split_zs: false,
        s8_stencil: false,
        prog_ok: [ProgKey::NONE; PROG_OK],
        prog_next: 0,
        overruns: 0,
        memory_retries: 0,
        memory_retries_drawn: 0,
        kicks: 0,
        open_alias: 0,
        waits: 0,
        tex_waits: 0,
        tex_waits_alias: 0,
        tilings: 0,
        eager_tilings: 0,
        tfu_min_bytes: 0,
        tfu_jobs: 0,
        tfu_failed: 0,
        samplers_packed: 0,
        samplers_reused: 0,
        tile_ns: [0; TILE_WHERE],
        tile_untimed: 0,
        spill_ok: false,
        spills_made: 0,
        psiz_draws: 0,
        tiled_bytes: 0,
        refused: 0,
        tiled: [Tiled::NONE; TILED_ENTRIES],
        tiled_live_bytes: 0,
        tiled_high_water: 0,
        oq: OsalGpuMem::ZERO,
        tiled_budget: DEFAULT_TILED_BUDGET,
        tiled_logged: 0,
        evictions: [0; 2],
        evicted_bytes: [0; 2],
        evicted_hot: [0; 2],
        kept_no_memory: 0,
        kept_refused: 0,
        stale_dropped: 0,
        stale_dropped_bytes: 0,
        frame_arenas: ArenaUse::ZERO,
        arenas_high_water: ArenaUse::ZERO,
        variants_logged: (0, 0),
        frames: 0,
        frame_ends_at_flush: false,
        draws_per_job: 0,
        bcl_bytes: 0,
        ind_bytes: 0,
        bin_raw_logged: false,
        logged: [core::ptr::null(); 32],
        nlogged: 0,
    };
}

static G: Global<V3dState> = Global::new(V3dState::ZERO);

fn g() -> &'static mut V3dState {
    // SAFETY: single-threaded by contract; each entry point takes it once.
    unsafe { G.get() }
}

/// Log a refusal once per cause, and give the errno the contract names for a
/// draw the backend does not record.
fn refuse(g: &mut V3dState, why: state::Refusal) -> i32 {
    count_refusal(g, why);
    -EINVAL
}

/// Log and count a draw whose GL state is legal but that this backend
/// cannot render yet ([`state::unsupported`]), and give the contract's
/// errno for a backend gap.
fn refuse_unsupported(g: &mut V3dState, why: state::Refusal) -> i32 {
    count_refusal(g, why);
    -ENOTSUP
}

/// Count a draw that is not drawn (`v3d_refused_draws`), and log its cause
/// the first time that cause is seen.
fn count_refusal(g: &mut V3dState, why: state::Refusal) {
    g.refused = g.refused.wrapping_add(1);
    let key = why.as_ptr();
    if !g.logged[..g.nlogged].contains(&key) {
        if g.nlogged < g.logged.len() {
            g.logged[g.nlogged] = key;
            g.nlogged += 1;
        }
        glog!("yakogl/v3d: draw refused: {}", why);
    }
}

/* ---- the depth/stencil history -------------------------------------------- */

/// What `target`'s depth and stencil hold: its entry, `Untouched` for a
/// target never seen while there was room, `Written` (unknown) once the table
/// is full.
fn zs_known(g: &V3dState, target: u64) -> (ZsKnown, ZsKnown) {
    match g.zs[..g.nzs].iter().find(|e| e.target == target) {
        Some(e) => (e.z, e.s),
        None if g.nzs == ZS_TARGETS => (ZsKnown::Written, ZsKnown::Written),
        None => (ZsKnown::Untouched, ZsKnown::Untouched),
    }
}

fn zs_record(g: &mut V3dState, target: u64, z: ZsKnown, s: ZsKnown) {
    let n = g.nzs;
    if let Some(e) = g.zs[..n].iter_mut().find(|e| e.target == target) {
        (e.z, e.s) = (z, s);
    } else if n < ZS_TARGETS {
        g.zs[n] = ZsEntry { target, z, s };
        g.nzs += 1;
    }
    /* full: the target stays unknown, which zs_known answers as Written */
}

/// The load/store packets' memory format and "Height in UB or Stride" for a
/// buffer of `cpp` bytes a pixel (4: D24S8, 1: S8) laid out as `slice`.
/// Mesa's UIF XOR is a page-cache optimisation that depends on the UIFCFG
/// the hub is set to; NO_XOR is the same layout without it. The height is
/// the padded height in UIF blocks, 2 utiles tall (v3dx_rcl.c:62-64
/// v3d_surface_get_height_in_ub_or_stride: padded_height / (2 *
/// v3d_utile_height(cpp)); a utile is 4 rows at 4 bytes, 8 at 1,
/// v3d_tiling.c), 0 for LT and UB-linear.
fn zs_layout(slice: &V3dTexSlice, cpp: u32) -> (u32, u32) {
    let memory_format = match slice.tiling {
        V3D_TEX_TILING_UIF_XOR => V3D_TEX_TILING_UIF_NO_XOR,
        t => t,
    };
    let utile_h = if cpp == 1 { 8 } else { 4 };
    let height_in_ub = if memory_format == V3D_TEX_TILING_UIF_NO_XOR { slice.padded_height / (2 * utile_h) } else { 0 };
    (memory_format, height_in_ub)
}

/// The slot of depth/stencil buffer `id` for a `w` x `h` target: found, or
/// allocated (a new one starts Untouched, or from what was parked for `id`).
/// `Err(-ENOMEM)` when every slot holds stored contents
/// (`ZsBuf::holds_contents`), or when there is no GPU memory for a new
/// buffer: the draw is refused by name, and no buffer is given up
/// (src/ds_retention.rs). It is never drawn with depth and stencil on-chip
/// only, which would lose what it writes. `owners` are the GL objects of
/// the planes.
/// A 4x target (`msaa`) gets a 4x buffer, laid out as Mesa lays out a
/// multisampled zsbuf (single-level UIF at twice the width and height), and
/// is never given, found or lent a single-sampled one, nor the other way.
fn zs_buf(
    g: &mut V3dState,
    id: u32,
    owners: [u32; 2],
    w: u32,
    h: u32,
    msaa: bool,
    s8: bool,
    keep: Option<usize>,
) -> Result<usize, i32> {
    g.zs_clock = g.zs_clock.wrapping_add(1);
    let now = g.zs_clock;
    let mut slot = None;
    for (i, b) in g.zsbufs.iter_mut().enumerate().take(ZS_BUFS) {
        if b.live() && b.id == id {
            if (b.w, b.h, b.msaa, b.s8) == (w, h, msaa, s8) {
                (b.used, b.owners) = (now, owners);
                return Ok(i);
            }
            /* resized under the same id: the old buffer goes back */
            osal::gpu_free(&mut b.mem);
            *b = ZsBuf::NONE;
            slot = Some(i);
            break;
        }
        if !b.live() && slot.is_none() {
            slot = Some(i);
        }
    }
    /* Lent: a live buffer of the same size whose depth and stencil are both
     * undefined holds nothing to keep (Untouched: after its surface's swap,
     * EGL 1.5 3.10.1, or never drawn), so it is re-keyed to this id rather
     * than a new one allocated. Two window surfaces swapped in turn (a
     * double-buffered window, DEMO12) then share one D24S8, which at
     * 1920x720 is 5.5 MB the carve-out does not have twice. A buffer that
     * holds anything is never lent, so a surface made current again without
     * a swap still finds its own (draw.rs resolve_ds_storage). The GPU runs
     * one job at a time, so the job that last used it is done with it
     * before the next job loads or stores it. */
    let lend = (0..ZS_BUFS).find(|&i| {
        let b = &g.zsbufs[i];
        b.live()
            && Some(i) != keep
            && (b.w, b.h, b.msaa, b.s8) == (w, h, msaa, s8)
            && (b.z, b.s) == (ZsKnown::Untouched, ZsKnown::Untouched)
    });
    if let Some(i) = lend {
        g.zs_lent += 1;
        let parked = g.zs_parked.take(id);
        let b = &mut g.zsbufs[i];
        (b.id, b.used, b.owners) = (id, now, owners);
        if let Some([z, s]) = parked {
            (b.z, b.s) = (z, s);
        }
        return Ok(i);
    }
    /* No free slot: one that holds no stored contents may go (its history,
     * if any, parked), the least recently used; never `keep`, the other
     * half's buffer for the same job. With every slot holding contents the
     * draw is refused before anything is allocated. Giving up the least
     * recently used buffer instead (as before) lost another framebuffer's
     * depth with no error. */
    let who = if owners[0] != 0 { owners[0] } else { owners[1] };
    if slot.is_none() {
        let Some(i) = zs_reclaimable(g, keep, now) else {
            return Err(ds_retention::refuse("v3d", who, ZS_BUFS));
        };
        zs_release(g, i);
        slot = Some(i);
    }
    let mut l = V3dTexLayout::default();
    // SAFETY: `l` is a live layout to fill.
    let laid_out = unsafe {
        if msaa {
            v3d_tex_layout_msaa(w, h, 4, &mut l)
        } else {
            v3d_tex_layout(w, h, 1, if s8 { 1 } else { 4 }, &mut l)
        }
    };
    /* level 0 at a 4 KiB boundary, as a Mesa BO; the OSAL's granule is smaller */
    let bytes = l.size as usize + 4096;
    if !laid_out {
        return Err(ds_retention::refuse_no_memory("v3d", who, bytes));
    }
    /* refused lately: not asked again for ZS_RETRY_JOBS requests, and refused */
    if let Some((refused, at)) = g.zs_refused {
        if bytes >= refused && now.wrapping_sub(at) < ZS_RETRY_JOBS {
            g.zs_skipped += 1;
            return Err(ds_retention::refuse_no_memory("v3d", who, bytes));
        }
    }
    /* No memory: the buffers that hold no contents GL keeps go first (their
     * histories parked, as above), then the draw is refused by name. It is
     * never drawn with depth and stencil on-chip only: a later job could not
     * load what it wrote (ES 2.0.25 4.4.3: the contents persist). */
    let mut mem = OsalGpuMem::ZERO;
    let address = loop {
        if osal::gpu_alloc(V3D_HEAP_DATA, bytes, OSAL_MEM_DATA, &mut mem) == 0 {
            match gpu32(mem.gpu.next_multiple_of(4096)) {
                Some(a) => break a,
                None => {
                    osal::gpu_free(&mut mem);
                    return Err(ds_retention::refuse_no_memory("v3d", who, bytes));
                }
            }
        }
        match zs_reclaimable(g, keep, now) {
            Some(i) => zs_release(g, i),
            None => {
                g.zs_refused = Some((bytes, now));
                return Err(ds_retention::refuse_no_memory("v3d", who, bytes));
            }
        }
    };
    if g.zs_refused.is_some_and(|(refused, _)| bytes >= refused) {
        g.zs_refused = None;
    }
    let i = slot.expect("a free slot was found, or one given up, above");
    let (memory_format, height_in_ub) = zs_layout(&l.slices[0], if s8 { 1 } else { 4 });
    let (z, s) = g.zs_parked.take(id).map_or((ZsKnown::Untouched, ZsKnown::Untouched), |[z, s]| (z, s));
    g.zsbufs[i] =
        ZsBuf { id, w, h, msaa, s8, mem, address, memory_format, height_in_ub, z, s, target: 0, used: now, owners };
    let live = g.zsbufs.iter().filter(|b| b.live());
    let (n, bytes_live) = live.fold((0u32, 0usize), |(n, sum), b| (n + 1, sum + b.mem.size));
    g.zs_slots_peak = g.zs_slots_peak.max(n);
    g.zs_bytes_peak = g.zs_bytes_peak.max(bytes_live);
    Ok(i)
}

/// The least recently used buffer that may be let go: live, holding no
/// stored contents, its history (if any) able to be parked, and not `keep`.
fn zs_reclaimable(g: &V3dState, keep: Option<usize>, now: u32) -> Option<usize> {
    let room = g.zs_parked.has_room();
    let can_go = |b: &ZsBuf| b.live() && !b.holds_contents() && (room || !b.holds_history());
    (0..ZS_BUFS)
        .filter(|&i| Some(i) != keep && can_go(&g.zsbufs[i]))
        .max_by_key(|&i| now.wrapping_sub(g.zsbufs[i].used))
}

/// Let buffer `i` go (`zs_reclaimable` chose it): its history parked under
/// its id, its memory freed, the slot empty.
fn zs_release(g: &mut V3dState, i: usize) {
    let b = g.zsbufs[i];
    if b.holds_history() {
        g.zs_parked.park(b.id, b.owners, [b.z, b.s]);
    }
    osal::gpu_free(&mut g.zsbufs[i].mem);
    g.zsbufs[i] = ZsBuf::NONE;
}

/// 4x colour targets whose samples are kept at once: the window surfaces a
/// client multisamples (a double-buffered window is two).
pub const MS_BUFS: usize = 2;

/// One 4x target's colour samples between jobs: RGBA8 laid out as Mesa lays
/// out a multisampled resource (single-level UIF at twice the width and
/// height, v3d_resource.c:612-642), the UIF XOR dropped as for `ZsBuf`.
/// The job builder stores RT 0's samples here and the resolved image into
/// the target (v3d_job.h `ms_address`).
#[derive(Clone, Copy, Debug)]
struct MsBuf {
    /// The colour target (its GPU address) and its size.
    target: u64,
    w: u32,
    h: u32,
    mem: OsalGpuMem,
    /// 4 KiB aligned inside `mem`.
    address: u32,
    memory_format: u32,
    height_in_ub: u32,
    /// A job has stored the target's samples here since it was made or its
    /// surface was swapped (EGL 1.5 3.10.1: the multisample buffer is
    /// undefined after eglSwapBuffers). A job that would load RT 0 from a
    /// buffer that is not valid starts from the clear colour instead: the
    /// content is undefined, and loading garbage is not better.
    valid: bool,
    used: u32,
}

impl MsBuf {
    const NONE: MsBuf = MsBuf {
        target: 0,
        w: 0,
        h: 0,
        mem: OsalGpuMem::ZERO,
        address: 0,
        memory_format: 0,
        height_in_ub: 0,
        valid: false,
        used: 0,
    };

    fn live(&self) -> bool {
        !self.mem.impl_.is_null()
    }
}

/// The slot of the 4x target `target` (`w` x `h` pixels): found, or
/// allocated (the least recently used slot given up when all are taken).
/// `None` when there is no memory: the job then keeps no samples, and
/// every job on the target starts from the clear colour.
fn ms_buf(g: &mut V3dState, target: u64, w: u32, h: u32) -> Option<usize> {
    g.zs_clock = g.zs_clock.wrapping_add(1);
    let now = g.zs_clock;
    if let Some(i) = (0..MS_BUFS).find(|&i| g.msbufs[i].live() && g.msbufs[i].target == target) {
        if (g.msbufs[i].w, g.msbufs[i].h) == (w, h) {
            g.msbufs[i].used = now;
            return Some(i);
        }
        osal::gpu_free(&mut g.msbufs[i].mem);
        g.msbufs[i] = MsBuf::NONE;
    }
    let mut l = V3dTexLayout::default();
    // SAFETY: `l` is a live layout to fill.
    let laid_out = unsafe { v3d_tex_layout_msaa(w, h, 4, &mut l) };
    let bytes = l.size as usize + 4096;
    let mut mem = OsalGpuMem::ZERO;
    if !laid_out || osal::gpu_alloc(V3D_HEAP_DATA, bytes, OSAL_MEM_DATA, &mut mem) != 0 {
        glog_once!("yakogl/v3d: no 4x colour buffer for the {}x{} target: its samples are not kept between jobs", w, h);
        return None;
    }
    let Some(address) = gpu32(mem.gpu.next_multiple_of(4096)) else {
        osal::gpu_free(&mut mem);
        return None;
    };
    let i = (0..MS_BUFS).find(|&i| !g.msbufs[i].live()).unwrap_or_else(|| {
        let i = (0..MS_BUFS).max_by_key(|&i| now.wrapping_sub(g.msbufs[i].used)).unwrap_or(0);
        osal::gpu_free(&mut g.msbufs[i].mem);
        i
    });
    let (memory_format, height_in_ub) = zs_layout(&l.slices[0], 4);
    g.msbufs[i] = MsBuf { target, w, h, mem, address, memory_format, height_in_ub, valid: false, used: now };
    Some(i)
}

/* ---- render images ----------------------------------------------------------- */

/// Is binding `b` a texture a render image can stand for (`RtImage`)? A
/// stamped 2D RGBA8 texture at the GL core's pitch, not an EGLImage's, of a
/// size a target can have. Named, the textures that keep the tiled copy:
/// - a cube map or an array (a render goes into one face; the TMU reads
///   every layer of one image);
/// - a depth texture (R32F storage) whose rendered depth is not read from
///   its D24S8 buffer (`depth_sample`: the GL core asked and the buffer
///   held no stored depth of the texture's size), or whose texels the CPU
///   wrote since; its copy is stamped by `resolve_depth`;
/// - L8, A8, LA8 (the GL core stores them narrow; a target is RGBA8), and
///   the sRGB class (an sRGB target's texture state decodes; the tiled copy
///   keeps it);
/// - an EGLImage's (`bgra`, or no stamp: a sibling may write it);
/// - render targets 1..3 of a multi-target framebuffer (RT 0 alone has an
///   image; the job builder stores one), bgr565 and 4x targets
///   (`v3d_job_begin` refuses an image with them).
fn rt_texture(b: &GlesTextureBind) -> bool {
    b.content_gen != 0
        && b.type_ == GLES_TEXTYPE_2D
        && texture::layers(b) == 1
        && b.format == GLES_TEXFMT_RGBA8
        && b.bgra == 0
        && b.stride_texels == target_pitch(b.width)
        && b.width != 0
        && b.height != 0
        && b.width <= MAX_TARGET
        && b.height <= MAX_TARGET
}

/// Can a draw sample binding `b` from a render image? [`rt_texture`], and
/// rendered into at some point (`target_kick`): a texture no job rendered
/// into has nothing to store an image from.
fn rt_eligible(b: &GlesTextureBind) -> bool {
    rt_texture(b) && b.target_kick != 0
}

/// The slot of the render image of the target at `gpu`, `w` x `h` at
/// `pitch` texels a row.
fn rt_find(g: &V3dState, gpu: u64, w: u32, h: u32, pitch: u32) -> Option<usize> {
    (0..RT_IMAGES).find(|&i| {
        let r = &g.rt_images[i];
        r.live() && r.gpu == gpu && (r.w, r.h, r.pitch) == (w, h, pitch)
    })
}

/// Give slot `i`'s image back (held by the OSAL until the job in flight has
/// run, as every free is), the slot's registration kept. With `keep_chain`,
/// a chain the image holds for the GL core goes into the storage first
/// (`rt_chain_back`): the GL core's levels 1.. are not lost with the image.
/// Without it the storage is going away too.
fn rt_free_image(g: &mut V3dState, i: usize, keep_chain: bool) {
    if keep_chain {
        rt_chain_back(g, i);
        rt_level0_back(g, i);
    }
    /* the open job stores into this image (a relayout asked for by a draw
     * that samples its own target, or by glGenerateMipmap): it goes back to
     * the raster load and store, which the storage, made current above,
     * serves */
    if g.j.open && g.j.rt == Some(i) {
        let t = &mut g.j.job.t;
        (t.uif_address, t.uif_memory_format, t.uif_height_in_ub) = (0, 0, 0);
        (t.uif_only, t.uif_load) = (false, false);
        g.j.rt = None;
    }
    let r = &mut g.rt_images[i];
    if r.has_image() {
        osal::gpu_free(&mut r.mem);
    }
    (r.mem, r.base, r.address, r.memory_format, r.height_in_ub) = (OsalGpuMem::ZERO, 0, 0, 0, 0);
    (r.stored, r.chain, r.raster_stale) = (false, false, false);
}

/// Give slot `i` back: its image to the OSAL (`rt_free_image`), the slot
/// free.
fn rt_free(g: &mut V3dState, i: usize, keep_chain: bool) {
    rt_free_image(g, i, keep_chain);
    g.rt_images[i] = RtImage::NONE;
}

/// Register the texture `b` as a render target the application samples,
/// with `levels` levels: a free slot, else the least recently used one that
/// the open job does not store into. The image itself is made when the
/// next job renders into the target (`rt_for_render`).
fn rt_register(g: &mut V3dState, b: &GlesTextureBind, levels: u32, now: u32) {
    let keep = g.j.rt.filter(|_| g.j.open);
    let i = match (0..RT_IMAGES).find(|&i| !g.rt_images[i].live()) {
        Some(i) => i,
        None => {
            let Some(i) =
                (0..RT_IMAGES).filter(|&i| Some(i) != keep).max_by_key(|&i| now.wrapping_sub(g.rt_images[i].used))
            else {
                return;
            };
            rt_free(g, i, true);
            i
        }
    };
    g.rt_images[i] = RtImage {
        gpu: b.base_gpu,
        w: b.width,
        h: b.height,
        pitch: b.stride_texels,
        levels: levels.max(1),
        seen_gen: b.content_gen,
        used: now,
        ..RtImage::NONE
    };
    g.rt_registered += 1;
}

/// Lay slot `i`'s image out again with `levels` levels: the image goes
/// (its chain into the storage first), and the next render makes the new
/// one.
fn rt_relayout(g: &mut V3dState, i: usize, levels: u32) {
    rt_free_image(g, i, true);
    g.rt_images[i].levels = levels.max(1);
}

/// The image a draw samples binding `b` from, its layout and the levels the
/// draw reads, when the texture has an image that holds what its storage
/// holds (`RtImage`); `None` sends the draw to the tiled copy. The texture
/// is registered here the first time, a stamp the image was not stored
/// under (a CPU write since) is recorded and waits for the next render,
/// and a sample with a mipmap filter reads the image only while its levels
/// below 0 are GL's chain (`chain`).
fn rt_sample(g: &mut V3dState, b: &GlesTextureBind) -> Option<(V3dTexLayout, u32, u32)> {
    g.copy_why = if b.format == GLES_TEXFMT_R32F { CopyReason::R32f } else { CopyReason::Plain };
    if !rt_eligible(b) {
        return None;
    }
    let now = g.kicks.wrapping_add(1);
    let levels = texture::levels_sampled(b);
    let Some(i) = rt_find(g, b.base_gpu, b.width, b.height, b.stride_texels) else {
        rt_register(g, b, levels, now);
        g.copy_why = CopyReason::FirstSample;
        return None;
    };
    let r = &mut g.rt_images[i];
    r.used = now;
    if r.seen_gen != b.content_gen {
        r.seen_gen = b.content_gen;
        r.stored = false;
        r.chain = false;
        g.rt_cpu_writes += 1;
        g.copy_why = CopyReason::CpuWrite;
        return None;
    }
    if levels > r.levels {
        rt_relayout(g, i, levels);
        g.copy_why = CopyReason::Relayout;
        return None;
    }
    let r = &g.rt_images[i];
    if !r.stored || !r.has_image() {
        g.copy_why = CopyReason::NotStored;
        return None;
    }
    if levels > 1 && !r.chain {
        g.copy_why = CopyReason::NoChain;
        return None;
    }
    let Some(layout) = texture::rt_layout(b.width, b.height, r.levels) else {
        g.copy_why = CopyReason::Layout;
        return None;
    };
    let base = r.base;
    g.rt_direct += 1;
    if g.rt_direct == 1 {
        glog!(
            "yakogl/v3d: a {}x{} render target is sampled from its render image ({} bytes, {} levels): no tiled copy",
            b.width,
            b.height,
            layout.size,
            layout.levels
        );
    }
    Some((layout, base, levels))
}

/// The render image a job on `target` stores RT 0 into: the slot registered
/// for the target, its image allocated now if it has none yet (`None`, and
/// the target stays raster alone, when the OSAL has no memory for it, the
/// layout refuses the size, or the target is one no image is made for).
/// Every other image whose storage the render overlaps (another view of the
/// same storage, a level above 0 attached) no longer holds what the storage
/// holds: its chain goes back into the storage first, before the job that
/// writes the storage is recorded.
fn rt_for_render(g: &mut V3dState, target: &Target) -> Option<usize> {
    let Target { gpu, w, h, pitch, bgr, rgb565, msaa, .. } = *target;
    if gpu == 0 {
        return None;
    }
    let bytes = u64::from(pitch) * u64::from(target.bpp()) * u64::from(h);
    let found = rt_find(g, gpu, w, h, pitch).filter(|_| !bgr && !rgb565 && !msaa);
    for i in 0..RT_IMAGES {
        if Some(i) != found && g.rt_images[i].overlaps(gpu, bytes) {
            rt_level0_back(g, i);
            rt_chain_back(g, i);
            g.rt_images[i].stored = false;
        }
    }
    let i = found?;
    let now = g.kicks.wrapping_add(1);
    g.rt_images[i].used = now;
    if !g.rt_images[i].has_image() {
        let levels = g.rt_images[i].levels;
        let l = texture::rt_layout(w, h, levels)?;
        /* the layout's start at a 4 KiB boundary, as a Mesa BO; the OSAL's
         * granule is smaller */
        let bytes = l.size as usize + 4096;
        let mut mem = OsalGpuMem::ZERO;
        if osal::gpu_alloc(V3D_HEAP_DATA, bytes, OSAL_MEM_DATA, &mut mem) != 0 {
            g.rt_no_memory += 1;
            glog_once!(
                "yakogl/v3d: no GPU memory for the {}x{} render image ({} bytes): the target is sampled through a \
                 tiled copy",
                w,
                h,
                bytes
            );
            return None;
        }
        let Some(base) = gpu32(mem.gpu.next_multiple_of(4096)) else {
            osal::gpu_free(&mut mem);
            return None;
        };
        let (memory_format, height_in_ub) = zs_layout(&l.slices[0], 4);
        let r = &mut g.rt_images[i];
        (r.mem, r.base, r.address) = (mem, base, base + l.slices[0].offset);
        (r.memory_format, r.height_in_ub, r.chain) = (memory_format, height_in_ub, false);
        let live: usize = g.rt_images.iter().filter(|r| r.has_image()).map(|r| r.mem.size).sum();
        g.rt_bytes_peak = g.rt_bytes_peak.max(live);
    }
    Some(i)
}

/// Drop every render image whose storage lies in `[base, base + bytes)`:
/// the storage is going away, so no chain goes back into it.
fn rt_drop(g: &mut V3dState, base: u64, bytes: u64) {
    for i in 0..RT_IMAGES {
        if g.rt_images[i].overlaps(base, bytes) {
            rt_free(g, i, false);
        }
    }
}

/// Write the chain slot `i`'s image holds for the GL core (`RtImage::chain`)
/// back into the GL core's storage: levels 1 .. `levels` - 1 untiled from
/// the image into the raster rows the V3D's chain layout gives each level
/// (`texture::gl_level_layered`, the GL core's `level_off` for this
/// backend). The image is read by the CPU as the kept copies are written by
/// it; nothing on the GPU writes those levels (a render writes level 0).
/// Afterwards the storage is GL's chain again and the image's levels below
/// 0 are no longer trusted.
fn rt_chain_back(g: &mut V3dState, i: usize) {
    let r = g.rt_images[i];
    if !r.chain || !r.has_image() || r.levels < 2 {
        return;
    }
    g.rt_images[i].chain = false;
    /* the TFU writes the levels read below: it has run first */
    if !rt_tfu_settle(g, i) {
        glog_once!("yakogl/v3d: the job a mip chain's TFU job ran behind failed: its levels are not written back");
        return;
    }
    let Some(layout) = texture::rt_layout(r.w, r.h, r.levels) else { return };
    let (end, _, _) = texture::gl_level_layered(r.w, r.h, 1, 4, r.levels);
    let Some(dst) = cpu_of(r.gpu, end) else { return };
    let (first, _, _) = texture::gl_level_layered(r.w, r.h, 1, 4, 1);
    let image = r.image_cpu() as *const u8;
    for level in 1..r.levels {
        let (off, pitch, rows) = texture::gl_level_layered(r.w, r.h, 1, 4, level);
        let lw = (r.w >> level).max(1);
        // SAFETY: the image is the layout's bytes (rt_for_render allocated
        // it); the storage covers every level through `end` (the OSAL
        // confirmed), and the level's rows are `pitch` texels apart.
        unsafe {
            v3d_tex_load_box(image, &layout, level, 0, 0, lw, rows, (dst as *mut u8).add(off as usize), pitch * 4)
        };
    }
    // SAFETY: the levels just written, inside the storage.
    unsafe { osal::cache_flush((dst as *mut u8).add(first as usize) as *mut c_void, (end - first) as usize) };
    g.rt_chains_returned += 1;
    copy_count(g, CopyReason::ChainBack);
}

/// Make the GL core's storage of slot `i`'s level 0 current: when the jobs
/// left it behind the image (`RtImage::raster_stale`, the lazy raster
/// store), the job in flight (the one that stored the image, or one after
/// it) is waited for and level 0 is untiled from the image into the raster
/// rows (`v3d_tex_load_box`), as `rt_chain_back` does for levels 1..
/// (docs/notes/v3d-lazy-raster.md, "Where the storage must be made
/// current"). The open job is not the one storing into the image: the GL
/// core's entry (`rt_storage_current`) submits that one first, and a draw
/// being recorded reads its own target only in a feedback loop, which GL
/// leaves undefined.
fn rt_level0_back(g: &mut V3dState, i: usize) {
    let r = g.rt_images[i];
    if !r.raster_stale || !r.has_image() {
        return;
    }
    g.rt_images[i].raster_stale = false;
    if wait_in_flight(g) < 0 {
        glog_once!("yakogl/v3d: the job that stored a render image failed: its level 0 is not written back");
        return;
    }
    let Some(layout) = texture::rt_layout(r.w, r.h, r.levels) else { return };
    let bytes = r.pitch * 4 * r.h;
    let Some(dst) = cpu_of(r.gpu, bytes) else { return };
    let image = r.image_cpu() as *const u8;
    // SAFETY: the image is the layout's bytes (rt_for_render allocated it),
    // level 0 at its slice offset; the storage covers `h` rows of `pitch`
    // texels (the OSAL confirmed), and no job uses either (waited above).
    unsafe {
        osal::cache_invd(image.add(layout.slices[0].offset as usize) as *mut c_void, layout.slices[0].size as usize);
        v3d_tex_load_box(image, &layout, 0, 0, 0, r.w, r.h, dst as *mut u8, r.pitch * 4);
        osal::cache_flush(dst as *mut c_void, bytes as usize);
    }
    g.rt_images[i].raster_wanted = true;
    g.rt_level0_backs += 1;
    copy_count(g, CopyReason::Level0Back);
    if g.rt_level0_backs == 1 {
        glog!(
            "yakogl/v3d: level 0 of a {}x{} render image written back into its storage for a CPU reader (the first; \
             later ones are counted)",
            r.w,
            r.h
        );
    }
}

/// The GL core is about to read or write the texels of the 2D texture `b`
/// describes on the CPU (`crate::backend::storage_current`: glReadPixels,
/// glCopyTex*, glBlitFramebuffer, and every write through the GL core's
/// `settle_texture`): when the texture is a render target whose jobs stored
/// only its render image, the open job storing into it is submitted and its
/// level 0 is written back into the storage (`rt_level0_back`). Nothing for
/// any other texture, and nothing before init. Not an entry of the table,
/// as `v3d_depth_sampled` is not: the untile is reached through the
/// function `v3d_init` sets, so the GL core's test binaries on the mock
/// table link this without the C layout code.
pub fn v3d_storage_current(b: &GlesTextureBind) {
    let g = g();
    if !g.inited {
        return;
    }
    if let Some(f) = g.storage_current {
        f(g, b);
    }
}

fn rt_storage_current(g: &mut V3dState, b: &GlesTextureBind) {
    let Some(i) = rt_find(g, b.base_gpu, b.width, b.height, b.stride_texels) else { return };
    if g.j.open && g.j.rt == Some(i) && flush_job(g) < 0 {
        glog_once!("yakogl/v3d: the job rendering into a texture the CPU reads failed");
    }
    rt_level0_back(g, i);
}

/// Why `v3d_generate_mipmap` answered 0 (the GL core then filters on the
/// CPU), one counter each (`V3dState::mip_refused`), printed with the arena
/// high-water line and at shutdown under `MIP_REFUSAL_NAMES`.
#[derive(Clone, Copy)]
enum MipRefusal {
    /// The backend is not initialised.
    NotInited,
    /// Fewer than two levels: no chain to make.
    Levels,
    /// Not a texture a render image is kept for (`rt_texture`), or none.
    NotRt,
    /// No render image registered for it (never sampled after a render).
    NotFound,
    /// A stamp the image was not stored under (a CPU write since).
    Stamp,
    /// The image has fewer levels: laid out again for the next render.
    Relayout,
    /// The OSAL runs no TFU jobs (OSAL_QUERY_V3D_TFU_MIN_BYTES).
    TfuOff,
    /// The open job rendering into the target could not be submitted.
    Flush,
    /// No submitted job stored the image (or it has no memory).
    NotStored,
    /// The OSAL's queue was full and the wait for the job in flight failed.
    Wait,
    /// The layout refused the size.
    Layout,
    /// The packer refused the job.
    Pack,
    /// The OSAL has no query, or does not queue TFU jobs.
    NoQuery,
    /// The TFU job failed within the query.
    TfuFailed,
}

/// Why this back end made a tiled copy of a texture or render target, or
/// stored a render image, or refused to reuse a copy it had
/// (docs/notes/v3d-sampled-copies.md, "The copy counters"). Counted, never
/// acted on: the frame-1 arena line prints them.
#[derive(Clone, Copy, Debug, PartialEq)]
enum CopyReason {
    /// A binding that is no render target and no float texture (a plain
    /// upload) tiled for a draw.
    Plain,
    /// An R32F binding tiled for a draw (the GL core's depth texels, or a
    /// float texture).
    R32f,
    /// A render target sampled for the first time: registered, tiled.
    FirstSample,
    /// A render target whose texels the CPU wrote since the image was stored.
    CpuWrite,
    /// A render target whose image was laid out again for more levels.
    Relayout,
    /// A render target with no stored image (not stored yet, or no image).
    NotStored,
    /// A mipmap-filtered sample of a render target whose levels 1.. are not
    /// GL's chain in the image.
    NoChain,
    /// The TMU layout refused the render image's size.
    Layout,
    /// A depth texture bound to its D24S8 buffer found no stored depth: the
    /// draw refused (`depth_sample`).
    DepthRefused,
    /// A texture tiled when it was uploaded (`eager_tile`).
    Upload,
    /// A binding's copy tiled into the job's arena, no kept copy made.
    Arena,
    /// A kept copy of another stamp of the same storage replaced by the new
    /// one (`tiled_copy`).
    StampReplaced,
    /// A stale kept copy given back before a job opened
    /// (`drop_stale_copies`).
    StaleGivenBack,
    /// A job stored RT 0 into a render image besides the raster storage
    /// (the raster-to-UIF store).
    ImageStore,
    /// Level 0 untiled from a render image into the storage for a CPU reader.
    Level0Back,
    /// A mip chain untiled from a render image into the storage.
    ChainBack,
}

const COPY_REASONS: usize = 16;
/// The first this many reasons are why a draw's binding was tiled or refused
/// (the first line); the rest are the other copy paths (the second).
const COPY_DRAW_REASONS: usize = 9;
const COPY_REASON_NAMES: [&str; COPY_REASONS] = [
    "plain",
    "r32f",
    "first-sample",
    "cpu-write",
    "relayout",
    "not-stored",
    "no-chain",
    "layout",
    "depth-refused",
    "upload",
    "arena",
    "stamp-replaced",
    "stale-given-back",
    "image-store",
    "level0-back",
    "chain-back",
];

/// The copy counters as "name n, name n, ..." for the log lines.
struct CopyCounts<'a>(&'a [u32; COPY_REASONS], core::ops::Range<usize>);

impl core::fmt::Display for CopyCounts<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        for i in self.1.clone() {
            write!(f, "{}{} {}", if i == self.1.start { "" } else { ", " }, COPY_REASON_NAMES[i], self.0[i])?;
        }
        Ok(())
    }
}

/// Count one tiled copy (or store) under `why`.
fn copy_count(g: &mut V3dState, why: CopyReason) {
    g.copy_counts[why as usize] += 1;
}

/// The number of copy reasons (`v3d_copy_counters`).
pub const V3D_COPY_REASONS: usize = COPY_REASONS;

/// The copy counters since init, by reason, in the order of the arena
/// line's names.
///
/// # Safety
/// `copies` is null or `V3D_COPY_REASONS` writable values.
#[no_mangle]
pub unsafe extern "C" fn v3d_copy_counters(copies: *mut u32) {
    let g = g();
    if !copies.is_null() {
        // SAFETY: the caller's contract.
        unsafe { core::ptr::copy_nonoverlapping(g.copy_counts.as_ptr(), copies, COPY_REASONS) };
    }
}

const MIP_REFUSALS: usize = 14;
const MIP_REFUSAL_NAMES: [&str; MIP_REFUSALS] = [
    "uninit", "levels", "not-rt", "unknown", "stamp", "relayout", "no-tfu", "flush", "unstored", "wait", "layout",
    "pack", "no-query", "tfu",
];

/// The refusal counters as "name n, name n, ..." for the log lines.
struct MipRefusals<'a>(&'a [u32; MIP_REFUSALS]);

impl core::fmt::Display for MipRefusals<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        for (i, (name, n)) in MIP_REFUSAL_NAMES.iter().zip(self.0.iter()).enumerate() {
            write!(f, "{}{} {}", if i == 0 { "" } else { ", " }, name, n)?;
        }
        Ok(())
    }
}

/// glGenerateMipmap on the 2D texture `b` describes, whose storage holds a
/// chain of `levels` levels (`YakoglBackend::generate_mipmap`): when the
/// texture has a render image that holds its level 0, the TFU makes levels
/// 1.. of the image from it, as Mesa's v3d_generate_mipmap does (one TFU
/// job with `levels - 1` levels after the first, `v3d_tex_tfu_pack_mipmap`),
/// and the answer is 1: the GL core then filters nothing and the next
/// mip-filtered sample reads the image.
///
/// No CPU wait: the open job rendering into the target is submitted, and
/// the TFU job is queued behind the job in flight (`after`, its fence;
/// OSAL_QUERY_V3D_TFU_QUEUE), as Mesa queues its TFU job with an in-sync on
/// the render's out-sync. The OSAL runs one job at a time and runs a queued
/// TFU job when it finishes that job, before the job's fence completes and
/// before the next submit (zephyr_v3d.c `finish_inflight`), so the TFU reads
/// what the TLB stored, and the next job that samples the chain, submitted
/// later, runs after the TFU. The CPU reads the levels only after waiting
/// for that fence (`rt_chain_back`, `RtImage::tfu_fence`). Before, the call
/// waited for the render on the CPU (glmark2 terrain on the Pi 4 calls it 5
/// times a frame, docs/notes/pi4-glmark2-cpu.md). Only when the OSAL's queue
/// is full does it wait for the job in flight (counted, `mip_queue_full`)
/// and queue again.
///
/// 0 in every other case, each counted by reason (`MipRefusal`), and the GL
/// core filters on the CPU as before: a texture without an image (not
/// registered, not rendered since its registration, no memory for it), a
/// stamp the image was not stored under, an image laid out with fewer
/// levels (laid out again for the next render), an OSAL without the TFU or
/// its queue, or a TFU job that failed within the query (logged once). A
/// queued job that fails later is the OSAL's to log and recover from, as a
/// failed TFU ioctl is the kernel's in Mesa.
pub unsafe extern "C" fn v3d_generate_mipmap(b: *const GlesTextureBind, levels: u32) -> i32 {
    let g = g();
    match generate_mipmap(g, b, levels) {
        Ok(()) => 1,
        Err(why) => {
            g.mip_refused[why as usize] = g.mip_refused[why as usize].wrapping_add(1);
            0
        }
    }
}

fn generate_mipmap(g: &mut V3dState, b: *const GlesTextureBind, levels: u32) -> Result<(), MipRefusal> {
    use MipRefusal::*;
    if !g.inited {
        return Err(NotInited);
    }
    // SAFETY: the caller's contract.
    let Some(b) = (unsafe { b.as_ref() }) else { return Err(NotRt) };
    if levels < 2 {
        return Err(Levels);
    }
    if !rt_texture(b) {
        return Err(NotRt);
    }
    let i = rt_find(g, b.base_gpu, b.width, b.height, b.stride_texels).ok_or(NotFound)?;
    let now = g.kicks.wrapping_add(1);
    g.rt_images[i].used = now;
    if g.rt_images[i].seen_gen != b.content_gen {
        return Err(Stamp);
    }
    if levels > g.rt_images[i].levels {
        rt_relayout(g, i, levels);
        return Err(Relayout);
    }
    if g.tfu_min_bytes == 0 {
        return Err(TfuOff);
    }
    /* the job rendering into the target may still be open: it stores the
     * image when it runs; submitted, not waited for */
    if g.j.open && g.j.rt == Some(i) && flush_job(g) < 0 {
        return Err(Flush);
    }
    let r = g.rt_images[i];
    if !r.stored || !r.has_image() {
        return Err(NotStored);
    }
    let layout = texture::rt_layout(r.w, r.h, r.levels).ok_or(Layout)?;
    let mut j = V3dTexTfuJob::default();
    // SAFETY: a live layout and job.
    if !unsafe { v3d_tex_tfu_pack_mipmap(&layout, V3D_TFU_TTYPE_RGBA8, r.base, &mut j) } {
        return Err(Pack);
    }
    let query = osal::ops().and_then(|o| o.query).ok_or(NoQuery)?;
    let tfu = OsalV3dTfu {
        icfg: j.icfg,
        iia: j.iia,
        iis: j.iis,
        ica: j.ica,
        iua: j.iua,
        ioa: j.ioa,
        ios: j.ios,
        coef0: j.coef0,
        ..OsalV3dTfu::default()
    };
    let ask = |q: &mut OsalV3dTfuQueue| {
        // SAFETY: a live struct osal_v3d_tfu_queue of the size passed.
        unsafe {
            query(
                OSAL_QUERY_V3D_TFU_QUEUE,
                q as *mut OsalV3dTfuQueue as *mut c_void,
                core::mem::size_of::<OsalV3dTfuQueue>(),
            )
        }
    };
    /* behind the job in flight (the one that stored the image, or one
     * submitted after it); none: every job has been waited for */
    let mut q = OsalV3dTfuQueue { tfu, after: g.in_flight.map_or(0, |f| f.fence), fence: 0 };
    let mut rc = ask(&mut q);
    if rc == -EBUSY {
        /* the OSAL keeps no more: the job in flight runs the ones it holds */
        g.mip_queue_full += 1;
        if wait_in_flight(g) < 0 {
            return Err(Wait);
        }
        q = OsalV3dTfuQueue { tfu, after: 0, fence: 0 };
        rc = ask(&mut q);
    }
    if rc == -ENOTSUP {
        return Err(NoQuery);
    }
    if rc != 0 || q.tfu.rc != 0 {
        g.rt_chains_failed += 1;
        if g.rt_chains_failed == 1 {
            glog!(
                "yakogl/v3d: the TFU did not make the {}-level chain of a {}x{} render image ({}, {}): the GL core \
                 filters it",
                r.levels,
                r.w,
                r.h,
                rc,
                q.tfu.rc
            );
        }
        return Err(TfuFailed);
    }
    g.rt_images[i].chain = true;
    g.rt_images[i].tfu_fence = q.fence;
    if q.fence != 0 {
        g.mip_queued += 1;
    }
    g.rt_chains += 1;
    if g.rt_chains == 1 {
        glog!(
            "yakogl/v3d: the TFU made the {}-level chain of a {}x{} render image: glGenerateMipmap filters nothing",
            r.levels,
            r.w,
            r.h
        );
    }
    Ok(())
}

/// The TFU job making slot `i`'s chain has run: when it was queued behind a
/// job the CPU has not seen complete, that job (the one in flight) is waited
/// for. False when the wait failed (the levels are not to be read).
fn rt_tfu_settle(g: &mut V3dState, i: usize) -> bool {
    let fence = core::mem::take(&mut g.rt_images[i].tfu_fence);
    if fence == 0 || fence_done(fence) {
        return true;
    }
    /* a fence older than both outstanding jobs has completed (`flush_job`
     * keeps two at most) */
    if g.in_flight.is_some_and(|f| f.fence == fence) {
        return wait_in_flight(g) >= 0;
    }
    if g.queued.is_some_and(|f| f.fence == fence) {
        return wait_queued(g) >= 0;
    }
    true
}

/// The GL core is about to read or write levels 1.. of the texture `b`
/// describes on the CPU (`YakoglBackend::return_mipmap_chain`): the chain
/// its render image holds goes back into the storage (`rt_chain_back`).
pub unsafe extern "C" fn v3d_return_mipmap_chain(b: *const GlesTextureBind, _levels: u32) {
    let g = g();
    // SAFETY: the caller's contract.
    let Some(b) = (unsafe { b.as_ref() }) else { return };
    if !g.inited {
        return;
    }
    if let Some(i) = rt_find(g, b.base_gpu, b.width, b.height, b.stride_texels) {
        rt_chain_back(g, i);
    }
}

/// The buffers a draw's `ds_id` keeps its depth and stencil in: the id
/// itself for one buffer, or, with the halves apart (`split`), the depth's
/// key and the stencil's. The GL core's FBO id is (depth name << 16) |
/// stencil name (`resolve_ds_storage`, draw.rs); a window's has bit 31 set
/// and always keeps one buffer. Apart only when both names are there and
/// differ: the depth then lives under (depth << 16), the key an FBO with
/// only that depth renderbuffer has, and the stencil under (stencil), the
/// key of an FBO with only that stencil renderbuffer. So two FBOs sharing a
/// depth renderbuffer share its depth whatever their stencils
/// (dEQP fbo.render.shared_depthbuffer), and an FBO whose stencil is
/// detached keeps its depth (recreate_stencilbuffer.no_rebind_*).
///
/// `s8` (`V3dState::s8_stencil`, with `split` on, single-sampled): a
/// stencil-only framebuffer's id is its stencil half alone, `(None,
/// Some(stencil))`, under the same key the stencil has once a depth is
/// attached, so its S8 contents stay with the stencil renderbuffer (ES
/// 2.0.25 4.4.3) whatever the framebuffer's other attachments.
fn zs_keys(id: u32, split: bool, s8: bool) -> (Option<u32>, Option<u32>) {
    let (depth, stencil) = (id >> 16, id & 0xffff);
    let fbo = id & 0x8000_0000 == 0;
    if split && fbo && depth != 0 && stencil != 0 && depth != stencil {
        (Some(depth << 16), Some(stencil))
    } else if split && s8 && fbo && depth == 0 && stencil != 0 {
        (None, Some(stencil))
    } else {
        (Some(id), None)
    }
}

/// 4x multisampling and with it the EGL 4x config (egl.rs offered_configs):
/// OFF until the board has measured it (gl_test 16's 4x edge and 145's
/// sample coverage, then dEQP-GLES2 multisample), as the GC7000's
/// `gc_debug_msaa`. The backend draws a 4x surface's draws either way; this
/// decides whether EGL offers one (docs/online-shaderc/v3d-backend.md
/// "Multisampling", step 3).
#[no_mangle]
pub static mut v3d_debug_msaa: i32 = 0;

/// Set [`v3d_debug_msaa`].
#[no_mangle]
pub extern "C" fn v3d_debug_set_msaa(on: i32) {
    // SAFETY: single-threaded by contract; a plain store of an int.
    unsafe { core::ptr::addr_of_mut!(v3d_debug_msaa).write(on) }
}

/// Does this process offer 4x surfaces ([`v3d_debug_msaa`])?
pub fn msaa_on() -> bool {
    // SAFETY: as above; a plain read.
    (unsafe { core::ptr::addr_of!(v3d_debug_msaa).read() }) != 0
}

/// Keep depth and stencil apart (`on`) or in one buffer per `ds_id` (the
/// default until the board has measured it): the open job is flushed and
/// every kept depth/stencil buffer given back, so neither way reads what
/// the other stored. Returns the previous setting. For gl_test and an
/// image's start-up; the log says when it changes.
#[no_mangle]
pub extern "C" fn v3d_split_zs(on: bool) -> bool {
    let g = g();
    let was = g.split_zs;
    if was == on {
        return was;
    }
    if g.inited && flush_job(g) < 0 {
        glog_once!("yakogl/v3d: the job open when depth and stencil were switched failed");
    }
    for b in g.zsbufs.iter_mut() {
        if b.live() {
            osal::gpu_free(&mut b.mem);
        }
        *b = ZsBuf::NONE;
    }
    g.zs_parked.clear();
    g.split_zs = on;
    glog_once!("yakogl/v3d: depth and stencil {}", if on { "apart (by renderbuffer)" } else { "in one buffer" });
    was
}

/* ---- jobs ------------------------------------------------------------------ */

/// Open a job on `target`: the lists reset, the prologue written, the colour
/// loaded unless a clear replaces it, and depth/stencil started from what the
/// history says they hold.
fn begin_job(g: &mut V3dState, target: Target) -> i32 {
    let rc = choose_arenas(g);
    if rc < 0 {
        return rc;
    }
    let Target { gpu, w, h, pitch, no_alpha, bgr, rgb565, msaa, .. } = target;
    /* a 4x target's colour samples, and its depth/stencil at 4x: Mesa's
     * multisampled layout (v3d_tex_layout_msaa), never lent to a
     * single-sampled target (zs_buf matches and lends by kind too) */
    let ms = if msaa { ms_buf(g, gpu, w, h) } else { None };
    /* this job renders into [gpu, + pitch * h): a copy tiled from that
     * memory is out of date (the stamp only moves on the CPU's writes), and
     * a later job that samples it tiles it again, after this one has run */
    if gpu != 0 {
        drop_tiled(g, gpu, pitch as u64 * target.bpp() as u64 * h as u64);
    }
    /* the render image of a texture the application samples: this job
     * stores RT 0 into it as well (v3d_job.h uif_*) */
    let rt = rt_for_render(g, &target);
    /* a target with a depth/stencil buffer starts from its history; one
     * without, from the per-target history of what the tile buffer held */
    let (zs, ss) = match target_zs_bufs(g, &target) {
        Ok(bufs) => bufs,
        Err(rc) => return rc,
    };
    ds_retention::admitted();
    for i in [zs, ss].into_iter().flatten() {
        g.zsbufs[i].target = gpu;
    }
    let (z, s) = match (zs, ss) {
        (Some(zi), Some(si)) => (g.zsbufs[zi].z, g.zsbufs[si].s),
        (Some(i), None) => (g.zsbufs[i].z, g.zsbufs[i].s),
        /* a stencil half alone (S8): the framebuffer has no depth */
        (None, Some(si)) => (ZsKnown::Untouched, g.zsbufs[si].s),
        (None, None) => zs_known(g, gpu),
    };
    let tile_alloc = g.tile_alloc.gpu as u32;
    /* ES 3.0 colour formats (int-rt-v3d, `state::rt_format`): the job
     * builder's typed render targets, each RT loaded and stored in its own
     * image format at its own bytes a pixel, and smaller tiles for 64- and
     * 128-bpp tile buffers; a job whose tiles outnumber what the pools hold
     * is refused by name. The ES 2.0 formats keep the untyped job, byte for
     * byte as before. */
    let (typed, max_bpp, formats) = (target.typed(), target.max_bpp(), target.formats());
    if typed {
        let mut tiling = V3dTiling::default();
        // SAFETY: plain computations on the out-parameter given.
        unsafe { v3d_frame_tiling_bpp(w, h, u32::from(target.rts.max(1)), msaa, max_bpp, &mut tiling) };
        if tiling.tiles_x * tiling.tiles_y > MAX_JOB_TILES {
            return refuse(g, "a 64- or 128-bpp colour target with more tiles than the job pools hold");
        }
    }
    let (bcl_cpu, bcl_gpu) = (g.arenas.bcl.cpu as *mut u8, g.arenas.bcl.gpu as u32);
    let (rcl_cpu, rcl_gpu) = (g.arenas.rcl.cpu as *mut u8, g.arenas.rcl.gpu as u32);
    let (ind_cpu, ind_gpu) = (g.arenas.ind.cpu as *mut u8, g.arenas.ind.gpu as u32);
    free_spills(g, 0); /* a job that was never submitted referenced them */
    let j = &mut g.j;
    *j = Job::ZERO;
    j.bcl = V3dCl::new(bcl_cpu, bcl_gpu, g.bcl_bytes);
    j.rcl = V3dCl::new(rcl_cpu, rcl_gpu, RCL_BYTES as u32);
    j.ind = V3dCl::new(ind_cpu, ind_gpu, g.ind_bytes);
    let t = V3dJobTarget {
        width: w,
        height: h,
        address: gpu as u32,
        /* tex_pitch(w), or a view's wider pitch: the raster load and store
         * stride (v3d_job.c:830, :919) */
        stride: pitch * target.bpp(),
        tile_alloc,
        rgb565,
        /* RT 1..3 of the draw's framebuffer, each in its own format at RT
         * 0's size and pitch (a GL_NONE one has address 0: configured,
         * never stored) */
        rts: u32::from(target.rts),
        rt: core::array::from_fn(|i| {
            let a = target.rt[i];
            let bytes = state::rt_row(target.rt_format[i]).bytes;
            V3dJobRt { address: a, stride: if a != 0 { target_pitch(w) * bytes } else { 0 } }
        }),
        msaa,
        ms_address: ms.map_or(0, |i| g.msbufs[i].address),
        ms_memory_format: ms.map_or(0, |i| g.msbufs[i].memory_format),
        ms_height_in_ub: ms.map_or(0, |i| g.msbufs[i].height_in_ub),
        uif_address: rt.map_or(0, |i| g.rt_images[i].address),
        uif_memory_format: rt.map_or(0, |i| g.rt_images[i].memory_format),
        uif_height_in_ub: rt.map_or(0, |i| g.rt_images[i].height_in_ub),
        typed,
        /* the lazy raster store: a target with a render image stores the
         * image alone, its storage written back when the CPU reads it
         * (rt_level0_back), until a CPU reader has needed it once
         * (`raster_wanted`); a job that loads the colour loads the image
         * while the storage is behind it */
        uif_only: rt.is_some_and(|i| !g.rt_images[i].raster_wanted),
        uif_load: rt.is_some_and(|i| g.rt_images[i].raster_stale),
        /* read by the job builder with `typed` only; an untyped (ES 2.0) job
         * is handed exactly what it was */
        rt_format: if typed { formats.map(|f| state::rt_row(f).output) } else { [0; 4] },
    };
    let (bcl, rcl, ind) = (&mut j.bcl as *mut V3dCl, &mut j.rcl as *mut V3dCl, &mut j.ind as *mut V3dCl);
    // SAFETY: the job and its three lists live in the backend's static state;
    // the lists point at the CPU mappings init allocated with these sizes.
    if !unsafe { v3d_job_begin(&mut j.job, &t, bcl, rcl, ind) } {
        return -EIO;
    }
    j.open = true;
    j.target = target;
    j.ms = ms;
    j.rt = rt;
    j.max_bpp = max_bpp;
    /* a 4x target whose samples no job has kept (a new surface, or one
     * swapped since, EGL 1.5 3.10.1): its content is undefined, so the job
     * starts from the clear colour rather than load what the buffer holds */
    if msaa && !ms.is_some_and(|i| g.msbufs[i].valid) {
        g.j.clear.load_rt0 = false;
    }
    let j = &mut g.j;
    job_open_to_osal(true);
    /* the alpha a format without one reads (ES 2.0 4.3.1): 1, loaded as 1
     * and never written (state.rs masks it), so it is stored as 1 */
    j.clear.force_alpha_1 = no_alpha;
    /* an XRGB8888 target's bytes are B,G,R,X: the TLB swaps at load and store */
    j.clear.r_b_swap = bgr;
    (j.z_valid, j.s_valid) = (true, true);
    j.zs = zs;
    j.ss = ss;
    match z {
        ZsKnown::Untouched => {}
        ZsKnown::Cleared(v) => j.clear.z = v,
        ZsKnown::Written => j.z_valid = false,
        ZsKnown::Stored => j.z_stored = true,
    }
    match s {
        ZsKnown::Untouched => {}
        ZsKnown::Cleared(v) => j.clear.stencil = v as u32,
        ZsKnown::Written => j.s_valid = false,
        ZsKnown::Stored => j.s_stored = true,
    }
    0
}

/// The depth/stencil buffers (depth half, stencil half; indices of
/// `zsbufs`) a job into `target` uses, found or allocated (`zs_buf`): one
/// D24S8 for both halves, or each half apart (`zs_keys`); none for a target
/// with no depth/stencil buffer. Every slot holding stored contents, or no
/// memory for a new buffer: `Err`, the draw refused by name (`zs_buf`),
/// never drawn on-chip only. `begin_job` and `seed_depth` ask it, so the
/// texels go into the buffer the job then loads.
fn target_zs_bufs(g: &mut V3dState, target: &Target) -> Result<(Option<usize>, Option<usize>), i32> {
    let (w, h, msaa) = (target.w, target.h, target.msaa);
    let (zkey, skey) = match target.ds {
        Some(id) => zs_keys(id, g.split_zs, g.s8_stencil && !msaa),
        None => (None, None),
    };
    /* the stencil half is S8 when the knob is on (with split, single-
     * sampled: zs_keys); the depth half and one buffer are D24S8 */
    let s8 = g.split_zs && g.s8_stencil && !msaa;
    /* the owners of each buffer: both objects for one buffer, each its own
     * apart (zs_keys) */
    let owners = target.ds_owners;
    let (zown, sown) = if skey.is_some() { ([owners[0], 0], [0, owners[1]]) } else { (owners, [0; 2]) };
    let zs = zkey.map(|k| zs_buf(g, k, zown, w, h, msaa, false, None)).transpose()?;
    let ss = skey.map(|k| zs_buf(g, k, sown, w, h, msaa, s8, zs)).transpose()?;
    Ok((zs, ss))
}

/// Write the depth texels draw `d` carries (`GlesDraw::depth_texels`: the
/// CPU wrote its depth texture since the buffer matched it) into the depth
/// buffer of the job `target` begins next, and mark its depth Stored: that
/// job loads it at every tile's start (`flush_job`'s `zs_load`) and the
/// draw tests against the texels. No job is open (the caller ended it), and
/// every submitted one is waited for before the CPU writes the buffer.
///
/// The layout is the one `v3d_depth_resolve` reads, written the other way:
/// the buffer's single D24S8 UIF level (`texture::rt_layout`, the XOR
/// dropped), each word depth in bits 31..8
/// ([`ds_retention::depth_unorm24`]) and stencil in bits 7..0, row 0 at
/// GL's y 0, tiled by `v3d_tex_store_box`. The whole level is written
/// through a raster scratch copy: what the buffer holds stays where its
/// history says it holds something (a Stored half is loaded back first
/// with `v3d_tex_load_box`), a Cleared half takes its value (so a job
/// that loads the buffer finds it, as the history's own rule wants), an
/// Untouched one depth 1.0 or stencil 0.
///
/// 0 also for a target with no depth buffer. `-ENOTSUP`, named once, for a
/// 4x or S8 buffer or one with no CPU view (a depth texture's buffer is
/// single-sampled D24S8, CPU-mapped); `-ENOMEM` with no scratch memory.
/// Either refuses the draw rather than test it against depth the texels no
/// longer hold.
fn seed_depth(g: &mut V3dState, target: &Target, t: ds_retention::DepthTexels) -> i32 {
    let zs = match target_zs_bufs(g, target) {
        Ok((Some(i), _)) => i,
        Ok((None, _)) => return 0,
        Err(rc) => return rc,
    };
    /* a submitted job may still run on the buffer (`flush_job` does not
     * wait for the job it submits, and `zs_buf` may lend a buffer a swapped
     * surface's job used): every one completes before the CPU writes it */
    let rc = wait_in_flight(g);
    if rc < 0 {
        return rc;
    }
    let b = g.zsbufs[zs];
    let layout = texture::rt_layout(b.w, b.h, 1).filter(|l| l.slices[0].offset == 0);
    let off = u64::from(b.address).checked_sub(b.mem.gpu).map(|o| o as usize);
    let image = match (layout, off) {
        (Some(l), Some(off)) if !b.msaa && !b.s8 && !b.mem.cpu.is_null() && off + l.size as usize <= b.mem.size => {
            Some((l, (b.mem.cpu as *mut u8).wrapping_add(off)))
        }
        _ => None,
    };
    let Some((l, image)) = image else {
        glog_once!(
            "yakogl/v3d: draw refused: the depth the CPU wrote into object 0x{:x} cannot go into its depth buffer \
             (4x, S8, or no CPU view)",
            b.owners[0]
        );
        return -ENOTSUP;
    };
    let (w, h) = (b.w as usize, b.h as usize);
    let mut scratch = OsalGpuMem::ZERO;
    if osal::gpu_alloc(V3D_HEAP_DATA, w * h * 4, OSAL_MEM_DATA, &mut scratch) != 0 || scratch.cpu.is_null() {
        osal::gpu_free(&mut scratch);
        glog_once!("yakogl/v3d: draw refused: no {} bytes to write the depth the CPU wrote into its buffer", w * h * 4);
        return -ENOMEM;
    }
    let rows = scratch.cpu as *mut u8;
    /* `image` is the buffer's live CPU mapping, `l.size` bytes (checked
     * above), which no job uses: none is open, and every submitted one was
     * waited for above. `rows` holds `h` rows of `4 * w` bytes. */
    let kept = matches!(b.z, ZsKnown::Stored) || matches!(b.s, ZsKnown::Stored);
    if kept {
        // SAFETY: see above.
        unsafe { osal::cache_invd(image as *mut c_void, l.size as usize) };
        // SAFETY: see above.
        if !unsafe { v3d_tex_load_box(image, &l, 0, 0, 0, b.w, b.h, rows, b.w * 4) } {
            osal::gpu_free(&mut scratch);
            return -EINVAL;
        }
    }
    let depth_else = match b.z {
        ZsKnown::Cleared(v) => Some(ds_retention::depth_unorm24(v)),
        ZsKnown::Stored => None,
        _ => Some(0xff_ffff),
    };
    let stencil = match b.s {
        ZsKnown::Cleared(v) => Some(v as u32 & 0xff),
        ZsKnown::Stored => None,
        _ => Some(0),
    };
    // SAFETY: `rows` holds `w * h` words (allocated above).
    let words = unsafe { core::slice::from_raw_parts_mut(rows as *mut u32, w * h) };
    for (y, row) in words.chunks_exact_mut(w).enumerate() {
        for (x, word) in row.iter_mut().enumerate() {
            let inside = (x as u32) < t.w && (y as u32) < t.h;
            let depth = if inside { Some(t.unorm24(x as u32, y as u32)) } else { depth_else };
            let d = depth.map_or(*word & 0xffff_ff00, |v| v << 8);
            *word = d | stencil.unwrap_or(*word & 0xff);
        }
    }
    // SAFETY: see above.
    let stored = unsafe { v3d_tex_store_box(image, &l, 0, 0, 0, b.w, b.h, rows, b.w * 4) };
    osal::gpu_free(&mut scratch);
    if !stored {
        return -EINVAL;
    }
    // SAFETY: the level just written, inside the mapping.
    unsafe { osal::cache_flush(image as *mut c_void, l.size as usize) };
    g.zsbufs[zs].z = ZsKnown::Stored;
    0
}

/// The row pitch, in pixels, of a target `w` wide: the GL core lays out
/// every colour buffer at whole pixel pairs (`gl::context::tex_pitch`,
/// GL_TEX_PITCH: storage, uploads and glReadPixels all use it), so an
/// odd-width texture target has one pixel of padding per row. The PowerVR
/// writes at the same pitch (`pvr/backend.rs`, the PBE's stride_texels).
pub const fn target_pitch(w: u32) -> u32 {
    w.wrapping_add(1) & !1
}

/// End, submit and wait for the open job; afterwards no job is open.
fn flush_job(g: &mut V3dState) -> i32 {
    if !g.j.open {
        return 0;
    }
    g.j.open = false;
    /* the draws it stood for are in the job ending now: the alias goes with
     * it into InFlight below */
    let alias = core::mem::take(&mut g.open_alias);
    /* the OSAL learns only after the job has run that nothing is open
     * (below), so what the job references is held until then */
    /* The depth/stencil buffer: loaded at every tile's start when it holds
     * stored depth or stencil the job did not clear (a partial clear with
     * a load was drawn as the quad, so a job never clears one of them by
     * TLB while loading the other: GFXH-1461); stored when the job wrote
     * depth or stencil. A job that only cleared stores nothing: the next
     * one starts from the same clear values (ZsKnown::Cleared). */
    let split = g.split_zs;
    match (g.j.zs, g.j.ss) {
        (Some(zi), Some(si)) => {
            /* apart: each half loaded from and stored to its own buffer;
             * a cleared half is stored too, so a buffer never holds a
             * "cleared, not stored" half that a job loading the other half
             * would lose (GFXH-1461) */
            let (zb, sb, j) = (g.zsbufs[zi], g.zsbufs[si], &mut g.j);
            j.clear.zs_address = zb.address;
            j.clear.zs_memory_format = zb.memory_format;
            j.clear.zs_height_in_ub = zb.height_in_ub;
            j.clear.zs_load = j.z_stored && !j.z_cleared;
            j.clear.zs_store = (j.z_written || j.z_cleared) && !j.zs_discard;
            j.clear.s_address = sb.address;
            j.clear.s_memory_format = sb.memory_format;
            j.clear.s_height_in_ub = sb.height_in_ub;
            j.clear.s_s8 = sb.s8;
            j.clear.s_load = j.s_stored && !j.s_cleared;
            j.clear.s_store = (j.s_written || j.s_cleared) && !j.zs_discard;
        }
        (None, Some(si)) => {
            /* a stencil-only framebuffer's S8 buffer (zs_keys): the
             * stencil half alone, as above, and no depth buffer (zs_address
             * 0: no Z load or store) */
            let (sb, j) = (g.zsbufs[si], &mut g.j);
            j.clear.s_address = sb.address;
            j.clear.s_memory_format = sb.memory_format;
            j.clear.s_height_in_ub = sb.height_in_ub;
            j.clear.s_s8 = sb.s8;
            j.clear.s_load = j.s_stored && !j.s_cleared;
            j.clear.s_store = (j.s_written || j.s_cleared) && !j.zs_discard;
        }
        (Some(i), _) => {
            let (b, j) = (&g.zsbufs[i], &mut g.j);
            let load = (j.z_stored && !j.z_cleared) || (j.s_stored && !j.s_cleared);
            j.clear.zs_address = b.address;
            j.clear.zs_memory_format = b.memory_format;
            j.clear.zs_height_in_ub = b.height_in_ub;
            j.clear.zs_load = load;
            /* with the halves apart anywhere, a cleared half is stored as
             * well (see above): no buffer keeps a "cleared" history */
            let touched = j.z_written || j.s_written || (split && (j.z_cleared || j.s_cleared));
            j.clear.zs_store = touched && !j.zs_discard;
        }
        _ => {}
    }
    let j = &mut g.j;
    // SAFETY: the job was begun on these lists, which are still live.
    if !unsafe { v3d_job_end(&mut j.job, &j.clear, core::ptr::null_mut(), core::ptr::null_mut()) } {
        glog_once!("yakogl/v3d: a job's lists did not end cleanly; the job is dropped");
        job_open_to_osal(false); /* dropped: it references nothing now */
        free_spills(g, 0);
        return -EIO;
    }
    let lists = ArenaUse {
        bcl: j.bcl.used,
        rcl: j.rcl.used,
        ind: j.ind.used,
        data: j.data_used as u32,
        tex: j.tex_used as u32,
        draws: j.job.draws,
        ..ArenaUse::ZERO
    };
    /* two jobs are outstanding at most: with two already, the older one has
     * run before this one is submitted (the OSAL runs one job at a time, and
     * keeps one more waiting for it). With one, none is waited for: this
     * job is queued behind it, and the CPU records on */
    let prev = wait_queued(g);
    let rc = submit_job(g);
    if let Some(f) = g.in_flight.as_mut().filter(|_| rc == 0) {
        f.alias = alias;
    }
    job_open_to_osal(false);
    /* a 4x job stored its samples (v3d_job.c: ALL_SAMPLES into ms_address):
     * the next job on the target loads them, unless the surface was swapped
     * while the job was open (they are undefined after it) */
    if let Some(i) = g.j.ms {
        g.msbufs[i].valid = rc == 0 && !g.j.zs_discard;
    }
    /* the job stored RT 0 into the target's render image: a submitted job
     * writes the image and the storage alike, so the image holds what the
     * storage holds (RtImage); a dropped job wrote neither */
    if let Some(i) = g.j.rt.filter(|&i| rc == 0 && g.rt_images[i].live() && g.rt_images[i].gpu == g.j.target.gpu) {
        g.rt_images[i].stored = true;
        /* the image alone (the lazy raster store): the storage is behind it
         * until a CPU reader asks (rt_level0_back) */
        if g.j.job.t.uif_only {
            g.rt_images[i].raster_stale = true;
        }
    }
    /* the frame's RT 0 stores, as v3d_job.c emits them */
    let t = g.j.job.t;
    if rc == 0 && t.address != 0 {
        let bytes = u64::from(t.width) * u64::from(t.height) * u64::from(g.j.target.bpp());
        let image = t.uif_address != 0 && !t.msaa;
        let fs = &mut g.frame_stores;
        if !(image && t.uif_only) {
            fs.raster += 1;
            fs.raster_bytes += bytes;
        }
        if image {
            fs.image += 1;
            g.copy_counts[CopyReason::ImageStore as usize] += 1;
            fs.image_bytes += bytes;
        }
    }
    /* the job may still be running: the OSAL holds these frees until it has
     * (src/osal/v3d_frees.h) */
    free_spills(g, 0);
    /* the job's use, the binner's pool as its binning left it included,
     * into the frame's largest (a failed job: its lists, the pool unread) */
    let bin = if rc == 0 { bin_pool_used(g) } else { ArenaUse { bin_unread: 1, ..ArenaUse::ZERO } };
    let used = ArenaUse {
        bin: bin.bin,
        bin_size: bin.bin_size,
        bin_unread: bin.bin_unread,
        bin_overflow: bin.bin_overflow,
        bin_overflow_blocks: bin.bin_overflow_blocks,
        bin_overflow_block: bin.bin_overflow_block,
        ..lists
    };
    g.frame_arenas = g.frame_arenas.max(used);
    if core::mem::take(&mut g.frame_ends_at_flush) {
        end_frame(g);
    }
    if prev < 0 {
        prev
    } else {
        rc
    }
}

/// Take the arena set the job about to begin records into: the 8 MiB set
/// when no outstanding job holds it, else the small one (two jobs are
/// outstanding at most: `flush_job` waits for the older before it submits a
/// third, and the set one does not hold is free, else the holder is waited
/// for). A tiling that asked for the 8 MiB set
/// (`want_big_arena`) gets it, after a wait when an outstanding job holds it;
/// so does every job when there is no second set.
fn choose_arenas(g: &mut V3dState) -> i32 {
    let big = core::mem::take(&mut g.want_big_arena);
    /* a set is busy while an outstanding job holds it and has not completed */
    let busy = |set: &ArenaSet, g: &V3dState| holder(set, g).is_some();
    if g.spare.live() {
        let spare_is_big = g.spare.tex_bytes > g.arenas.tex_bytes;
        let (arenas_busy, spare_busy) = (busy(&g.arenas, g), busy(&g.spare, g));
        let take_spare = if big || !(arenas_busy || spare_busy) {
            spare_is_big /* the 8 MiB set */
        } else {
            arenas_busy /* the one the jobs outstanding do not hold */
        };
        if take_spare {
            core::mem::swap(&mut g.arenas, &mut g.spare);
        }
    }
    match holder(&g.arenas, g) {
        None => 0,
        /* the older job holds it: that one alone is waited for */
        Some(h) if g.queued.is_some_and(|q| q.kick == h.kick) => wait_queued(g),
        Some(_) => wait_in_flight(g),
    }
}

/// The outstanding job that holds `set` and has not completed, if any.
fn holder(set: &ArenaSet, g: &V3dState) -> Option<InFlight> {
    if set.kick == 0 {
        return None;
    }
    [g.queued, g.in_flight].into_iter().flatten().find(|f| f.kick == set.kick && !fence_done(f.fence))
}

/// Wait for every job outstanding, if any: 0, or the first error (after an
/// MMU fault, with the job's fault record logged).
fn wait_in_flight(g: &mut V3dState) -> i32 {
    let older = wait_queued(g);
    let newer = match g.in_flight.take() {
        Some(f) => wait_job(g, f),
        None => 0,
    };
    if older < 0 {
        older
    } else {
        newer
    }
}

/// Wait for the older of two outstanding jobs (`queued`), if there is one;
/// the newer stays outstanding, waiting behind it in the OSAL.
fn wait_queued(g: &mut V3dState) -> i32 {
    match g.queued.take() {
        Some(f) => wait_job(g, f),
        None => 0,
    }
}

/// Wait for job `f`: 0, or its error.
fn wait_job(g: &mut V3dState, f: InFlight) -> i32 {
    let Some(wait) = osal::ops().and_then(|o| o.wait) else {
        return -ENODEV;
    };
    // SAFETY: a wait on a fence the OSAL returned.
    let rc = unsafe { wait(f.fence, JOB_TIMEOUT_US) };
    g.waits += 1;
    if rc != 0 {
        glog_once!("yakogl/v3d: job {} did not complete ({})", f.kick, rc);
        if rc == -EFAULT {
            if g.ended.kick == f.kick {
                fault_record(g);
            } else {
                /* the record kept is the newest job's */
                glog_once!("yakogl/v3d: job {} MMU fault: its record was replaced by job {}'s", f.kick, g.ended.kick);
            }
        }
        return if rc < 0 { rc } else { -EIO };
    }
    0
}

/// Whether the OSAL has completed `fence` (without waiting). An OSAL that
/// cannot say keeps the job pending.
fn fence_done(fence: i64) -> bool {
    match osal::ops().and_then(|o| o.completed) {
        // SAFETY: a plain query.
        Some(c) => (unsafe { c() }) >= fence,
        None => false,
    }
}

/// The job that just ended with an MMU fault (-EFAULT): what the GPU could
/// have fetched a wild address from, as it is in memory now. Each texture
/// record against the bytes packed for it (overwritten after packing: its
/// base is garbage), with the image it should point at; each program's
/// code on the GPU against the CPU copy the backend checked (the GPU copy
/// overwritten: the QPU ran garbage). The fault's address is the OSAL's
/// first-failure line (vio_addr); a base or code range next to it names it.
fn fault_record(g: &mut V3dState) {
    let e = g.ended;
    let kick = e.kick;
    for i in 0..e.ntex {
        let (rec, image) = (e.tex_states[i], e.tex_images[i]);
        let packed = e.tex_packed[i];
        match cpu_of(rec as u64, V3D_TEX_STATE_BYTES as u32) {
            Some(p) => {
                // SAFETY: cpu_of confirmed V3D_TEX_STATE_BYTES at `p`.
                let now = unsafe { core::slice::from_raw_parts(p, V3D_TEX_STATE_BYTES) };
                match now.iter().zip(packed.iter()).position(|(a, b)| a != b) {
                    None => glog!(
                        "yakogl/v3d: job {} MMU fault: texture record 0x{:08x} as packed (image 0x{:08x})",
                        kick,
                        rec,
                        image
                    ),
                    Some(k) => glog!(
                        "yakogl/v3d: job {} MMU fault: texture record 0x{:08x} CHANGED from byte {}: now {:02x?}, \
                         packed {:02x?} (image 0x{:08x})",
                        kick,
                        rec,
                        k,
                        &now[k..(k + 8).min(V3D_TEX_STATE_BYTES)],
                        &packed[k..(k + 8).min(V3D_TEX_STATE_BYTES)],
                        image
                    ),
                }
            }
            None => glog!("yakogl/v3d: job {} MMU fault: texture record 0x{:08x} not readable", kick, rec),
        }
    }
    for &(address, stride, staged) in &e.attrs[..e.nattrs] {
        glog!(
            "yakogl/v3d: job {} MMU fault: vertex array 0x{:08x} stride {} ({})",
            kick,
            address,
            stride,
            if staged != 0 { "staged, see bytes" } else { "a buffer's" }
        );
        if staged != 0 {
            glog!(
                "yakogl/v3d: job {} MMU fault:   staged {} bytes: 0x{:08x}..0x{:08x}",
                kick,
                staged,
                address,
                address.wrapping_add(staged)
            );
        }
    }
    for &(stage, addr, list, n) in &e.streams[..e.nstreams] {
        stream_record(kick, stage, addr, list as *const V3dUniform, n);
    }
    for &(gpu, usc, dw) in &e.progs[..e.nprogs] {
        let bytes = dw as usize * 4;
        let Some(p) = cpu_of(gpu, bytes as u32) else {
            glog!("yakogl/v3d: job {} MMU fault: program code 0x{:08x} not readable", kick, gpu);
            continue;
        };
        // SAFETY: cpu_of confirmed `bytes` at `p`; `usc` is the GL core's
        // copy of the same dw words, live while its program is.
        let (on_gpu, on_cpu) = unsafe {
            (
                core::slice::from_raw_parts(p as *const u32, dw as usize),
                core::slice::from_raw_parts(usc as *const u32, dw as usize),
            )
        };
        match on_gpu.iter().zip(on_cpu.iter()).position(|(a, b)| a != b) {
            None => {
                glog!("yakogl/v3d: job {} MMU fault: program code 0x{:08x}+{} matches its CPU copy", kick, gpu, bytes)
            }
            Some(k) => glog!(
                "yakogl/v3d: job {} MMU fault: program code 0x{:08x}+{} DIFFERS from its CPU copy at word {}: \
                 gpu 0x{:08x} cpu 0x{:08x}",
                kick,
                gpu,
                bytes,
                k,
                on_gpu[k],
                on_cpu[k]
            ),
        }
    }
}

/// One uniform stream of a faulted job as it is in memory now: per word, its
/// index, the compiler's contents tag and data, and the word the QPU reads
/// (the P0/P1 words the TMU was configured with among them). `list` is the
/// program's uniform list, `n` words long.
fn stream_record(kick: u32, stage: u8, addr: u32, list: *const V3dUniform, n: u32) {
    let shown = n.min(STREAM_WORDS_SHOWN);
    glog!(
        "yakogl/v3d: job {} MMU fault: {} uniforms at 0x{:08x}, {} words{}",
        kick,
        stage as char,
        addr,
        n,
        if shown < n { " (the first shown)" } else { "" }
    );
    let Some(p) = cpu_of(addr as u64, shown * 4) else {
        glog!("yakogl/v3d: job {} MMU fault:   not readable", kick);
        return;
    };
    // SAFETY: cpu_of confirmed shown * 4 bytes at `p`; `list` is the
    // program's uniform list of `n` entries, live while its program is.
    let (words, tags) = unsafe {
        (
            core::slice::from_raw_parts(p as *const u32, shown as usize),
            core::slice::from_raw_parts(list, shown as usize),
        )
    };
    for (i, (w, u)) in words.iter().zip(tags).enumerate() {
        glog!(
            "yakogl/v3d: job {} MMU fault:   {}[{:2}] contents {:3} data 0x{:08x} -> 0x{:08x}",
            kick,
            stage as char,
            i,
            u.contents,
            u.data,
            w
        );
    }
}

/// Tell the OSAL whether a job that may reference GPU memory is being
/// recorded (OSAL_QUERY_V3D_JOB_OPEN): with none open it gives freed memory
/// back at once instead of holding it for the next job
/// (src/osal/v3d_frees.h). An OSAL without the query keeps holding.
fn job_open_to_osal(open: bool) {
    if let Some(q) = osal::ops().and_then(|o| o.query) {
        let mut v = open as u32;
        // SAFETY: a u32 in-parameter of the size passed.
        let _ = unsafe { q(OSAL_QUERY_V3D_JOB_OPEN, &mut v as *mut u32 as *mut c_void, 4) };
    }
}

/// The tile state the job itself uses: 256 bytes a tile of ITS tiling (the
/// size, the RT count, 4x), as Mesa sizes a job's TSDA (v3d_util.c
/// v3d_tile_alloc_sizes, `*tile_state_size = layers * tiles_x * tiles_y *
/// 256`; v3d_job.c alloc_tile_state, per job). The OSAL zeroes this much
/// before binning (zephyr_v3d.c). It was the whole allocation, which MRT
/// step 3 (227e5dec) sized for four RTs at 2048x2048 (4096 tiles, 1 MiB):
/// the cluster's single-RT 1920x720 job (360 tiles, 90 KiB) zeroed 1 MiB
/// a job, glue's DEMO15 "prep" 76 -> 307 us.
fn job_tile_state_bytes(t: &V3dJobTarget, max_bpp: u32) -> u32 {
    let mut tiling = V3dTiling::default();
    // SAFETY: plain computations on the out-parameter given.
    unsafe { v3d_frame_tiling_bpp(t.width, t.height, t.rts.max(1), t.msaa, max_bpp, &mut tiling) };
    tiling.tiles_x * tiling.tiles_y * 256
}

/// Submit the job just ended and let it run (`in_flight`; the next job
/// records into the other arena set): 0, or the OSAL's error.
fn submit_job(g: &mut V3dState) -> i32 {
    let j = &mut g.j;
    // SAFETY: the bytes written are inside the CPU mappings.
    unsafe {
        osal::cache_flush(g.arenas.bcl.cpu, j.bcl.used as usize);
        osal::cache_flush(g.arenas.rcl.cpu, j.rcl.used as usize);
        osal::cache_flush(g.arenas.ind.cpu, j.ind.used as usize);
        osal::cache_flush(g.arenas.data.cpu, j.data_used);
        osal::cache_flush(g.arenas.tex.cpu, j.tex_used);
    }
    let kick = V3dKick {
        magic: V3D_KICK_MAGIC,
        bcl_start: j.bcl.gpu,
        bcl_end: j.bcl.gpu + j.bcl.used,
        rcl_start: j.rcl.gpu,
        rcl_end: j.rcl.gpu + j.rcl.used,
        tile_alloc: g.tile_alloc.gpu as u32,
        tile_alloc_size: g.tile_alloc_size,
        tile_state: g.tile_state.gpu as u32,
        tile_state_size: job_tile_state_bytes(&j.job.t, j.max_bpp).min(g.tile_state_size),
        pad: 0,
    };

    /* the history the next job on this target starts from: the buffer's,
     * or the target's when it has none */
    let target = g.j.target.gpu;
    if let (None, Some(si)) = (g.j.zs, g.j.ss) {
        /* a stencil half alone (S8): as each half apart below */
        let j = &g.j;
        let s = if j.zs_discard {
            ZsKnown::Untouched
        } else if j.clear.s_store {
            ZsKnown::Stored
        } else {
            g.zsbufs[si].s
        };
        (g.zsbufs[si].z, g.zsbufs[si].s) = (ZsKnown::Untouched, s);
    } else if let (Some(zi), Some(si)) = (g.j.zs, g.j.ss) {
        /* apart: each half's history in its own buffer; the other half of
         * each buffer is not kept (Untouched: any value is right) */
        let j = &g.j;
        let half = |now: ZsKnown, stored: bool| {
            if j.zs_discard {
                ZsKnown::Untouched
            } else if stored {
                ZsKnown::Stored
            } else {
                now
            }
        };
        let (z, s) = (half(g.zsbufs[zi].z, j.clear.zs_store), half(g.zsbufs[si].s, j.clear.s_store));
        (g.zsbufs[zi].z, g.zsbufs[zi].s) = (z, ZsKnown::Untouched);
        (g.zsbufs[si].z, g.zsbufs[si].s) = (ZsKnown::Untouched, s);
    } else {
        let (mut z, mut s) = match g.j.zs {
            Some(i) => (g.zsbufs[i].z, g.zsbufs[i].s),
            None => zs_known(g, target),
        };
        let j = &g.j;
        if j.zs_discard {
            /* posted: undefined from now on (EGL 1.5 3.10.1) */
            (z, s) = (ZsKnown::Untouched, ZsKnown::Untouched);
        } else if j.clear.zs_store {
            /* The store writes both halves, but each plane holds contents
             * GL keeps only as far as this job gave it some: written, it is
             * Stored; cleared whole and not written, it is the clear value
             * (the tile held it, so the buffer does too); neither, it keeps
             * what it was. An Untouched plane stays Untouched (the store
             * wrote whatever the tile started from, which nothing may read),
             * so a stencil-only framebuffer's buffer never counts its depth
             * as contents. A Cleared plane the job loaded from the buffer
             * was stored there with its clear value: a whole clear of one
             * half by the TLB happens only in a job that does not load the
             * other (GFXH-1461, `record`), so the store after it wrote the
             * value. */
            let plane = |before: ZsKnown, written: bool, cleared: Option<f32>| match (written, cleared) {
                (true, _) => ZsKnown::Stored,
                (false, Some(v)) => ZsKnown::Cleared(v),
                (false, None) => before,
            };
            z = plane(z, j.z_written, j.z_cleared.then_some(j.clear.z));
            s = plane(s, j.s_written, j.s_cleared.then_some(j.clear.stencil as f32));
        } else {
            if j.z_written {
                z = ZsKnown::Written;
            } else if j.z_cleared {
                z = ZsKnown::Cleared(j.clear.z);
            }
            if j.s_written {
                s = ZsKnown::Written;
            } else if j.s_cleared {
                s = ZsKnown::Cleared(j.clear.stencil as f32);
            }
        }
        match g.j.zs {
            Some(i) => (g.zsbufs[i].z, g.zsbufs[i].s) = (z, s),
            None => zs_record(g, target, z, s),
        }
    }

    let Some(submit) = osal::ops().and_then(|o| o.submit) else {
        return -ENODEV;
    };
    // SAFETY: `kick` is a live V3dKick of the size passed.
    let fence = unsafe { submit(&kick as *const V3dKick as *const c_void, core::mem::size_of::<V3dKick>()) };
    if fence < 0 {
        glog_once!("yakogl/v3d: submit failed ({})", fence);
        return -EIO;
    }
    g.kicks += 1;
    g.arenas.kick = g.kicks;
    g.ended = JobRecord::of(&g.j, g.kicks);
    g.queued = g.in_flight.take();
    g.in_flight = Some(InFlight { kick: g.kicks, fence, alias: 0 });
    0
}

/// Copy `n` bytes the GL core staged for this draw into the job's data
/// arena: the GPU address, or `None` when the arena is full. A draw recorded
/// into a job begun for it alone (`spill_ok`) whose data the arena cannot
/// take gets GPU memory of its own for the piece instead: dEQP
/// texture.vertex.2d stages 1,225,804 bytes for one draw (a 127x127 grid of
/// client arrays), which no job's arena held, and was refused (-E2BIG) as
/// Mesa's upload manager never refuses.
fn stage(g: &mut V3dState, src: *const c_void, n: usize) -> Option<u32> {
    let at = g.j.data_used.next_multiple_of(16);
    if n == 0 {
        return None;
    }
    if at + n > DATA_BYTES {
        return if g.spill_ok { spill(g, src, n) } else { None };
    }
    // SAFETY: the arena's CPU mapping is DATA_BYTES long and `at + n` fits;
    // the GL core's staging holds `n` bytes at `src`.
    unsafe { core::ptr::copy_nonoverlapping(src as *const u8, (g.arenas.data.cpu as *mut u8).add(at), n) };
    g.j.data_used = at + n;
    gpu32(g.arenas.data.gpu + at as u64)
}

/// `n` bytes of a draw's client data in GPU memory of their own, kept with
/// the open job (`Job::spills`): the GPU address, or `None` when the job
/// holds JOB_SPILLS pieces already or the memory is not there.
fn spill(g: &mut V3dState, src: *const c_void, n: usize) -> Option<u32> {
    if g.j.nspills >= JOB_SPILLS {
        return None;
    }
    let mut mem = OsalGpuMem::ZERO;
    if osal::gpu_alloc(V3D_HEAP_DATA, n, OSAL_MEM_DATA, &mut mem) != 0 || mem.cpu.is_null() {
        glog_once!("yakogl/v3d: no GPU memory for {} bytes of a draw's client data", n);
        return None;
    }
    let Some(gpu) = gpu32(mem.gpu) else {
        osal::gpu_free(&mut mem);
        return None;
    };
    // SAFETY: the allocation's CPU view holds `n` bytes; the GL core's
    // staging holds `n` bytes at `src`.
    unsafe {
        core::ptr::copy_nonoverlapping(src as *const u8, mem.cpu as *mut u8, n);
        osal::cache_flush(mem.cpu, n);
    }
    if g.spills_made == 0 {
        glog!(
            "yakogl/v3d: a draw's client data outgrew the job's data arena ({} bytes): staged in GPU memory of its own",
            n
        );
    }
    g.spills_made = g.spills_made.wrapping_add(1);
    g.j.spills[g.j.nspills] = mem;
    g.j.nspills += 1;
    Some(gpu)
}

/// Give back the job's spilled client data from piece `from` on: after the
/// job has run, or for a draw or a job that is dropped (nothing references
/// it then; the OSAL holds a free while a job is open anyway).
fn free_spills(g: &mut V3dState, from: usize) {
    for mem in &mut g.j.spills[from..g.j.nspills] {
        osal::gpu_free(mem);
    }
    g.j.nspills = from.min(g.j.nspills);
}

/// A GPU address the V3D's 32-bit MMU can hold (0 is never mapped).
fn gpu32(a: u64) -> Option<u32> {
    u32::try_from(a).ok().filter(|&v| v != 0)
}

/* ---- the programs' code, checked before the GPU runs it -------------------- */

/// A program whose code was checked (`check_program`): the code's CPU address
/// and length, and a checksum of its words. A key equal in all three is the
/// same code.
#[derive(Clone, Copy, PartialEq)]
struct ProgKey {
    usc: usize,
    dw: u32,
    sum: u32,
}

impl ProgKey {
    const NONE: ProgKey = ProgKey { usc: 0, dw: 0, sum: 0 };
}

/// The programs remembered as checked.
const PROG_OK: usize = 32;

/// What is wrong with one stage's code.
#[derive(Clone, Copy, Debug, PartialEq)]
enum CodeFault {
    /// Fewer than the three words a program end needs, or outside the code.
    Short,
    /// A word the QPU decoder does not know.
    Undecodable,
    /// The third-to-last word is not the program-end thread switch, or one
    /// of its two delay slots switches again: the program would not end
    /// where the compiler ends it (v3d-m5: sched.rs pads to k + 3).
    NoEnd,
    /// The words that consume a uniform are not as many as the stream holds.
    Uniforms,
}

/// The QPU instruction at word `k` of `w` (little-endian pairs of u32).
fn qpu_word(w: &[u32], k: usize) -> u64 {
    w[2 * k] as u64 | (w[2 * k + 1] as u64) << 32
}

/// One stage's code (`words`, whole instructions) against the compiler's
/// contract (v3d-m5, 2026-09-24): the program-end THRSW on the third-to-last
/// word and none on the two after it; exactly `num_uniforms` words that
/// consume a uniform: ldunif, ldunifrf, wrtmuc, a TLBU write (magic waddr 8)
/// on either unit, and a branch that moves the stream (ub). The compiler
/// emits no ldunifa/ldunifarf, so nothing else reads the stream.
fn check_stage(words: &[u32], num_uniforms: u32) -> Result<(), CodeFault> {
    let n = words.len() / 2;
    if n < 3 {
        return Err(CodeFault::Short);
    }
    let mut consumers = 0u32;
    for k in 0..n {
        let i = qpu_decode(qpu_word(words, k)).map_err(|_| CodeFault::Undecodable)?;
        let thrsw = matches!(i, QpuInstr::Alu(a) if a.sig.thrsw);
        if (k == n - 3 && !thrsw) || (k > n - 3 && thrsw) {
            return Err(CodeFault::NoEnd);
        }
        consumers += match i {
            QpuInstr::Alu(a) => {
                (a.sig.ldunif
                    || a.sig.ldunifrf
                    || a.sig.wrtmuc
                    || (a.add.magic && a.add.waddr == 8)
                    || (a.mul.magic && a.mul.waddr == 8)) as u32
            }
            QpuInstr::Branch(b) => b.ub as u32,
        };
    }
    if consumers != num_uniforms {
        return Err(CodeFault::Uniforms);
    }
    Ok(())
}

/// The refusal for `stage` (0 FS, 1 VS, 2 CS) and `f`: one string each, so
/// the log names both.
fn code_refusal(stage: usize, f: CodeFault) -> state::Refusal {
    const R: [[state::Refusal; 4]; 3] = [
        [
            "the fragment shader's code is shorter than a program end",
            "the fragment shader's code holds a word the QPU decoder does not know",
            "the fragment shader's code does not end with the program-end thread switch",
            "the fragment shader's uniform stream is not as long as its code reads",
        ],
        [
            "the vertex shader's code is shorter than a program end",
            "the vertex shader's code holds a word the QPU decoder does not know",
            "the vertex shader's code does not end with the program-end thread switch",
            "the vertex shader's uniform stream is not as long as its code reads",
        ],
        [
            "the coordinate shader's code is shorter than a program end",
            "the coordinate shader's code holds a word the QPU decoder does not know",
            "the coordinate shader's code does not end with the program-end thread switch",
            "the coordinate shader's uniform stream is not as long as its code reads",
        ],
    ];
    R[stage][f as usize]
}

/// The linked program's three stages checked (`check_stage`) before any job
/// runs them: a program that does not end, or reads more uniforms than its
/// stream holds, would stall the QPU or send the TMU to a wild address (the
/// E runs' 1 s draws, 2026-09-24). Each code is decoded once: a program that
/// passed is remembered by its code's address, length and checksum.
fn check_program(g: &mut V3dState, sh: &ShcShader) -> Result<(), state::Refusal> {
    if sh.usc.is_null() || sh.usc_dw == 0 {
        return Err("the program's code has no CPU copy to check");
    }
    // SAFETY: the GL core's copy of the linked code, usc_dw words, live for
    // the draw.
    let all = unsafe { core::slice::from_raw_parts(sh.usc, sh.usc_dw as usize) };
    let sum = all.iter().fold(0x811c_9dc5u32, |h, &w| (h ^ w).wrapping_mul(0x0100_0193));
    let key = ProgKey { usc: sh.usc as usize, dw: sh.usc_dw, sum };
    if g.prog_ok.contains(&key) {
        return Ok(());
    }
    let p = &sh.v3d;
    let stages = [
        (p.fs.code_off, p.fs.code_bytes, p.fs.num_uniforms),
        (p.vs.code_off, p.vs.code_bytes, p.vs.num_uniforms),
        (p.cs.code_off, p.cs.code_bytes, p.cs.num_uniforms),
    ];
    for (stage, &(off, bytes, nu)) in stages.iter().enumerate() {
        let (a, b) = ((off / 4) as usize, ((off as u64 + bytes as u64) / 4) as usize);
        if !off.is_multiple_of(8) || !bytes.is_multiple_of(8) || b > all.len() {
            return Err(code_refusal(stage, CodeFault::Short));
        }
        check_stage(&all[a..b], nu).map_err(|f| code_refusal(stage, f))?;
    }
    g.prog_ok[g.prog_next % PROG_OK] = key;
    g.prog_next = (g.prog_next + 1) % PROG_OK;
    Ok(())
}

/// The three programs of the linked fragment shader, at their code addresses.
fn programs(d: &GlesDraw, fs: &ShcShader) -> Result<(V3dFsProg, V3dVsProg, V3dVsProg), state::Refusal> {
    let p = &fs.v3d;
    if p.linked != 1 {
        return Err("the fragment shader is not a program linked for the V3D");
    }
    let bytes = fs.usc_dw as u64 * 4;
    let code = |off: u32, len: u32| -> Result<u32, state::Refusal> {
        if !off.is_multiple_of(8) || len == 0 || off as u64 + len as u64 > bytes {
            return Err("a program's code lies outside the uploaded code");
        }
        gpu32(d.fs_usc_gpu + off as u64).ok_or("the program's code is outside the V3D's 32-bit address space")
    };
    let vs = |h: &ShcV3dVs| -> Result<V3dVsProg, state::Refusal> {
        Ok(V3dVsProg {
            code: code(h.code_off, h.code_bytes)?,
            threads: h.threads as u32,
            single_seg: h.single_seg != 0,
            vpm_input_size: h.vpm_input_size,
            vpm_output_size: h.vpm_output_size,
            separate_segments: h.separate_segments != 0,
            vcm_cache_size: h.vcm_cache_size,
            uses_vid: h.uses_vid != 0,
            uses_iid: h.uses_iid != 0,
            uses_biid: h.uses_biid != 0,
            vattr_sizes: h.vattr_sizes,
            uniforms: h.uniforms as *const V3dUniform,
            num_uniforms: h.num_uniforms,
        })
    };
    let f = &p.fs;
    let fsp = V3dFsProg {
        code: code(f.code_off, f.code_bytes)?,
        threads: f.threads as u32,
        single_seg: f.single_seg != 0,
        num_inputs: f.num_inputs,
        flat_shade_flags: f.flat_shade_flags,
        noperspective_flags: f.noperspective_flags,
        centroid_flags: f.centroid_flags,
        writes_z: f.writes_z != 0,
        disable_ez: f.disable_ez != 0,
        uses_center_w: f.uses_center_w != 0,
        lock_scoreboard_on_first_thrsw: f.lock_scoreboard_on_first_thrsw != 0,
        uses_implicit_point_line_varyings: f.uses_implicit_point_line_varyings != 0,
        uses_pid: f.uses_pid != 0,
        uniforms: f.uniforms as *const V3dUniform,
        num_uniforms: f.num_uniforms,
    };
    /* a program reading gl_PointCoord takes two implicit varyings before
     * its declared ones; only points have them (agreed with v3d-m5 and the
     * lead, 2026-09-24: points only), so any other primitive would read
     * every declared varying two slots off. One string per primitive: the
     * log names what it saw. */
    if fsp.uses_implicit_point_line_varyings && d.prim != GLES_PRIM_POINTS {
        return Err(match d.prim {
            GLES_PRIM_LINES => "a program reading gl_PointCoord drawn as GL_LINES",
            GLES_PRIM_LINE_STRIP => "a program reading gl_PointCoord drawn as GL_LINE_STRIP",
            GLES_PRIM_LINE_LOOP => "a program reading gl_PointCoord drawn as GL_LINE_LOOP",
            GLES_PRIM_TRIANGLES => "a program reading gl_PointCoord drawn as GL_TRIANGLES",
            GLES_PRIM_TRI_STRIP => "a program reading gl_PointCoord drawn as GL_TRIANGLE_STRIP",
            GLES_PRIM_TRI_FAN => "a program reading gl_PointCoord drawn as GL_TRIANGLE_FAN",
            _ => "a program reading gl_PointCoord drawn as a primitive other than points",
        });
    }
    Ok((fsp, vs(&p.vs)?, vs(&p.cs)?))
}

/// One stage's GL uniform block, as the job builder reads it.
fn storage(cpu: *const c_void, dwords: u16) -> V3dUniformStorage {
    if cpu.is_null() || dwords == 0 {
        V3dUniformStorage::NONE
    } else {
        V3dUniformStorage { words: cpu as *const u32, count: dwords as u32 }
    }
}

/// Why one attempt at recording a draw did not record it.
enum Attempt {
    /// The job or its data arena is full: end the job, try in a new one.
    Full,
    /// The OSAL had no memory for a kept tiled copy the draw cannot do
    /// without (larger than the texture arena). The open job may hold that
    /// memory: the OSAL gives a free made while a job is open back only
    /// after that job has run (src/osal/v3d_frees.h), and the eviction that
    /// made room for this copy freed a copy the open job samples (T5 on
    /// 68b6df6a: a 16,781,312-byte copy held for the open job, and 22,376,448
    /// bytes fitting only with it given back). End the job, try once in a
    /// new one; there the draw is refused with the reason.
    NoMemory(state::Refusal),
    /// Not drawn, with the reason.
    Refused(state::Refusal),
}

/// Record the draw into the open job (the arrays staged, the job builder
/// called). Nothing is left in the job when it fails: the job builder rolls
/// its lists back, and the data arena returns to where it stood.
fn record(g: &mut V3dState, d: &GlesDraw, st: &V3dJobState, fs: &ShcShader) -> Result<(), Attempt> {
    let (reads_z, reads_s, writes_z, writes_s) = state::zs_use(d);
    if reads_z && !g.j.z_valid {
        return Err(Attempt::Refused(
            "the draw reads depth an earlier job wrote, and the target has no depth/stencil buffer to keep it in",
        ));
    }
    if reads_s && !g.j.s_valid {
        return Err(Attempt::Refused(
            "the draw reads stencil an earlier job wrote, and the target has no depth/stencil buffer to keep it in",
        ));
    }
    let (fsp, vsp, csp) = programs(d, fs).map_err(Attempt::Refused)?;
    check_program(g, fs).map_err(Attempt::Refused)?;
    /* remembered for the MMU-fault record */
    let prog = (d.fs_usc_gpu, fs.usc as usize, fs.usc_dw);
    if !g.j.progs[..g.j.nprogs].contains(&prog) && g.j.nprogs < JOB_PROGS {
        g.j.progs[g.j.nprogs] = prog;
        g.j.nprogs += 1;
    }
    let (data_mark, tex_mark, ntex_mark, spill_mark) = (g.j.data_used, g.j.tex_used, g.j.ntex, g.j.nspills);
    let (mut fs_tmu, mut vs_tmu) = ([V3dJobTmu::default(); MAX_UNITS], [V3dJobTmu::default(); MAX_UNITS]);
    let fs_binds = &d.textures[..(d.num_textures as usize).min(GLES_MAX_TEX_BINDS)];
    let vs_binds = &d.vs_textures[..(d.num_vs_textures as usize).min(GLES_MAX_VS_TEX_BINDS)];
    let fs_streams = [(fsp.uniforms, fsp.num_uniforms)];
    let vs_streams = [(vsp.uniforms, vsp.num_uniforms), (csp.uniforms, csp.num_uniforms)];
    /* which units are cubes: the FS's from the linked program's uniform
     * table, the VS's (and so the CS's) from the vertex shader's reflection
     * (GlesDraw::vs), whose sampler bindings the VS streams' P0/P1 name */
    let fs_cubes = cube_units_of(fs);
    // SAFETY: the GL core's vertex-shader reflection, live for the draw, or null.
    let vs_cubes = unsafe { d.vs.as_ref() }.map_or(0, cube_units_of);
    let result = texture_units(g, fs_binds, &fs_streams, fs_cubes, &mut fs_tmu).and_then(|nf| {
        let nv = texture_units(g, vs_binds, &vs_streams, vs_cubes, &mut vs_tmu)?;
        record_arrays_and_draw(g, d, st, fs, (&fsp, &vsp, &csp), (&fs_tmu[..nf], &vs_tmu[..nv]))
    });
    match result {
        Ok(()) => {
            g.j.z_written |= writes_z;
            g.j.s_written |= writes_s;
            Ok(())
        }
        Err(e) => {
            (g.j.data_used, g.j.tex_used, g.j.ntex) = (data_mark, tex_mark, ntex_mark);
            free_spills(g, spill_mark);
            Err(e)
        }
    }
}

fn record_arrays_and_draw(
    g: &mut V3dState,
    d: &GlesDraw,
    st: &V3dJobState,
    fs: &ShcShader,
    progs: (&V3dFsProg, &V3dVsProg, &V3dVsProg),
    (tmu, vs_tmu): (&[V3dJobTmu], &[V3dJobTmu]),
) -> Result<(), Attempt> {
    let v3d = &fs.v3d;
    /* one instance of an instanced draw the GL core draws instance by
     * instance (a divisor past 0xffff): the hardware's instance ID would be
     * 0 for it; a program that reads gl_InstanceID is refused by name
     * (VERTEX_ARRAY_SINGLE_INSTANCE_PRIMS and SET_INSTANCEID are not
     * measured on the board) */
    if d.instance_id != 0 && v3d.vs.uses_iid != 0 {
        return Err(Attempt::Refused("an instance drawn on its own whose program reads gl_InstanceID"));
    }
    /* the arrays, one per attribute record */
    let mut attrs = [V3dVertexAttr::default(); V3D_MAX_ATTRS];
    let mut filled = 0u32;
    for a in &d.attribs[..(d.num_attribs as usize).min(d.attribs.len())] {
        let Some(&rec) = v3d.record_of_location.get(a.vi_reg as usize) else {
            return Err(Attempt::Refused("an attribute location past 15"));
        };
        if rec == SHC_V3D_NO_RECORD {
            continue; /* the program does not read this location */
        }
        if a.format != GLES_ATTRIBF_FLOAT32 {
            return Err(Attempt::Refused("an attribute format other than float32 reached the backend"));
        }
        let address = if !a.cpu.is_null() {
            stage(g, a.cpu, a.cpu_bytes as usize).ok_or(Attempt::Full)?
        } else {
            gpu32(a.buffer_gpu + a.offset as u64)
                .ok_or(Attempt::Refused("a vertex array outside the V3D's 32-bit address space"))?
        };
        /* an ES 3.0 per-instance array: element i / divisor for instance i,
         * the record's Instance Divisor (the GL core passes divisors up to
         * this backend's max_instance_divisor, 0xffff; GlesAttrib.divisor 0
         * from an older producer means 1). In a draw of one instance every
         * vertex reads element 0, as GL wants. */
        let divisor = if a.per_instance != 0 { u32::from(a.divisor.max(1)) } else { 0 };
        attrs[rec as usize] = V3dVertexAttr { address, stride: a.stride, components: a.components as u32, divisor };
        filled |= 1 << rec;
        /* remembered for the MMU-fault record */
        let seen = (address, a.stride, if a.cpu.is_null() { 0 } else { a.cpu_bytes });
        if !g.j.attrs[..g.j.nattrs].contains(&seen) && g.j.nattrs < JOB_ATTRS {
            g.j.attrs[g.j.nattrs] = seen;
            g.j.nattrs += 1;
        }
    }
    if filled != (1u32 << v3d.num_records) - 1 {
        return Err(Attempt::Refused("an attribute the program reads has no array"));
    }

    /* the indices */
    let (mut indexed, mut index_type, mut index_buffer, mut index_bytes) = (false, 0, 0, 0);
    if d.index_buf_gpu != 0 || !d.index_cpu.is_null() {
        indexed = true;
        index_type = match d.index_size {
            1 => V3D_INDEX_TYPE_8,
            2 => V3D_INDEX_TYPE_16,
            4 => V3D_INDEX_TYPE_32,
            _ => return Err(Attempt::Refused("an index size other than 1, 2 or 4 bytes")),
        };
        let need = d.index_count as usize * d.index_size as usize;
        if !d.index_cpu.is_null() {
            if (d.index_bytes as usize) < need {
                return Err(Attempt::Refused("the staged indices are shorter than index_count"));
            }
            index_buffer = stage(g, d.index_cpu, d.index_bytes as usize).ok_or(Attempt::Full)?;
            index_bytes = d.index_bytes;
        } else {
            index_buffer = gpu32(d.index_buf_gpu)
                .ok_or(Attempt::Refused("an index buffer outside the V3D's 32-bit address space"))?;
            index_bytes = u32::try_from(need).map_err(|_| Attempt::Refused("an index list too long to address"))?;
        }
    }

    let jd = V3dJobDraw {
        st,
        fs: progs.0,
        vs: progs.1,
        cs: progs.2,
        vs_storage: storage(d.vs_uniform_cpu, d.vs_uniform_dwords),
        fs_storage: storage(d.fs_uniform_cpu, d.fs_uniform_dwords),
        attrs: attrs.as_ptr(),
        num_attrs: v3d.num_records as u32,
        prim: state::prim(d.prim).ok_or(Attempt::Refused("an unknown primitive"))?,
        count: d.index_count,
        first: 0, /* the GL core puts the first vertex into the array offsets */
        indexed,
        index_type,
        index_buffer,
        index_buffer_bytes: index_bytes,
        index_offset: 0,
        tmu: tmu.as_ptr(),
        num_tmu: tmu.len() as u32,
        vs_tmu: vs_tmu.as_ptr(),
        num_vs_tmu: vs_tmu.len() as u32,
        vs_writes_psiz: v3d.vs.writes_psiz != 0,
        instances: d.instance_count,
    };
    let mut lo = V3dJobDrawLayout::ZERO;
    // SAFETY: the job is open on live lists; every pointer in `jd` is to a
    // local or to memory the GL core keeps for the call; `lo` is a live
    // struct v3d_job_draw_layout (ffi_layout_check.c).
    match unsafe { v3d_job_draw(&mut g.j.job, &jd, &mut lo as *mut V3dJobDrawLayout as *mut c_void) } {
        V3D_JOB_OK => {
            g.psiz_draws = g.psiz_draws.wrapping_add(jd.vs_writes_psiz as u32);
            /* remembered for the MMU-fault record */
            for (stage, addr, p) in [
                (b'F', lo.fs_uniforms, (progs.0.uniforms as usize, progs.0.num_uniforms)),
                (b'V', lo.vs_uniforms, (progs.1.uniforms as usize, progs.1.num_uniforms)),
                (b'C', lo.cs_uniforms, (progs.2.uniforms as usize, progs.2.num_uniforms)),
            ] {
                if g.j.nstreams < JOB_STREAMS {
                    g.j.streams[g.j.nstreams] = (stage, addr, p.0, p.1);
                    g.j.nstreams += 1;
                }
            }
            Ok(())
        }
        V3D_JOB_FULL => Err(Attempt::Full),
        _ => Err(Attempt::Refused(
            "the job builder refused the draw (a uniform, program or primitive it does not write)",
        )),
    }
}

/// The clear program a quad draws with: the integer one (`CLEAR_FS_INT`,
/// built here the first time it is asked for) for a quad that writes an
/// integer colour target, otherwise the float one `v3d_init` built. `Err` is
/// the refusal every clear quad of that kind gets when it did not build; the
/// build logged the chain's error, once.
fn clear_program(g: &mut V3dState, integer: bool) -> Result<state::ClearProg<'static>, state::Refusal> {
    if integer && g.clear_program_int.status == ClearStatus::NotBuilt {
        g.clear_program_int = made_clear_program("integer ", CLEAR_VS_INT, CLEAR_FS_INT);
    }
    let p = if integer { &g.clear_program_int } else { &g.clear_program };
    if p.status != ClearStatus::Built {
        return Err(if integer {
            "the integer clear program did not build, so a clear the TLB cannot do of an integer target is not drawn"
        } else {
            "the clear program did not build, so a clear the TLB cannot do is not drawn"
        });
    }
    // SAFETY: the program lives in the backend's static state until shutdown,
    // which no draw outlives.
    let fs: &'static ShcShader = unsafe { &*(&p.fs as *const ShcShader) };
    Ok(state::ClearProg { fs, code_gpu: p.code.gpu, location: p.location })
}

/// The text of a compiler log (NUL-terminated in `log`).
fn log_text(log: &[core::ffi::c_char]) -> &str {
    // SAFETY: c_char and u8 have the same size and alignment.
    let bytes = unsafe { core::slice::from_raw_parts(log.as_ptr().cast::<u8>(), log.len()) };
    let n = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    core::str::from_utf8(&bytes[..n]).unwrap_or("(the log is not UTF-8)")
}

/// Build one clear program and say once what happened: its code size, or
/// the stage that failed and the compiler's own error. `what` prefixes the
/// line ("" or "integer ").
fn made_clear_program(what: &str, vs: &[u8], fs: &[u8]) -> ClearProgram {
    use core::ffi::c_char;
    let mut log = [0 as c_char; 256];
    match build_clear_program(&mut log, vs, fs) {
        Ok(p) => {
            glog!("yakogl/v3d: {}clear program built: {} bytes of code", what, p.fs.usc_dw * 4);
            p
        }
        Err(why) => {
            glog!("yakogl/v3d: {}clear program did not build: {}: {}", what, why, log_text(&log));
            ClearProgram::FAILED
        }
    }
}

/// Build the clear program at init. The integer one is built on the first
/// quad that needs it (`clear_program`), so a back end that never clears an
/// integer target part-way spends neither the compile nor its code memory.
fn init_clear_program(g: &mut V3dState) {
    g.clear_program = made_clear_program("", CLEAR_VS, CLEAR_FS);
}

/// Compile and link a clear program (`vs`, `fs`: NUL-terminated sources)
/// through the shader back end, as the GL core compiles an application's,
/// and upload its code. The compiler's error, when there is one, is left in
/// `log`.
fn build_clear_program(
    log: &mut [core::ffi::c_char; 256],
    vs_src: &[u8],
    fs_src: &[u8],
) -> Result<ClearProgram, &'static str> {
    use core::ffi::c_char;
    let be = crate::shaderc::backend().ok_or("no shader back end")?;
    let (compile, link) = match (be.compile, be.link) {
        (Some(c), Some(l)) => (c, l),
        _ => return Err("the shader back end cannot compile and link"),
    };
    let (mut vs, mut fs) = (ShcShader::ZERO, ShcShader::ZERO);
    // SAFETY: NUL-terminated sources, live out-parameters, a log of the size
    // given.
    let rc =
        unsafe { compile(SHC_STAGE_VERTEX, vs_src.as_ptr() as *const c_char, &mut vs, log.as_mut_ptr(), log.len()) };
    if rc != 0 {
        return Err("the vertex shader did not compile");
    }
    // SAFETY: as above.
    let rc =
        unsafe { compile(SHC_STAGE_FRAGMENT, fs_src.as_ptr() as *const c_char, &mut fs, log.as_mut_ptr(), log.len()) };
    if rc != 0 {
        crate::shaderc::release_shader(&vs);
        return Err("the fragment shader did not compile");
    }
    // SAFETY: two compiled shaders; `fs` becomes the linked program.
    let rc = unsafe { link(&vs, &mut fs, log.as_mut_ptr(), log.len()) };
    crate::shaderc::release_shader(&vs);
    if rc == 0 {
        log[0] = 0; /* what fails below is ours, not the compiler's */
    }
    let give_back = |fs: &ShcShader| crate::shaderc::release_shader(fs);
    if rc != 0 || fs.v3d.linked != 1 {
        give_back(&fs);
        return Err("the program did not link");
    }
    /* The quad hands the colour as a four-dword fragment block; the program
     * must read it from dwords 0..3 (the job builder refuses a uniform past
     * the block, so a different layout could not draw a wrong colour either,
     * but it would refuse every quad). */
    let at_zero = fs
        .uniform_table()
        .iter()
        .any(|u| crate::util::name_bytes(&u.name) == b"u_color" && u.dword_off == 0 && u.dwords == 4);
    if !at_zero {
        give_back(&fs);
        return Err("u_color is not the fragment block's dwords 0..3");
    }
    let rec = fs.v3d.record_of_location;
    let location = match rec.iter().position(|&r| r != SHC_V3D_NO_RECORD) {
        Some(l) if fs.v3d.num_records == 1 => l as u8,
        _ => {
            give_back(&fs);
            return Err("the program does not read exactly one attribute");
        }
    };
    let bytes = fs.usc_dw as usize * 4;
    let mut code = OsalGpuMem::ZERO;
    if bytes == 0
        || osal::gpu_alloc(V3D_HEAP_SHADER, bytes, OSAL_MEM_DATA, &mut code) != 0
        || code.cpu.is_null()
        || gpu32(code.gpu).is_none()
    {
        give_back(&fs);
        return Err("no GPU memory for its code");
    }
    // SAFETY: the allocation is `bytes` long, and the program's code is
    // `usc_dw` dwords.
    unsafe {
        core::ptr::copy_nonoverlapping(fs.usc as *const u8, code.cpu as *mut u8, bytes);
        osal::cache_flush(code.cpu, bytes);
    }
    Ok(ClearProgram { status: ClearStatus::Built, fs, code, location })
}

/* ---- textures ------------------------------------------------------------- */

/// Where a tiling's CPU time is counted (`V3dState::tile_ns`): a kept copy
/// made at an upload from the caller's rows (`v3d_texture_uploaded`, inside
/// glTexImage2D/glTexSubImage2D), a kept copy made at a draw from the GL
/// core's storage, and a copy tiled into the job's arena at a draw (the
/// fallback when no kept copy could be made).
const TILE_AT_UPLOAD: usize = 0;
const TILE_KEPT_AT_DRAW: usize = 1;
const TILE_ARENA: usize = 2;
const TILE_WHERE: usize = 3;

/// What a TFU tiling came to (`tfu_tile`).
#[derive(Clone, Copy, PartialEq, Debug)]
enum Tfu {
    /// Not a TFU job: the TFU is off, the image is small, or the texture
    /// is one the TFU path does not take.
    NotTried,
    Made,
    /// Asked and not made (the OSAL's error): the CPU tiles it.
    Failed,
}

/// Tile level 0 of `b` from the GL core's storage into the kept copy at
/// `image_gpu` with one TFU job (OSAL_QUERY_V3D_TFU_RUN). Only a one-layer,
/// one-level image of at least `min` bytes (0: never) of 1, 2 or 4 bytes a
/// texel, moved as a byte copy of that width (R8, R16F, R32F: Mesa's blit
/// types, v3dx_tfu.c:63-73; gl_test case 140 measured each byte-exact with
/// arbitrary bytes, so the texture's channel order does not matter). The
/// storage's rows are tex_pitch texels apart and its base 4-byte aligned or
/// the job is not packed (v3d_tex_tfu_pack). The TFU writes the whole padded
/// slice, which the kept copy's image holds.
fn tfu_tile(min: usize, b: &GlesTextureBind, layout: &V3dTexLayout, image_gpu: u32) -> Tfu {
    let ttype = match texture::texel_bytes(b) {
        1 => V3D_TFU_TTYPE_R8,
        2 => V3D_TFU_TTYPE_R16F,
        4 => V3D_TFU_TTYPE_R32F,
        _ => return Tfu::NotTried,
    };
    if min == 0 || (layout.size as usize) < min || layout.levels != 1 || texture::layers(b) != 1 {
        return Tfu::NotTried;
    }
    let Some(src) = gpu32(b.base_gpu) else {
        return Tfu::NotTried;
    };
    let bpp = texture::texel_bytes(b);
    let (_, pitch, _) = texture::gl_level_layered(b.width, b.height, 1, bpp, 0);
    let mut j = V3dTexTfuJob::default();
    // SAFETY: a live layout and job.
    if !unsafe { v3d_tex_tfu_pack(layout, ttype, src, pitch * bpp, image_gpu, &mut j) } {
        return Tfu::NotTried;
    }
    let Some(query) = osal::ops().and_then(|o| o.query) else {
        return Tfu::NotTried;
    };
    let mut q = OsalV3dTfu {
        icfg: j.icfg,
        iia: j.iia,
        iis: j.iis,
        ica: j.ica,
        iua: j.iua,
        ioa: j.ioa,
        ios: j.ios,
        coef0: j.coef0,
        ..OsalV3dTfu::default()
    };
    // SAFETY: a live struct osal_v3d_tfu of the size passed.
    let rc = unsafe {
        query(OSAL_QUERY_V3D_TFU_RUN, &mut q as *mut OsalV3dTfu as *mut c_void, core::mem::size_of::<OsalV3dTfu>())
    };
    match (rc, q.rc) {
        (0, 0) => Tfu::Made,
        (0, _) => Tfu::Failed,
        _ => Tfu::NotTried,
    }
}

/// Count a tiling's TFU outcome; the first failure is logged.
fn tfu_count(g: &mut V3dState, t: Tfu, bytes: usize) {
    match t {
        Tfu::Made => g.tfu_jobs += 1,
        Tfu::Failed => {
            g.tfu_failed += 1;
            if g.tfu_failed == 1 {
                glog!("yakogl/v3d: a TFU job for a tiled copy of {} bytes failed: tiled on the CPU", bytes);
            }
        }
        Tfu::NotTried => {}
    }
}

/// Count a tiling that started at `t0` (`osal::clock_ns()` before it) as ended
/// now, under `at`; a tiling either end of which the clock could not read is
/// counted as untimed.
fn tile_timed(g: &mut V3dState, at: usize, t0: Option<u64>) {
    match (t0, osal::clock_ns()) {
        (Some(a), Some(b)) => g.tile_ns[at] += b.saturating_sub(a),
        _ => g.tile_untimed += 1,
    }
}

/// `bytes` of the job's texture arena at `align`: (GPU address, CPU view),
/// or `None` when the arena is full. Full with a request the small set
/// could never take, the next job takes the 8 MiB set, where the draw is
/// tried again (`draw_in_job`, `choose_arenas`).
fn tex_alloc(g: &mut V3dState, bytes: usize, align: usize) -> Option<(u32, *mut u8)> {
    let at = g.j.tex_used.next_multiple_of(align);
    if bytes == 0 || at + bytes > g.arenas.tex_bytes {
        g.want_big_arena |= bytes + align > SPARE_TEX_BYTES;
        return None;
    }
    g.j.tex_used = at + bytes;
    let gpu = gpu32(g.arenas.tex.gpu + at as u64)?;
    // SAFETY: inside the arena's CPU mapping (tex_bytes long).
    Some((gpu, unsafe { (g.arenas.tex.cpu as *mut u8).add(at) }))
}

/// The CPU view of `bytes` of V3D memory at `gpu` (the OSAL's
/// OSAL_QUERY_V3D_CPU_OF_GPU), or `None` when it is not the OSAL's.
fn cpu_of(gpu: u64, bytes: u32) -> Option<*const u8> {
    let mut q = OsalV3dCpuOf { gpu, bytes: bytes as u64, cpu: core::ptr::null_mut() };
    let query = osal::ops().and_then(|o| o.query)?;
    // SAFETY: `q` is a live struct osal_v3d_cpu_of of the size passed.
    let rc = unsafe {
        query(
            OSAL_QUERY_V3D_CPU_OF_GPU,
            &mut q as *mut OsalV3dCpuOf as *mut c_void,
            core::mem::size_of::<OsalV3dCpuOf>(),
        )
    };
    (rc == 0 && !q.cpu.is_null()).then_some(q.cpu as *const u8)
}

/// The texture state record of binding `b` in this job: found again when the
/// job already tiled the same storage, otherwise laid out, tiled from the GL
/// core's levels and written into the texture arena
/// (docs/online-shaderc/v3d-backend.md, "M6 textures").
fn tex_state_for(g: &mut V3dState, b: &GlesTextureBind, ret32: bool) -> Result<u32, Attempt> {
    if let Some(why) = texture::refusal_for(b, ret32) {
        return Err(Attempt::Refused(why));
    }
    let key = TexKey::of(b);
    if let Some(i) = g.j.tex_keys[..g.j.ntex].iter().position(|k| *k == key) {
        return Ok(g.j.tex_states[i]);
    }
    /* A depth texture whose rendered depth the GL core left in this
     * backend's hands (depth_id, v3d_depth_sampled) reads the D24S8 buffer
     * the jobs stored, as its depth. A render target the application
     * samples reads its render image, which the jobs rendering into it
     * stored in the TMU's layout (RtImage): no copy, and no wait (the GPU
     * runs the jobs in order, and neither buffer nor image is loaded by a
     * job that samples it, so a job reads it only after the one that wrote
     * it). Any other binding reads a copy tiled from the GL core's storage. */
    let (layout, image_gpu, max_level, from_depth_buffer) = if b.depth_id != 0 {
        let (layout, buffer_gpu) = depth_sample(g, b)?;
        (layout, buffer_gpu, 0, true)
    } else {
        match rt_sample(g, b) {
            Some((layout, base, levels)) => (layout, base, levels - 1, false),
            None => {
                let (layout, image_gpu) = copy_image_for(g, b)?;
                (layout, image_gpu, layout.levels - 1, false)
            }
        }
    };
    let mut st = if from_depth_buffer {
        texture::depth_buffer_state(b, &layout, image_gpu)
    } else {
        texture::state(b, &layout, image_gpu)
    };
    /* an image laid out with more levels than the draw samples (a chain
     * made for a mipmap filter, sampled without one): the levels below the
     * last sampled are at the same places whatever the count */
    st.max_level = max_level;
    let (rec, rec_cpu) = tex_alloc(g, V3D_TEX_STATE_BYTES, V3D_TEX_RECORD_ALIGN as usize).ok_or(Attempt::Full)?;
    // SAFETY: V3D_TEX_STATE_BYTES of the arena.
    if !unsafe { v3d_tex_pack_state(rec_cpu, &st) } {
        return Err(Attempt::Refused("the texture state does not fit its record (v3d_tex_pack_state refused a field)"));
    }
    if g.j.ntex < TEX_CACHE {
        g.j.tex_keys[g.j.ntex] = key;
        g.j.tex_states[g.j.ntex] = rec;
        g.j.tex_images[g.j.ntex] = image_gpu;
        // SAFETY: the record's V3D_TEX_STATE_BYTES, just packed.
        g.j.tex_packed[g.j.ntex] = unsafe { *(rec_cpu as *const [u8; V3D_TEX_STATE_BYTES]) };
        g.j.ntex += 1;
    }
    Ok(rec)
}

/// The slot of the single-sampled D24S8 buffer whose depth half GL object
/// `id` owns (`ZsBuf::owners[0]`; a depth texture's `GlesDraw::depth_id`
/// form), if there is one.
fn depth_buffer_of(g: &V3dState, id: u32) -> Option<usize> {
    (0..ZS_BUFS).find(|&i| {
        let b = &g.zsbufs[i];
        b.live() && !b.s8 && !b.msaa && b.owners[0] == id
    })
}

/// Does the buffer in slot `i` hold stored depth a `w` x `h` depth texture
/// can be sampled from (`v3d_depth_sampled`, `depth_sample`): the
/// buffer's size is the texture's and a submitted job stored its depth
/// (`ZsKnown::Stored`). A buffer the last job only cleared holds its depth
/// in the tile buffer alone (`Cleared`), one no job wrote holds nothing,
/// and one written but not stored (`Written`) is gone: none of them can be
/// read, and the CPU resolve (`v3d_depth_resolve`) answers for them as
/// before.
fn depth_buffer_sampleable(g: &V3dState, i: usize, w: u32, h: u32) -> bool {
    let b = &g.zsbufs[i];
    b.z == ZsKnown::Stored && (b.w, b.h) == (w, h)
}

/// Will the next draw sample the depth GL object `id` (a depth texture's
/// `GlesDraw::depth_id` form, `w` x `h`) rendered from the buffer that
/// holds it, instead of the GL core's texels
/// (`crate::backend::samples_rendered_depth`)? True when the buffer owned
/// by `id` holds stored depth of that size (`depth_buffer_sampleable`): the
/// GL core then puts `id` into the draw's binding
/// (`GlesTextureBind::depth_id`) and resolves nothing, and `depth_sample`
/// points the draw's texture state at the buffer. False: the GL core
/// resolves on the CPU as before.
///
/// A reading of the buffers' history, nothing more: the GL core has
/// submitted the kick that rendered the depth first (`backend::flush`, as
/// `texture::resolve_depth` did before it read the buffer), so the depth
/// pass that was the open job a moment ago has been ended and `flush_job`
/// has recorded what it stored. Glmark2 binds the canvas and draws the
/// ground right after the depth pass, so that submit is the one `v3d_draw`
/// would have made at the target change anyway. No submit and no wait
/// here, and no C: the GL core's test binaries on the mock backend table
/// link this function without the job builder.
pub fn v3d_depth_sampled(id: u32, w: u32, h: u32) -> bool {
    let g = g();
    if !g.inited || id == 0 {
        return false;
    }
    depth_buffer_of(g, id).is_some_and(|i| depth_buffer_sampleable(g, i, w, h))
}

/// May the GL core defer a depth texture's R32F storage
/// (`crate::backend::defers_depth_storage`)? Yes while initialised: a draw
/// that samples a rendered depth texture reads this backend's D24S8 buffer
/// (`v3d_depth_sampled`, `depth_sample`) and never the texels, so glmark2's
/// shadow and refract need no 7.68 MB R32F store for their 1600x1200 shadow
/// map. A draw that falls back to the texels, and every CPU read or write of
/// them, has the GL core allocate them first (`gl::texture::ensure_storage`).
pub fn v3d_defers_depth_storage() -> bool {
    g().inited
}

/// The depth buffer binding `b` reads (`GlesTextureBind::depth_id`, set
/// after `v3d_depth_sampled` answered 1 for it): its layout (one D24S8
/// level, `texture::rt_layout`'s single-level case, which is how `zs_buf`
/// laid the buffer out) and the buffer's address. The texture state then
/// has the TMU read the buffer's words as DEPTH24_X8
/// (`texture::depth_buffer_state`): the same bits 31..8 the CPU resolve
/// read (`v3d_depth_resolve`), converted by the texture unit instead of the
/// CPU, and no copy of 7.68 MB per frame for glmark2's shadow and refract
/// (docs/online-shaderc/v3d-backend.md, "Depth textures sampled from their
/// depth buffer").
///
/// Refused by name when the buffer is not there to sample (gone, or not
/// `Stored` at the texture's size): the GL core left the texels stale on
/// the strength of the answer it got, so a copy of them would be the wrong
/// picture. Between the answer and the draw nothing frees a stored buffer
/// (`zs_reclaimable` never takes one holding contents, and the one draw in
/// between is the asker), so this is a check, not a path.
fn depth_sample(g: &mut V3dState, b: &GlesTextureBind) -> Result<(V3dTexLayout, u32), Attempt> {
    let found = depth_buffer_of(g, b.depth_id).filter(|&i| depth_buffer_sampleable(g, i, b.width, b.height));
    let Some(i) = found else {
        g.depth_refused += 1;
        copy_count(g, CopyReason::DepthRefused);
        return Err(Attempt::Refused(
            "the depth buffer a depth texture was to be sampled from holds no stored depth of the texture's size",
        ));
    };
    let layout = texture::rt_layout(b.width, b.height, 1)
        .filter(|l| l.slices[0].offset == 0)
        .ok_or(Attempt::Refused("the TMU layout refused the depth buffer's size"))?;
    let address = g.zsbufs[i].address;
    g.depth_direct += 1;
    if g.depth_direct == 1 {
        glog!(
            "yakogl/v3d: a {}x{} depth texture is sampled from its depth buffer (DEPTH24_X8): no CPU resolve, no \
             tiled copy",
            b.width,
            b.height
        );
    }
    Ok((layout, address))
}

/// The tiled image of binding `b` for this job and its layout: the copy
/// kept across jobs (`tiled_copy`, found again or made now), or one tiled
/// into the job's arena when no kept copy could be made.
fn copy_image_for(g: &mut V3dState, b: &GlesTextureBind) -> Result<(V3dTexLayout, u32), Attempt> {
    let key = TexKey::of(b);
    /* the copy is tiled from the storage: a render target whose jobs stored
     * only its render image has level 0 written back first (a mip filter
     * sampled without the TFU's chain, a moved stamp) */
    if let Some(i) = rt_find(g, b.base_gpu, b.width, b.height, b.stride_texels) {
        rt_level0_back(g, i);
    }
    /* The job that rendered into the texture (an FBO's colour attachment)
     * may still be running: since the async swap (6b6ec1e1) a flush submits
     * and returns. The copy below is tiled by the CPU (or the TFU) from the
     * GL core's storage, so that job completes first, or the copy holds the
     * texels from before the render (dEQP fbo.render's tex2d cases, 56
     * "Image comparison failed" on the Pi 4). The binding's target_kick
     * names it (the GL core's framebuffer::texture_target_use), as the
     * GC7000's tiled copy waits on it (vivante/tiled_copy.rs refresh). Only
     * a submitted job: the open one renders into another target (a target
     * change submits the job before it), and a draw sampling its own target
     * is a feedback loop, which GL leaves undefined. A completed render
     * costs nothing (v3d_kick_pending asks the OSAL's fence). */
    if b.target_kick != 0 && b.target_kick <= g.kicks && v3d_kick_pending(b.target_kick) {
        let by_alias =
            [g.queued, g.in_flight].into_iter().flatten().any(|f| b.target_kick < f.kick && b.target_kick == f.alias);
        g.tex_waits = g.tex_waits.wrapping_add(1);
        g.tex_waits_alias = g.tex_waits_alias.wrapping_add(u32::from(by_alias));
        if wait_in_flight(g) < 0 {
            return Err(Attempt::Refused("the job that rendered into a sampled texture did not complete"));
        }
    }
    let layout = texture::layout(b).ok_or(Attempt::Refused("the TMU layout refused the texture's size"))?;
    let image_bytes =
        texture::image_bytes(b, &layout).ok_or(Attempt::Refused("the TMU layout refused the texture's size"))? as usize;
    let src = cpu_of(b.base_gpu, texture::gl_bytes(b))
        .ok_or(Attempt::Refused("the texture's storage is not in the V3D OSAL's memory"))?;
    /* a stamped texture keeps one tiled copy across jobs; an unstamped one
     * (an EGLImage's) and one the cache has no room for are tiled into this
     * job's arena, as before */
    let src_bytes = texture::gl_bytes(b) as u64;
    let (min, mut tfu, mut made) = (g.tfu_min_bytes, Tfu::NotTried, false);
    let why = g.copy_why;
    let fill = |image: *mut u8, image_gpu: u32| {
        made = true;
        tfu = tfu_tile(min, b, &layout, image_gpu);
        if tfu != Tfu::Made {
            // SAFETY: `image` is the copy's image_bytes; `src` covers the
            // sampled levels of every layer (gl_bytes), as the OSAL confirmed.
            unsafe { texture::tile(b, &layout, image, src) };
        }
        true
    };
    let kept = tiled_copy(g, key, image_bytes, src_bytes, TILE_KEPT_AT_DRAW, fill);
    tfu_count(g, tfu, image_bytes);
    if made {
        copy_count(g, why);
    }
    let image_gpu = match kept {
        Ok(gpu) => gpu,
        Err(miss) => {
            /* the per-job arena only when the kept copies could not take it:
             * a texture larger than the arena but inside the tiled budget
             * is sampled from its kept copy. Refused, the message names why
             * the kept copy was not made: the arena's 8 MiB is only the
             * fallback's limit (T5 on eaf98dc8 read "8 MiB" for a 22 MB
             * chain inside a 24 MiB budget) */
            if image_bytes + 4096 + 64 > TEX_BYTES {
                let why = match miss {
                    KeptMiss::Unstamped => {
                        "an unstamped (EGLImage) texture larger than the V3D's texture arena (8 MiB tiled)"
                    }
                    KeptMiss::OverBudget => {
                        "a texture larger than the tiled-copy budget (CONFIG_YAKOGL_V3D_TILED_COPIES_BYTES) \
                         and the texture arena (8 MiB tiled)"
                    }
                    KeptMiss::NoMemory => {
                        "no GPU memory for the texture's tiled copy (the OSAL's line names the bytes), and \
                         it is larger than the texture arena (8 MiB tiled)"
                    }
                    KeptMiss::NoSlot => {
                        "no free slot for the texture's tiled copy, and it is larger than the texture arena \
                         (8 MiB tiled)"
                    }
                    KeptMiss::FillRefused => {
                        "the tiling refused the texture's image, and it is larger than the texture arena \
                         (8 MiB tiled)"
                    }
                    KeptMiss::WorkingSetFull => {
                        "the kept tiled copies are all in use by the last two jobs (the working set), and it is \
                         larger than the texture arena (8 MiB tiled)"
                    }
                    KeptMiss::Address => {
                        "the texture's tiled copy lies past the V3D's 32-bit address space, and it is larger \
                         than the texture arena (8 MiB tiled)"
                    }
                };
                /* no memory may be memory the open job holds: run it first */
                return Err(if miss == KeptMiss::NoMemory { Attempt::NoMemory(why) } else { Attempt::Refused(why) });
            }
            /* level 0 is laid out at a 4 KiB boundary of the image (Mesa's
             * BOs are page aligned), so the image is too */
            let (image_gpu, image) = tex_alloc(g, image_bytes + CANARY_BYTES, 4096).ok_or(Attempt::Full)?;
            // SAFETY: `image` is image_bytes + CANARY_BYTES of the arena;
            // `src` covers the sampled levels of every layer (gl_bytes), as
            // the OSAL confirmed.
            unsafe { canary_set(image.add(image_bytes), CANARY_BYTES) };
            let t0 = osal::clock_ns();
            // SAFETY: as above.
            unsafe { texture::tile(b, &layout, image, src) };
            tile_timed(g, TILE_ARENA, t0);
            // SAFETY: as above.
            unsafe { canary_check(g, b, image_bytes, image.add(image_bytes), CANARY_BYTES) };
            g.tilings += 1;
            g.tiled_bytes += image_bytes as u64;
            copy_count(g, CopyReason::Arena);
            copy_count(g, why);
            image_gpu
        }
    };
    Ok((layout, image_gpu))
}

/// Why a kept tiled copy was not made (`tiled_copy`): the draw then tiles
/// into the job's arena, and a refusal names this, not only the arena.
#[derive(Clone, Copy, Debug, PartialEq)]
enum KeptMiss {
    /// No stamp (an EGLImage's storage): never kept.
    Unstamped,
    /// Larger than the whole budget.
    OverBudget,
    /// The OSAL had no memory for it (the OSAL's own line says how much,
    /// and how many frees it holds for the open job).
    NoMemory,
    /// Every slot held a copy the eviction could not free.
    NoSlot,
    /// The tiling refused the image.
    FillRefused,
    /// The copy's address is past the V3D's 32-bit space.
    Address,
    /// Full (entries or bytes), and the least recently sampled copy was
    /// sampled by the open or the previous job: the working set, which is not
    /// given up for a newcomer (the newcomer goes to the job's arena).
    WorkingSetFull,
}

/// The GPU address of the tiled copy of stamped key `key`, kept across
/// jobs: found when the same storage with the same stamp was tiled before
/// (and not rendered into since), otherwise made now in memory of its own
/// and filled by `fill` (given the image, `image_bytes` bytes; false when it
/// wrote nothing, and then no copy is kept). `src_bytes` is how much of the
/// GL core's storage the copy stands for (a job rendering into it drops the
/// copy). `None` for an unstamped key (`content_gen` 0), for an image larger
/// than the budget, when the OSAL has no memory for it, or when `fill`
/// refused: the caller then tiles into the job's arena, or tiles nothing.
/// The fill, the canary check and the cache flush after it are timed under `at`
/// (`TILE_AT_UPLOAD` or `TILE_KEPT_AT_DRAW`).
fn tiled_copy(
    g: &mut V3dState,
    key: TexKey,
    image_bytes: usize,
    src_bytes: u64,
    at: usize,
    fill: impl FnOnce(*mut u8, u32) -> bool,
) -> Result<u32, KeptMiss> {
    if key.content_gen == 0 {
        return Err(KeptMiss::Unstamped);
    }
    let now = g.kicks.wrapping_add(1);
    if let Some(t) = g.tiled.iter_mut().find(|t| t.live() && t.key == key) {
        t.last_kick = now;
        return Ok(t.image_gpu);
    }
    /* another stamp of the same storage is out of date */
    let base = key.base_gpu;
    for i in 0..TILED_ENTRIES {
        if g.tiled[i].live() && g.tiled[i].key.base_gpu == base {
            free_tiled(g, i);
            copy_count(g, CopyReason::StampReplaced);
        }
    }
    /* the OSAL's granule is smaller than a page: room to align level 0 */
    let bytes = image_bytes + 4096;
    if bytes > g.tiled_budget {
        return Err(KeptMiss::OverBudget);
    }
    while g.tiled_live_bytes + bytes > g.tiled_budget || g.tiled.iter().all(|t| t.live()) {
        let cause = usize::from(g.tiled_live_bytes + bytes > g.tiled_budget);
        let victim = tiled_victim(g, now).ok_or(KeptMiss::NoSlot)?;
        /* sampled by the previous job or the open one: the working set.
         * It is not given up for a newcomer: the newcomer is tiled into the
         * job's arena instead (the caller's fallback), and counted. Giving
         * it up cost the cluster its 5.5 MB backdrop, which then found no
         * contiguous carve-out room again and was tiled into every job
         * (DEMO4, 634 "no GPU memory for a tiled copy"). Only when the
         * arena can take the newcomer: one larger than the arena (dEQP
         * texture.size 2048x2048, 8-12 MiB tiled) has no other way to be
         * drawn, and evicts as before */
        let age = now.wrapping_sub(g.tiled[victim].last_kick) as i32;
        if age <= 1 && image_bytes + 4096 + 64 <= TEX_BYTES {
            g.kept_refused += 1;
            return Err(KeptMiss::WorkingSetFull);
        }
        g.evictions[cause] += 1;
        g.evicted_bytes[cause] += g.tiled[victim].bytes as u64;
        if age <= 1 {
            g.evicted_hot[cause] += 1;
        }
        free_tiled(g, victim);
    }
    let slot = g.tiled.iter().position(|t| !t.live()).ok_or(KeptMiss::NoSlot)?;
    let mut mem = OsalGpuMem::ZERO;
    if osal::gpu_alloc(V3D_HEAP_DATA, bytes, OSAL_MEM_DATA, &mut mem) != 0 {
        g.kept_no_memory += 1;
        /* the numbers the refusal's static text cannot carry; the OSAL's
         * own line adds the frees it holds for the open job */
        glog_once!(
            "yakogl/v3d: no GPU memory for a tiled copy of {} bytes ({} of {} bytes of copies live, a job {})",
            bytes,
            g.tiled_live_bytes,
            g.tiled_budget,
            if g.j.open { "open" } else { "not open" }
        );
        return Err(KeptMiss::NoMemory);
    }
    let pad = (mem.gpu.next_multiple_of(4096) - mem.gpu) as usize;
    let Some(image_gpu) = gpu32(mem.gpu + pad as u64) else {
        osal::gpu_free(&mut mem);
        return Err(KeptMiss::Address);
    };
    /* a GPU address without a CPU view cannot be filled */
    if mem.cpu.is_null() {
        osal::gpu_free(&mut mem);
        return Err(KeptMiss::NoMemory);
    }
    // SAFETY: `mem` is `bytes` long, and the image (image_bytes) starts
    // `pad` < 4096 bytes into it.
    let image = unsafe { (mem.cpu as *mut u8).add(pad) };
    /* the slack after the image (4096 - pad bytes, at least 1) holds a
     * canary: a fill that writes past its image breaks it first */
    let slack = (bytes - pad - image_bytes).min(CANARY_BYTES);
    // SAFETY: `slack` bytes after the image, inside `mem`.
    unsafe { canary_set(image.add(image_bytes), slack) };
    let t0 = osal::clock_ns();
    if !fill(image, image_gpu) {
        tile_timed(g, at, t0);
        osal::gpu_free(&mut mem);
        return Err(KeptMiss::FillRefused);
    }
    // SAFETY: as above.
    let broken = unsafe { canary_first_broken(image.add(image_bytes), slack) };
    if let Some(k) = broken {
        g.overruns += 1;
        glog!(
            "yakogl/v3d: a tiled copy of {} bytes was written past its end (from byte {} after it; key {}x{}, {} \
             levels, {} layers)",
            image_bytes,
            k,
            key.width,
            key.height,
            key.levels,
            key.layers
        );
    }
    // SAFETY: the image's image_bytes, just written.
    unsafe { osal::cache_flush(image as *mut c_void, image_bytes) };
    tile_timed(g, at, t0);
    g.tiled[slot] = Tiled { key, src_bytes, mem, image_gpu, bytes, last_kick: now };
    g.tiled_live_bytes += bytes;
    g.tilings += 1;
    g.tiled_bytes += image_bytes as u64;
    if g.tiled_live_bytes > g.tiled_high_water {
        g.tiled_high_water = g.tiled_live_bytes;
        if g.tiled_high_water >= 2 * g.tiled_logged.max(64 * 1024) {
            g.tiled_logged = g.tiled_high_water;
            glog!("yakogl/v3d: tiled copies high-water {} of {} bytes", g.tiled_high_water, g.tiled_budget);
        }
    }
    Ok(image_gpu)
}

/// The kept copy to give up for another: the least recently sampled, and of
/// those sampled equally recently, the one with the fewest bytes (the
/// cheapest to tile again). Every copy one job samples carries that job's
/// kick, so when the whole table is the working set of the last frame they
/// all tie; the first slot then went, which is the texture tiled first
/// (the cluster's 1920x720 backdrop, 5.5 MB), and a frame re-tiled 8.5 MB
/// (v3d_gl_cluster DEMO3N, f5f28256, 2026-09-25). A copy the open job
/// samples may still go: the OSAL frees it after that job has run.
fn tiled_victim(g: &V3dState, now: u32) -> Option<usize> {
    (0..TILED_ENTRIES)
        .filter(|&i| g.tiled[i].live())
        .min_by_key(|&i| (g.tiled[i].last_kick.wrapping_sub(now) as i32, g.tiled[i].bytes))
}

/// glTexImage2D / glTexSubImage2D stored the caller's rows `src` (rows
/// `stride` bytes apart, cacheable memory) into a box of level `level` of the
/// texture `b` describes (`YakoglBackend::texture_uploaded`). When the box is
/// the whole of level 0 and the texture is sampled at one level, the tiled
/// copy is made now, from those rows, under the key a draw will look for
/// (`b.content_gen` is the stamp after this write): the first draw then
/// tiles nothing, and the rows are read from cacheable memory, 6.2 times
/// faster than the GL core's non-cacheable storage (docs/online-shaderc/
/// v3d-backend.md, the second tiling bench).
///
/// Any other upload (a sub-box, a mip level, a texture sampled with its
/// mip chain, an XRGB image, one the sampler refuses, an unstamped one)
/// makes nothing: the write took a new stamp, so the next draw tiles the
/// storage as before (v3d-tex's rules 3-5). A copy whose sampled levels
/// change later is keyed on them and is not found (rule 4).
///
/// # Safety
/// `b` is a live binding; `src` holds `h` rows of `stride` bytes.
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn v3d_texture_uploaded(
    b: *const GlesTextureBind,
    level: u32,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    src: *const c_void,
    stride: u32,
) {
    let g = g();
    // SAFETY: the caller's contract.
    let Some(b) = (unsafe { b.as_ref() }) else { return };
    /* the caller's rows are RGBA8 texels (note_uploaded: GL_RGBA /
     * GL_UNSIGNED_BYTE); a narrow texture's are not the stored ones */
    if !g.inited
        || src.is_null()
        || b.content_gen == 0
        || b.bgra != 0
        || !gles_texfmt_is_rgba8_bytes(b.format)
        || texture::refusal(b).is_some()
    {
        return;
    }
    let whole = level == 0 && (x, y, w, h) == (0, 0, b.width, b.height);
    if !whole || texture::levels_sampled(b) != 1 {
        return;
    }
    let Some(layout) = texture::layout(b) else { return };
    let src = src as *const u8;
    /* a large image by the TFU from the GL core's storage, which holds
     * these texels now; else the CPU from the caller's cacheable rows */
    let (min, mut tfu, mut made) = (g.tfu_min_bytes, Tfu::NotTried, false);
    let fill = |image: *mut u8, image_gpu: u32| {
        made = true;
        tfu = tfu_tile(min, b, &layout, image_gpu);
        tfu == Tfu::Made
            // SAFETY: `image` is the copy's layout.size bytes (one layer: a
            // 2D upload); `src` holds the level's h rows of `stride` bytes
            // (the caller's contract), and v3d_tex_store_box refuses a
            // stride shorter than a row.
            || unsafe { v3d_tex_store_box(image, &layout, 0, 0, 0, w, h, src, stride) }
    };
    let key = TexKey::of(b);
    let kept = tiled_copy(g, key, layout.size as usize, texture::gl_bytes(b) as u64, TILE_AT_UPLOAD, fill);
    tfu_count(g, tfu, layout.size as usize);
    if kept.is_ok() {
        g.eager_tilings += 1;
    }
    if made && kept.is_ok() {
        copy_count(g, CopyReason::Upload);
    }
}

/// Free tiled copy `i` (the OSAL frees it once the job being recorded has
/// run).
fn free_tiled(g: &mut V3dState, i: usize) {
    g.tiled_live_bytes -= g.tiled[i].bytes;
    osal::gpu_free(&mut g.tiled[i].mem);
    g.tiled[i] = Tiled::NONE;
}

/// Before a job opens, give back the kept copies of the draw's textures
/// whose storage has a new stamp. `tiled_copy` frees them anyway when the
/// draw samples the new stamp, but by then the job is open, and the OSAL
/// holds a free made with a job open until that job has run (v3d_frees.h:
/// the job being recorded may read it). The open job never reads a stale
/// copy (it looks copies up by the new key), so the old copy and the new one
/// were both in memory until the job ended. For glmark2's shadow, whose
/// depth texture gets a new stamp every frame (texture::resolve_depth), that
/// is two 7.8 MB copies of a 1600x1200 R32F map. Given back here, with no
/// job open, the free waits only for the job in flight, which may still
/// sample the copy. With a job open nothing is dropped: that job may have
/// sampled the copy.
fn drop_stale_copies(g: &mut V3dState, d: &GlesDraw) {
    if g.j.open {
        return;
    }
    let fs = &d.textures[..(d.num_textures as usize).min(GLES_MAX_TEX_BINDS)];
    let vs = &d.vs_textures[..(d.num_vs_textures as usize).min(GLES_MAX_VS_TEX_BINDS)];
    for b in fs.iter().chain(vs).filter(|b| b.content_gen != 0) {
        let key = TexKey::of(b);
        for i in 0..TILED_ENTRIES {
            let t = &g.tiled[i];
            if t.live() && t.key.base_gpu == b.base_gpu && t.key != key {
                let bytes = t.bytes;
                free_tiled(g, i);
                g.stale_dropped += 1;
                copy_count(g, CopyReason::StaleGivenBack);
                g.stale_dropped_bytes += bytes as u64;
                if g.stale_dropped == 1 {
                    glog!(
                        "yakogl/v3d: a stale tiled copy of {} bytes given back before the job opened (the first; \
                         later ones are counted)",
                        bytes
                    );
                }
            }
        }
    }
}

/// Free every tiled copy made from memory in `[base, base + bytes)`.
fn drop_tiled(g: &mut V3dState, base: u64, bytes: u64) {
    for i in 0..TILED_ENTRIES {
        if g.tiled[i].overlaps(base, bytes) {
            free_tiled(g, i);
        }
    }
}

/// The GL core is about to release texel storage `[base_gpu, + bytes)`
/// (`YakoglBackend::texture_released`): drop the tiled copies made from it
/// before the address can be handed out again.
pub extern "C" fn v3d_texture_released(base_gpu: u64, bytes: u64) {
    let g = g();
    if g.inited {
        drop_tiled(g, base_gpu, bytes);
        rt_drop(g, base_gpu, bytes);
    }
}

/// The GL core says the depth/stencil contents of object `id` are gone
/// (`YakoglBackend::attachment_gone`): its halves are Untouched in every
/// buffer and in `zs_parked`, and a buffer left with no owner or holding
/// nothing is freed.
/// One buffer per id keeps the other object's half: deleting a stencil
/// renderbuffer does not take the framebuffer's depth (dEQP
/// recreate_stencilbuffer.rebind_*, ES 2.0 4.4.3). The open job, if it uses
/// such a buffer, is submitted first, so its history is recorded before the
/// half is undefined and no memory is freed under a job not yet made.
pub extern "C" fn v3d_attachment_gone(id: u32) {
    let g = g();
    if !g.inited || id == 0 {
        return;
    }
    g.zs_parked.forget(id);
    let open_uses = g.j.open && [g.j.zs, g.j.ss].into_iter().flatten().any(|i| g.zsbufs[i].owners.contains(&id));
    if open_uses {
        let rc = flush_job(g);
        if rc < 0 {
            glog_once!("yakogl/v3d: the job drawing into object 0x{:x} failed ({}) before it went", id, rc);
        }
    }
    for b in g.zsbufs.iter_mut().filter(|b| b.live() && b.owners.contains(&id)) {
        if b.owners[0] == id {
            (b.owners[0], b.z) = (0, ZsKnown::Untouched);
        }
        if b.owners[1] == id {
            (b.owners[1], b.s) = (0, ZsKnown::Untouched);
        }
        if b.owners == [0; 2] || !b.holds_history() {
            osal::gpu_free(&mut b.mem);
            *b = ZsBuf::NONE;
        }
    }
}

/// Write the depth of GL object `id` (a depth texture's `GlesDraw::depth_id`
/// form) into `w` x `h` floats at `dst`, `pitch` floats a row
/// (`YakoglBackend::depth_resolve`). The GL core has waited for the job that
/// rendered it (texture.rs `resolve_depth`).
///
/// The buffer whose depth half `id` owns (`ZsBuf::owners[0]`):
/// - Cleared(v): v at every texel;
/// - Stored: the D24S8 buffer the jobs store, laid out as Mesa's zsbuf
///   (`v3d_tex_layout`, one level, 4 bytes a pixel) in UIF without the XOR
///   (`zs_layout`), read back by Mesa's v3d_load_tiled_image
///   (`v3d_tex_load_box`, which tools/v3d/tex_check.c compares with Mesa's)
///   into the texel rows, each word then read as depth = bits 31..8 /
///   0xffffff: Mesa's S8_UINT_Z24_UNORM, stencil in bits 7..0
///   (v3dx_format_table.c:149), the same word layout the GC7000 measured.
///   Row 0 is GL's y 0: an FBO is drawn without `flip_y` (state.rs
///   `viewport`).
/// - anything else (Untouched; Written, which no job stored; a 4x buffer;
///   no buffer) is 1: the texels stay as they are.
///
/// Since a draw samples the buffer itself (`depth_sample`), this is the
/// CPU's path alone: a write of the texels (glTexSubImage2D into a rendered
/// depth texture), a copy or blit that reads them, and a draw whose buffer
/// is not there to sample (`v3d_depth_sampled` answered 0). Counted
/// (`depth_resolves`, `v3d_depth_samples`): glmark2's shadow and refract
/// must show 0 a frame.
///
/// Unmeasured on the board until gl_test 169 runs on the Pi 4.
///
/// # Safety
/// `dst` holds `h` rows of `pitch` floats, `w <= pitch`.
pub unsafe extern "C" fn v3d_depth_resolve(id: u32, dst: *mut f32, w: u32, h: u32, pitch: u32) -> i32 {
    let g = g();
    if !g.inited || id == 0 || dst.is_null() || w > pitch {
        return 1;
    }
    let Some(b) = g.zsbufs.iter().find(|b| b.live() && !b.s8 && b.owners[0] == id) else { return 1 };
    let (rw, rh) = (w.min(b.w), h.min(b.h));
    match b.z {
        ZsKnown::Cleared(v) => {
            // SAFETY: the caller's contract.
            unsafe { ds_retention::write_depth_texels(dst, w, h, pitch, |_, _| Some(v)) };
            g.depth_resolves += 1;
            return 0;
        }
        ZsKnown::Stored if !b.msaa => {}
        _ => return 1,
    }
    /* the buffer's layout (zs_buf): one D24S8 level, the UIF XOR dropped;
     * texture::rt_layout's single-level case, which depth_sample reads it
     * through as well */
    let Some(l) = texture::rt_layout(b.w, b.h, 1).filter(|l| l.slices[0].offset == 0) else {
        return -EINVAL;
    };
    let Some(off) = u64::from(b.address).checked_sub(b.mem.gpu).map(|o| o as usize) else { return -EINVAL };
    if b.mem.cpu.is_null() || off + l.size as usize > b.mem.size {
        return -EINVAL;
    }
    let image = (b.mem.cpu as *mut u8).wrapping_add(off);
    // SAFETY: the buffer's live CPU mapping, `l.size` bytes from `image`
    // (checked above); no job uses it (the GL core waited).
    unsafe { osal::cache_invd(image as *mut c_void, l.size as usize) };
    // SAFETY: `image` holds the laid-out level; `dst` holds `rh` rows of
    // `pitch` floats, each at least `rw` 4-byte words (rw <= w <= pitch).
    if !unsafe { v3d_tex_load_box(image, &l, 0, 0, 0, rw, rh, dst as *mut u8, pitch * 4) } {
        return -EINVAL;
    }
    for y in 0..rh as usize {
        for x in 0..rw as usize {
            // SAFETY: inside the rows just written.
            unsafe {
                let p = dst.add(y * pitch as usize + x);
                let word = p.read().to_bits();
                p.write((word >> 8) as f32 / 16_777_215.0);
            }
        }
    }
    g.depth_resolves += 1;
    0
}

/// Depth textures and their rendered depth, since init (`V3dState`
/// `depth_direct`, `depth_resolves`, `depth_refused`): the draw bindings
/// that sampled a depth buffer directly (`depth_sample`), the CPU resolves
/// that wrote texels (`v3d_depth_resolve`), and the draws refused because
/// the buffer the GL core was told of was not there to sample. Read them as
/// differences: a glmark2 shadow or refract frame is one direct sample and
/// no resolve.
///
/// # Safety
/// Each pointer is null or writable.
#[no_mangle]
pub unsafe extern "C" fn v3d_depth_samples(direct: *mut u32, resolves: *mut u32, refused: *mut u32) {
    let g = g();
    for (out, v) in [(direct, g.depth_direct), (resolves, g.depth_resolves), (refused, g.depth_refused)] {
        // SAFETY: the caller's contract.
        if let Some(o) = unsafe { out.as_mut() } {
            *o = v;
        }
    }
}

/// `eglSwapBuffers` is posting the window surface at `surface_gpu`
/// (`YakoglBackend::surface_swapped`, before the swap's flush). EGL 1.5
/// 3.10.1: "The contents of ancillary buffers are always undefined after
/// calling eglSwapBuffers." So the surface's depth and stencil start
/// Untouched in the next frame (the tile buffer's clear values, which is what
/// femtovg's stencil strokes expect: Slint never clears the stencil), and the
/// frame's last job does not store them.
pub extern "C" fn v3d_surface_swapped(surface_gpu: u64) {
    let g = g();
    if !g.inited {
        return;
    }
    if g.j.open && g.j.target.gpu == surface_gpu {
        g.j.zs_discard = true; /* its flush records Untouched */
    }
    zs_record(g, surface_gpu, ZsKnown::Untouched, ZsKnown::Untouched);
    for b in g.zsbufs.iter_mut().filter(|b| b.live() && b.target == surface_gpu) {
        (b.z, b.s) = (ZsKnown::Untouched, ZsKnown::Untouched);
    }
    /* the multisample buffer is an ancillary buffer too */
    for b in g.msbufs.iter_mut().filter(|b| b.live() && b.target == surface_gpu) {
        b.valid = false;
    }
    /* the frame's last job is flushed by the swap, after this hook */
    if g.j.open {
        g.frame_ends_at_flush = true;
    } else {
        end_frame(g);
    }
}

/// A frame ended: when its largest job used more of any arena than any job
/// before, one line names the new high-water of every arena against its
/// size (for v3d-slint: whether Slint's frames fit the job, and by how
/// much). A steady frame prints nothing.
fn end_frame(g: &mut V3dState) {
    g.frames += 1;
    let hw = g.arenas_high_water.max(g.frame_arenas);
    if hw != g.arenas_high_water {
        g.arenas_high_water = hw;
        glog!(
            "yakogl/v3d: job arenas high-water at frame {}: BCL {} of {}, RCL {} of {}, indirect {} of {}, \
             data {} of {}, textures {} of {} bytes, {} draws",
            g.frames,
            hw.bcl,
            g.bcl_bytes,
            hw.rcl,
            RCL_BYTES,
            hw.ind,
            g.ind_bytes,
            hw.data,
            DATA_BYTES,
            hw.tex,
            TEX_BYTES,
            hw.draws
        );
        /* its own line: glog's 256 bytes hold the arena line with Slint's
         * numbers, not the pool's "ran out" form after it */
        glog!("yakogl/v3d: binner tile pool high-water at frame {}: {}", g.frames, BinPool(hw));
        /* beside it (frame 1 always prints): why glGenerateMipmap stayed on
         * the CPU so far, by reason, and the chains the TFU made */
        glog!(
            "yakogl/v3d: TFU mipmap at frame {}: {} chains ({} queued behind a job, {} waited), refused: {}",
            g.frames,
            g.rt_chains,
            g.mip_queued,
            g.mip_queue_full,
            MipRefusals(&g.mip_refused)
        );
        /* and the tiled copies and image stores made, by reason */
        glog!(
            "yakogl/v3d: tiled copies for draws at frame {}, by reason: {}",
            g.frames,
            CopyCounts(&g.copy_counts, 0..COPY_DRAW_REASONS)
        );
        glog!(
            "yakogl/v3d: other copies at frame {}: {}",
            g.frames,
            CopyCounts(&g.copy_counts, COPY_DRAW_REASONS..COPY_REASONS)
        );
    }
    /* The draw-time sampler variants, each a compile and a program link:
     * printed when the count moved, so a steady scene prints nothing and a
     * program that re-links every frame prints every frame. Its own line
     * for glog's 256 bytes, as the pool's. */
    let variants = crate::shaderc::online::backend::v3d_sampler_variants();
    if variants != g.variants_logged {
        g.variants_logged = variants;
        glog!(
            "yakogl/v3d: sampler variants high-water at frame {}: {} built, {} refused (a program and key re-link \
             once)",
            g.frames,
            variants.0,
            variants.1
        );
    }
    g.frame_arenas = ArenaUse::ZERO;
    g.last_frame_stores = core::mem::take(&mut g.frame_stores);
}

/// The tiled copies kept across jobs: how many are live, their bytes, and
/// the most bytes they held at once since init.
///
/// # Safety
/// Each pointer is null or writable.
#[no_mangle]
pub unsafe extern "C" fn v3d_tiled_copies(live: *mut u32, bytes: *mut u64, high_water: *mut u64) {
    let g = g();
    // SAFETY: the caller's contract.
    unsafe {
        if let Some(l) = live.as_mut() {
            *l = g.tiled.iter().filter(|t| t.live()).count() as u32;
        }
        if let Some(b) = bytes.as_mut() {
            *b = g.tiled_live_bytes as u64;
        }
        if let Some(h) = high_water.as_mut() {
            *h = g.tiled_high_water as u64;
        }
    }
}

/// The render images (`RtImage`): how many hold an image now, their bytes,
/// and since init the draw bindings sampled from one (no tiled copy), the
/// samples sent back to the copy path by a CPU write since the image was
/// stored, the images not made for want of memory, the mip chains the TFU
/// made in an image (glGenerateMipmap delegated), and the chains written
/// back into the GL core's storage. Counters since init; read them as
/// differences.
///
/// # Safety
/// Each pointer is null or writable.
#[no_mangle]
pub unsafe extern "C" fn v3d_rt_images(
    live: *mut u32,
    bytes: *mut u64,
    direct: *mut u32,
    cpu_writes: *mut u32,
    no_memory: *mut u32,
    chains: *mut u32,
    chains_returned: *mut u32,
) {
    let g = g();
    // SAFETY: the caller's contract.
    unsafe {
        if let Some(l) = live.as_mut() {
            *l = g.rt_images.iter().filter(|r| r.has_image()).count() as u32;
        }
        if let Some(b) = bytes.as_mut() {
            *b = g.rt_images.iter().filter(|r| r.has_image()).map(|r| r.mem.size as u64).sum();
        }
        for (out, v) in [
            (direct, g.rt_direct),
            (cpu_writes, g.rt_cpu_writes),
            (no_memory, g.rt_no_memory),
            (chains, g.rt_chains),
            (chains_returned, g.rt_chains_returned),
        ] {
            if let Some(o) = out.as_mut() {
                *o = v;
            }
        }
    }
}

/// The reasons glGenerateMipmap stays on the CPU (`MipRefusal`, the
/// arena line's names in order; `V3D_MIP_REFUSALS` of them).
pub const V3D_MIP_REFUSALS: usize = MIP_REFUSALS;

/// glGenerateMipmap delegated to the TFU since init: the calls refused, by
/// reason (`refused` null or `V3D_MIP_REFUSALS` writable values, in the
/// order of the arena line's names), the chains queued behind a job in
/// flight, and the calls that found the OSAL's queue full and waited.
///
/// # Safety
/// Each pointer is null or writable (as above).
#[no_mangle]
pub unsafe extern "C" fn v3d_mip_counters(refused: *mut u32, queued: *mut u32, queue_full: *mut u32) {
    let g = g();
    // SAFETY: the caller's contract.
    unsafe {
        if !refused.is_null() {
            core::ptr::copy_nonoverlapping(g.mip_refused.as_ptr(), refused, MIP_REFUSALS);
        }
        if let Some(q) = queued.as_mut() {
            *q = g.mip_queued;
        }
        if let Some(f) = queue_full.as_mut() {
            *f = g.mip_queue_full;
        }
    }
}

/// The depth/stencil buffers kept between jobs: the most live at once since
/// init (of `ZS_BUFS` slots, also written) and the most bytes they held at
/// once, so a record shows the headroom of both bounds.
///
/// # Safety
/// Each pointer is null or writable.
#[no_mangle]
pub unsafe extern "C" fn v3d_zs_usage(slots_peak: *mut u32, slots: *mut u32, bytes_peak: *mut u64) {
    let g = g();
    // SAFETY: the caller's contract.
    unsafe {
        if let Some(p) = slots_peak.as_mut() {
            *p = g.zs_slots_peak;
        }
        if let Some(n) = slots.as_mut() {
            *n = ZS_BUFS as u32;
        }
        if let Some(b) = bytes_peak.as_mut() {
            *b = g.zs_bytes_peak as u64;
        }
    }
}

/// Draws that waited for the job rendering into a texture they sample, since
/// init, and how many of those named that job by its alias (the stamp of
/// the draw that ended the job before, `InFlight::alias`).
///
/// # Safety
/// Each pointer is null or writable.
#[no_mangle]
pub unsafe extern "C" fn v3d_tex_waits(waits: *mut u32, by_alias: *mut u32) {
    let g = g();
    // SAFETY: the caller's contract.
    unsafe {
        if let Some(w) = waits.as_mut() {
            *w = g.tex_waits;
        }
        if let Some(a) = by_alias.as_mut() {
            *a = g.tex_waits_alias;
        }
    }
}

/// Tiled copies given up to make room for another since init, by cause:
/// index 0 the entry count (`TILED_ENTRIES` full), 1 the byte budget. Each
/// pointer is null or points to two values: how many, their bytes, and how
/// many of them the previous or the open job had sampled ("hot": the
/// working set, tiled again by the next job that samples it). Counters
/// since init; read them as differences. A drop by a new stamp, a render
/// into the storage or a released texture is not an eviction.
///
/// # Safety
/// Each pointer is null or writable for two values.
#[no_mangle]
pub unsafe extern "C" fn v3d_tiled_evictions(count: *mut u32, bytes: *mut u64, hot: *mut u32) {
    let g = g();
    // SAFETY: the caller's contract.
    unsafe {
        if !count.is_null() {
            core::ptr::copy_nonoverlapping(g.evictions.as_ptr(), count, 2);
        }
        if !bytes.is_null() {
            core::ptr::copy_nonoverlapping(g.evicted_bytes.as_ptr(), bytes, 2);
        }
        if !hot.is_null() {
            core::ptr::copy_nonoverlapping(g.evicted_hot.as_ptr(), hot, 2);
        }
    }
}

/// Kept tiled copies not made since init: `no_memory`, the OSAL had no
/// memory for the copy (fragmentation or a full carve-out; the OSAL logs the
/// bytes); `kept_refused`, the table was full and every candidate for
/// eviction belonged to the working set (sampled by the open or the previous
/// job), so the newcomer was tiled into the job's arena instead. Either
/// pointer is null or writable. Counters since init; read differences.
///
/// # Safety
/// Each pointer is null or writable.
#[no_mangle]
pub unsafe extern "C" fn v3d_tiled_misses(no_memory: *mut u32, kept_refused: *mut u32) {
    let g = g();
    // SAFETY: the caller's contract.
    unsafe {
        if let Some(n) = no_memory.as_mut() {
            *n = g.kept_no_memory;
        }
        if let Some(r) = kept_refused.as_mut() {
            *r = g.kept_refused;
        }
    }
}

/// The CPU time of the texture tilings since init, in ns of the OSAL's clock:
/// `ns[0]` kept copies made at an upload from the caller's rows (inside
/// glTexImage2D/glTexSubImage2D), `ns[1]` kept copies made at a draw from the
/// GL core's storage, `ns[2]` copies tiled into the job's arena at a draw;
/// each is the store into the tiled layout (v3d_tex_store_level/_box), and
/// for a kept copy the cache flush of its image. `untimed`: tilings the clock
/// could not time (the OSAL has no OSAL_QUERY_CLOCK_NS); while it is not
/// zero the times are short by those tilings and are not a measurement.
/// Either pointer is null or writable (`ns` for three values). Counters
/// since init; read differences.
///
/// # Safety
/// `ns` is null or writable for three u64; `untimed` null or writable.
#[no_mangle]
pub unsafe extern "C" fn v3d_tiling_time(ns: *mut u64, untimed: *mut u32) {
    let g = g();
    // SAFETY: the caller's contract.
    unsafe {
        if !ns.is_null() {
            core::ptr::copy_nonoverlapping(g.tile_ns.as_ptr(), ns, TILE_WHERE);
        }
        if let Some(u) = untimed.as_mut() {
            *u = g.tile_untimed;
        }
    }
}

/// Tiled copies the TFU made since init, and those it was asked for and
/// failed (the CPU tiled them). Either pointer is null or writable.
///
/// # Safety
/// Each pointer is null or writable.
#[no_mangle]
pub unsafe extern "C" fn v3d_tfu_counters(jobs: *mut u32, failed: *mut u32) {
    let g = g();
    // SAFETY: the caller's contract.
    unsafe {
        if let Some(j) = jobs.as_mut() {
            *j = g.tfu_jobs;
        }
        if let Some(f) = failed.as_mut() {
            *f = g.tfu_failed;
        }
    }
}

/// Log the kept copies, one line each (slot, size, layers, levels, format,
/// bytes, and age in jobs since it was last sampled, 0 = the open job), and
/// return how many were live. For a one-time look at what fills the table.
#[no_mangle]
pub extern "C" fn v3d_tiled_dump() -> u32 {
    let g = g();
    let now = g.kicks.wrapping_add(1);
    let mut n = 0u32;
    for (i, t) in g.tiled.iter().enumerate().filter(|(_, t)| t.live()) {
        glog!(
            "yakogl/v3d: tiled[{}] {}x{} x{} layers, {} levels, format {}, {} bytes, age {} jobs",
            i,
            t.key.width,
            t.key.height,
            t.key.layers,
            t.key.levels,
            t.key.format,
            t.bytes,
            now.wrapping_sub(t.last_kick)
        );
        n += 1;
    }
    glog!(
        "yakogl/v3d: tiled copies: {} of {} entries live, {} of {} bytes",
        n,
        TILED_ENTRIES,
        g.tiled_live_bytes,
        g.tiled_budget
    );
    n
}

/// A sampler state record for binding `b` in the texture arena.
/// The job packs one record per sampler state and finds it again for the
/// next draw that samples with the same state (the arena lives as long as the
/// job, and a new job starts with an empty cache).
fn sampler_for(g: &mut V3dState, b: &GlesTextureBind) -> Result<u32, Attempt> {
    let s = texture::sampler(b);
    let j = &g.j;
    if let Some(i) = j.sampler_keys[..j.nsamplers].iter().position(|k| *k == s) {
        g.samplers_reused = g.samplers_reused.wrapping_add(1);
        return Ok(g.j.sampler_recs[i]);
    }
    let (rec, cpu) = tex_alloc(g, V3D_SAMPLER_STATE_BYTES, V3D_TEX_RECORD_ALIGN as usize).ok_or(Attempt::Full)?;
    // SAFETY: V3D_SAMPLER_STATE_BYTES of the arena.
    if !unsafe { v3d_tex_pack_sampler(cpu, &s) } {
        return Err(Attempt::Refused(
            "the sampler state does not fit its record (v3d_tex_pack_sampler refused a field)",
        ));
    }
    g.samplers_packed = g.samplers_packed.wrapping_add(1);
    let j = &mut g.j;
    if j.nsamplers < SAMPLER_CACHE {
        (j.sampler_keys[j.nsamplers], j.sampler_recs[j.nsamplers]) = (s, rec);
        j.nsamplers += 1;
    }
    Ok(rec)
}

/// Sampler records packed and found again in their job, since init. Either
/// pointer is null or writable.
///
/// # Safety
/// Each pointer is null or writable.
#[no_mangle]
pub unsafe extern "C" fn v3d_sampler_counters(packed: *mut u32, reused: *mut u32) {
    let g = g();
    // SAFETY: the caller's contract.
    unsafe {
        if let Some(p) = packed.as_mut() {
            *p = g.samplers_packed;
        }
        if let Some(p) = reused.as_mut() {
            *p = g.samplers_reused;
        }
    }
}

/// One stage's sampler bindings into `out`: how many. The unit in a P0/P1
/// uniform's data is the stage's sampler binding, which is
/// `GlesTextureBind::unit` (the GL core fills it with the binding and finds
/// the GL unit's texture itself, draw.rs); the fragment and the vertex stage
/// number their bindings apart, so each has its own table. A binding given
/// twice keeps its first. GL's incomplete texture samples the backend's own
/// (0, 0, 0, 1).
///
/// A unit the stage's uniform streams sample (a TMU P0/P1 word, the unit in
/// its data's bits 24..31) that no binding names also gets the stand-in: the
/// GL core leaves a sampler with texture object 0 bound out of the draw
/// (nothing to bind), and texture object 0 is incomplete, so GL ES 2.0 3.7.10
/// says it samples (0, 0, 0, 1). Without it the job builder refuses the draw
/// (a P0/P1 word for a unit it has no records for).
///
/// Each entry carries the base level's size the record samples from, which
/// the stream's `QUNIFORM_TEXTURE_WIDTH` / `_HEIGHT` words read (textureSize,
/// and the face size of the ES 2.0 non-seamless cube look-up's level of
/// detail, prepare.rs `cube_lod`); a unit only such a word names gets the
/// stand-in too (1 x 1).
fn texture_units(
    g: &mut V3dState,
    binds: &[GlesTextureBind],
    streams: &[(*const V3dUniform, u32)],
    cube_units: u32,
    out: &mut [V3dJobTmu; MAX_UNITS],
) -> Result<usize, Attempt> {
    /* a unit whose look-ups the program compiled for the 32-bit TMU output:
     * P1's bit 0 (select.rs, LodKey::ret32) */
    let ret32_units = streams.iter().fold(0u64, |m, &(u, count)| {
        let words: &[V3dUniform] = if u.is_null() || count == 0 {
            &[]
        } else {
            // SAFETY: a program's stream of `count` words, live for the draw.
            unsafe { core::slice::from_raw_parts(u, count as usize) }
        };
        words
            .iter()
            .filter(|w| w.contents == V3D_QUNIFORM_TMU_CONFIG_P1 && w.data & 1 != 0 && w.data >> 24 < 64)
            .fold(m, |m, w| m | 1 << (w.data >> 24))
    });
    let mut n = 0;
    for b in binds.iter().take(MAX_UNITS) {
        if out[..n].iter().any(|t| t.unit == b.unit as u32) {
            continue;
        }
        let ret32 = b.unit < 64 && ret32_units & (1 << b.unit) != 0;
        let tex_state = if b.incomplete == 0 {
            tex_state_for(g, b, ret32)?
        } else if ret32 || texture::format_is_integer(b.format) {
            black_typed_state(g, b, ret32)?
        } else {
            black_state(g, b.type_ == GLES_TEXTYPE_CUBE)
        };
        let sampler = sampler_for(g, b)?;
        let (width, height) = if b.incomplete != 0 { (1, 1) } else { (b.width, b.height) };
        out[n] = V3dJobTmu { unit: b.unit as u32, tex_state, sampler, width, height };
        n += 1;
    }
    let mut black_sampler = None;
    for &(u, count) in streams {
        let words: &[V3dUniform] = if u.is_null() || count == 0 {
            &[]
        } else {
            // SAFETY: a program's stream of `count` words, live for the draw.
            unsafe { core::slice::from_raw_parts(u, count as usize) }
        };
        for w in words {
            /* the unit: P0/P1 carry it in bits 24..31, the size words
             * (select.rs QUNIFORM_TEXTURE_WIDTH / _HEIGHT, Mesa's values)
             * as their whole data */
            let unit = match w.contents {
                V3D_QUNIFORM_TMU_CONFIG_P0 | V3D_QUNIFORM_TMU_CONFIG_P1 => w.data >> 24,
                QUNIFORM_TEXTURE_WIDTH | QUNIFORM_TEXTURE_HEIGHT => w.data,
                _ => continue,
            };
            if out[..n].iter().any(|t| t.unit == unit) {
                continue;
            }
            if unit < 64 && ret32_units & (1 << unit) != 0 {
                return Err(Attempt::Refused(
                    "an incomplete texture read by a look-up compiled for the 32-bit TMU output",
                ));
            }
            if n == MAX_UNITS {
                return Err(Attempt::Refused("a stage samples more units than the V3D's tables hold"));
            }
            let sampler = match black_sampler {
                Some(s) => s,
                None => {
                    let s = sampler_for(g, &black_binding())?;
                    black_sampler = Some(s);
                    s
                }
            };
            let cube = unit < 32 && cube_units & (1 << unit) != 0;
            out[n] = V3dJobTmu { unit, tex_state: black_state(g, cube), sampler, width: 1, height: 1 };
            n += 1;
        }
    }
    Ok(n)
}

/// The stand-in's texture state record: the 1x1 2D one, or the 1x1 cube.
fn black_state(g: &V3dState, cube: bool) -> u32 {
    g.black.gpu as u32 + if cube { BLACK_CUBE_OFF + BLACK_STATE_OFF } else { BLACK_STATE_OFF }
}

/// The stand-in record of an incomplete texture `b` of a class the RGBA8 one
/// cannot answer for: an integer class, whose look-up reads raw integers
/// (the RGBA8 record's f16 1.0 would read 0x3c00), or one of 32-bit
/// channels read at the 32-bit output (`ret32`). The RGBA8 stand-in's image
/// and layout (`init_black`) with the class's own TMU type
/// ([`texture::tmu_format`]) and the swizzle 0, 0, 0, 1, which the TMU
/// gives in the type's own terms (an integer type's ONE is the integer 1,
/// as Mesa's R8UI row's SWIZ_X001 relies on, v3dx_format_table.c:119): GL's
/// (0, 0, 0, 1), whatever the texels hold. Packed into this job's texture
/// arena, uncached.
fn black_typed_state(g: &mut V3dState, b: &GlesTextureBind, ret32: bool) -> Result<u32, Attempt> {
    if ret32 != texture::return_32(b) {
        return Err(Attempt::Refused("an incomplete texture read by a look-up compiled for the 32-bit TMU output"));
    }
    let cube = b.type_ == GLES_TEXTYPE_CUBE;
    let (off, stand_in) = if cube { (BLACK_CUBE_OFF, black_cube_binding()) } else { (0, black_binding()) };
    let layout =
        texture::layout(&stand_in).ok_or(Attempt::Refused("the incomplete texture's stand-in has no layout"))?;
    let (type_, _) = texture::tmu_format(&GlesTextureBind { format: b.format, ..stand_in });
    let st = V3dTexState {
        type_,
        swizzle: [V3D_SWIZZLE_ZERO, V3D_SWIZZLE_ZERO, V3D_SWIZZLE_ZERO, V3D_SWIZZLE_ONE],
        ..texture::state(&stand_in, &layout, g.black.gpu as u32 + off)
    };
    let (rec, rec_cpu) = tex_alloc(g, V3D_TEX_STATE_BYTES, V3D_TEX_RECORD_ALIGN as usize).ok_or(Attempt::Full)?;
    // SAFETY: V3D_TEX_STATE_BYTES of the arena.
    if !unsafe { v3d_tex_pack_state(rec_cpu, &st) } {
        return Err(Attempt::Refused("the texture state does not fit its record (v3d_tex_pack_state refused a field)"));
    }
    Ok(rec)
}

/// The sampler bindings `sh`'s uniform table declares as cube samplers, as
/// a mask by binding (an array's elements take the bindings after its
/// first).
fn cube_units_of(sh: &ShcShader) -> u32 {
    let mut m = 0u32;
    for u in sh.uniform_table() {
        if u.gl_type == GL_SAMPLER_CUBE || u.gl_type == GL_SAMPLER_CUBE_SHADOW {
            for k in 0..u32::from(u.array_len.max(1)) {
                let b = u32::from(u.binding) + k;
                if b < 32 {
                    m |= 1 << b;
                }
            }
        }
    }
    m
}

/// The incomplete texture's stand-in as a binding: 1x1 RGBA8 at the GL
/// core's pair-padded pitch, one level, NEAREST (`init_black` lays it out
/// from this; a unit no binding names is sampled with it).
fn black_binding() -> GlesTextureBind {
    GlesTextureBind {
        width: 1,
        height: 1,
        stride_texels: 2,
        layers: 1,
        mip_levels: 1,
        format: GLES_TEXFMT_RGBA8,
        ..GlesTextureBind::ZERO
    }
}

/// The incomplete texture's stand-in: a 1x1 RGBA8 (0, 0, 0, 1), tiled and
/// described once, in `g.black` (GL ES 2.0 3.7.10: an incomplete texture
/// samples (0, 0, 0, 1)).
fn init_black(g: &mut V3dState) -> bool {
    if g.black.cpu.is_null() {
        return false;
    }
    /* six faces of one texel each, stacked at the GL core's pitch (2) */
    let texels = [[0u8, 0, 0, 255]; 12];
    let cpu = g.black.cpu as *mut u8;
    for (off, b) in [(0u32, black_binding()), (BLACK_CUBE_OFF, black_cube_binding())] {
        let Some(layout) = texture::layout(&b) else {
            return false;
        };
        if texture::image_bytes(&b, &layout).is_none_or(|n| n > BLACK_STATE_OFF) {
            return false;
        }
        // SAFETY: the 8192-byte allocation holds each image (at most
        // BLACK_STATE_OFF bytes) at `off` and its record BLACK_STATE_OFF after
        // it; `texels` covers a cube's six stacked faces (gl_bytes 48).
        unsafe {
            texture::tile(&b, &layout, cpu.add(off as usize), texels.as_ptr() as *const u8);
            let st = texture::state(&b, &layout, g.black.gpu as u32 + off);
            if !v3d_tex_pack_state(cpu.add((off + BLACK_STATE_OFF) as usize), &st) {
                return false;
            }
        }
    }
    // SAFETY: the allocation's CPU view, just written.
    unsafe { osal::cache_flush(g.black.cpu, 8192) };
    true
}

/// The cube stand-in as a binding: 1x1, six faces, one level.
fn black_cube_binding() -> GlesTextureBind {
    GlesTextureBind { type_: GLES_TEXTYPE_CUBE, layers: 6, ..black_binding() }
}

/// Record one draw of the open job's target, in a new job when the open one
/// is full.
fn draw_in_job(g: &mut V3dState, target: Target, d: &GlesDraw, st: &V3dJobState, fs: &ShcShader) -> i32 {
    let mut retried_for_memory = false;
    for attempt in 0..2 {
        if !g.j.open {
            let rc = begin_job(g, target);
            if rc < 0 {
                return rc;
            }
        }
        /* the second attempt is a job begun for this draw alone: data the
         * arena cannot take gets memory of its own there (stage) */
        g.spill_ok = attempt == 1 && g.j.job.draws == 0;
        let recorded = record(g, d, st, fs);
        g.spill_ok = false;
        match recorded {
            Ok(()) => {
                if attempt == 1 && retried_for_memory {
                    g.memory_retries_drawn += 1;
                }
                return 0;
            }
            Err(Attempt::Refused(why)) => return refuse(g, why),
            Err(Attempt::Full) if attempt == 0 => {
                let rc = flush_job(g);
                if rc < 0 {
                    return rc;
                }
            }
            Err(Attempt::Full) => break,
            Err(Attempt::NoMemory(_)) if attempt == 0 => {
                /* the frees held for the open job go back once it has run
                 * (flush_job submits it and tells the OSAL no job is open;
                 * the wait drains them) */
                g.memory_retries += 1;
                retried_for_memory = true;
                if g.memory_retries <= 16 {
                    glog!(
                        "yakogl/v3d: no GPU memory for a kept tiled copy with a job open ({} draws in it): the job \
                         runs first, and the draw is tried in a new one (retry {})",
                        g.j.job.draws,
                        g.memory_retries
                    );
                }
                let mut rc = flush_job(g);
                if rc >= 0 {
                    rc = wait_in_flight(g);
                }
                if rc < 0 {
                    return rc;
                }
            }
            Err(Attempt::NoMemory(why)) => return refuse(g, why),
        }
    }
    glog_once!("yakogl/v3d: a draw does not fit an empty job ({} indices)", d.index_count);
    /* lost, by this backend's limits: a refusal to count (a harness scores
     * it as not drawn, not as a wrong picture); still -E2BIG for the core */
    count_refusal(g, "no empty job holds the draw (its lists, or its client data with no memory of its own)");
    -E2BIG
}

/// Draw the part of `d`'s clear the TLB cannot do as the clear quad
/// (`state::clear_quad_draw`).
fn draw_clear_quad(g: &mut V3dState, target: Target, d: &GlesDraw, b: state::QuadBuffers) -> i32 {
    /* an integer RT 0 takes the integer program's four raw words; a quad
     * that writes no colour keeps the float one (its mask is 0) */
    let p = match clear_program(g, b.colour && gles_surf_is_integer(target.format)) {
        Ok(p) => p,
        Err(why) => return refuse(g, why),
    };
    let vertices = state::clear_quad_vertices(d.clear_depth);
    let colour = d.clear_rgba;
    let q = state::clear_quad_draw(d, b, d.clear_quad, &p, &vertices, &colour);
    let st = match state::job_state(&q) {
        Ok(st) => st,
        Err(why) => return refuse(g, why),
    };
    draw_in_job(g, target, &q, &st, p.fs)
}

/* ---- the entry points --------------------------------------------------------- */

/// Allocate the job memory, and check the VPM the programs are compiled for.
pub extern "C" fn v3d_init() -> i32 {
    let g = g();
    if g.inited {
        return 0;
    }
    *g = V3dState::ZERO;

    /* The online compiler links every program for V3D_VPM_BYTES; a board
     * with another VPM would run programs whose VCM cache size is wrong. */
    let mut vpm = 0u32;
    let rc = match osal::ops().and_then(|o| o.query) {
        // SAFETY: `vpm` is a u32 of the size passed.
        Some(q) => unsafe { q(OSAL_QUERY_V3D_VPM_BYTES, &mut vpm as *mut u32 as *mut c_void, 4) },
        None => -ENODEV,
    };
    let want = crate::shaderc::online::codegen::V3D_VPM_BYTES;
    if rc != 0 || vpm != want {
        glog!(
            "yakogl/v3d: the VPM is {} bytes (query {}), the programs are compiled for {}: not starting",
            vpm,
            rc,
            want
        );
        return -ENODEV;
    }

    /* the draw budget one job is sized for: the OSAL's, or the default */
    let mut draws = 0u32;
    let asked = match osal::ops().and_then(|o| o.query) {
        // SAFETY: a u32 out-parameter of the size passed.
        Some(q) => unsafe { q(OSAL_QUERY_V3D_DRAWS_PER_JOB, &mut draws as *mut u32 as *mut c_void, 4) },
        None => -ENODEV,
    };
    let from = if asked == 0 && draws != 0 { "the platform's budget" } else { "the default" };
    if asked != 0 || draws == 0 {
        draws = DEFAULT_DRAWS_PER_JOB;
    }
    let Some((bcl_bytes, ind_bytes)) = arena_sizes(draws) else {
        glog!("yakogl/v3d: a job of {} draws does not fit 32-bit arenas: not starting", draws);
        return -ENOMEM;
    };
    (g.draws_per_job, g.bcl_bytes, g.ind_bytes) = (draws, bcl_bytes, ind_bytes);
    /* the tiled copies' budget: the OSAL's, or the default */
    let mut budget = 0u32;
    let asked = match osal::ops().and_then(|o| o.query) {
        // SAFETY: a u32 out-parameter of the size passed.
        Some(q) => unsafe { q(OSAL_QUERY_V3D_TILED_BUDGET, &mut budget as *mut u32 as *mut c_void, 4) },
        None => -ENODEV,
    };
    g.tiled_budget = if asked == 0 && budget != 0 { budget as usize } else { DEFAULT_TILED_BUDGET };
    /* the TFU for large kept copies: the OSAL's threshold, off without one */
    let mut tfu_min = 0u32;
    let asked = match osal::ops().and_then(|o| o.query) {
        // SAFETY: a u32 out-parameter of the size passed.
        Some(q) => unsafe { q(OSAL_QUERY_V3D_TFU_MIN_BYTES, &mut tfu_min as *mut u32 as *mut c_void, 4) },
        None => -ENODEV,
    };
    g.tfu_min_bytes = if asked == 0 { tfu_min as usize } else { 0 };
    if g.tfu_min_bytes != 0 {
        glog!("yakogl/v3d: tiled copies of {} bytes or more by the TFU", g.tfu_min_bytes);
    }
    /* depth and stencil apart: the OSAL's setting, off without an answer */
    let mut split = 0u32;
    let asked = match osal::ops().and_then(|o| o.query) {
        // SAFETY: a u32 out-parameter of the size passed.
        Some(q) => unsafe { q(OSAL_QUERY_V3D_SPLIT_ZS, &mut split as *mut u32 as *mut c_void, 4) },
        None => -ENODEV,
    };
    g.split_zs = asked == 0 && split != 0;
    if g.split_zs {
        glog!("yakogl/v3d: depth and stencil apart (by renderbuffer), the platform's setting");
    }
    /* stencil halves in S8 buffers: the OSAL's setting, off without an
     * answer, and only with the halves apart (V3dState::s8_stencil) */
    let mut s8 = 0u32;
    let asked = match osal::ops().and_then(|o| o.query) {
        // SAFETY: a u32 out-parameter of the size passed.
        Some(q) => unsafe { q(OSAL_QUERY_V3D_S8_STENCIL, &mut s8 as *mut u32 as *mut c_void, 4) },
        None => -ENODEV,
    };
    let s8_asked = asked == 0 && s8 != 0;
    g.s8_stencil = s8_asked && g.split_zs;
    if g.s8_stencil {
        glog!("yakogl/v3d: stencil halves in S8 buffers (1 byte a pixel), the platform's setting");
    } else if s8_asked {
        glog!("yakogl/v3d: S8 stencil asked for without depth and stencil apart: D24S8 kept");
    }

    let mut t = V3dTiling::default();
    let (mut ta, mut ts) = (0u32, 0u32);
    // SAFETY: plain computations on the out-parameters given.
    unsafe {
        /* the smallest tiles a job can have (4 render targets: 32x32), so
         * the pool holds the most tiles any job bins */
        v3d_frame_tiling_rts(MAX_TARGET, MAX_TARGET, 4, &mut t);
        /* Mesa's rule for the same draw budget: at this target size the
         * pool reaches its 512 KiB cap from 1024 draws; a binner that needs
         * more gets the OSAL's overflow blocks (v3d_wait_bin), and a job
         * that runs them dry fails by name (-ENOMEM) */
        v3d_frame_alloc_sizes(&t, draws, 4096, &mut ta, &mut ts);
    }
    (g.tile_alloc_size, g.tile_state_size) = (ta, ts);
    glog!(
        "yakogl/v3d: job arenas for {} draws ({}): BCL {}, indirect {}, binner tile pool {} bytes; \
         tiled copies kept up to {} bytes",
        draws,
        from,
        bcl_bytes,
        ind_bytes,
        ta,
        g.tiled_budget
    );
    for (mem, bytes) in [
        (&mut g.arenas.bcl, bcl_bytes as usize),
        (&mut g.arenas.rcl, RCL_BYTES),
        (&mut g.arenas.ind, ind_bytes as usize),
        (&mut g.arenas.data, DATA_BYTES),
        (&mut g.arenas.tex, TEX_BYTES),
        (&mut g.black, 8192),
        (&mut g.tile_alloc, ta as usize),
        (&mut g.tile_state, ts as usize),
    ] {
        if osal::gpu_alloc(V3D_HEAP_DATA, bytes, OSAL_MEM_DATA, mem) != 0
            || mem.cpu.is_null()
            || gpu32(mem.gpu).is_none()
        {
            glog!("yakogl/v3d: out of GPU memory for the job buffers ({} bytes)", bytes);
            return -ENOMEM;
        }
    }
    g.arenas.tex_bytes = TEX_BYTES;
    /* the second set, so a job is recorded while the one before runs; without
     * it every job waits for the one before (as before the async swap) */
    let mut spare = ArenaSet { tex_bytes: SPARE_TEX_BYTES, ..ArenaSet::ZERO };
    let mut spare_ok = true;
    for (mem, bytes) in [
        (&mut spare.bcl, bcl_bytes as usize),
        (&mut spare.rcl, RCL_BYTES),
        (&mut spare.ind, ind_bytes as usize),
        (&mut spare.data, DATA_BYTES),
        (&mut spare.tex, SPARE_TEX_BYTES),
    ] {
        spare_ok = spare_ok
            && osal::gpu_alloc(V3D_HEAP_DATA, bytes, OSAL_MEM_DATA, mem) == 0
            && !mem.cpu.is_null()
            && gpu32(mem.gpu).is_some();
    }
    if spare_ok {
        g.spare = spare;
        glog!(
            "yakogl/v3d: two job arena sets (the second's texture arena {} bytes): a job is recorded while the one \
             before runs",
            SPARE_TEX_BYTES
        );
    } else {
        for mem in [&mut spare.bcl, &mut spare.rcl, &mut spare.ind, &mut spare.data, &mut spare.tex] {
            osal::gpu_free(mem);
        }
        glog!("yakogl/v3d: no GPU memory for a second job arena set: every job waits for the one before");
    }
    /* The clear quad's program, by the product compiler, once. A failure
     * does not stop init: only the clears the TLB cannot do are refused. */
    if !init_black(g) {
        glog!("yakogl/v3d: the incomplete texture's stand-in could not be made");
        return -ENOMEM;
    }
    init_clear_program(g);
    g.storage_current = Some(rt_storage_current);
    g.inited = true;
    0
}

pub extern "C" fn v3d_shutdown() {
    let g = g();
    if !g.inited {
        return;
    }
    let _ = flush_job(g);
    let _ = wait_in_flight(g);
    for p in [&mut g.clear_program, &mut g.clear_program_int] {
        if p.status == ClearStatus::Built {
            crate::shaderc::release_shader(&p.fs);
            osal::gpu_free(&mut p.code);
        }
        *p = ClearProgram::NOT_BUILT;
    }
    glog!(
        "yakogl/v3d: tiled copies high-water {} of {} bytes ({} tilings, {} of them at upload, {} bytes tiled; \
         {} draws tried again after the open job ran for want of memory, {} of them drawn)",
        g.tiled_high_water,
        g.tiled_budget,
        g.tilings,
        g.eager_tilings,
        g.tiled_bytes,
        g.memory_retries,
        g.memory_retries_drawn
    );
    for i in 0..TILED_ENTRIES {
        if g.tiled[i].live() {
            free_tiled(g, i);
        }
    }
    glog!(
        "yakogl/v3d: render images: {} registered, {} draw bindings sampled from one, {} sent back to a copy by a \
         CPU write, {} not made for want of memory, {} mip chains made by the TFU ({} failed, {} written back), {} \
         bytes at most",
        g.rt_registered,
        g.rt_direct,
        g.rt_cpu_writes,
        g.rt_no_memory,
        g.rt_chains,
        g.rt_chains_failed,
        g.rt_chains_returned,
        g.rt_bytes_peak
    );
    glog!(
        "yakogl/v3d: TFU mipmap at shutdown: {} chains ({} queued behind a job, {} waited), refused: {}",
        g.rt_chains,
        g.mip_queued,
        g.mip_queue_full,
        MipRefusals(&g.mip_refused)
    );
    glog!("yakogl/v3d: {} jobs submitted, {} CPU waits for a job", g.kicks, g.waits);
    let ls = g.last_frame_stores;
    glog!(
        "yakogl/v3d: colour stores in the last frame: {} raster ({} bytes), {} render image ({} bytes); level 0 \
         written back for a CPU reader {} times",
        ls.raster,
        ls.raster_bytes,
        ls.image,
        ls.image_bytes,
        g.rt_level0_backs
    );
    for i in 0..RT_IMAGES {
        if g.rt_images[i].live() {
            rt_free(g, i, false);
        }
    }
    glog!(
        "yakogl/v3d: depth textures: {} draw bindings sampled from their depth buffer, {} CPU resolves into texels, \
         {} draws refused for a buffer not there to sample",
        g.depth_direct,
        g.depth_resolves,
        g.depth_refused
    );
    for b in g.zsbufs.iter_mut() {
        if b.live() {
            osal::gpu_free(&mut b.mem);
        }
        *b = ZsBuf::NONE;
    }
    g.zs_parked.clear();
    for b in g.msbufs.iter_mut() {
        if b.live() {
            osal::gpu_free(&mut b.mem);
        }
        *b = MsBuf::NONE;
    }
    for mem in [
        &mut g.arenas.bcl,
        &mut g.arenas.rcl,
        &mut g.arenas.ind,
        &mut g.arenas.data,
        &mut g.arenas.tex,
        &mut g.spare.bcl,
        &mut g.spare.rcl,
        &mut g.spare.ind,
        &mut g.spare.data,
        &mut g.spare.tex,
        &mut g.black,
        &mut g.tile_alloc,
        &mut g.tile_state,
        &mut g.oq,
    ] {
        osal::gpu_free(mem);
    }
    g.inited = false;
}

/* ---- occlusion queries ---------------------------------------------------------- */

/// Bytes between two handles' counters: one counter group of the TLB's
/// each. The hardware's counters are 4-byte words in groups of 16, each
/// group 1024-byte aligned (Mesa's v3dv_query.c:285-287 and its offsets
/// `group * 1024 + index * 4`, :347-348; gallium's counter is always a
/// fresh BO's offset 0, v3d_query_pipe.c:95-99). At a 64-byte stride every
/// handle but 16, 32 and 48 named word 16 or more of its group, no counter
/// at all, and the Pi 4 read 0 for every query (gl_test 196). Counter 0 of
/// its own group: the CPU zeroing or reading one never shares a cache line,
/// or a group, with a counter the GPU adds into for another handle
/// (docs/es3/occlusion-query.md 3.1).
const OQ_STRIDE: usize = 1024;
const OQ_BYTES: usize = GLES_QUERY_HANDLES as usize * OQ_STRIDE;

/// OCCLUSION_QUERY_COUNTER's address for a draw counted on `handle`
/// (`GlesDraw::occlusion_query`): its word of the counter buffer; 0 for
/// none, and for a handle `v3d_query_begin` never started a buffer for.
fn oq_addr(g: &V3dState, handle: u32) -> u32 {
    if handle == 0 || handle >= GLES_QUERY_HANDLES || g.oq.cpu.is_null() {
        return 0;
    }
    gpu32(g.oq.gpu + handle as u64 * OQ_STRIDE as u64).unwrap_or(0)
}

/// The CPU view of `handle`'s counter word, allocating the buffer first.
fn oq_word(g: &mut V3dState, handle: u32) -> Result<*mut u32, i32> {
    if handle == 0 || handle >= GLES_QUERY_HANDLES {
        return Err(-EINVAL);
    }
    if g.oq.cpu.is_null() {
        let mut mem = OsalGpuMem::ZERO;
        if osal::gpu_alloc(V3D_HEAP_DATA, OQ_BYTES, OSAL_MEM_DATA, &mut mem) != 0
            || mem.cpu.is_null()
            || gpu32(mem.gpu + OQ_BYTES as u64).is_none()
            || !mem.gpu.is_multiple_of(OQ_STRIDE as u64)
        {
            osal::gpu_free(&mut mem);
            glog_once!("yakogl/v3d: no GPU memory for the occlusion query counters ({} bytes)", OQ_BYTES);
            return Err(-ENOMEM);
        }
        g.oq = mem;
    }
    Ok((g.oq.cpu as *mut u8).wrapping_add(handle as usize * OQ_STRIDE) as *mut u32)
}

/// An occlusion query starts on `handle`: its counter is zeroed (Mesa's
/// v3d_begin_query: a fresh BO whose first word is 0, v3d_query_pipe.c:91-101).
/// The GL core has waited for every job that counted on it before.
pub extern "C" fn v3d_query_begin(handle: u32) -> i32 {
    let g = g();
    if !g.inited {
        return -ENODEV;
    }
    match oq_word(g, handle) {
        Ok(w) => {
            // SAFETY: the counter buffer's live mapping, one aligned word of
            // the handle's line; no job adds into it (the GL core waited).
            unsafe {
                w.write_volatile(0);
                osal::cache_flush(w as *mut c_void, 4);
            }
            0
        }
        Err(rc) => rc,
    }
}

/// The samples counted on `handle`: the word the TLB added every counted
/// job's count to (v3d_query_pipe.c:204-233, after the jobs completed).
///
/// # Safety
/// `samples` is null or valid for writing one u64.
pub unsafe extern "C" fn v3d_query_result(handle: u32, samples: *mut u64) -> i32 {
    let g = g();
    if !g.inited || samples.is_null() {
        return -EINVAL;
    }
    if handle == 0 || handle >= GLES_QUERY_HANDLES || g.oq.cpu.is_null() {
        return -EINVAL; /* never started */
    }
    let w = (g.oq.cpu as *mut u8).wrapping_add(handle as usize * OQ_STRIDE) as *mut u32;
    // SAFETY: the counter buffer's live mapping; every job that counted on
    // the handle has completed (the GL core's contract); `samples` is the
    // caller's live u64.
    unsafe {
        osal::cache_invd(w as *mut c_void, 4);
        *samples = u64::from(w.read_volatile());
    }
    0
}

/// Record one draw (see [`crate::backend::draw`] for the contract).
///
/// # Safety
/// `draw` is null or points at a valid `GlesDraw` whose pointers are live
/// for the call.
pub unsafe extern "C" fn v3d_draw(draw: *const GlesDraw) -> i32 {
    /* the id the GL core stamped this draw's target, textures and buffers
     * with (`v3d_kick_id`, asked before this call) */
    let stamped = g().kicks.wrapping_add(1);
    // SAFETY: the caller's contract, passed on.
    let rc = unsafe { draw_call(draw) };
    let g = g();
    if g.j.open && g.kicks.wrapping_add(1) != stamped {
        /* the draw ended the job it was stamped for and is in this one:
         * what it uses stays pending until this job has run (dEQP
         * fbo.render.texsubimage on 8f2c90a3 and 5ff3076c: the render into
         * the texture ended the window's clear job, its stamp named that
         * job, and glTexSubImage2D wrote under the render, which then ran
         * over the sub-image) */
        g.open_alias = stamped;
    }
    rc
}

/// The draw's colour attachments past the first (multiple render targets,
/// docs/online-shaderc/v3d-backend.md): how many render targets the job
/// configures, RT 1..3's addresses and their surface formats. The job
/// builder takes them at RT 0's size in any format with a TLB row
/// (`state::rt_format`); anything else is refused by name.
fn extra_targets(d: &GlesDraw) -> Result<(u8, [u32; 3], [u8; 3]), state::Refusal> {
    let n = d.num_targets.min(GLES_MAX_TARGETS as u8);
    let mut rt = [0u32; 3];
    let mut formats = [GLES_SURF_RGBA8888; 3];
    if n <= 1 {
        return Ok((0, rt, formats));
    }
    for (i, t) in d.targets[1..n as usize].iter().enumerate() {
        if t.gpu == 0 {
            continue; /* a GL_NONE draw buffer: configured, never stored */
        }
        if (t.w, t.h) != (d.target_w, d.target_h) {
            return Err("colour attachments of different sizes");
        }
        /* any format with a TLB row (state::rt_format): RGBA8, the ES 3.0
         * integer and float ones; the sRGB surface has none */
        if state::rt_format(t.format).is_none() {
            return Err("a colour attachment past the first of a format the backend does not render");
        }
        rt[i] = gpu32(t.gpu).ok_or("a colour attachment outside the V3D's 32-bit address space")?;
        formats[i] = t.format;
    }
    Ok((n, rt, formats))
}

/// `v3d_draw` without the stamp bookkeeping.
///
/// # Safety
/// As `v3d_draw`.
unsafe fn draw_call(draw: *const GlesDraw) -> i32 {
    let g = g();
    if !g.inited {
        return -ENODEV;
    }
    // SAFETY: the caller's contract.
    let Some(d) = (unsafe { draw.as_ref() }) else {
        return -EINVAL;
    };
    if let Some(why) = state::refusal(d) {
        return refuse(g, why);
    }
    if let Some(why) = state::unsupported(d) {
        return refuse_unsupported(g, why);
    }
    drop_stale_copies(g, d);
    /* target_gpu 0: no colour buffer (DRAWS_WITHOUT_COLOUR), a job at
     * address 0. Such a framebuffer always has a depth or stencil
     * attachment (the GL core's status), and is single-sampled. */
    let addr = if d.target_gpu == 0 {
        if !d.ds_persist {
            return refuse(g, "a draw with no colour target and no depth/stencil buffer");
        }
        if d.samples == 4 {
            return refuse(g, "4x multisampling with no colour target");
        }
        0
    } else {
        match gpu32(d.target_gpu) {
            Some(a) => a,
            None => return refuse(g, "the target is outside the V3D's 32-bit address space"),
        }
    };
    if d.target_w == 0 || d.target_h == 0 || d.target_w > MAX_TARGET || d.target_h > MAX_TARGET {
        return refuse(g, "a target larger than the tile memory is sized for (2048x2048), or empty");
    }
    let (rts, rt, rt_format) = match extra_targets(d) {
        Ok(x) => x,
        Err(why) => return refuse(g, why),
    };
    let target = Target {
        gpu: addr as u64,
        w: d.target_w,
        h: d.target_h,
        pitch: crate::backend::draw_pitch(d),
        no_alpha: d.target_alpha_bits == 0,
        bgr: d.target_format == GLES_SURF_XRGB8888,
        rgb565: d.target_format == GLES_SURF_RGB565,
        format: d.target_format,
        ds: d.ds_persist.then_some(d.ds_id),
        ds_owners: [d.depth_id, d.stencil_id],
        rts,
        rt,
        rt_format,
        msaa: d.samples == 4,
    };
    /* 4x of several colour attachments: the job builder keeps one RT's
     * samples (v3d_job.h); an FBO is single-sampled, so this needs a 4x
     * surface with attachments, which the GL core does not make */
    if target.msaa && rts > 1 {
        return refuse(g, "4x multisampling of several colour attachments");
    }
    /* 4x of an ES 3.0 colour format: the kept samples and the resolve are
     * RGBA8 (v3d_job.h); nothing in the GL core makes such a surface */
    if target.msaa && target.typed() {
        return refuse(g, "4x multisampling of an ES 3.0 colour format");
    }
    /* The clear quad writes gl_FragColor, RT 0 only, so a colour clear it
     * would do (scissored, or with a partial colour mask) cannot reach the
     * other attachments: refused, before anything is recorded. */
    if rts > 1 && d.do_clear && (d.clear_quad || state::colour_needs_quad(d)) {
        return refuse(g, "a scissored or masked colour clear of several colour attachments");
    }
    /* The draw's own state, checked before anything is recorded, so a
     * refused draw leaves its fused clear unrecorded too. A clear-only draw
     * has no state of its own (a quad makes its own, `clear_quad_draw`). */
    let st = if d.index_count != 0 {
        match state::job_state(d) {
            /* the occlusion query's counter (the clear quad, made from
             * its own GlesDraw, never carries one) */
            Ok(st) => Some(V3dJobState { oq_addr: oq_addr(g, d.occlusion_query), ..st }),
            Err(why) => return refuse(g, why),
        }
    } else {
        None
    };
    // SAFETY: the GL core hands a live linked fragment shader, or null.
    let fs = unsafe { d.fs.as_ref() };
    if d.index_count != 0 && fs.is_none() {
        return refuse(g, "a draw with no linked program");
    }
    /* A colour output at a location the framebuffer has no attachment slot
     * for: GL drops the write, but the TLB needs the RT configured to take
     * it, and the job's RT count comes from the framebuffer (the tile size
     * is fixed when the job starts). Refused by name until a program variant
     * keyed by the RT count exists (docs, "Multiple render targets"). */
    if let Some(fs) = fs.filter(|_| d.index_count != 0) {
        let slots = u32::from(rts.max(1));
        if u32::from(fs.v3d.fs.rt_mask) >> slots != 0 {
            return refuse(g, "a colour output with no attachment slot");
        }
    }

    if g.j.open && g.j.target != target {
        let rc = flush_job(g);
        if rc < 0 {
            return rc;
        }
    }
    /* Depth texels the CPU wrote go into the depth buffer before a job
     * loads it: the job open now (if any) ends first, and the draw begins
     * one of its own, which loads them (seed_depth). */
    // SAFETY: the GL core's contract for the draw it hands over.
    if let Some(t) = unsafe { ds_retention::DepthTexels::of(d) }.filter(|_| target.ds.is_some()) {
        let rc = flush_job(g);
        if rc < 0 {
            return rc;
        }
        let rc = seed_depth(g, &target, t);
        if rc < 0 {
            return rc;
        }
    }

    /* What the TLB clears: the tile buffer starts from it, so after draws
     * it needs a job of its own (Mesa's v3d_clear takes the TLB path only
     * for a job with no draw queued, v3dx_draw.c:1667-1673). A clear_quad
     * leaves all of it to the quad, and a colour clear with a partial mask
     * leaves its colour to the quad (`state::colour_needs_quad`). */
    let quad_colour = state::colour_needs_quad(d);
    let tlb_colour = d.do_clear && !quad_colour;
    let (mut tlb_depth, mut tlb_stencil) =
        if d.clear_quad { (false, false) } else { (d.do_clear_depth, d.do_clear_stencil) };
    /* GFXH-1461: with depth and stencil loaded from their buffer, the TLB
     * cannot clear one of them alone (the clear is lost when the other is
     * loaded, v3dx_draw.c:1676-1689); Mesa draws that half instead. So a
     * partial clear whose other half must be loaded goes to the quad. */
    let (mut quad_depth, mut quad_stencil) = (false, false);
    if tlb_colour || tlb_depth || tlb_stencil {
        if g.j.open && g.j.job.draws > 0 {
            let rc = flush_job(g);
            if rc < 0 {
                return rc;
            }
        }
        if !g.j.open {
            let rc = begin_job(g, target);
            if rc < 0 {
                return rc;
            }
        }
        let j = &mut g.j;
        /* decided on the job the clear goes into, now that it is open */
        if tlb_depth && !tlb_stencil && j.s_stored && !j.s_cleared {
            (tlb_depth, quad_depth) = (false, true);
        }
        if tlb_stencil && !tlb_depth && j.z_stored && !j.z_cleared {
            (tlb_stencil, quad_stencil) = (false, true);
        }
        if tlb_colour {
            /* glClear clears every draw buffer (ES 3.0 4.2.3): each RT the
             * job configures starts from the same colour; glClearBuffer(
             * GL_COLOR, n) names draw buffer n alone (`clear_target_mask`,
             * bit n), the others load what they hold. Each RT starts from
             * the colour as its own format holds it (`state::clear_words`:
             * the one RGBA8 word of an ES 2.0 format, as before; an integer
             * target's raw integers narrowed to its channels). */
            let mask = if d.clear_target_mask == 0 { 0xf } else { d.clear_target_mask };
            for (n, &f) in target.formats().iter().enumerate() {
                if mask & (1 << n) == 0 {
                    continue;
                }
                let w = state::clear_words(f, d.clear_rgba);
                j.clear.clear_rt[n] = w;
                if n == 0 {
                    j.clear.load_rt0 = false;
                    j.clear.rgba8 = w[0];
                } else {
                    j.clear.load_rt[n - 1] = false;
                    j.clear.rgba8_rt[n - 1] = w[0];
                }
            }
        }
        if tlb_depth {
            j.clear.z = d.clear_depth;
            (j.z_valid, j.z_cleared) = (true, true);
        }
        if tlb_stencil {
            j.clear.stencil = d.clear_stencil as u32;
            (j.s_valid, j.s_cleared) = (true, true);
        }
    }
    /* What the TLB cannot clear is drawn, in order, before the draw it may
     * be fused into (st_cb_clear.c:528, the quad after the TLB clear). */
    let b = state::QuadBuffers {
        colour: quad_colour,
        depth: (d.clear_quad && d.do_clear_depth) || quad_depth,
        stencil: (d.clear_quad && d.do_clear_stencil) || quad_stencil,
    };
    if b.colour || b.depth || b.stencil {
        let rc = draw_clear_quad(g, target, d, b);
        if rc != 0 {
            return rc;
        }
    }
    let (Some(fs), Some(st)) = (fs.filter(|_| d.index_count != 0), st) else {
        return 0; /* a clear-only draw: recorded */
    };
    draw_in_job(g, target, d, &st, fs)
}

pub extern "C" fn v3d_flush() -> i32 {
    let g = g();
    if !g.inited {
        return -ENODEV;
    }
    flush_job(g)
}

/// Submit the open job and wait for every submitted one.
pub extern "C" fn v3d_finish() -> i32 {
    let g = g();
    if !g.inited {
        return -ENODEV;
    }
    let rc = flush_job(g);
    let waited = wait_in_flight(g);
    if rc < 0 {
        rc
    } else {
        waited
    }
}

/// Wait until kick `kick_id` has completed, submitting nothing recorded
/// after it (`YakoglBackend::wait_kick`, for eglClientWaitSync and
/// glClientWaitSync). Three cases:
/// - the job in flight is that kick or a later one, or answers to it
///   (`InFlight::alias`): wait for it (`wait_in_flight`); only that job,
///   never the open one;
/// - an older kick than both jobs outstanding: it has completed already,
///   since `flush` waits for the older of two before it submits the next;
/// - a kick not submitted yet (the open job; a fence is made after a flush,
///   so a sync wait never names it): submit it and wait, as finish does.
pub extern "C" fn v3d_wait_kick(kick_id: u32) -> i32 {
    let g = g();
    if !g.inited {
        return -ENODEV;
    }
    if kick_id == 0 {
        return 0;
    }
    if kick_id > g.kicks {
        let rc = flush_job(g);
        let waited = wait_in_flight(g);
        return if rc < 0 { rc } else { waited };
    }
    let answers = |f: Option<InFlight>| f.is_some_and(|f| f.kick >= kick_id || f.alias == kick_id);
    if answers(g.in_flight) {
        /* the newer job: the older one runs before it */
        wait_in_flight(g)
    } else if answers(g.queued) {
        wait_queued(g)
    } else {
        0
    }
}

/// Wait for every job but the last submitted: with two jobs outstanding
/// (`flush_job`), that is the older one (`wait_queued`); the newest may still
/// run when this returns. The EGL swap calls this before it shows the previous
/// frame, which must be complete. Returns the older job's error.
pub extern "C" fn v3d_finish_previous() -> i32 {
    let g = g();
    if !g.inited {
        return -ENODEV;
    }
    wait_queued(g)
}

/// Textures tiled on the CPU since init, and their bytes: what tiling once
/// per job costs (docs/online-shaderc/v3d-backend.md, "M6 textures").
///
/// # Safety
/// Both pointers are null or writable.
#[no_mangle]
pub unsafe extern "C" fn v3d_texture_counters(tilings: *mut u32, bytes: *mut u64) {
    let g = g();
    // SAFETY: the caller's contract.
    unsafe {
        if let Some(t) = tilings.as_mut() {
            *t = g.tilings;
        }
        if let Some(b) = bytes.as_mut() {
            *b = g.tiled_bytes;
        }
    }
}

/// Draws recorded since init whose VS writes the point size into its VPM
/// output, so their record says so. For a board case that must show the path
/// ran: 0 means no such draw reached the job, not that one drew right. Wraps
/// at 2^32; read it as a difference.
#[no_mangle]
pub extern "C" fn v3d_psiz_draws() -> u32 {
    g().psiz_draws
}

/// Draws refused since init: every draw `refuse()` turned away (a state
/// the backend does not draw, a draw the job builder refused, the target, no
/// program) and every draw that does not fit even an empty job (-E2BIG: its
/// lists, or its client data with no memory of its own), where the log has
/// one line per cause. A failure that is not a refusal (a full job whose draw
/// goes into the next, the GPU, no device) is not counted. For a harness
/// that marks a case as refused when this grew across it (v3d_deqp); it
/// wraps at 2^32, so read it as a difference.
#[no_mangle]
pub extern "C" fn v3d_refused_draws() -> u32 {
    g().refused
}

/// The kick being recorded: the open job, or the next one. A draw's
/// `last_use` is this, so the GL core can ask whether that draw has run
/// before it writes what the draw reads (`texture.rs` `settle_texture`, the
/// buffer paths). The PowerVR's contract (`pvr_backend_kick_id`: the last
/// fence + 1).
pub extern "C" fn v3d_kick_id() -> u32 {
    g().kicks.wrapping_add(1)
}

/// Whether kick `kick_id` has not run yet: the one being recorded (the GL
/// core's `flush` then `finish` submits and waits for it), and the job in
/// flight until the OSAL has completed it (the async swap: `flush` returns
/// with it running). It used to answer false always, so a CPU write between
/// two draws of one job reached the memory the earlier draw still read
/// (2026-09-24). The open job also answers to `open_alias`: the stamp of a
/// draw that ended the job it was stamped for and was recorded into this
/// one; the job in flight answers to the alias it was submitted with
/// (`InFlight::alias`) until it completes.
pub extern "C" fn v3d_kick_pending(kick_id: u32) -> bool {
    let g = g();
    kick_id != 0
        && (kick_id > g.kicks
            || (g.j.open && kick_id == g.open_alias)
            || [g.queued, g.in_flight]
                .into_iter()
                .flatten()
                .any(|f| (kick_id >= f.kick || kick_id == f.alias) && !fence_done(f.fence)))
}

pub extern "C" fn v3d_heap_base(heap: GlesHeap) -> u64 {
    match heap {
        GLES_HEAP_SHADER | GLES_HEAP_PROGRAM => V3D_HEAP_SHADER,
        _ => V3D_HEAP_DATA,
    }
}

/// The jobs submitted and the CPU waits for one since init (also the host
/// harness's per-frame `gm2: v3d-frames` line, tools/hostharness/glmark2/shim.c).
///
/// # Safety
/// Both pointers are null or writable.
#[no_mangle]
pub unsafe extern "C" fn v3d_counters(kicks: *mut u32, waits: *mut u32) {
    let g = g();
    // SAFETY: the caller's contract.
    unsafe {
        if let Some(k) = kicks.as_mut() {
            *k = g.kicks;
        }
        if let Some(w) = waits.as_mut() {
            *w = g.waits;
        }
    }
}

/// The binner's pool as the last job that binned without error left it
/// (OSAL_QUERY_V3D_BIN_POOL): the jobs binned so far, the overflow blocks
/// that job took and the blocks there are, and the jobs submitted since
/// init. The jobs submitted are written always; false, with nothing else
/// written, when the OSAL has no answer (none, or no job binned yet). For
/// gl_test case 124 binovf.
///
/// # Safety
/// Each pointer is null or writable.
#[no_mangle]
pub unsafe extern "C" fn v3d_bin_last(
    jobs: *mut u32,
    overflow_used: *mut u32,
    overflow_blocks: *mut u32,
    kicks: *mut u32,
) -> bool {
    // SAFETY: the caller's contract.
    if let Some(k) = unsafe { kicks.as_mut() } {
        *k = g().kicks;
    }
    let Some(query) = osal::ops().and_then(|o| o.query) else { return false };
    let mut p = OsalV3dBinPool::default();
    // SAFETY: `p` is a live struct osal_v3d_bin_pool of the size passed.
    let rc = unsafe {
        query(
            OSAL_QUERY_V3D_BIN_POOL,
            &mut p as *mut OsalV3dBinPool as *mut c_void,
            core::mem::size_of::<OsalV3dBinPool>(),
        )
    };
    if rc != 0 {
        return false;
    }
    // SAFETY: the caller's contract.
    unsafe {
        for (out, v) in [(jobs, p.jobs), (overflow_used, p.overflow_used), (overflow_blocks, p.overflow_blocks)] {
            if let Some(o) = out.as_mut() {
                *o = v;
            }
        }
    }
    true
}

/// The V3D backend's table (`g_backend` on a `gpu-v3d` build).
#[no_mangle]
pub static yakogl_backend_v3d: YakoglBackend = YakoglBackend {
    name: c"Broadcom V3D 4.2 (Raspberry Pi 4)".as_ptr(),
    /* the attribute records read float32 (M6); the GL core converts the rest */
    native_attrib_formats: gles_attribf_bit(GLES_ATTRIBF_FLOAT32),
    /* STORE_TILE_BUFFER_GENERAL with RGBA8 writes R, G, B, A in memory: M4
     * read every byte of the clear back on the board */
    rt_rgba8888_is_bgra: false,
    mip_level_align: 0,
    compressed_blocks_in_rows: false,
    /* the TMU samples a cube itself (depth 6, array stride): the GL core
     * gives one face's size and the layer count, and stacks the faces */
    native_layered_textures: true,
    front_facing_shader_variant: false,
    /* no ETC1 row in Mesa's V3D 4.2 format table (v3dx_format_table.c):
     * Mesa decodes ETC1 at upload for this GPU too */
    etc1_decoded_at_upload: true,
    init: Some(v3d_init),
    shutdown: Some(v3d_shutdown),
    draw: Some(v3d_draw),
    flush: Some(v3d_flush),
    finish: Some(v3d_finish),
    finish_previous: Some(v3d_finish_previous),
    kick_id: Some(v3d_kick_id),
    kick_pending: Some(v3d_kick_pending),
    heap_base: Some(v3d_heap_base),
    counters: Some(v3d_counters),
    texture_released: Some(v3d_texture_released),
    surface_swapped: Some(v3d_surface_swapped),
    texture_uploaded: Some(v3d_texture_uploaded),
    /* the attribute record's 16-bit Instance Divisor (v3d_packet.xml:1493) */
    max_instance_divisor: 0xffff,
    wait_kick: Some(v3d_wait_kick),
    /* the TMU samples the tiled copies tex_state_for makes when the draw is
     * recorded (kept, or in the job's arena), never the GL core's storage */
    samples_texel_copies: true,
    attachment_gone: Some(v3d_attachment_gone),
    depth_resolve: Some(v3d_depth_resolve),
    query_begin: Some(v3d_query_begin),
    query_end: None,
    query_result: Some(v3d_query_result),
    /* a render target's mip chain is the TFU's, in its render image
     * (RtImage), and goes back into the storage before the CPU reads it */
    generate_mipmap: Some(v3d_generate_mipmap),
    return_mipmap_chain: Some(v3d_return_mipmap_chain),
    linear_row_align: 0,
    uniforms_host_only: true,
    ds_write: None,
    stencil_resolve: None,
};

#[cfg(test)]
#[path = "backend_tests.rs"]
mod backend_tests;
