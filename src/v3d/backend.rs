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
    self, OsalGpuMem, OsalV3dBinPool, OsalV3dCpuOf, OsalV3dTfu, OSAL_MEM_DATA, OSAL_QUERY_V3D_BIN_POOL,
    OSAL_QUERY_V3D_CPU_OF_GPU, OSAL_QUERY_V3D_DRAWS_PER_JOB, OSAL_QUERY_V3D_JOB_OPEN, OSAL_QUERY_V3D_S8_STENCIL,
    OSAL_QUERY_V3D_SPLIT_ZS, OSAL_QUERY_V3D_TFU_MIN_BYTES, OSAL_QUERY_V3D_TFU_RUN, OSAL_QUERY_V3D_TILED_BUDGET,
    OSAL_QUERY_V3D_VPM_BYTES,
};
use crate::shaderc::online::v3d::decode::decode as qpu_decode;
use crate::shaderc::online::v3d::isa::Instr as QpuInstr;
use crate::shaderc::online::v3d::select::{QUNIFORM_TEXTURE_HEIGHT, QUNIFORM_TEXTURE_WIDTH};
use crate::shaderc::{ShcShader, ShcV3dVs, SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX, SHC_V3D_NO_RECORD};
use crate::util::{Global, EINVAL, EIO, ENODEV, ENOMEM, ENOTSUP};

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
}

impl TexKey {
    const ZERO: TexKey =
        TexKey { base_gpu: 0, width: 0, height: 0, levels: 0, bgra: 0, format: 0, content_gen: 0, layers: 0 };

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

/// The largest target the tile memory is sized for at init.
pub const MAX_TARGET: u32 = 2048;

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
    ds: Option<u32>,
    /// The depth and stencil objects behind `ds` (`GlesDraw::depth_id` and
    /// `stencil_id`): the owners of its buffers.
    ds_owners: [u32; 2],
    /// Render targets the draw's framebuffer has (0 and 1: RT 0 alone), and
    /// the GPU address of RT 1..3 (0: a GL_NONE draw buffer), each RGBA8 at
    /// RT 0's size. Part of the job's identity: another set is another job.
    rts: u8,
    rt: [u32; 3],
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
        ds: None,
        ds_owners: [0; 2],
        rts: 0,
        rt: [0; 3],
        msaa: false,
    };

    /// Bytes a pixel of the target.
    fn bpp(&self) -> u32 {
        if self.rgb565 {
            2
        } else {
            4
        }
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

/// The job submitted and not yet waited for: its kick and the OSAL's fence.
/// At most one: a job is submitted only after the one before it has been
/// waited for (`flush_job`), as the OSAL runs one job at a time.
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
    /// The job submitted and not yet waited for, and the fault record of the
    /// last job submitted.
    in_flight: Option<InFlight>,
    ended: JobRecord,
    /// The incomplete texture's image and state record, made at init.
    black: OsalGpuMem,
    tile_alloc: OsalGpuMem,
    tile_state: OsalGpuMem,
    tile_alloc_size: u32,
    tile_state_size: u32,
    j: Job,
    clear_program: ClearProgram,
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
        ended: JobRecord::ZERO,
        black: OsalGpuMem::ZERO,
        tile_alloc: OsalGpuMem::ZERO,
        tile_state: OsalGpuMem::ZERO,
        tile_alloc_size: 0,
        tile_state_size: 0,
        j: Job::ZERO,
        clear_program: ClearProgram::NOT_BUILT,
        zs: [ZsEntry { target: 0, z: ZsKnown::Untouched, s: ZsKnown::Untouched }; ZS_TARGETS],
        nzs: 0,
        zsbufs: [ZsBuf::NONE; ZS_BUFS],
        zs_parked: Parked::new(0, ZsKnown::Untouched),
        msbufs: [MsBuf::NONE; MS_BUFS],
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
    let Target { gpu, w, h, pitch, no_alpha, bgr, rgb565, ds, msaa, .. } = target;
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
    /* a target with a depth/stencil buffer starts from its history; one
     * without, from the per-target history of what the tile buffer held */
    let (zkey, skey) = match ds {
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
    /* every slot holding stored contents, or no memory for a new buffer:
     * the draw is refused by name (zs_buf), never drawn on-chip only */
    let zs = match zkey.map(|k| zs_buf(g, k, zown, w, h, msaa, false, None)) {
        Some(Err(rc)) => return rc,
        Some(Ok(i)) => Some(i),
        None => None,
    };
    let ss = match skey.map(|k| zs_buf(g, k, sown, w, h, msaa, s8, zs)) {
        Some(Err(rc)) => return rc,
        Some(Ok(i)) => Some(i),
        None => None,
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
        /* RT 1..3 of the draw's framebuffer, RGBA8 at RT 0's size and
         * pitch (a GL_NONE one has address 0: configured, never stored) */
        rts: u32::from(target.rts),
        rt: target.rt.map(|a| V3dJobRt { address: a, stride: if a != 0 { target_pitch(w) * 4 } else { 0 } }),
        msaa,
        ms_address: ms.map_or(0, |i| g.msbufs[i].address),
        ms_memory_format: ms.map_or(0, |i| g.msbufs[i].memory_format),
        ms_height_in_ub: ms.map_or(0, |i| g.msbufs[i].height_in_ub),
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
    /* the job before has run before this one is submitted: the OSAL runs one
     * job at a time, and its fault record is still the one kept */
    let prev = wait_in_flight(g);
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
/// when no job in flight holds it, else the small one (at most one job is in
/// flight: `flush_job` waits for it before it submits the next, so the set
/// it does not hold is free). A tiling that asked for the 8 MiB set
/// (`want_big_arena`) gets it, after a wait when the job in flight holds it;
/// so does every job when there is no second set.
fn choose_arenas(g: &mut V3dState) -> i32 {
    let big = core::mem::take(&mut g.want_big_arena);
    /* a set is busy while the job in flight holds it and has not completed */
    let busy = |set: &ArenaSet, f: Option<InFlight>| {
        set.kick != 0 && f.is_some_and(|f| f.kick == set.kick && !fence_done(f.fence))
    };
    if g.spare.live() {
        let spare_is_big = g.spare.tex_bytes > g.arenas.tex_bytes;
        let (arenas_busy, spare_busy) = (busy(&g.arenas, g.in_flight), busy(&g.spare, g.in_flight));
        let take_spare = if big || !(arenas_busy || spare_busy) {
            spare_is_big /* the 8 MiB set */
        } else {
            arenas_busy /* the one the job in flight does not hold */
        };
        if take_spare {
            core::mem::swap(&mut g.arenas, &mut g.spare);
        }
    }
    if busy(&g.arenas, g.in_flight) {
        return wait_in_flight(g);
    }
    0
}

/// Wait for the job in flight, if any: 0, or its error (after an MMU fault,
/// with the job's fault record logged).
fn wait_in_flight(g: &mut V3dState) -> i32 {
    let Some(f) = g.in_flight.take() else {
        return 0;
    };
    let Some(wait) = osal::ops().and_then(|o| o.wait) else {
        return -ENODEV;
    };
    // SAFETY: a wait on a fence the OSAL returned.
    let rc = unsafe { wait(f.fence, JOB_TIMEOUT_US) };
    g.waits += 1;
    if rc != 0 {
        glog_once!("yakogl/v3d: job {} did not complete ({})", f.kick, rc);
        if rc == -EFAULT {
            fault_record(g);
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
fn job_tile_state_bytes(t: &V3dJobTarget) -> u32 {
    let mut tiling = V3dTiling::default();
    // SAFETY: plain computations on the out-parameter given.
    unsafe { v3d_frame_tiling_ms(t.width, t.height, t.rts.max(1), t.msaa, &mut tiling) };
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
        tile_state_size: job_tile_state_bytes(&j.job.t).min(g.tile_state_size),
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

/// The clear program `v3d_init` built. `Err` is the refusal every clear quad
/// gets when it did not build; init logged the chain's error.
fn clear_program(g: &mut V3dState) -> Result<state::ClearProg<'static>, state::Refusal> {
    let p = &g.clear_program;
    if p.status != ClearStatus::Built {
        return Err("the clear program did not build, so a clear the TLB cannot do is not drawn");
    }
    // SAFETY: the program lives in the backend's static state until shutdown,
    // which no draw outlives.
    let fs: &'static ShcShader = unsafe { &*(&p.fs as *const ShcShader) };
    Ok(state::ClearProg { fs, code_gpu: p.code.gpu, location: p.location })
}

/// The text of a compiler log (NUL-terminated in `log`).
fn log_text(log: &[core::ffi::c_char]) -> &str {
    // SAFETY: c_char and u8 have the same size and alignment.
    let bytes = unsafe { core::slice::from_raw_parts(log.as_ptr() as *const u8, log.len()) };
    let n = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    core::str::from_utf8(&bytes[..n]).unwrap_or("(the log is not UTF-8)")
}

/// Build the clear program at init and say once what happened: its code
/// size, or the stage that failed and the compiler's own error.
fn init_clear_program(g: &mut V3dState) {
    use core::ffi::c_char;
    let mut log = [0 as c_char; 256];
    g.clear_program = match build_clear_program(&mut log) {
        Ok(p) => {
            glog!("yakogl/v3d: clear program built: {} bytes of code", p.fs.usc_dw * 4);
            p
        }
        Err(what) => {
            glog!("yakogl/v3d: clear program did not build: {}: {}", what, log_text(&log));
            ClearProgram::FAILED
        }
    };
}

/// Compile and link the clear program through the shader back end, as the
/// GL core compiles an application's, and upload its code. The compiler's
/// error, when there is one, is left in `log`.
fn build_clear_program(log: &mut [core::ffi::c_char; 256]) -> Result<ClearProgram, &'static str> {
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
        unsafe { compile(SHC_STAGE_VERTEX, CLEAR_VS.as_ptr() as *const c_char, &mut vs, log.as_mut_ptr(), log.len()) };
    if rc != 0 {
        return Err("the vertex shader did not compile");
    }
    // SAFETY: as above.
    let rc = unsafe {
        compile(SHC_STAGE_FRAGMENT, CLEAR_FS.as_ptr() as *const c_char, &mut fs, log.as_mut_ptr(), log.len())
    };
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
        let by_alias = g.in_flight.is_some_and(|f| b.target_kick < f.kick && b.target_kick == f.alias);
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
    let (min, mut tfu) = (g.tfu_min_bytes, Tfu::NotTried);
    let fill = |image: *mut u8, image_gpu: u32| {
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
            image_gpu
        }
    };
    let st = texture::state(b, &layout, image_gpu);
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
        || b.format != GLES_TEXFMT_RGBA8
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
    let (min, mut tfu) = (g.tfu_min_bytes, Tfu::NotTried);
    let fill = |image: *mut u8, image_gpu: u32| {
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
            return 0;
        }
        ZsKnown::Stored if !b.msaa => {}
        _ => return 1,
    }
    let mut l = V3dTexLayout::default();
    // SAFETY: `l` is a live layout to fill.
    if !unsafe { v3d_tex_layout(b.w, b.h, 1, 4, &mut l) } || l.slices[0].offset != 0 {
        return -EINVAL;
    }
    if l.slices[0].tiling == V3D_TEX_TILING_UIF_XOR {
        l.slices[0].tiling = V3D_TEX_TILING_UIF_NO_XOR;
    }
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
    0
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
/// The refusal of a draw whose program reads a texture's size (textureSize):
/// the words are compiled (select.rs) but not filled by the job builder yet.
pub(crate) const TEXTURE_SIZE_NOT_FILLED: &str = "textureSize: the job builder does not fill the size words yet";

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
        if b.incomplete != 0 && ret32 {
            return Err(Attempt::Refused("an incomplete texture read by a look-up compiled for the 32-bit TMU output"));
        }
        let tex_state =
            if b.incomplete != 0 { black_state(g, b.type_ == GLES_TEXTYPE_CUBE) } else { tex_state_for(g, b, ret32)? };
        let sampler = sampler_for(g, b)?;
        out[n] = V3dJobTmu { unit: b.unit as u32, tex_state, sampler };
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
            /* textureSize's words (select.rs QUNIFORM_TEXTURE_WIDTH /
             * _HEIGHT, Mesa's values): the job builder has no case for them
             * yet (it would fail the job, v3d_job.c bad_field), so the draw
             * is refused by name before anything is recorded. */
            if w.contents == QUNIFORM_TEXTURE_WIDTH || w.contents == QUNIFORM_TEXTURE_HEIGHT {
                return Err(Attempt::Refused(TEXTURE_SIZE_NOT_FILLED));
            }
            if w.contents != V3D_QUNIFORM_TMU_CONFIG_P0 && w.contents != V3D_QUNIFORM_TMU_CONFIG_P1 {
                continue;
            }
            let unit = w.data >> 24;
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
            out[n] = V3dJobTmu { unit, tex_state: black_state(g, cube), sampler };
            n += 1;
        }
    }
    Ok(n)
}

/// The stand-in's texture state record: the 1x1 2D one, or the 1x1 cube.
fn black_state(g: &V3dState, cube: bool) -> u32 {
    g.black.gpu as u32 + if cube { BLACK_CUBE_OFF + BLACK_STATE_OFF } else { BLACK_STATE_OFF }
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
    let p = match clear_program(g) {
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
    if g.clear_program.status == ClearStatus::Built {
        crate::shaderc::release_shader(&g.clear_program.fs);
        osal::gpu_free(&mut g.clear_program.code);
    }
    g.clear_program = ClearProgram::NOT_BUILT;
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
    ] {
        osal::gpu_free(mem);
    }
    g.inited = false;
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
/// configures and RT 1..3's addresses. The job builder takes them RGBA8 at
/// RT 0's size; anything else is refused by name.
fn extra_targets(d: &GlesDraw) -> Result<(u8, [u32; 3]), state::Refusal> {
    let n = d.num_targets.min(GLES_MAX_TARGETS as u8);
    let mut rt = [0u32; 3];
    if n <= 1 {
        return Ok((0, rt));
    }
    for (i, t) in d.targets[1..n as usize].iter().enumerate() {
        if t.gpu == 0 {
            continue; /* a GL_NONE draw buffer: configured, never stored */
        }
        if (t.w, t.h) != (d.target_w, d.target_h) {
            return Err("colour attachments of different sizes");
        }
        if t.format != GLES_SURF_RGBA8888 {
            return Err("a colour attachment past the first that is not RGBA8");
        }
        rt[i] = gpu32(t.gpu).ok_or("a colour attachment outside the V3D's 32-bit address space")?;
    }
    Ok((n, rt))
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
    let (rts, rt) = match extra_targets(d) {
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
        ds: d.ds_persist.then_some(d.ds_id),
        ds_owners: [d.depth_id, d.stencil_id],
        rts,
        rt,
        msaa: d.samples == 4,
    };
    /* 4x of several colour attachments: the job builder keeps one RT's
     * samples (v3d_job.h); an FBO is single-sampled, so this needs a 4x
     * surface with attachments, which the GL core does not make */
    if target.msaa && rts > 1 {
        return refuse(g, "4x multisampling of several colour attachments");
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
            Ok(st) => Some(st),
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
             * job configures starts from the same colour */
            j.clear.load_rt0 = false;
            j.clear.rgba8 = state::pack_rgba8(d.clear_rgba);
            j.clear.load_rt = [false; 3];
            j.clear.rgba8_rt = [j.clear.rgba8; 3];
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
/// - an older kick: it has completed already, since `flush` waits for the
///   job in flight before it submits the next;
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
    match g.in_flight {
        Some(f) if f.kick >= kick_id || f.alias == kick_id => wait_in_flight(g),
        _ => 0,
    }
}

/// Wait for every job but the last submitted: `flush` waits for the job in
/// flight before it submits the next, so at most the last one runs.
pub extern "C" fn v3d_finish_previous() -> i32 {
    if g().inited {
        0
    } else {
        -ENODEV
    }
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
            || g.in_flight.is_some_and(|f| (kick_id >= f.kick || kick_id == f.alias) && !fence_done(f.fence)))
}

pub extern "C" fn v3d_heap_base(heap: GlesHeap) -> u64 {
    match heap {
        GLES_HEAP_SHADER | GLES_HEAP_PROGRAM => V3D_HEAP_SHADER,
        _ => V3D_HEAP_DATA,
    }
}

/// # Safety
/// Both pointers are null or writable.
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
};

#[cfg(test)]
#[path = "backend_tests.rs"]
mod backend_tests;
