// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! One frame for the Mali-G610: the descriptors a clear and one draw read,
//! and the command stream that runs them (milestones L1/L2 of
//! driver/docs/nanopc-t6-mali.md).
//!
//! The reference is Mesa 25.0.7, the version the board ran when its traces
//! were taken (tools/mali/board-dump/tri-frame-2026-10-18.txt), read in its
//! own source (src/gallium/drivers/panfrost/pan_csf.c, pan_cmdstream.c,
//! pan_cmdstream.h, pan_job.c; src/panfrost/lib/pan_desc.c, pan_clear.c,
//! pan_samples.c, pan_util.h, pan_format.{c,h}; MIT). The descriptors are
//! the generated packers of [`super::genxml::v10`] and the instructions the
//! encoder of [`super::cs`]. Each value says where Mesa takes it from; where
//! Mesa 26.2.1 (the tree the packers come from) chooses differently, the
//! comment says so too. `frame_tests.rs` builds the board's frame and
//! compares every byte and every word with the trace.
//!
//! A frame is built in three steps, as Mesa builds a batch: [`begin_frame`]
//! (the batch's init words and the framebuffer descriptor's room),
//! [`add_draw`] once per draw, and [`end_frame`] (the framebuffer
//! descriptor, the fragment job, the batch end). [`build_frame`] is the
//! three for a frame of at most one draw. A frame is one RUN_FRAGMENT: its
//! draws share one tiler context, and every draw after the first is
//! depth-tested against the ones before it in the tile buffer.
//!
//! A 4x frame is a 4x view of its single-sampled target: averaged into it
//! on writeback. A 4x frame split into several kicks keeps its samples in a
//! [`SampleStore`] instead, as Mesa keeps a multisampled surface
//! (tools/cts/log/2026-10-28-splitref): each kick writes the four samples
//! back there, the next draws them back once a sample, and a last frame
//! with the store's `resolve` averages them into the target
//! (`frame_split_tests.rs`). The backend keeps every 4x frame's samples so
//! (src/mali/backend.rs, "the sample store").
//!
//! Depth and stencil: a draw carries the depth test, depth write and the
//! two stencil faces ([`DepthStencilState`], Mesa's ZSA CSO and its
//! draw-time part), and the frame a linear D24S8 depth/stencil attachment
//! ([`ZsAttachment`]), written back at the frame's end through the ZS/CRC
//! extension, as Mesa does on this GPU (the board's trace of 2026-10-19,
//! 0dce654a8:tools/mali/board-ref/2026-10-19-frame-depth). The tile buffer starts at
//! the attachment's two clear values: the builder has no preload, so the
//! caller gives an attachment only to a frame whose depth and stencil start
//! from known values. A draw writes colour through a colour mask; a draw
//! that writes no colour runs no fragment shader, as in Mesa's frame
//! (`panfrost_fs_required`), and writes only depth and stencil.
//!
//! What the builder does not build, each refused by name ([`FrameError`]):
//! a frame without a colour clear (that needs the preload of the old image),
//! instancing, a vertex
//! attribute other than one to four 32-bit floats, a fragment shader that
//! does more than write colour. A window (top-down) target is drawn with
//! Mesa's flipped viewport ([`DrawCall::flip_y`]). A vertex shader's varyings are built when it carries
//! the IDVS varying program (Mesa's `secondary_*`, [`SecondaryProgram`]).
//! Its input has no blending or culling: the descriptors are those of both
//! off, and the backend refuses a GL state that has either on (`backend.rs`,
//! a `NOT_BUILT_*` line for each). It does not build the tiler OOM
//! context or the three incremental-rendering framebuffers Mesa adds to
//! every batch with a draw (pan_csf.c:1032-1051): the frame runs without an
//! OOM handler that can flush the tiler, so the heap must be large enough
//! for it (the design page, "Tiler OOM"). Nothing is preloaded, there is no
//! AFBC and no CRC.
//!
//! The layout is Mesa's: the same blocks, allocated in the same order with
//! the same alignment, so that a frame built at the trace's addresses lands
//! where Mesa's did. The blocks Mesa allocates between the draw's state and
//! the tiler context (the OOM context and the incremental-rendering
//! framebuffers) are not allocated, so the tiler context and the two
//! resource tables come earlier than in the trace; `frame_tests.rs` names
//! each pointer that moves with them.

use super::arena::{Arena, ArenaError, Block};
use super::cs::{CsError, Instr, Stream};
use super::genxml::pack::PackError;
use super::genxml::v10::*;
use core::fmt;

/// Why a frame, a program or a context descriptor was not built.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FrameError {
    /// An arena had no room.
    Arena(ArenaError),
    /// A descriptor field the generated packer refused.
    Pack(PackError),
    /// An instruction the encoder refused.
    Cs(CsError),
    /// A frame or draw the builder does not build yet: one of the
    /// `NOT_BUILT_*` lines.
    NotBuilt(&'static str),
    /// A value outside what the hardware or the descriptor can hold, or a
    /// caller's input that breaks the contract (an `INVALID_*` line): not a
    /// gap the builder owes, so the backend returns `-EINVAL` for it.
    Invalid(&'static str),
}

impl From<ArenaError> for FrameError {
    fn from(e: ArenaError) -> Self {
        FrameError::Arena(e)
    }
}
impl From<PackError> for FrameError {
    fn from(e: PackError) -> Self {
        FrameError::Pack(e)
    }
}
impl From<CsError> for FrameError {
    fn from(e: CsError) -> Self {
        FrameError::Cs(e)
    }
}

impl fmt::Display for FrameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FrameError::Arena(e) => write!(f, "{e}"),
            FrameError::Pack(e) => write!(f, "{e}"),
            FrameError::Cs(e) => write!(f, "{e}"),
            FrameError::NotBuilt(why) | FrameError::Invalid(why) => f.write_str(why),
        }
    }
}

/// The point size of a points draw whose vertex shader does not write
/// gl_PointSize (undefined in ES 2.0, 2.13.1): Mesa's rasterizer
/// `point_size`, GL's `ctx->Point.Size`, which ES 2.0 cannot change from
/// its initial 1.0 (the board's Mesa writes d60 0x3F800000,
/// 0dce654a8:tools/mali/board-ref/2026-10-19-points `nopsiz`).
pub const FIXED_POINT_SIZE: f32 = 1.0;
/// A no_psiz offset outside the position program's part of the binary, not
/// 128-byte aligned, or given for a fragment shader: a record defect.
pub const INVALID_NO_PSIZ: &str =
    "mali frame: a no_psiz program outside its binary, not 128-byte aligned in it, or given for a fragment shader";
pub const NOT_BUILT_NO_CLEAR: &str =
    "mali frame: a frame without a colour clear needs the preload of the old image, which is not built yet";
/// An index buffer whose address is not 2-byte aligned for its u16
/// indices: the GL core's contract (`GlesDraw::index_buf_gpu`).
pub const INVALID_INDEX_ALIGN: &str = "mali frame: a u16 index buffer at an odd address";
/// An indexed draw with a first vertex: Mesa's VERTEX_OFFSET of an indexed
/// draw is the index bias (pan_csf.c:1713-1714), which ES 2.0 does not have.
/// The indices' bytes (2 * count) do not fit INDEX_BUFFER_SIZE's 32 bits.
pub const INVALID_INDEX_BYTES: &str = "mali frame: an indexed draw of more than 2^31 indices";
pub const INVALID_INDEXED_FIRST_VERTEX: &str = "mali frame: an indexed draw with a first vertex (an index bias)";
/// A per-instance attribute whose divisor is not 1: Mesa's `1D POT Divisor`
/// with `Divisor R` above 0 or its `1D NPOT Divisor`, which no board dump
/// shows yet (the GL core hands the Mali divisor 1 only,
/// `YakoglBackend::max_instance_divisor` 0).
pub const NOT_BUILT_INSTANCE_DIVISOR: &str =
    "mali frame: a per-instance vertex attribute with a divisor other than 1 is not built yet";
pub const NOT_BUILT_ATTRIBUTES: &str =
    "mali frame: a draw with no vertex attribute, more than 16, or one of other than 1 to 4 32-bit floats is not built yet";
/// A stage with varyings and no IDVS varying program: the program record
/// broke its contract (a compiler defect), not a gap.
pub const INVALID_VARYINGS: &str =
    "mali frame: a stage reads or writes varyings but the vertex shader carries no varying program";
pub const NOT_BUILT_FS_EFFECTS: &str =
    "mali frame: a fragment shader that writes depth, stencil, coverage or memory is not built yet";
pub const NOT_BUILT_PRELOAD_STRIDE: &str =
    "mali frame: a preload of a target whose rows are not a multiple of 64 bytes is not built yet";
pub const INVALID_TEXTURE: &str =
    "mali frame: a shader stage that samples a texture and a draw without one for it, or the other way round";
pub const NOT_BUILT_TEXTURE_SIZE: &str =
    "mali frame: a texture larger than 65536 texels a side or with a row stride narrower than its rows is not built yet";
pub const INVALID_TEXTURE_ALIGN: &str = "mali frame: a texture's texels not on a 64-byte boundary";
/// A [`SampleStore`] for a target of other than 4 samples, off a 64-byte
/// boundary, or with planes closer than one plane of the target; or a
/// resolve that does not read the samples the store holds back.
pub const INVALID_SAMPLE_STORE: &str =
    "mali frame: a sample store for a target of other than 4 samples, off 64 bytes, or with overlapping planes";
pub const NOT_BUILT_FS_OUTPUTS: &str =
    "mali frame: a fragment shader that writes other than colour output 0 is not built yet";
pub const NOT_BUILT_STAGE: &str = "mali frame: a program given for the wrong stage";
pub const NOT_BUILT_PUSH: &str = "mali frame: a push word names a sysval the shader's sysval block does not hold";
pub const INVALID_SECONDARY: &str =
    "mali frame: a varying program outside its binary, not 128-byte aligned in it, or given for a fragment shader";
pub const NOT_BUILT_PUSH_UNIFORM: &str =
    "mali frame: a push word names a uniform dword the draw's uniform storage does not hold";
pub const NOT_BUILT_PUSH_COUNT: &str = "mali frame: more push words than the FAU's 4 pages of 32 slots hold";
/// A row of the uniform block a shader loads from memory
/// ([`ShaderMeta::ubo`]) names dwords past the draw's uniform storage, or
/// the rows are not one block from word 0 up.
pub const NOT_BUILT_UBO_BLOCK: &str =
    "mali frame: a shader's uniform block names uniform dwords the draw's uniform storage does not hold";
pub const NOT_BUILT_SYSVALS: &str = "mali frame: a shader with more sysval slots than the builder takes";
pub const INVALID_ATTRIBUTE_OFFSET: &str =
    "mali frame: a vertex attribute offset above i32::MAX does not fit the Attribute descriptor's signed offset";
pub const NOT_BUILT_TILER_LEVELS: &str =
    "mali frame: a tiler with other than 8 hierarchy levels (TILER_FEATURES max_levels) is not built yet";
pub const INVALID_FAU_ADDRESS: &str =
    "mali frame: a push block above 2^56 has no room for the FAU register's slot count in bits 56-63";
pub const INVALID_SRT_ALIGN: &str =
    "mali frame: a resource table not 64-byte aligned has no room for the SRT register's table count";
pub const INVALID_BLEND_ALIGN: &str =
    "mali frame: a blend descriptor not 64-byte aligned has no room for the BLEND_DESC register's count";
/// [`DrawCall::occlusion_query`] not on a qword: the hardware writes the
/// counter as 64-bit words.
pub const INVALID_OQ_ALIGN: &str = "mali frame: an occlusion counter not 8-byte aligned";
/// An equation Mesa runs in a blend shader (min/max, SRC_ALPHA_SATURATE,
/// factors that neither match up to an invert nor are ZERO or ONE,
/// `pan_blend_can_fixed_function`; or constants that differ between the
/// channels read, `pan_blend_is_homogenous_constant`, `panfrost_get_blend`)
/// drawn without one ([`DrawCall::blend_shader`]): the backend builds the
/// constant colour's blend shaders into a single-sampled RGBA8 target
/// written whole, case 211's 4x constant colour and case 190's GL_MIN and
/// ONE_MINUS_DST_ALPHA columns (src/mali/blend.rs `shader_equation`); no
/// other.
pub const NOT_BUILT_BLEND_SHADER: &str =
    "mali frame: a blend shader other than a constant colour's (min/max, SRC_ALPHA_SATURATE, unmatched factors, separate colour and alpha, a masked, 4x or non-RGBA8 target) is not built yet";
/// [`DrawCall::blend_shader`] not on 16 bytes, outside the fragment
/// program's 4 GiB window (the descriptor holds its low 32 bits, Mesa
/// 26.2.1 pan_cmdstream.c:365-377), or given for a draw whose blend the
/// fixed-function unit runs: the caller's defect.
pub const INVALID_BLEND_SHADER: &str =
    "mali frame: a blend shader the draw does not need, or not 16-byte aligned in the fragment program's 4 GiB window";
/// A target of other than 1 or 4 samples.
pub const NOT_BUILT_SAMPLES: &str = "mali frame: a target of other than 1 or 4 samples is not built yet";
/// A draw into an RGBA16F target that is not an opaque write of every
/// channel: a blend equation that reads the destination or a partial
/// colour mask. The board's Mesa 25.0.7 keeps such a target as RAW64 in
/// the tile buffer (tools/mali/board-dump/f16rt-2026-10-24.txt), a layout
/// the fixed-function unit does not blend; `panfrost_get_blend`
/// (pan_context.c:178-197) runs fixed function on a non-blendable format
/// only when the write is opaque and a blend shader otherwise, which
/// `blend::shader_equation` builds for RGBA8888 only. The only dump is the
/// opaque draw of case 54.
pub const NOT_BUILT_FLOAT_BLEND: &str =
    "mali frame: a blended or colour-masked draw into an RGBA16F target is not built yet";
pub const NOT_BUILT_EMPTY_TARGET: &str = "mali frame: a target with no pixels";
/// A draw tests or writes depth or stencil in a frame given no
/// depth/stencil attachment: the caller's contract (GL gives such a draw no
/// test, ES 2.0 4.1.5-4.1.6), not a gap.
pub const INVALID_NO_ZS: &str =
    "mali frame: a draw tests or writes depth or stencil but the frame has no depth/stencil attachment";
/// The draw's descriptors or its stream words do not fit the open frame's
/// room (the arenas, and the stream words the frame's end still needs).
pub const NOT_BUILT_FRAME_ROOM: &str = "mali frame: a draw that does not fit the open frame needs a second frame";
/// A frame's end that would not fit: the words [`end_frame`] writes (18,
/// and one more for each of its three 64-bit moves above 2^48).
const END_FRAME_WORDS: usize = 24;
/// A [`resolve_frame`] after a frame's end: [`begin_frame`]'s two words and
/// an end's.
const RESOLVE_WORDS: usize = 2 + END_FRAME_WORDS;
/// An [`OpenFrame`] not made by [`begin_frame`]: a caller defect.
pub const INVALID_OPEN_FRAME: &str = "mali frame: an open frame without its framebuffer descriptor or local storage";

/// `TILER_FEATURES` max_levels of the one board measured (0x809).
const MEASURED_TILER_LEVELS: u32 = 8;

/// The most vertex attributes a draw takes (the GL core binds 16,
/// `GLES_MAX_ATTRIBS`).
const MAX_ATTRIBUTES: usize = 16;

/// The most sysval slots a shader may name. Mesa's sysvals are a closed list
/// of `PAN_SYSVAL_*` (a few vec4 slots a shader); the bound keeps the sysval
/// block's size in the UBO Buffer descriptor's 32 bits and a push word's
/// slot index small.
const MAX_SYSVALS: usize = 1024;

/// Mali-G610's tile buffer, halved as Mesa's `panfrost_query_optimal_tib_size`
/// does: `MODEL(0xa867, ..., 32768, ...)` and `/ 2` (Mesa 25.0.7,
/// src/panfrost/lib/pan_props.c:71, 199-208).
const TIB_BUDGET: u32 = 32768 / 2;
/// `panfrost_max_effective_tile_size` for arch 10 (pan_props.h:132-138).
const MAX_EFFECTIVE_TILE: u32 = 32 * 32;
/// `PAN_NUM_RESOURCE_TABLES` and the table order (pan_context.h:321-331).
const RESOURCE_TABLES: usize = 7;
const TABLE_UBO: usize = 0;
const TABLE_ATTRIBUTE: usize = 1;
const TABLE_ATTRIBUTE_BUFFER: usize = 2;
const TABLE_SAMPLER: usize = 3;
const TABLE_TEXTURE: usize = 4;
/// Bytes a resource-table entry counts per descriptor: Mesa sizes every
/// table as `count * pan_size(BUFFER)` (pan_encoder.h:232-243).
const RESOURCE_DESCRIPTOR_BYTES: u64 = 32;
/// The scoreboard slots: the render endpoint and the load/store slot
/// (`cs_set_scoreboard_entry(b, 2, 0)`, pan_csf.c:267; `cs_wait_slot(b, 2)`
/// after the tiler and fragment work, `cs_wait_slot(b, 0)` after a load,
/// pan_csf.c:807, 832, 844).
const SB_RENDER: u32 = 2;
const SB_LS: u32 = 0;
/// The registers the CS uses (pan_csf.c:798-847, 1053-1270, 1289-1316,
/// 328-361; the IDVS map is v10.xml:890-928, the fragment map :930-936).
mod reg {
    pub const SRT_VERTEX: u32 = 0;
    pub const SRT_FRAGMENT: u32 = 4;
    pub const FAU_VERTEX: u32 = 8;
    pub const FAU_FRAGMENT: u32 = 12;
    pub const SPD_VERTEX: u32 = 16;
    pub const SPD_FRAGMENT: u32 = 20;
    pub const TSD_0: u32 = 24;
    pub const TSD_3: u32 = 30;
    pub const GLOBAL_ATTRIBUTE_OFFSET: u32 = 32;
    pub const INDEX_COUNT: u32 = 33;
    pub const INSTANCE_COUNT: u32 = 34;
    pub const INDEX_OFFSET: u32 = 35;
    pub const VERTEX_OFFSET: u32 = 36;
    pub const INSTANCE_OFFSET: u32 = 37;
    pub const DCD2: u32 = 38;
    pub const INDEX_BUFFER_SIZE: u32 = 39;
    pub const TILER_CTX: u32 = 40;
    pub const SCISSOR_BOX: u32 = 42;
    pub const LOW_DEPTH_CLAMP: u32 = 44;
    pub const HIGH_DEPTH_CLAMP: u32 = 45;
    /// `MALI_IDVS_SR_OQ`, the occlusion counter's address (v10.xml:919).
    pub const OQ: u32 = 46;
    pub const VARY_SIZE: u32 = 48;
    pub const BLEND_DESC: u32 = 50;
    pub const ZSD: u32 = 52;
    pub const TILER_FLAGS: u32 = 56;
    pub const DCD0: u32 = 57;
    pub const DCD1: u32 = 58;
    pub const PRIMITIVE_SIZE: u32 = 60;
    /// `MALI_IDVS_SR_INDEX_BUFFER` (v10.xml:923).
    pub const INDEX_BUFFER: u32 = 54;
    /// `MALI_IDVS_SR_VERTEX_VARY_SPD`, the varying program's SPD
    /// (`MALI_IDVS_SR_SPD_1`, Mesa 26.2.1 cs_builder.h:32, v10.xml:901).
    pub const VARY_SPD: u32 = 18;
    pub const FBD_POINTER: u32 = 40;
    pub const BBOX_MIN: u32 = 42;
    pub const BBOX_MAX: u32 = 43;
    pub const TEM_POINTER: u32 = 44;
    pub const TEM_ROW_STRIDE: u32 = 46;
    /// The flush ID register of the batch end (pan_csf.c:349).
    pub const FLUSH_ID: u32 = 74;
    /// The tiler context's address and the four words loaded from it
    /// before FINISH_FRAGMENT (pan_csf.c:841-846).
    pub const TILER_DESC: u32 = 90;
    pub const HEAP_CHUNKS: u32 = 86;
    /// The tiler heap context of the queue-init stream (Mesa 25.0.7
    /// pan_csf.c:1471, `cs_reg64(&b, 72)`).
    pub const HEAP_CTX: u32 = 72;
}

/// The per-context objects a frame points at (built once, by
/// [`tiler_heap_descriptor`] and [`sample_positions`], and by the OSAL).
#[derive(Clone, Copy, Debug)]
pub struct Context {
    /// The tiler heap descriptor.
    pub heap: u64,
    /// The tiler's temporary geometry buffer and its size (Mesa: a 64 KiB
    /// BO, `POSITION_FIFO_SIZE`, pan_csf.c:1362, 1444-1445).
    pub geometry_buffer: u64,
    pub geometry_buffer_size: u32,
    /// The sample-position table.
    pub sample_positions: u64,
    /// `TILER_FEATURES` bits 11:8 (`max_levels`, pan_props.c:116); the
    /// board reads Tiler:0x809, so 8.
    pub tiler_max_levels: u32,
}

/// The tiler heap descriptor, as Mesa writes it once per context
/// (pan_csf.c:1437-1442): the chunk size, the first chunk the kernel gave
/// (`first_heap_chunk_gpu_va` of DRM_PANTHOR_TILER_HEAP_CREATE), the bottom
/// past the chunk's 64-byte header and the top at the chunk's end. The GPU
/// moves bottom and top as it allocates.
pub fn tiler_heap_descriptor(arena: &mut Arena, first_chunk: u64, chunk_size: u32) -> Result<Block, FrameError> {
    let b = arena.alloc(TilerHeap::LENGTH, 32)?;
    let heap = TilerHeap {
        size: chunk_size,
        base: first_chunk,
        bottom: first_chunk + 64,
        top: first_chunk + u64::from(chunk_size),
        ..Default::default()
    };
    arena.write(b, 0, &heap.pack()?)?;
    Ok(b)
}

/// The queue-init stream, run once per queue before its first frame: the
/// tiler heap context the kernel made (`tiler_heap_ctx_gpu_va` of
/// DRM_PANTHOR_TILER_HEAP_CREATE) into d72 and `HEAP_SET` from it (Mesa
/// 25.0.7 pan_csf.c:1469-1473; the trace's first two words).
///
/// Mesa's stream goes on to install the TILER_OOM exception handler
/// (pan_csf.c:1475-1480). That handler is not built ([`build_frame`] builds
/// no OOM context), so this stream ends after `HEAP_SET`: a TILER_OOM is
/// then handled by the kernel or faults the queue, which the caller reports;
/// it is never silent.
pub fn queue_init(cs: &mut Stream, heap_ctx: u64) -> Result<(), FrameError> {
    cs.move64(reg::HEAP_CTX, heap_ctx)?;
    cs.push(Instr::HeapSet(CsHeapSet { address: reg::HEAP_CTX, ..Default::default() }))?;
    Ok(())
}

/// Mesa's sample-position table (`sample_position_lut`, Mesa 26.2.1
/// pan_samples.c), one 256-byte block a pattern in `MALI_SAMPLE_PATTERN_*`
/// order (`struct mali_sample_positions`: 32 positions, the origin, padding,
/// each two u16), up to the rotated 4x grid the 4x target uses: the single
/// pattern (position 0 and the origin at the pixel centre, `SAMPLE4(0, 0)`
/// = (128, 128)), the ordered 4x grid and the rotated 4x grid
/// (`SAMPLE8(-1, -3)`, `(3, -1)`, `(-3, 1)`, `(1, 3)`). The unused positions
/// are 0. The FBD names a pattern at `pan_sample_positions_offset`, its
/// index times 256 ([`sample_positions_of`]).
pub fn sample_positions(arena: &mut Arena) -> Result<Block, FrameError> {
    /* SAMPLE16(x, y) = ((x + 8) * 16, (y + 8) * 16); SAMPLE4 x4, SAMPLE8 x2 */
    let s16 = |x: i32, y: i32| [((x + 8) * 16) as u16, ((y + 8) * 16) as u16];
    let s4 = |x, y| s16(x * 4, y * 4);
    let s8 = |x, y| s16(x * 2, y * 2);
    let patterns: [&[[u16; 2]]; 3] =
        [&[s4(0, 0)], &[s4(-1, -1), s4(1, -1), s4(-1, 1), s4(1, 1)], &[s8(-1, -3), s8(3, -1), s8(-3, 1), s8(1, 3)]];
    let xy = |p: [u16; 2]| {
        let (x, y) = (p[0].to_le_bytes(), p[1].to_le_bytes());
        [x[0], x[1], y[0], y[1]]
    };
    let b = arena.alloc(patterns.len() * SAMPLE_PATTERN_BYTES, 64)?;
    for (i, positions) in patterns.iter().enumerate() {
        let at = i * SAMPLE_PATTERN_BYTES;
        for (k, p) in positions.iter().enumerate() {
            arena.write(b, at + 4 * k, &xy(*p))?;
        }
        /* the origin, after the 32 positions; every pattern's is the centre */
        arena.write(b, at + 32 * 4, &xy(s4(0, 0)))?;
    }
    Ok(b)
}

/// One pattern's block in [`sample_positions`].
const SAMPLE_PATTERN_BYTES: usize = 64 * 4;

/// The FBD's sample locations: the pattern's block of the context's table
/// (`pan_sample_positions_offset`, pan_samples.c).
fn sample_positions_of(ctx: &Context, pattern: SamplePattern) -> u64 {
    ctx.sample_positions + u64::from(pattern.0) * SAMPLE_PATTERN_BYTES as u64
}

/// Mesa's tiler geometry buffer (`POSITION_FIFO_SIZE`, Mesa 26.2.1
/// pan_csf.c:1787; 25.0.7 pan_csf.c:1444-1445).
pub const GEOMETRY_BUFFER_BYTES: u32 = 64 * 1024;

/// The per-context objects: the tiler heap descriptor and the sample
/// positions, built once into `ctx` from the heap the OSAL made (the GPU
/// moves the heap descriptor's Bottom and Top, so it is never rebuilt while
/// the context lives; the design page, "What L1 must provide").
pub fn build_context(
    ctx: &mut Arena,
    heap_first_chunk: u64,
    heap_chunk_size: u32,
    geometry_buffer: u64,
    tiler_max_levels: u32,
) -> Result<Context, FrameError> {
    let heap = tiler_heap_descriptor(ctx, heap_first_chunk, heap_chunk_size)?;
    let positions = sample_positions(ctx)?;
    Ok(Context {
        heap: heap.gpu,
        geometry_buffer,
        geometry_buffer_size: GEOMETRY_BUFFER_BYTES,
        sample_positions: positions.gpu,
        tiler_max_levels,
    })
}

/// The registers a stream may name: the queue's `cs_reg_count` less the
/// kernel's `max(unpreserved_cs_reg_count, 4)` (Mesa 26.2.1
/// pan_csf.c:475-476; Mesa 25.0.7 fixes 96 and 4, pan_csf.c:250-252).
/// `None` when the query leaves none.
pub fn user_regs(cs_reg_count: u32, unpreserved: u32) -> Option<u32> {
    cs_reg_count.checked_sub(unpreserved.max(4)).filter(|&n| n > 0)
}

/// `TILER_FEATURES` bits 11:8, `max_levels` (Mesa pan_props.c:116).
pub fn tiler_max_levels(tiler_features: u32) -> u32 {
    (tiler_features >> 8) & 0xf
}

/// A system value a shader reads from its sysval block (Mesa's
/// `PAN_SYSVAL_*`, uploaded by `panfrost_upload_sysvals`,
/// pan_cmdstream.c:1216-1240), one vec4 slot each.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sysval {
    /// The viewport scale, x y z (pan_cmdstream.c:989-998).
    ViewportScale,
    /// The viewport offset, x y z (pan_cmdstream.c:1001-1010).
    ViewportOffset,
    /// The GL blend colour, r g b a (`panfrost_upload_blend_constants_sysval`,
    /// Mesa 26.2.1 pan_cmdstream.c:1281-1287): a fragment shader's first
    /// sysval slot whether it blends or not (pan_nir_lower_sysvals.c:258-261),
    /// pushed as its first four FAU words (`bi_pick_blend_constants`,
    /// bi_opt_push_ubo.c:91-114).
    BlendConstants,
}

/// One 32-bit word of a shader's push constants (FAU), in FAU order: a
/// component of one slot of its sysval block (a relocation of the sysval
/// UBO), a word of the stage's uniform block (a relocation of UBO 0), or an
/// immediate the compiler promoted. Mesa copies the relocations from the
/// blocks it uploaded and the immediate as it is (`panfrost_emit_const_buf`,
/// Mesa 26.2.1 pan_cmdstream.c:1590-1651: `pan_fau_foreach_reloc`,
/// `pan_fau_foreach_imm`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PushWord {
    Sysval {
        sysval: usize,
        component: usize,
    },
    Constant(u32),
    /// Dword `dword` of the stage's GL uniform storage (the draw's
    /// `DrawCall::vs_uniforms` / `fs_uniforms`), as it is.
    Uniform {
        dword: usize,
    },
    /// Two dwords of the GL uniform storage as fp16, the first in the low
    /// half (Mesa's fp16 uniform block, `LowerPrecisionFloat16Uniforms`;
    /// st/mesa converts at glUniform time with `_mesa_float_to_half`,
    /// round to nearest even); `None` is a zero half.
    UniformHalves {
        lo: usize,
        hi: Option<usize>,
    },
    /// Component `component` of gl_FragCoord's y transform, the state
    /// uniform `gl_FbWposYTransform` (Mesa 26.2.1 prog_statevars.c:736-751,
    /// `STATE_FB_WPOS_Y_TRANSFORM`): `(1, 0, -1, height)` for a target stored
    /// bottom-up, `(-1, height, 1, 0)` for one stored top-down
    /// ([`DrawCall::flip_y`], Mesa's `FlipY`); [`fb_wpos_y_transform`].
    FbWposYTransform {
        component: usize,
    },
}

/// One uniform, or one element of a uniform array, of the block a shader
/// loads from memory with `LD_PKA` (a uniform array element read at a
/// run-time index, src/shaderc/online/mali/nir_fs.rs; Mesa's UBO 0, which
/// `ubo_mask` bit 0 makes `panfrost_emit_const_buf` upload, Mesa 26.2.1
/// pan_cmdstream.c:1557-1577): `comps` dwords of the stage's GL uniform
/// storage from `dword`, at Mesa's 32-bit `word` of the block, as fp16
/// `halves` (two a word, the low component first, `f32_to_f16`) or whole
/// words. The rows fill the block from word 0 up, in order; the block's
/// bytes are the last row's end ([`ubo_block_bytes`]: the exact `Size` of
/// the Buffer descriptor, 36 for case 122's `vec4 fcols[4]` and `int fidx`,
/// 0dce654a8:tools/mali/board-ref/2026-10-20-mesa-refs/mesa/mesa-uniidxi.pandecode:261).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UboRow {
    pub dword: usize,
    pub word: usize,
    pub comps: usize,
    pub halves: bool,
}

impl UboRow {
    /// The 32-bit words the row takes.
    pub const fn words(&self) -> usize {
        if self.halves {
            self.comps.div_ceil(2)
        } else {
            self.comps
        }
    }
}

/// The bytes of a uniform block ([`UboRow`]): through its last row.
pub fn ubo_block_bytes(rows: &[UboRow]) -> usize {
    rows.iter().map(|r| 4 * (r.word + r.words())).max().unwrap_or(0)
}

/// Component `c` of `gl_FbWposYTransform` for a draw into a target of
/// `height` rows stored top-down (`flip_y`) or bottom-up
/// ([`PushWord::FbWposYTransform`]).
pub fn fb_wpos_y_transform(flip_y: bool, height: u32, c: usize) -> f32 {
    let h = height as f32;
    let t = if flip_y { [-1.0, h, 1.0, 0.0] } else { [1.0, 0.0, -1.0, h] };
    t[c & 3]
}

/// `_mesa_float_to_half` (Mesa 26.2.1 src/util/half_float.c, the
/// round-to-nearest-even conversion) of an fp32 value's bits.
pub fn f32_to_f16(bits: u32) -> u16 {
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exp = ((bits >> 23) & 0xff) as i32;
    let man = bits & 0x7f_ffff;
    if exp == 0xff {
        return sign | if man != 0 { 0x7e00 } else { 0x7c00 };
    }
    /* the fp16 biased exponent */
    let e = exp - 127 + 15;
    if e >= 31 {
        return sign | 0x7c00;
    }
    let round = |q: u32, r: u32, half: u32| q + (r > half || (r == half && q & 1 == 1)) as u32;
    if e <= 0 {
        /* an fp16 subnormal; below half the smallest one it is zero */
        if e < -10 || exp == 0 {
            return sign;
        }
        let m = man | 0x80_0000;
        let shift = (14 - e) as u32;
        /* a carry to 0x400 is the smallest normal, encoded as it is */
        return sign | round(m >> shift, m & ((1 << shift) - 1), 1 << (shift - 1)) as u16;
    }
    /* a carry out of the mantissa bumps the exponent, up to inf */
    sign | round((e as u32) << 10 | man >> 13, man & 0x1fff, 0x1000) as u16
}

/// A vertex shader's second program, the IDVS varying shader (Mesa's
/// `info.vs.secondary_*`): where it starts in the binary and its own
/// preload and register allocation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SecondaryProgram {
    /// Byte offset from the binary's start (`secondary_offset`).
    pub offset: u32,
    pub preload: u32,
    pub register_allocation: ShaderRegisterAllocation,
}

/// What a shader binary needs from the descriptors and the frame: the
/// compiler's output beside the code (Mesa's `pan_shader_info`).
#[derive(Clone, Copy, Debug)]
pub struct ShaderMeta<'a> {
    pub stage: ShaderStage,
    /// From the work-register count (`pan_register_allocation`,
    /// pan_shader.h:91-98).
    pub register_allocation: ShaderRegisterAllocation,
    /// Registers r48-r63 the hardware preloads (`info.preload >> 48`).
    pub preload: u32,
    /// The sysval block the shader's resource table names as its last UBO
    /// (UBO 0 without a [`ShaderMeta::ubo`] block, else UBO 1).
    pub sysvals: &'a [Sysval],
    /// The push constants, in FAU order.
    pub push: &'a [PushWord],
    /// The uniform block the shader loads from memory (UBO 0 of its
    /// resource table), in Mesa's packed layout; empty when every uniform
    /// read is pushed (Mesa's `ubo_mask` bit 0 clear: no block uploaded).
    pub ubo: &'a [UboRow],
    /// Vertex: the varyings written; fragment: read.
    pub varyings: u32,
    /// Vertex: the bytes of one vertex's generic varyings
    /// (`varyings.formats.generic_size_B`, the IDVS VARY_SIZE register;
    /// pan_nir_collect_varyings.c:415-444).
    pub varying_bytes: u32,
    /// Vertex: the varying program, when the shader has one.
    pub secondary: Option<SecondaryProgram>,
    /// Vertex, when the shader writes gl_PointSize (Mesa's
    /// `vs.writes_point_size`): the byte offset of the position program
    /// without the point size store (`vs.no_psiz_offset`), which a draw
    /// that is not of points runs. The program at offset 0 stores the
    /// point size.
    pub no_psiz_offset: Option<u32>,
    /// Fragment: the colour outputs written, bit n for target n.
    pub colour_outputs: u32,
    /// The shader samples: a fragment shader one texture or more, texture i
    /// at index i of its stage's texture and sampler tables
    /// ([`DrawCall::textures`]), a vertex shader one, at index 0
    /// ([`DrawCall::vs_texture`]).
    pub samples_texture: bool,
    /// Mesa's `contains_barrier`: a barrier, or a fragment shader whose quads
    /// need their helper invocations (NIR's
    /// `needs_coarse_quad_helper_invocations`, an implicit-LOD look-up;
    /// `pan_shader_update_info`, Mesa 26.2.1 pan_compiler.c:217-218, 276):
    /// the SPD's "Requires helper threads" (pan_cmdstream.c:4518-4519).
    pub requires_helper_threads: bool,
    /// Fragment: anything that changes the depth/stencil pass or has a side
    /// effect. Only none is built.
    pub can_discard: bool,
    pub writes_depth: bool,
    pub writes_stencil: bool,
    pub writes_coverage: bool,
    pub writes_global: bool,
}

/// A shader binary in GPU memory, its descriptor and its meta.
#[derive(Clone, Copy, Debug)]
pub struct Program<'a> {
    /// The Shader Program descriptor a draw that is not of points runs
    /// (a vertex shader's IDVS triangle variant, `programs[1]`).
    pub spd: u64,
    /// A vertex shader's descriptor for GL_POINTS (`programs[0]`, the
    /// program at offset 0, which stores the point size when the shader
    /// writes it); [`Program::spd`] otherwise.
    pub points_spd: u64,
    /// The varying program's Shader Program descriptor (a vertex shader with
    /// a [`SecondaryProgram`]).
    pub varying_spd: Option<u64>,
    pub binary: u64,
    pub meta: ShaderMeta<'a>,
}

/// Copy a binary into `code` and write its Shader Program descriptor into
/// `spds` (Mesa's `prepare_shader`, pan_cmdstream.c:3963-4040).
///
/// `binary` is the program's instructions. It starts on a 128-byte boundary
/// and is followed by zeros as Mesa's compiler pads it: at least 16 bytes,
/// up to a multiple of 128 (`ALIGN_POT(size + 16, 128)`, Mesa 26.2.1
/// src/panfrost/compiler/bifrost/valhall/va_pack.c:1225-1239; the design
/// page's "zero padding to 128 bytes"). A binary that already ends so (our
/// Valhall back end pads each program, src/shaderc/online/mali/pack.rs) is
/// taken at its length; any other is padded here. A vertex shader gets the
/// IDVS triangle variant at `no_psiz_offset` and its varying program (Mesa
/// 26.2.1 pan_cmdstream.c:4520-4553: `programs[1]` and `[2]`), and, when it
/// writes gl_PointSize, the points variant at offset 0 (`programs[0]`,
/// :4498-4513), which a GL_POINTS draw runs (`panfrost_get_position_shader`,
/// pan_cmdstream.h:239-250). Without gl_PointSize the two variants are the
/// same program and one descriptor serves both.
pub fn upload_program<'a>(
    code: &mut Arena,
    spds: &mut Arena,
    binary: &[u8],
    meta: ShaderMeta<'a>,
) -> Result<Program<'a>, FrameError> {
    let len = padded_len(binary, &meta)?;
    let bin = code.alloc(len, 128)?;
    code.write(bin, 0, binary)?;
    let d = program_descriptors(&meta, bin.gpu)?;
    let points_spd = spds.upload(&d.points, 32)?;
    let spd = match d.triangles {
        None => points_spd,
        Some(t) => spds.upload(&t, 32)?,
    };
    let varying_spd = match d.varying {
        None => None,
        Some(v) => Some(spds.upload(&v, 32)?.gpu),
    };
    Ok(Program { spd: spd.gpu, points_spd: points_spd.gpu, varying_spd, binary: bin.gpu, meta })
}

/// The bytes a binary takes in the code arena (see [`upload_program`]),
/// after checking its secondary program's and its no_psiz copy's offsets.
fn padded_len(binary: &[u8], meta: &ShaderMeta) -> Result<usize, FrameError> {
    if let Some(s) = meta.secondary {
        let at = s.offset as usize;
        if meta.stage != ShaderStage::VERTEX || at == 0 || at >= binary.len() || !at.is_multiple_of(128) {
            return Err(FrameError::Invalid(INVALID_SECONDARY));
        }
    }
    if let Some(o) = meta.no_psiz_offset {
        let at = o as usize;
        let end = meta.secondary.map_or(binary.len(), |s| s.offset as usize);
        if meta.stage != ShaderStage::VERTEX || at == 0 || at >= end || !at.is_multiple_of(128) {
            return Err(FrameError::Invalid(INVALID_NO_PSIZ));
        }
    }
    let padded =
        binary.len().is_multiple_of(128) && binary.len() >= 128 && binary[binary.len() - 16..].iter().all(|&b| b == 0);
    Ok(if padded { binary.len() } else { (binary.len() + 16).div_ceil(128) * 128 })
}

/// The Shader Program descriptors of a binary ([`program_descriptors`]).
struct Descriptors {
    /// The program at offset 0 (a vertex shader's points variant).
    points: [u8; ShaderProgram::LENGTH],
    /// The triangle variant at the no_psiz copy, when there is one.
    triangles: Option<[u8; ShaderProgram::LENGTH]>,
    /// The varying program's.
    varying: Option<[u8; ShaderProgram::LENGTH]>,
}

/// The Shader Program descriptors of a binary at `bin`: its own (and its
/// triangle variant's at the no_psiz copy), and its varying program's when
/// it has one.
fn program_descriptors(meta: &ShaderMeta, bin: u64) -> Result<Descriptors, FrameError> {
    let mut desc = ShaderProgram {
        stage: meta.stage,
        register_allocation: meta.register_allocation,
        preload: Preload { r48_r63: meta.preload, ..Default::default() },
        binary: bin,
        /* panfrost_ftz_mode: fp32 code keeps subnormals */
        flush_to_zero_mode: FlushToZeroMode::PRESERVE_SUBNORMALS,
        ..Default::default()
    };
    /* the two meanings of bit 8: the vertex warp limit, the fragment
     * coverage bitmask type (pan_cmdstream.c:4007-4010, 4028) */
    if meta.stage == ShaderStage::FRAGMENT {
        desc.fragment_coverage_bitmask_type = CoverageBitmaskType::GL;
        desc.requires_helper_threads = meta.requires_helper_threads;
    } else {
        desc.vertex_warp_limit = WarpLimit::HALF;
    }
    let points = desc.pack()?;
    /* the triangle variant: the same descriptor at the no_psiz copy */
    let triangles = match meta.no_psiz_offset {
        None => None,
        Some(o) => Some(ShaderProgram { binary: bin + u64::from(o), ..desc }.pack()?),
    };
    let varying = match meta.secondary {
        None => None,
        /* `programs[2]`: the full warp limit, its own registers and
         * preload, the binary at the secondary offset */
        Some(s) => Some(
            ShaderProgram {
                stage: meta.stage,
                vertex_warp_limit: WarpLimit::FULL,
                register_allocation: s.register_allocation,
                preload: Preload { r48_r63: s.preload, ..Default::default() },
                binary: bin + u64::from(s.offset),
                flush_to_zero_mode: FlushToZeroMode::PRESERVE_SUBNORMALS,
                ..Default::default()
            }
            .pack()?,
        ),
    };
    Ok(Descriptors { points, triangles, varying })
}

/// How many programs a frame keeps for reuse ([`ProgramCache`]).
pub const PROGRAM_CACHE_ENTRIES: usize = 16;

/// One program uploaded into the open frame: its binary and descriptors.
#[derive(Clone, Copy, Debug)]
struct CachedProgram {
    bin: Block,
    spd: Block,
    points_spd: Block,
    varying_spd: Option<Block>,
}

/// The programs the open frame has uploaded. A draw whose program is
/// already in the frame (the same binary bytes and the same descriptors)
/// points at that copy instead of uploading it again: Mesa uploads a
/// program once, when it is compiled (`panfrost_shader_get`), so a frame of
/// many draws of one program costs its code once, not once a draw (gl_test
/// case 11 draws 113 and 300 in one frame). The table keeps the last
/// [`PROGRAM_CACHE_ENTRIES`] programs; a program it no longer holds is
/// uploaded again, which costs room and is never wrong.
#[derive(Clone, Copy, Debug, Default)]
pub struct ProgramCache {
    entries: [Option<CachedProgram>; PROGRAM_CACHE_ENTRIES],
    next: usize,
    /// Uploads a lookup saved.
    pub hits: u32,
}

/// [`upload_program`] through the open frame's [`ProgramCache`]: an
/// entry is taken only when the code arena holds exactly this binary,
/// padded, and the descriptor arena exactly the descriptors it would write.
pub fn upload_program_cached<'a>(
    cache: &mut ProgramCache,
    code: &mut Arena,
    spds: &mut Arena,
    binary: &[u8],
    meta: ShaderMeta<'a>,
) -> Result<Program<'a>, FrameError> {
    let len = padded_len(binary, &meta)?;
    for e in cache.entries.iter().flatten() {
        if e.bin.len != len || code.bytes(e.bin)?[..binary.len()] != *binary {
            continue;
        }
        let d = program_descriptors(&meta, e.bin.gpu)?;
        if spds.bytes(e.points_spd)? != d.points {
            continue;
        }
        let same_triangles = match d.triangles {
            None => e.spd == e.points_spd,
            Some(t) => e.spd != e.points_spd && spds.bytes(e.spd)? == t,
        };
        let same_varying = match (d.varying, e.varying_spd) {
            (None, None) => true,
            (Some(v), Some(b)) => spds.bytes(b)? == v,
            _ => false,
        };
        if same_triangles && same_varying {
            cache.hits += 1;
            return Ok(Program {
                spd: e.spd.gpu,
                points_spd: e.points_spd.gpu,
                varying_spd: e.varying_spd.map(|b| b.gpu),
                binary: e.bin.gpu,
                meta,
            });
        }
    }
    let bin = code.alloc(len, 128)?;
    code.write(bin, 0, binary)?;
    let d = program_descriptors(&meta, bin.gpu)?;
    let points_spd = spds.upload(&d.points, 32)?;
    let spd = match d.triangles {
        None => points_spd,
        Some(t) => spds.upload(&t, 32)?,
    };
    let varying_spd = match d.varying {
        None => None,
        Some(v) => Some(spds.upload(&v, 32)?),
    };
    cache.entries[cache.next] = Some(CachedProgram { bin, spd, points_spd, varying_spd });
    cache.next = (cache.next + 1) % PROGRAM_CACHE_ENTRIES;
    Ok(Program {
        spd: spd.gpu,
        points_spd: points_spd.gpu,
        varying_spd: varying_spd.map(|b| b.gpu),
        binary: bin.gpu,
        meta,
    })
}

/// The colour target: linear, one sample.
#[derive(Clone, Copy, Debug)]
pub struct Target {
    pub gpu: u64,
    pub width: u32,
    pub height: u32,
    /// Bytes from one row to the next.
    pub row_stride: u32,
    pub format: TargetFormat,
    /// Samples a pixel in the tile buffer, 1 or 4 (any other is refused,
    /// [`NOT_BUILT_SAMPLES`]): a 4x target is rasterised with 4 samples on
    /// chip and averaged into its single-sampled memory on writeback
    /// (src/backend.h `samples`; Mesa's EXT_multisampled_render_to_texture,
    /// `writeback_msaa` AVERAGE in pan_prepare_rt).
    pub samples: u32,
}

impl Target {
    /// The tile buffer's sample pattern: the rotated 4x grid Mesa takes
    /// for 4 samples (`pan_sample_pattern`, pan_desc.h).
    fn sample_pattern(&self) -> SamplePattern {
        if self.samples == 4 {
            SamplePattern::ROTATED_4X_GRID
        } else {
            SamplePattern::SINGLE_SAMPLED
        }
    }
}

/// A colour target's memory layout (the GL core's `GLES_SURF_*`), each as
/// Mesa draws the pipe format of the same layout (`pan_blendable_formats`,
/// Mesa 26.2.1 pan_format.c:85-101; the board's dumps,
/// 0dce654a8:tools/mali/board-ref/2026-10-19-formats/mesa).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TargetFormat {
    /// R, G, B, A bytes (`PIPE_FORMAT_R8G8B8A8_UNORM`).
    Rgba8888,
    /// B, G, R, X bytes, DRM XRGB8888 (`PIPE_FORMAT_B8G8R8X8_UNORM`).
    Xrgb8888,
    /// 16 bits, red in the high 5, DRM RGB565 (`PIPE_FORMAT_B5G6R5_UNORM`).
    Rgb565,
    /// R, G, B, A halves (`PIPE_FORMAT_R16G16B16A16_FLOAT`), as the board's
    /// Mesa 25.0.7 draws a `GL_RGBA16F` face (tools/mali/board-dump/
    /// f16rt-2026-10-24.txt; docs/notes/mali-frame-formats.md sections 4, 6):
    /// RAW64 in the tile buffer, so only opaque writes of every channel
    /// ([`NOT_BUILT_FLOAT_BLEND`]).
    Rgba16f,
}

impl TargetFormat {
    /// Bytes a pixel.
    pub fn bytes(self) -> u32 {
        match self {
            TargetFormat::Rgba8888 | TargetFormat::Xrgb8888 => 4,
            TargetFormat::Rgb565 => 2,
            TargetFormat::Rgba16f => 8,
        }
    }

    /// Bytes a sample in the tile buffer (`pan_format_tib_size`,
    /// pan_format.h:127-139): 4 for the 8-bit and 565 layouts, the raw 8
    /// bytes of four halves (the board's 16F frames: `Color Buffer
    /// Allocation` 8192 where RGBA8 has 4096).
    fn tib_bytes(self) -> u32 {
        match self {
            TargetFormat::Rgba8888 | TargetFormat::Xrgb8888 | TargetFormat::Rgb565 => 4,
            TargetFormat::Rgba16f => 8,
        }
    }

    /// The tile buffer's format and the writeback format (BFMT_SRGB and
    /// BFMT2 rows of pan_format.c:85-101).
    fn formats(self) -> (ColorBufferInternalFormat, ColorFormat) {
        match self {
            TargetFormat::Rgba8888 | TargetFormat::Xrgb8888 => {
                (ColorBufferInternalFormat::R8G8B8A8, ColorFormat::R8G8B8A8)
            }
            TargetFormat::Rgb565 => (ColorBufferInternalFormat::R5G6B5A0, ColorFormat::R5G6B5),
            /* the board's dump: `Internal Format` RAW64 (0x23) and
             * `Writeback Format` RAW64 (5), not the RGBA16F pair 26.2.1's
             * BFMT_FLOAT names (mali-frame-formats.md section 6) */
            TargetFormat::Rgba16f => (ColorBufferInternalFormat::RAW64, ColorFormat::RAW64),
        }
    }

    /// The writeback swizzle, X Y Z W as Mali channels at 3 bits each
    /// (`panfrost_translate_swizzle_4`; 4 is the constant 0): the identity
    /// for R, G, B, A bytes, and B, G, R, 0 for the BGR layouts, as the
    /// board's Mesa writes them ("Swizzle: 2058 (BGR0)").
    fn swizzle(self) -> u32 {
        let s = |x: u32, y: u32, z: u32, w: u32| x | y << 3 | z << 6 | w << 9;
        match self {
            TargetFormat::Rgba8888 | TargetFormat::Rgba16f => s(0, 1, 2, 3),
            TargetFormat::Xrgb8888 | TargetFormat::Rgb565 => s(2, 1, 0, 4),
        }
    }

    /// The blend descriptor's conversion: the tile buffer's pixel format
    /// with dithering on (`pan_dithered_format_from_pipe_format`,
    /// pan_format.h:355-363; MALI_BLEND_AU_R8G8B8A8 and
    /// MALI_BLEND_AU_R5G6B5A0, pan_format.c:22-31, the component order of
    /// v10: pan_format.c:18-19).
    fn blend_memory_format(self) -> u32 {
        match self {
            TargetFormat::Rgba8888 | TargetFormat::Xrgb8888 => Format::RGBA8_TB.0 << 12 | RgbComponentOrder::RGBA.0,
            TargetFormat::Rgb565 => Format::R5G6B5_AU.0 << 12 | RgbComponentOrder::RGB1.0,
            /* MALI_BLEND_RGBA16F (BFMT_FLOAT, pan_format.c:34-37, 117-121);
             * the board's blend `Format (v7): RGBA16F RGBA` */
            TargetFormat::Rgba16f => Format::RGBA16F.0 << 12 | RgbComponentOrder::RGBA.0,
        }
    }

    /// The blend conversion of the colour preload: the pixel format without
    /// dithering (`pan_preload_emit_blend` passes `dithered` false to
    /// `pan_dithered_format_from_pipe_format`, pan_fb_preload.c:134-136):
    /// R5G6B5 PU, where a draw's blend has R5G6B5 AU, and RGBA8 TB for the
    /// 8-bit layouts as for a draw (the board's dumps, 0dce654a8:tools/mali/board-ref/
    /// 2026-10-19-preload-formats).
    fn preload_blend_format(self) -> u32 {
        match self {
            TargetFormat::Rgba8888 | TargetFormat::Xrgb8888 => Format::RGBA8_TB.0 << 12 | RgbComponentOrder::RGBA.0,
            TargetFormat::Rgb565 => Format::R5G6B5_PU.0 << 12 | RgbComponentOrder::RGB1.0,
            /* BFMT_FLOAT has the one format dithered or not */
            TargetFormat::Rgba16f => Format::RGBA16F.0 << 12 | RgbComponentOrder::RGBA.0,
        }
    }

    /// The target as the colour preload's texture (`pan_format_from_pipe_format`
    /// of its pipe format, pan_format.c): the texture format word and the
    /// plane's clump format.
    fn preload_view(self) -> (u32, ClumpFormat) {
        match self {
            TargetFormat::Rgba8888 => (Format::RGBA8_UNORM.0 << 12 | RgbComponentOrder::RGBA.0, ClumpFormat::RAW32),
            /* B, G, R, X bytes: PIPE_FORMAT_B8G8R8X8_UNORM */
            TargetFormat::Xrgb8888 => (Format::RGBA8_UNORM.0 << 12 | RgbComponentOrder::BGR1.0, ClumpFormat::RAW32),
            /* PIPE_FORMAT_B5G6R5_UNORM */
            TargetFormat::Rgb565 => (Format::RGB565.0 << 12 | RgbComponentOrder::BGR1.0, ClumpFormat::RAW16),
            /* PIPE_FORMAT_R16G16B16A16_FLOAT, 64 bits a texel */
            TargetFormat::Rgba16f => (Format::RGBA16F.0 << 12 | RgbComponentOrder::RGBA.0, ClumpFormat::RAW64),
        }
    }

    /// Channel 0's bits, the size Mesa packs the blend constant at
    /// (`format_desc->channel[0].size`: B of B8G8R8X8 and of B5G6R5).
    fn chan_size(self) -> u32 {
        match self {
            TargetFormat::Rgba8888 | TargetFormat::Xrgb8888 => 8,
            TargetFormat::Rgb565 => 5,
            TargetFormat::Rgba16f => 16,
        }
    }

    /// The layout has an alpha channel (`util_format_has_alpha`).
    fn has_alpha(self) -> bool {
        matches!(self, TargetFormat::Rgba8888 | TargetFormat::Rgba16f)
    }
}

/// The GL viewport and depth range of the draw, the target bottom-up (an
/// FBO).
#[derive(Clone, Copy, Debug)]
pub struct Viewport {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub near: f32,
    pub far: f32,
}

/// A vertex buffer: where it starts and how many bytes it has.
#[derive(Clone, Copy, Debug)]
pub struct VertexBuffer {
    pub gpu: u64,
    pub size: u32,
}

/// One vertex attribute: one to four 32-bit floats per vertex, its own
/// buffer.
#[derive(Clone, Copy, Debug)]
pub struct VertexAttribute {
    pub buffer: VertexBuffer,
    pub offset: u32,
    pub stride: u32,
    pub components: u32,
    /// 0: one element per vertex; n: one element per n instances (ES 3.0
    /// `glVertexAttribDivisor`). The builder builds 1 only, the divisor
    /// the board's dump shows (tools/mali/board-dump/instsplit-2026-10-24.txt).
    pub instance_divisor: u32,
}

/// One stencil face as the Depth/stencil descriptor holds it (Mesa's
/// `panfrost_create_depth_stencil_state`, Mesa 26.2.1 pan_cmdstream.c:
/// 4252-4270, and the reference value of `panfrost_emit_depth_stencil`,
/// :879-882).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StencilFace {
    pub func: Func,
    pub fail: StencilOp,
    pub depth_fail: StencilOp,
    pub depth_pass: StencilOp,
    pub value_mask: u8,
    pub write_mask: u8,
    pub reference: u8,
}

impl StencilFace {
    /// A face of a disabled `pipe_stencil_state` (all zero: NEVER, KEEP,
    /// masks 0) with GL's reference value, which Mesa writes whether the
    /// test is on or not.
    pub const fn off(reference: u8) -> StencilFace {
        StencilFace {
            func: Func::NEVER,
            fail: StencilOp::KEEP,
            depth_fail: StencilOp::KEEP,
            depth_pass: StencilOp::KEEP,
            value_mask: 0,
            write_mask: 0,
            reference,
        }
    }
}

/// The depth and stencil state of one draw. `depth_func` is ALWAYS when the
/// test is off (Mesa: `zsa->depth_enabled ? depth_func : ALWAYS`,
/// pan_cmdstream.c:4232-4233), and `depth_write` is GL's write mask only
/// when the test is on (ES 2.0 4.1.6; st/mesa's `depth_writemask`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DepthStencilState {
    pub depth_func: Func,
    pub depth_write: bool,
    pub stencil_test: bool,
    /// Front, back: the faces as Mesa packs them; [`StencilFace::off`] when
    /// the test is off.
    pub faces: [StencilFace; 2],
}

impl DepthStencilState {
    /// Depth and stencil off.
    pub const OFF: DepthStencilState = DepthStencilState {
        depth_func: Func::ALWAYS,
        depth_write: false,
        stencil_test: false,
        faces: [StencilFace::off(0), StencilFace::off(0)],
    };

    /// `pipe_zs_always_passes` (Mesa 26.2.1 pan_cmdstream.c:4207-4220).
    fn always_passes(&self) -> bool {
        self.depth_func == Func::ALWAYS && !(self.stencil_test && self.faces.iter().any(|f| f.func != Func::ALWAYS))
    }

    /// `util_writes_depth_stencil` (Mesa 26.2.1 src/gallium/auxiliary/util/
    /// u_inlines.h:961-982, the ZSA CSO's `writes_zs`, pan_cmdstream.c:
    /// 4290-4291): the depth is written (the test on with the write mask,
    /// and not NEVER, which writes nothing), or a stencil face writes (the
    /// test on, its write mask non-zero, an op other than KEEP).
    pub fn writes_zs(&self) -> bool {
        let depth = self.depth_write && self.depth_func != Func::NEVER;
        let face = |f: &StencilFace| {
            f.write_mask != 0 && [f.fail, f.depth_fail, f.depth_pass].iter().any(|op| *op != StencilOp::KEEP)
        };
        depth || (self.stencil_test && self.faces.iter().any(face))
    }

    /// Whether the draw reads or writes the depth/stencil attachment.
    pub fn uses_attachment(&self) -> bool {
        self.depth_func != Func::ALWAYS || self.depth_write || self.stencil_test
    }
}

/// The frame's depth/stencil attachment: a linear D24S8 buffer (depth in
/// the low 24 bits, stencil in the high byte, Mesa's
/// `PIPE_FORMAT_Z24_UNORM_S8_UINT`, `MALI_ZS_FORMAT_D24S8`, pan_desc.c:
/// 188-189), and the values the tile buffer starts at (Mesa's
/// `fb->zs.clear_value`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ZsAttachment {
    pub gpu: u64,
    /// Bytes from one row to the next (4 a pixel).
    pub row_stride: u32,
    pub clear_depth: f32,
    pub clear_stencil: u8,
}

/// The primitives a draw makes: GL's modes, each the Mali draw mode of the
/// same name (`pan_draw_mode`, Mesa 26.2.1 pan_cmdstream.h:161-186).
/// Points take the vertex shader's point size, or [`FIXED_POINT_SIZE`]
/// from a shader that does not write it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Topology {
    Triangles,
    TriangleStrip,
    TriangleFan,
    Lines,
    LineStrip,
    LineLoop,
    Points,
}

/// The winding of the faces a triangle draw culls (`GlesDraw::cull`):
/// glEnable(GL_CULL_FACE) with glCullFace and glFrontFace folded into the
/// winding of the culled faces. Points and lines are never culled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cull {
    None,
    Cw,
    Ccw,
}

impl Topology {
    /// Mesa's `u_reduced_prim(mode) == TRIANGLES`.
    fn polygon(self) -> bool {
        matches!(self, Topology::Triangles | Topology::TriangleStrip | Topology::TriangleFan)
    }

    fn draw_mode(self) -> DrawMode {
        match self {
            Topology::Triangles => DrawMode::TRIANGLES,
            Topology::TriangleStrip => DrawMode::TRIANGLE_STRIP,
            Topology::TriangleFan => DrawMode::TRIANGLE_FAN,
            Topology::Lines => DrawMode::LINES,
            Topology::LineStrip => DrawMode::LINE_STRIP,
            Topology::LineLoop => DrawMode::LINE_LOOP,
            Topology::Points => DrawMode::POINTS,
        }
    }

    /// `u_reduced_prim(mode) == MESA_PRIM_LINES`.
    fn lines(self) -> bool {
        matches!(self, Topology::Lines | Topology::LineStrip | Topology::LineLoop)
    }
}

/// The levels a texture descriptor names (`MAX_TEXTURE_LEVELS`: a 2D image
/// of at most 65536 texels a side has 17; the GL core keeps 16,
/// `CAP_MIP_LEVELS`).
pub const MAX_TEXTURE_LEVELS: usize = 16;

/// One level of a [`SampledTexture`]: its linear texels at `gpu`,
/// `row_stride` bytes a row.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TextureLevel {
    pub gpu: u64,
    pub row_stride: u32,
}

/// The textures (and samplers) one stage's tables hold
/// ([`DrawCall::textures`]): GL_MAX_TEXTURE_IMAGE_UNITS, the GL core's
/// `CAP_SAMPLERS` (src/gl/context.rs).
pub const MAX_STAGE_TEXTURES: usize = 16;

/// The layers of a 2D array texture: ES 3.0's GL_MAX_ARRAY_TEXTURE_LAYERS
/// minimum, 256 (the descriptor's `Array size` holds 65536).
pub const MAX_ARRAY_LAYERS: u32 = 256;

/// A 2D texture, a cube map or a 2D array, `levels` of its chain linear in
/// memory, and the GL sampler state it is read with: what a shader's
/// look-up reads at its index of the stage's texture and sampler tables.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SampledTexture {
    /// Level 0's extent (one face's for a cube); level l is `max(w >> l,
    /// 1)` x `max(h >> l, 1)`.
    pub width: u32,
    pub height: u32,
    /// Level l's face 0: face f of level l follows at `f * rows *
    /// row_stride` (the GL core stacks a level's faces on one pitch,
    /// `chain_layout_npot`, src/gl/texture.rs).
    pub level: [TextureLevel; MAX_TEXTURE_LEVELS],
    /// The levels the descriptor names, 1 or more.
    pub levels: u32,
    /// The faces: 1 for a 2D image, 6 for a cube map (`Dimension = Cube`,
    /// the planes face-major, level-minor: "faces as extra layers",
    /// Mesa 26.2.1 pan_texture.c:1074-1100); for a 2D array
    /// ([`SampledTexture::array`]) its layers, stacked as a cube's faces
    /// are (the GL core's level-major stack, `chain_layout_npot`).
    pub faces: u32,
    /// A 2D array: `Dimension = 2D`, `Array size` = [`SampledTexture::faces`]
    /// layers, a plane a layer and level, layer-major as a cube's
    /// (pan_texture.c:1074-1100, :1198-1217; the shim's texmore_46 and
    /// texmore_206_array traces).
    pub array: bool,
    /// The texture descriptor's format word (`MALI_PACK_FMT`).
    pub format: u32,
    /// The planes' clump format: RAW32 for 4-byte texels, a block format's
    /// own (ETC2).
    pub clump: ClumpFormat,
    /// A block format's bytes a 4x4 block, 0 for 4-byte texels: a block
    /// format's texels are interleaved (`texel_interleave`,
    /// pan_texture.c:1310-1311) and a row stride is a row of blocks.
    pub block_bytes: u32,
    /// `pan_translate_swizzle_4` of the channels the shader reads.
    pub swizzle: u32,
    /// GL_TEXTURE_MIN_FILTER and _MAG_FILTER are GL_NEAREST or
    /// GL_NEAREST_MIPMAP_* (minification), GL_NEAREST (magnification).
    pub min_nearest: bool,
    pub mag_nearest: bool,
    /// From the minification filter's mipmap part (`pan_pipe_to_mipmode`,
    /// Mesa 26.2.1 pan_cmdstream.c:152-170): none, nearest, trilinear.
    pub mipmap: MipmapMode,
    pub wrap_s: WrapMode,
    pub wrap_t: WrapMode,
    /// GL_TEXTURE_WRAP_R, which st/mesa passes for every target (REPEAT,
    /// GL's default, unless set); a cube map's is clamped whatever it is.
    pub wrap_r: WrapMode,
}

/// GL's default GL_TEXTURE_MAX_LOD, 1000, as the sampler's 5.8 field holds
/// it: Mesa clamps it there (`util_bitpack_ufixed_clamp`, the `ulod` type,
/// Mesa 26.2.1 src/panfrost/genxml/gen_pack.py:354-357; the board's dump
/// reads "Maximum LOD: 31.996094"), our packer refuses a value it cannot
/// hold, so the clamp is made here.
const SAMPLER_MAX_LOD: f32 = 8191.0 / 256.0;

/// `pan_translate_swizzle_4` (Mesa 26.2.1 src/panfrost/lib/pan_format.h):
/// three bits a channel, R in the low three; a source channel X, Y, Z, W is
/// 0..3, the constant 0 is 4 and 1 is 5.
pub const fn swizzle4(r: u32, g: u32, b: u32, a: u32) -> u32 {
    r | g << 3 | b << 6 | a << 9
}

/// The identity (R, G, B, A).
pub const SWIZZLE_RGBA: u32 = swizzle4(0, 1, 2, 3);
/// RGB read as stored, alpha 1: a GL base format without alpha kept in
/// RGBA8 storage.
pub const SWIZZLE_RGB1: u32 = swizzle4(0, 1, 2, 5);
/// Storage in B, G, R, A order read as R, G, B, A.
pub const SWIZZLE_BGRA: u32 = swizzle4(2, 1, 0, 3);
/// BGRA storage of a format without alpha.
pub const SWIZZLE_BGR1: u32 = swizzle4(2, 1, 0, 5);
/// The red channel in R, G and B, alpha 1: GL_OES_depth_texture's luminance
/// form `(d, d, d, 1.0)` over a one-channel depth storage, Mesa's `.XXXX`
/// composition for a depth view (pan_cmdstream.c
/// `panfrost_create_sampler_view_bo`); the board's `Swizzle: 2560 (RRR1)`.
pub const SWIZZLE_RRR1: u32 = swizzle4(0, 0, 0, 5);

impl SampledTexture {
    /// A cube map: six faces that are not a 2D array's layers.
    pub fn is_cube(&self) -> bool {
        self.faces == 6 && !self.array
    }

    /// Level `l`'s extent.
    pub fn extent(&self, l: u32) -> (u32, u32) {
        ((self.width >> l).max(1), (self.height >> l).max(1))
    }

    /// The sampler (`panfrost_create_sampler_state`, Mesa 26.2.1
    /// pan_cmdstream.c:200-260, from st/mesa's state for a GLES 2 texture:
    /// GL_TEXTURE_WRAP_R repeat, LOD bias 0, minimum LOD 0, no compare, a
    /// zero border, seamless cube maps; nearest minification and
    /// magnification clear round-to-nearest-even, :243-245). A cube map's
    /// three wrap modes are clamp-to-edge whatever GL's are: st/mesa
    /// normalises a seamless cube map's CSO so (st_atom_sampler.c:80-95;
    /// the board's cubemap dump, `Wrap Mode R: Clamp to Edge` with GL's
    /// default REPEAT on R).
    pub fn sampler(&self) -> Sampler {
        let cube = self.is_cube();
        Sampler {
            magnify_nearest: self.mag_nearest,
            minify_nearest: self.min_nearest,
            normalized_coordinates: true,
            lod_bias: 0.0,
            minimum_lod: 0.0,
            maximum_lod: SAMPLER_MAX_LOD,
            wrap_mode_s: if cube { WrapMode::CLAMP_TO_EDGE } else { self.wrap_s },
            wrap_mode_t: if cube { WrapMode::CLAMP_TO_EDGE } else { self.wrap_t },
            wrap_mode_r: if cube { WrapMode::CLAMP_TO_EDGE } else { self.wrap_r },
            mipmap_mode: self.mipmap,
            compare_function: Func::NEVER,
            seamless_cube_map: true,
            round_to_nearest_even: !(self.mag_nearest && self.min_nearest),
            ..Default::default()
        }
    }

    /// Level `l`'s rows of texels, or of 4x4 blocks.
    fn rows(&self, l: u32) -> u32 {
        let h = self.extent(l).1;
        if self.block_bytes != 0 {
            h.div_ceil(4)
        } else {
            h
        }
    }

    /// Face `face`'s address at level `l`: the GL core's face stride is the
    /// level's rows (`face * lh` rows, src/gl/texture.rs
    /// `cube_faces_are_stacked_rows_of_one_storage`).
    pub fn face_gpu(&self, face: u32, l: u32) -> u64 {
        let lv = self.level[l as usize];
        lv.gpu + u64::from(face) * u64::from(self.rows(l)) * u64::from(lv.row_stride)
    }

    /// Face `face`, level `l`'s plane (`emit_generic_plane`, Mesa 26.2.1
    /// src/panfrost/lib/pan_texture.c:425-459): the level's size, the
    /// face's address (`:388-422`, the layer term at `:418`) and row
    /// stride, no slice stride for a single-sampled 2D or cube image
    /// (:410-421; Mesa 25.0.7 wrote the layer stride there, the shim's
    /// 26.2.1 writes 0 in every plane of a cube, tools/mali/testdata/
    /// cube_vs), the level's extent.
    pub fn plane(&self, face: u32, l: u32) -> GenericPlane {
        let lv = self.level[l as usize];
        let (w, h) = self.extent(l);
        GenericPlane {
            clump_ordering: ClumpOrdering::LINEAR,
            clump_format: self.clump,
            size: lv.row_stride * self.rows(l),
            pointer: self.face_gpu(face, l),
            row_stride: lv.row_stride,
            slice_stride: 0,
            width: w,
            height: h,
            ..Default::default()
        }
    }

    /// The texture descriptor (`pan_sampled_texture_emit`, Mesa 26.2.1
    /// pan_texture.c:1272-1325): 2D (or, for six faces, a cube: `Dimension
    /// = Cube`, the width and height one face's, `Array size` 1, the
    /// layer count over 6, `pan_texture_get_array_size`, :1198-1217; a 2D
    /// array's `Array size` is its layer count)
    /// RGBA8 UNORM read through the swizzle, linear (no texel interleave),
    /// its levels from 0, LOD 0 to the last level, the planes, one a face
    /// and level, at `surfaces`.
    pub fn texture(&self, surfaces: u64) -> Texture {
        Texture {
            dimension: if self.is_cube() { TextureDimension::CUBE } else { TextureDimension::_2D },
            format: self.format,
            width: self.width,
            height: self.height,
            sample_count: 1,
            swizzle: self.swizzle,
            texel_interleave: self.block_bytes != 0,
            levels: self.levels,
            array_size: if self.array { self.faces } else { 1 },
            surfaces,
            minimum_lod: 0.0,
            maximum_lod: (self.levels - 1) as f32,
            ..Default::default()
        }
    }
}

/// The u16 indices of an indexed draw: [`DrawCall::vertex_count`] of them
/// at `gpu`, Mesa's `batch->indices`. The GL core widens u8 and u32
/// indices to u16 (`GlesDraw::index_size`), so u16 is the one type here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Indices {
    pub gpu: u64,
}

/// One draw: `vertex_count` vertices from `first_vertex`, or, with
/// `indices`, `vertex_count` indices.
#[derive(Clone, Copy, Debug)]
pub struct DrawCall<'a> {
    pub vs: &'a Program<'a>,
    pub fs: &'a Program<'a>,
    pub primitive: Topology,
    pub first_vertex: u32,
    pub vertex_count: u32,
    pub instance_count: u32,
    pub indices: Option<Indices>,
    pub attributes: &'a [VertexAttribute],
    pub viewport: Viewport,
    /// The GL scissor box (x, y, width, height) when the test is on.
    pub scissor: Option<[i32; 4]>,
    /// The GL line width, which Mesa puts in the primitive size of every
    /// draw that is not points (`panfrost_emit_primitive_size`).
    pub line_width: f32,
    /// The target is stored top-down (a window surface): Mesa's Y_0_TOP
    /// framebuffer. The viewport transform is mirrored (scale y negated,
    /// offset y = height - offset y, st_atom_viewport.c:63-68), and so are
    /// the draw's box (from the mirrored viewport, pan_cmdstream.c:723-724,
    /// and the scissor, st_atom_scissor.c:86-94) and the front face
    /// (st_atom_rasterizer.c:83-93 inverts it for an FBO, not for a
    /// window).
    pub flip_y: bool,
    /// The GL blend colour (`glBlendColor`), the fragment shader's
    /// [`Sysval::BlendConstants`].
    pub blend_color: [f32; 4],
    /// Blending on: the equations ([`super::blend`]); None for off.
    pub blend: Option<super::blend::Blending>,
    /// The GPU address of the blend shader of target 0, when its blend is
    /// one Mesa runs in a blend shader (`super::blend::needs_shader`): the
    /// descriptor is then in Shader mode and the fragment shader's
    /// `BRANCHZI` calls it (src/shaderc/online/mali/blend.rs). None
    /// otherwise.
    pub blend_shader: Option<u64>,
    /// GL_POLYGON_OFFSET_FILL on: glPolygonOffset's (factor, units).
    pub polygon_offset: Option<[f32; 2]>,
    /// The samples a 4x draw writes (bit n: sample n; glSampleCoverage,
    /// `GlesDraw::sample_mask`); not read for a single-sampled target.
    pub sample_mask: u8,
    /// The two stages' GL uniform storage, one dword per component in the
    /// front end's layout (`GlesDraw::vs_uniform_cpu`, `fs_uniform_cpu`):
    /// what [`PushWord::Uniform`] and [`PushWord::UniformHalves`] name.
    pub vs_uniforms: &'a [u32],
    pub fs_uniforms: &'a [u32],
    /// The depth test and write and the stencil test.
    pub zs: DepthStencilState,
    /// glColorMask (bit 0 R .. 3 A). 0 is a draw without a fragment shader
    /// (Mesa's `panfrost_fs_required`): it writes only depth and stencil.
    pub colour_mask: u8,
    /// glFrontFace(GL_CW): which winding the stencil's front face is.
    pub front_cw: bool,
    /// The winding of the faces to cull (triangles only).
    pub cull: Cull,
    /// The textures the fragment shader samples, texture i (and its
    /// sampler) at index i of the stage's texture and sampler tables
    /// (`MaliShader::samplers`: GL sampler binding `samplers[i]`'s); empty
    /// when it samples none ([`ShaderMeta::samples_texture`]). At most
    /// [`MAX_STAGE_TEXTURES`].
    pub textures: &'a [SampledTexture],
    /// The texture the vertex shader samples, when it samples one: both
    /// IDVS programs read it from the vertex resource table (Mesa 26.2.1
    /// pan_csf.c:1726-1727 selects the vertex SRT for the varying shader).
    pub vs_texture: Option<SampledTexture>,
    /// The occlusion counter this draw's samples set, when an occlusion
    /// query counts it: its GPU address, 8-byte aligned. ES 3.0's targets
    /// (GL_ANY_SAMPLES_PASSED and its conservative twin) are gallium's
    /// OCCLUSION_PREDICATE pair, so the mode is always PREDICATE: the
    /// hardware sets the first qword to 1 when a sample passes (the board's
    /// Mesa, 0dce654a8:tools/mali/board-ref/2026-10-20-query).
    pub occlusion_query: Option<u64>,
    /// GL_SAMPLE_ALPHA_TO_COVERAGE on a multisampled target
    /// (`GlesDraw::alpha_to_coverage`): the fragment's alpha takes samples
    /// out of its coverage. The program does not change (every fragment
    /// shader ends with ATEST, whose datum the hardware feeds from DCD0's
    /// `Alpha To Coverage`); the draw's DCD0 does (Mesa 26.2.1
    /// pan_csf.c:1585-1614).
    pub alpha_to_coverage: bool,
}

/// A frame of at most one draw: the target, its clear, its depth/stencil
/// attachment and the draw ([`build_frame`]).
#[derive(Clone, Copy, Debug)]
pub struct FrameInput<'a> {
    pub target: Target,
    pub clear_rgba: Option<[f32; 4]>,
    pub zs: Option<ZsAttachment>,
    pub draw: Option<DrawCall<'a>>,
}

/// Where the builder put each block (the draw's are `None` without a draw;
/// in a frame of several draws, the last draw's).
#[derive(Clone, Copy, Debug, Default)]
pub struct Layout {
    /// Framebuffer descriptor, room for the ZS/CRC extension, render target.
    pub fbd: Option<Block>,
    pub tls: Option<Block>,
    pub depth_stencil: Option<Block>,
    pub blend: Option<Block>,
    pub attributes: Option<Block>,
    pub buffers: Option<Block>,
    pub vs_sampler: Option<Block>,
    pub vs_sysvals: Option<Block>,
    pub vs_ubos: Option<Block>,
    pub vs_push: Option<Block>,
    /// The vertex shader's texture descriptor and its planes, when it
    /// samples one.
    pub vs_texture: Option<Block>,
    pub vs_plane: Option<Block>,
    /// The fragment shader's samplers, one a texture (or the workaround
    /// sampler), back to back.
    pub fs_sampler: Option<Block>,
    /// The fragment shader's texture descriptors, back to back, and every
    /// texture's planes (one a layer and level, texture 0's first), when it
    /// samples one.
    pub fs_texture: Option<Block>,
    pub fs_plane: Option<Block>,
    /// Empty (length 0) when the shader names no sysval.
    pub fs_sysvals: Option<Block>,
    pub fs_ubos: Option<Block>,
    /// None when the fragment shader pushes nothing.
    pub fs_push: Option<Block>,
    pub tiler: Option<Block>,
    pub vs_resources: Option<Block>,
    pub fs_resources: Option<Block>,
}

/// A built frame.
#[derive(Clone, Copy, Debug)]
pub struct Frame {
    pub layout: Layout,
    /// The FBD pointer RUN_FRAGMENT reads, with its tag bits.
    pub fbd_pointer: u64,
}

/// A frame between [`begin_frame`] and [`end_frame`]: its blocks so far.
/// Plain values, so the caller may keep it while the arenas and the stream
/// are made again over the same memory ([`Arena::resume`],
/// [`Stream::resume`]).
#[derive(Clone, Copy, Debug)]
pub struct OpenFrame {
    pub layout: Layout,
    /// The draws added.
    pub draws: u32,
    /// Whether a draw used the depth/stencil attachment.
    pub uses_zs: bool,
    /// A points draw was added to a bottom-up target: gl_PointCoord's t
    /// runs from the top of the sprite, which is the high y there
    /// (`batch->sprite_coord_origin`, pan_cmdstream.c:3515-3519, from
    /// st/mesa's `sprite_coord_mode`: ES's GL_UPPER_LEFT origin is
    /// LOWER_LEFT for a Y_0_BOTTOM framebuffer, st_atom_rasterizer.c:207-211).
    pub sprite_origin_max_y: bool,
    /// The frame starts from the target's old image, drawn back by this
    /// preload program instead of a clear ([`end_frame`]).
    pub colour_preload: Option<FrameShader>,
    /// The frame starts from the old depth, stencil or both of its
    /// [`ZsAttachment`], drawn back by this program (pre frame 1).
    pub zs_preload: Option<ZsPreload>,
    /// The programs uploaded into this frame ([`upload_program_cached`]).
    pub programs: ProgramCache,
    /// A 4x frame whose four samples are kept in memory, one plane a
    /// sample ([`SampleStore`]), as Mesa keeps a multisampled surface: the
    /// frame writes them back there instead of averaging them into the
    /// target, and its colour preload, if any, reads them from there, each
    /// sample its own. None: the 4x frame is a view of its single-sampled
    /// target (averaged on writeback, preloaded 1x into every sample).
    pub sample_store: Option<SampleStore>,
}

/// The four samples of a 4x frame in memory (Mesa's multisampled resource,
/// tools/cts/log/2026-10-28-splitref, sections 4.2 and 4.6): sample `n`'s
/// plane at `gpu + n * surface_stride`, each laid out as the target
/// (its row stride and height).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SampleStore {
    pub gpu: u64,
    /// Bytes from one sample's plane to the next.
    pub surface_stride: u32,
    /// The store holds the target's samples, and the frame's colour
    /// preload, if any, reads them back, each sample its own. Off: the store
    /// holds nothing of the target yet (a 4x frame that starts from the
    /// target's image and follows no frame that kept its samples), so the
    /// preload reads the single-sampled target into every sample, as
    /// without a store; the frame still writes its samples to the store.
    pub reload: bool,
    /// The frame resolves: its tile buffer, preloaded from the store, is
    /// averaged into the target ([`Msaa::AVERAGE`]) and every tile written
    /// whole; the store is only read. Off: the frame writes its samples
    /// back to the store ([`Msaa::LAYERED`]).
    pub resolve: bool,
}

/// The colour preload program in GPU memory (src/shaderc/online/mali/
/// preload.rs): its binary and its preloaded r48-r63.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameShader {
    pub binary: u64,
    pub preload: u32,
}

/// A frame's depth/stencil preload: its program, which planes it draws
/// back (`z` the depth, `s` the stencil), and whether the frame clears
/// the plane it does not (`cleared`: the tiles' depth/stencil are then
/// written clean, `pan_get_clean_tile_info`, pan_desc.c:1281-1297).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ZsPreload {
    pub shader: FrameShader,
    pub z: bool,
    pub s: bool,
    pub cleared: bool,
}

/// The resource tables of the preload's draw (`PAN_BLIT_TABLE_*`,
/// pan_fb_preload.c:55-61): attribute, attribute buffer, sampler, texture.
const PRELOAD_TABLES: usize = 4;

fn require(ok: bool, why: &'static str) -> Result<(), FrameError> {
    if ok {
        Ok(())
    } else {
        Err(FrameError::NotBuilt(why))
    }
}

/// Mesa's `float_to_fixed(f, 8, 0, false)` for an RGBA8 tile buffer
/// (src/panfrost/lib/pan_clear.c:61-73).
fn unorm8(f: f32) -> u32 {
    unorm(f, 8)
}

/// Mesa's `float_to_fixed(f, bits, 0, false)` (pan_clear.c:61-73), before
/// the shift by the fraction bits: saturate, scale by 2^bits - 1, round
/// half to even.
fn unorm(f: f32, bits: u32) -> u32 {
    /* SATURATE; NaN is not > 0, so it saturates to 0 */
    let f = if f > 0.0 {
        if f < 1.0 {
            f
        } else {
            1.0
        }
    } else {
        0.0
    };
    let v = f * ((1u32 << bits) - 1) as f32;
    let t = v as u32;
    let frac = v - t as f32;
    if frac > 0.5 || (frac == 0.5 && t & 1 == 1) {
        t + 1
    } else {
        t
    }
}

/// `pan_pack_color`, undithered (a clear, `panfrost_batch_clear`,
/// pan_job.c:815-816): each channel in the tile buffer's layout
/// (`tib_layouts`, pan_clear.c:68-75: R8G8B8A8 8.0 bits a channel,
/// R5G6B5A0 R 5.5, G 6.4, B 5.5, A 0.2), R lowest, the integer bits above
/// the fraction, alpha 1.0 for a layout without it, the 32-bit pattern in
/// all four words (pan_clear.c:49-54, 104-160). A float layout is raw
/// (`pan_pack_raw`, pan_clear.c:61-113): the four halves, R lowest, in 8
/// bytes, written twice (the board's 16F blue: 0, 0x3C003C00, 0,
/// 0x3C003C00).
fn clear_colour(rgba: [f32; 4], format: TargetFormat) -> RtClear {
    if format == TargetFormat::Rgba16f {
        let h = |c: usize| u32::from(f32_to_f16(rgba[c].to_bits()));
        let (lo, hi) = (h(0) | h(1) << 16, h(2) | h(3) << 16);
        return RtClear { color_0: lo, color_1: hi, color_2: lo, color_3: hi };
    }
    let alpha = if format.has_alpha() { rgba[3] } else { 1.0 };
    let v = match format.formats().0 {
        ColorBufferInternalFormat::R5G6B5A0 => {
            /* float_to_fixed undithered: the integer bits above the
             * fraction; alpha has no integer bit (0.2), so it is 0 */
            unorm(rgba[0], 5) << 5 | unorm(rgba[1], 6) << 4 << 10 | unorm(rgba[2], 5) << 5 << 20
        }
        _ => unorm8(rgba[0]) | unorm8(rgba[1]) << 8 | unorm8(rgba[2]) << 16 | unorm8(alpha) << 24,
    };
    RtClear { color_0: v, color_1: v, color_2: v, color_3: v }
}

/// `util_logbase2_ceil`.
fn log2_ceil(v: u32) -> u32 {
    if v <= 1 {
        0
    } else {
        32 - (v - 1).leading_zeros()
    }
}

/// What every target must be before anything of its frame is written: some
/// pixels, and 1 sample or 4. A 4x frame's depth/stencil is kept with 4
/// samples on chip too (the ZS target's MSAA, [`end_frame`]).
fn check_target(t: &Target) -> Result<(), FrameError> {
    require(t.width > 0 && t.height > 0, NOT_BUILT_EMPTY_TARGET)?;
    require(t.samples == 1 || t.samples == 4, NOT_BUILT_SAMPLES)
}

/// The effective tile size in pixels and the colour buffer allocation in
/// bytes for one target (`GENX(pan_select_tile_size)`, pan_desc.c:382-399):
/// the format's bytes a sample in the tile buffer
/// ([`TargetFormat::tib_bytes`]), times the samples (pan_desc.c:336-352);
/// the largest power-of-two tile the budget holds, clamped to the
/// hardware's; the allocation 1 KiB-aligned. The board's Mesa writes 1024
/// and 16384 for a 4x RGBA8 target, 1024 and 8192 for an RGBA16F one.
fn tile_size(t: &Target) -> (u32, u32) {
    let bpp = t.format.tib_bytes() * t.samples;
    let tile = (TIB_BUDGET >> log2_ceil(bpp)).min(MAX_EFFECTIVE_TILE);
    (tile, (bpp * tile).div_ceil(1024) * 1024)
}

/// The scissor of one draw as Mesa derives it (`panfrost_emit_viewport`,
/// pan_cmdstream.c:709-775): the viewport's rectangle clamped to the
/// target, cut by the GL scissor; `[min, max)`, or `None` when empty
/// (Mesa then draws nothing, `scissor_culls_everything`).
fn draw_box(d: &DrawCall, t: &Target) -> Option<[u32; 4]> {
    let v = &d.viewport;
    let clamp = |a: i64, hi: u32| a.clamp(0, i64::from(hi)) as u32;
    let (vx, vy) = (i64::from(v.x), i64::from(v.y));
    let mut minx = clamp(vx, t.width);
    let mut maxx = clamp(vx + i64::from(v.width), t.width);
    let mut miny = clamp(vy, t.height);
    let mut maxy = clamp(vy + i64::from(v.height), t.height);
    if let Some([x, y, w, h]) = d.scissor {
        let (x, y) = (i64::from(x), i64::from(y));
        minx = minx.max(clamp(x, t.width));
        miny = miny.max(clamp(y, t.height));
        maxx = maxx.min(clamp(x + i64::from(w), t.width));
        maxy = maxy.min(clamp(y + i64::from(h), t.height));
    }
    if minx >= maxx || miny >= maxy {
        return None;
    }
    /* a top-down target's rows: Mesa clamps the mirrored viewport and the
     * mirrored scissor; mirroring maps [0, height] onto itself, so it may
     * as well come after the clamp and the cut */
    if d.flip_y {
        (miny, maxy) = (t.height - maxy, t.height - miny);
    }
    Some([minx, miny, maxx, maxy])
}

/// A 64-byte-aligned address with a small tag in its low 6 bits (an SRT or
/// BLEND_DESC register: the address and its count). An address that is not
/// aligned would be changed by the tag, not tagged.
pub(super) fn tagged(gpu: u64, tag: u64, why: &'static str) -> Result<u64, FrameError> {
    if gpu & 63 != 0 || tag > 63 {
        return Err(FrameError::Invalid(why));
    }
    Ok(gpu | tag)
}

/// A stage's FAU register: the push block and its length in 64-bit slots in
/// bits 56-63 (csf_emit_shader_regs, pan_csf.c:858-865); 0 for none. The
/// slot count is OR-ed in; `emit_stage_uniforms` refused a push block whose
/// address uses those bits.
fn fau(push: Option<Block>, words: usize) -> u64 {
    push.map_or(0, |b| b.gpu | (words.div_ceil(2) as u64) << 56)
}

fn pack_u64(bytes: [u8; 8]) -> u64 {
    u64::from_le_bytes(bytes)
}
fn pack_u32(bytes: [u8; 4]) -> u32 {
    u32::from_le_bytes(bytes)
}

/// Build one frame of at most one draw: [`begin_frame`], [`add_draw`] and
/// [`end_frame`] (see the module documentation for what is and is not
/// built). Every refusal comes before anything is written.
pub fn build_frame(desc: &mut Arena, cs: &mut Stream, ctx: &Context, input: &FrameInput) -> Result<Frame, FrameError> {
    let t = input.target;
    check_target(&t)?;
    let clear = input.clear_rgba.ok_or(FrameError::NotBuilt(NOT_BUILT_NO_CLEAR))?;
    if let Some(d) = &input.draw {
        check_draw(d, ctx, &t, input.zs.is_some())?;
    }
    let mut open = begin_frame(desc, cs)?;
    if let Some(d) = &input.draw {
        add_draw(&mut open, desc, cs, ctx, &t, input.zs.is_some(), d)?;
    }
    end_frame(&open, desc, cs, ctx, &t, clear, input.zs)
}

/// Open a frame: `csf_init_batch` (pan_csf.c:262-276): the resources, the
/// scoreboard entries, the framebuffer descriptor's aggregate
/// (FRAMEBUFFER, ZS_CRC_EXTENSION, one RENDER_TARGET, each 64-byte
/// aligned: `alloc_fbd`, pan_csf.c:220-226) and the local storage. The
/// descriptor itself is written by [`end_frame`], when the frame's clear
/// and attachments are known.
pub fn begin_frame(desc: &mut Arena, cs: &mut Stream) -> Result<OpenFrame, FrameError> {
    cs.push(Instr::ReqResource(CsReqResource {
        compute: true,
        fragment: true,
        tiler: true,
        idvs: true,
        ..Default::default()
    }))?;
    cs.push(Instr::SetSbEntry(CsSetSbEntry { endpoint_entry: SB_RENDER, other_entry: SB_LS, ..Default::default() }))?;
    let fbd = desc.alloc(128 + 64 + 64, 64)?;
    let tls = desc.alloc(LocalStorage::LENGTH, 64)?;
    Ok(OpenFrame {
        layout: Layout { fbd: Some(fbd), tls: Some(tls), ..Default::default() },
        draws: 0,
        uses_zs: false,
        sprite_origin_max_y: false,
        colour_preload: None,
        zs_preload: None,
        programs: ProgramCache::default(),
        sample_store: None,
    })
}

/// Add one draw to an open frame. `has_zs`: the frame will be ended with a
/// depth/stencil attachment ([`end_frame`]'s `zs`).
///
/// A refused draw leaves the frame as it was: the checks come first, and a
/// draw that does not fit is undone (the arena's and the stream's tops put
/// back). Not fitting is an arena or the stream running out, or fewer
/// stream words left than the frame's end needs; for the frame's first
/// draw that is the error as it is (the arenas are too small for one
/// draw), for a later one [`NOT_BUILT_FRAME_ROOM`] (the draws so far fit;
/// the caller goes on in a second frame that preloads this one's image).
pub fn add_draw(
    open: &mut OpenFrame,
    desc: &mut Arena,
    cs: &mut Stream,
    ctx: &Context,
    t: &Target,
    has_zs: bool,
    d: &DrawCall,
) -> Result<(), FrameError> {
    check_target(t)?;
    check_draw(d, ctx, t, has_zs)?;
    /* a draw whose scissor is empty draws nothing (pan_cmdstream.c:3186-3191
     * and `panfrost_batch_skip_rasterization`) */
    let Some(sbox) = draw_box(d, t) else {
        return Ok(());
    };
    let (desc_top, cs_len, before) = (desc.top(), cs.words().len(), *open);
    /* a frame that keeps its samples is followed in its stream by the
     * resolve ([`resolve_frame`]) */
    let end_words = END_FRAME_WORDS + if open.sample_store.is_some_and(|s| !s.resolve) { RESOLVE_WORDS } else { 0 };
    let room = |cs: &Stream| {
        if cs.capacity() - cs.words().len() < end_words {
            Err(FrameError::Cs(CsError::StreamFull { capacity: cs.capacity() }))
        } else {
            Ok(())
        }
    };
    match emit_draw(desc, cs, ctx, t, d, sbox, open).and_then(|()| room(cs)) {
        Ok(()) => {
            open.draws += 1;
            open.uses_zs |= d.zs.uses_attachment();
            open.sprite_origin_max_y |= d.primitive == Topology::Points && !d.flip_y;
            Ok(())
        }
        Err(e) => {
            desc.rewind(desc_top);
            cs.truncate(cs_len);
            *open = before;
            let full =
                matches!(e, FrameError::Arena(ArenaError::Full { .. }) | FrameError::Cs(CsError::StreamFull { .. }));
            Err(if full && before.draws > 0 { FrameError::NotBuilt(NOT_BUILT_FRAME_ROOM) } else { e })
        }
    }
}

/// End an open frame: the framebuffer descriptor and the fragment job
/// (`GENX(csf_emit_fragment_job)`, pan_csf.c:798-847), then the batch end
/// (`csf_emit_batch_end`, pan_csf.c:328-361). `clear` is the colour the
/// frame starts from, `zs` its depth/stencil attachment: given, the tile
/// buffer's depth and stencil start at its clear values and are written
/// back to it.
pub fn end_frame(
    open: &OpenFrame,
    desc: &mut Arena,
    cs: &mut Stream,
    ctx: &Context,
    t: &Target,
    clear: [f32; 4],
    zs: Option<ZsAttachment>,
) -> Result<Frame, FrameError> {
    check_target(t)?;
    if open.uses_zs && zs.is_none() {
        return Err(FrameError::Invalid(INVALID_NO_ZS));
    }
    let store = open.sample_store;
    if let Some(s) = store {
        let plane = u64::from(t.row_stride) * u64::from(t.height);
        let ok = t.samples == 4
            && s.gpu.is_multiple_of(64)
            && s.surface_stride.is_multiple_of(64)
            && u64::from(s.surface_stride) >= plane;
        if !ok {
            return Err(FrameError::Invalid(INVALID_SAMPLE_STORE));
        }
    }
    /* without a sample store, a 4x target is a 4x view of a single-sampled
     * image: Mesa's preload reads the image's sample count
     * (`pan_preload_is_ms`, pan_fb_preload.c:162-179), so its pre-frame
     * draws are the 1x ones, which write each pixel's value into all four
     * samples of the tile buffer
     * (0dce654a8:tools/mali/board-ref/2026-10-23-msaa-preload). With one,
     * the image has 4 samples and the preload reads each sample back
     * (2026-10-28-splitref section 5) */
    let (Some(fbd), Some(tls)) = (open.layout.fbd, open.layout.tls) else {
        return Err(FrameError::Invalid(INVALID_OPEN_FRAME));
    };
    let tiler = open.layout.tiler;

    /* the batch's bounding box: the clear covers the target
     * (pan_job.c:836-837), and every frame clears; a draw's scissor
     * (pan_cmdstream.c:746) is inside it */
    let bbox = [0, 0, t.width, t.height];

    /* GENX(csf_emit_fbds) / pan_emit_fbd (Mesa 26.2.1 pan_desc.c:1380-1460):
     * the framebuffer parameters, the ZS/CRC extension when there is a
     * depth/stencil attachment, the render target after them */
    let (tile, cbuf_allocation) = tile_size(t);
    let mut params = FramebufferParameters {
        pre_frame_0: PrePostFrameShaderMode::NEVER,
        pre_frame_1: PrePostFrameShaderMode::NEVER,
        post_frame: PrePostFrameShaderMode::NEVER,
        sample_locations: sample_positions_of(ctx, t.sample_pattern()),
        width: t.width,
        height: t.height,
        bound_max_x: t.width - 1,
        bound_max_y: t.height - 1,
        effective_tile_size: tile,
        tie_break_rule: TieBreakRule::MINUS_180_IN_0_OUT,
        render_target_count: 1,
        /* "Default to 24 bit depth if there's no surface"; a D24S8 surface
         * is D24 too (`pan_get_z_internal_format`) */
        z_internal_format: ZInternalFormat::D24,
        color_buffer_allocation: cbuf_allocation,
        sample_count: t.samples,
        sample_pattern: t.sample_pattern(),
        /* `fb->sprite_coord_origin` (pan_job.c:486): the batch's tristate,
         * unset without a points draw. One target per frame, so every
         * points draw of a frame agrees */
        point_sprite_coord_origin_max_y: open.sprite_origin_max_y,
        tiler: tiler.map_or(0, |b| b.gpu),
        ..Default::default()
    };
    let mut rt_at = 128;
    if let Some(z) = zs {
        /* `fb->zs.clear_value`, written whether cleared or not
         * (pan_desc.c:1417-1418); the depth written back (`!discard.z`,
         * :1445); no separate stencil view for D24S8 (pan_job.c:596-612:
         * the S view is made only for a stencil-only format or a separate
         * stencil), so S Write Enable stays off (:1446) */
        params.z_clear = z.clear_depth;
        params.s_clear = u32::from(z.clear_stencil);
        params.z_write_enable = true;
        params.has_zs_crc_extension = true;
        /* pan_emit_zs_crc_ext (pan_desc.c:502-552) for a linear D24S8
         * view (`pan_emit_linear_zs_attachment`, :356-369): no CRC, no S
         * part; clean tile writes on when the tile buffer starts from the
         * clear values, off when it is preloaded (`pan_get_clean_tile_info`,
         * :1281-1290: `force_clean_write` is the clear). A 4x frame's
         * single-sampled D24S8 is a view of 4 samples on an image of 1
         * (EXT_multisampled_render_to_texture, pan_job.c:571): AVERAGE
         * (`mali_sampling_mode`, pan_desc.c:26-48), as the colour's */
        let ext = ZsTarget {
            write_format: ZsFormat::D24S8,
            block_format: BlockFormat::LINEAR,
            msaa: if t.samples > 1 { Msaa::AVERAGE } else { Msaa::SINGLE },
            clean_tile_write_enable: open.zs_preload.is_none_or(|p| p.cleared),
            base: z.gpu,
            row_stride: z.row_stride,
            surface_stride: 0,
        };
        desc.write(fbd, 128, &ext.pack()?)?;
        rt_at = 128 + ZsCrcExtension::LENGTH;
    }
    /* a frame that starts from the old image or the old depth and
     * stencil: its pre-frame draws (`pan_preload_emit_pre_frame_dcd`,
     * pan_fb_preload.c:1174-1252: pre frame 0, colour, INTERSECT; pre frame
     * 1, depth/stencil, EARLY_ZS_ALWAYS on v8-v12) */
    let zs_part = match (open.zs_preload, zs.as_ref()) {
        (Some(p), Some(z)) => Some((z, p)),
        (Some(_), None) => return Err(FrameError::Invalid(INVALID_NO_ZS)),
        (None, _) => None,
    };
    let resolve = store.is_some_and(|s| s.resolve);
    if resolve && (open.colour_preload.is_none() || store.is_some_and(|s| !s.reload)) {
        /* a resolve averages the samples the store holds: it must load them */
        return Err(FrameError::Invalid(INVALID_SAMPLE_STORE));
    }
    /* the image the colour preload reads: the store's samples, or the
     * target's one sample while the store holds nothing of it */
    let reload = store.filter(|s| s.reload);
    if open.colour_preload.is_some() || zs_part.is_some() {
        params.frame_shader_dcds = preload_dcds(desc, t, zs_part, open.colour_preload, reload, tls.gpu)?;
        if open.colour_preload.is_some() {
            /* INTERSECT loads only the tiles the frame's draws reach, the
             * others keep their memory; a resolve writes every tile
             * (below), so it loads every tile: ALWAYS, Mesa's mode for a
             * full-screen frame shader (pan_fb_preload.c:1194-1201, 1244) */
            params.pre_frame_0 =
                if resolve { PrePostFrameShaderMode::ALWAYS } else { PrePostFrameShaderMode::INTERSECT };
        }
        if zs_part.is_some() {
            params.pre_frame_1 = PrePostFrameShaderMode::EARLY_ZS_ALWAYS;
        }
    }
    desc.write(fbd, 0, &params.pack()?)?;
    /* pan_prepare_rt (pan_desc.c:491-618) for a linear view: cleared,
     * or preloaded (no clean-tile write, no clear colour) */
    let cleared = open.colour_preload.is_none();
    let (internal_format, writeback_format) = t.format.formats();
    /* a 4x tile buffer averaged into single-sampled memory; or its four
     * samples written to the store, one plane a sample (Mesa's
     * `mali_sampling_mode` LAYERED for a multisampled view, pan_desc.c:26-48;
     * 2026-10-28-splitref section 4.2: Writeback MSAA 3, surface stride a
     * plane), unless the frame resolves them into the target */
    let (writeback_msaa, writeback_buffer) = match store {
        Some(s) if !s.resolve => {
            (Msaa::LAYERED, RtBuffer { base: s.gpu, row_stride: t.row_stride, surface_stride: s.surface_stride })
        }
        _ => (
            if t.samples > 1 { Msaa::AVERAGE } else { Msaa::SINGLE },
            RtBuffer { base: t.gpu, row_stride: t.row_stride, surface_stride: 0 },
        ),
    };
    let rt = RgbRenderTarget {
        writeback_mode: WritebackMode::COMPAT,
        internal_format,
        write_enable: true,
        writeback_format,
        writeback_block_format: BlockFormat::LINEAR,
        writeback_msaa,
        dithering_enable: true,
        swizzle: t.format.swizzle(),
        /* `force_clean_writes = fb->rts[i].clear` (pan_desc.c:495); a
         * resolve writes every tile, those its preload left clean too (the
         * preload's fragments are clean writes, `colour_preload_dcd`) */
        clean_tile_write_enable: cleared || resolve,
        writeback_buffer,
        clear: if cleared { clear_colour(clear, t.format) } else { RtClear::default() },
        ..Default::default()
    };
    desc.write(fbd, rt_at, &rt.pack()?)?;
    desc.write(tls, 0, &LocalStorage::default().pack()?)?;
    let fbd_pointer = pack_u64(
        FramebufferPointer {
            pointer: fbd.gpu,
            zs_crc_extension_present: zs.is_some(),
            render_target_count: 1,
            ..Default::default()
        }
        .pack()?,
    );

    /* GENX(csf_emit_fragment_job) (pan_csf.c:798-847) */
    if tiler.is_some() {
        cs.push(Instr::FinishTiling(CsFinishTiling::default()))?;
        cs.wait(1 << SB_RENDER)?;
        cs.push(Instr::HeapOperation(CsHeapOperation {
            operation: CsHeapOperationEnum::VERTEX_TILER_COMPLETED,
            ..Default::default()
        }))?;
    }
    cs.move64(reg::FBD_POINTER, fbd_pointer)?;
    cs.move32(reg::BBOX_MIN, bbox[1] << 16 | bbox[0])?;
    cs.move32(reg::BBOX_MAX, (bbox[3] - 1) << 16 | (bbox[2] - 1))?;
    cs.move64(reg::TEM_POINTER, 0)?;
    cs.move32(reg::TEM_ROW_STRIDE, 0)?;
    /* Mesa now loads the OOM counter and, if the handler ran, points
     * RUN_FRAGMENT at the last incremental-rendering FBD
     * (pan_csf.c:820-827); neither exists here */
    cs.push(Instr::RunFragment(CsRunFragment { tile_order: TileRenderOrder::Z_ORDER, ..Default::default() }))?;
    cs.wait(1 << SB_RENDER)?;
    if let Some(tiler) = tiler {
        /* return the heap chunks the fragment job freed: the tiler
         * context's Completed top/bottom (words 10-13) */
        cs.move64(reg::TILER_DESC, tiler.gpu)?;
        cs.push(Instr::LoadMultiple(CsLoadMultiple {
            offset: 40,
            mask: 0xf,
            address: reg::TILER_DESC,
            base_register: reg::HEAP_CHUNKS,
            ..Default::default()
        }))?;
        cs.wait(1 << SB_LS)?;
        cs.push(Instr::FinishFragment(CsFinishFragment {
            increment_fragment_completed: true,
            first_heap_chunk: reg::HEAP_CHUNKS,
            last_heap_chunk: reg::HEAP_CHUNKS + 2,
            ..Default::default()
        }))?;
    }

    /* csf_emit_batch_end (pan_csf.c:328-361) without the PAN_DBG_SYNC
     * error-state store */
    cs.wait(0xff)?;
    cs.move32(reg::FLUSH_ID, 0)?;
    cs.push(Instr::FlushCache2(CsFlushCache2 {
        l2_flush_mode: CsFlushMode::CLEAN,
        lsc_flush_mode: CsFlushMode::CLEAN,
        other_flush_mode: CsOtherFlushMode::INVALIDATE,
        latest_flush_id: reg::FLUSH_ID,
        /* cs_defer(0, 0): wait on nothing, signal slot 0 */
        ..Default::default()
    }))?;
    cs.wait(1 << SB_LS)?;
    Ok(Frame { layout: open.layout, fbd_pointer })
}

/// Average the four samples `store` holds into the single-sampled target:
/// a frame of no draws after the one that wrote them back, in the same
/// stream (its batch end cleaned the caches its writes went through and
/// invalidated the texture caches its reads go through). Its pre-frame draw
/// draws them back with `preload`, the per-sample colour preload program,
/// as the next frame would (pre frame 0 ALWAYS: every tile, as no draw
/// marks one), and its render target averages them into `t`, every tile
/// written ([`SampleStore::resolve`]). Not Mesa's: Mesa keeps the samples
/// and resolves when the image is read, a blit of four `TEX_FETCH`es it
/// averages in a third frame (tools/cts/log/2026-10-28-splitref section 4);
/// here the GL core reads the target's memory after the kick, so every kick
/// of a 4x frame that keeps its samples resolves them.
pub fn resolve_frame(
    desc: &mut Arena,
    cs: &mut Stream,
    ctx: &Context,
    t: &Target,
    store: SampleStore,
    preload: FrameShader,
) -> Result<Frame, FrameError> {
    let mut open = begin_frame(desc, cs)?;
    open.colour_preload = Some(preload);
    open.sample_store = Some(SampleStore { reload: true, resolve: true, ..store });
    end_frame(&open, desc, cs, ctx, t, [0.0; 4], None)
}

/// Refuse a draw the builder does not build, each cause by name.
fn check_draw(d: &DrawCall, ctx: &Context, t: &Target, has_zs: bool) -> Result<(), FrameError> {
    let (vs, fs) = (&d.vs.meta, &d.fs.meta);
    require(vs.stage == ShaderStage::VERTEX && fs.stage == ShaderStage::FRAGMENT, NOT_BUILT_STAGE)?;
    if let Some(i) = d.indices {
        if i.gpu % 2 != 0 {
            return Err(FrameError::Invalid(INVALID_INDEX_ALIGN));
        }
        if d.first_vertex != 0 {
            return Err(FrameError::Invalid(INVALID_INDEXED_FIRST_VERTEX));
        }
    }
    require(d.attributes.iter().all(|a| a.instance_divisor <= 1), NOT_BUILT_INSTANCE_DIVISOR)?;
    require(
        (1..=MAX_ATTRIBUTES).contains(&d.attributes.len())
            && d.attributes.iter().all(|a| (1..=4).contains(&a.components)),
        NOT_BUILT_ATTRIBUTES,
    )?;
    if (vs.varyings != 0 || fs.varyings != 0) && d.vs.varying_spd.is_none() {
        return Err(FrameError::Invalid(INVALID_VARYINGS));
    }
    /* a discarding shader is built (`emit_draw`: the draw runs it whatever
     * its colour mask, forward pixel kill off, the coverage marked modified,
     * the depth/stencil update late when written; docs/notes/
     * mali-discard.md); the other side effects are not */
    require(!(fs.writes_depth || fs.writes_stencil || fs.writes_coverage || fs.writes_global), NOT_BUILT_FS_EFFECTS)?;
    require(fs.colour_outputs == 1, NOT_BUILT_FS_OUTPUTS)?;
    if fs.samples_texture == d.textures.is_empty() || vs.samples_texture != d.vs_texture.is_some() {
        return Err(FrameError::Invalid(INVALID_TEXTURE));
    }
    require(d.textures.len() <= MAX_STAGE_TEXTURES, NOT_BUILT_TEXTURE_SIZE)?;
    for t in d.vs_texture.iter().chain(d.textures) {
        require(
            (1..=65536).contains(&t.width)
                && (1..=65536).contains(&t.height)
                && (1..=MAX_TEXTURE_LEVELS as u32).contains(&t.levels)
                && if t.array { (1..=MAX_ARRAY_LAYERS).contains(&t.faces) } else { t.faces == 1 || t.faces == 6 },
            NOT_BUILT_TEXTURE_SIZE,
        )?;
        for l in 0..t.levels {
            let lv = t.level[l as usize];
            let row = if t.block_bytes != 0 { t.block_bytes * t.extent(l).0.div_ceil(4) } else { 4 * t.extent(l).0 };
            require(lv.row_stride >= row, NOT_BUILT_TEXTURE_SIZE)?;
            /* every plane's pointer on 64 bytes: a face's too (the board's
             * Mesa samples faces at 64-byte steps, not 4 KiB ones:
             * docs/notes/mali-cube-textures.md (e)) */
            if (0..t.faces).any(|f| t.face_gpu(f, l) % 64 != 0) {
                return Err(FrameError::Invalid(INVALID_TEXTURE_ALIGN));
            }
        }
    }
    /* a target the draw does not write takes no blend shader whatever the
     * equation (`panfrost_get_blend`, pan_context.c:186-190) */
    let mask = d.colour_mask & 0xf;
    if let Some(b) = d.blend.filter(|_| mask != 0) {
        if super::blend::needs_shader(&b, &d.blend_color) {
            require(d.blend_shader.is_some(), NOT_BUILT_BLEND_SHADER)?;
        } else if d.blend_shader.is_some() {
            return Err(FrameError::Invalid(INVALID_BLEND_SHADER));
        }
    } else if d.blend_shader.is_some() {
        return Err(FrameError::Invalid(INVALID_BLEND_SHADER));
    }
    /* an RGBA16F target (RAW64 in the tile buffer) takes only an opaque
     * write of every channel: `pan_blend_is_opaque` on the equation, no
     * partial mask (which reads the destination) */
    if t.format == TargetFormat::Rgba16f && mask != 0 {
        let blended = d.blend.is_some_and(|b| !super::blend::is_opaque(&b));
        require(mask == 0xf && !blended, NOT_BUILT_FLOAT_BLEND)?;
    }
    for (m, uniforms) in [(vs, d.vs_uniforms), (fs, d.fs_uniforms)] {
        let named = |w: &PushWord| match *w {
            PushWord::Sysval { sysval, component } => sysval < m.sysvals.len() && component < 4,
            PushWord::FbWposYTransform { component } => component < 4,
            _ => true,
        };
        require(m.push.iter().all(named), NOT_BUILT_PUSH)?;
        let held = |w: &PushWord| match *w {
            PushWord::Uniform { dword } => dword < uniforms.len(),
            PushWord::UniformHalves { lo, hi } => lo < uniforms.len() && hi.is_none_or(|h| h < uniforms.len()),
            _ => true,
        };
        require(m.push.iter().all(held), NOT_BUILT_PUSH_UNIFORM)?;
        require(m.push.len() <= 4 * 32 * 2, NOT_BUILT_PUSH_COUNT)?;
        require(m.sysvals.len() <= MAX_SYSVALS, NOT_BUILT_SYSVALS)?;
        /* the block the shader loads: its rows inside the storage, one
         * block from word 0 up (`emit_stage_uniforms` writes it so) */
        let mut next = 0;
        for r in m.ubo {
            require(r.comps > 0 && r.dword + r.comps <= uniforms.len() && r.word == next, NOT_BUILT_UBO_BLOCK)?;
            next += r.words();
        }
    }
    if d.attributes.iter().any(|a| a.offset > i32::MAX as u32) {
        return Err(FrameError::Invalid(INVALID_ATTRIBUTE_OFFSET));
    }
    /* the hierarchy mask rule is measured for 8 levels only */
    require(ctx.tiler_max_levels == MEASURED_TILER_LEVELS, NOT_BUILT_TILER_LEVELS)?;
    if d.zs.uses_attachment() && !has_zs {
        return Err(FrameError::Invalid(INVALID_NO_ZS));
    }
    Ok(())
}

/// A frame's pre-frame draws (`GENX(pan_preload_fb)`, Mesa 26.2.1
/// pan_fb_preload.c:1302-1340, v9-v12): the four corners of the target in
/// pixels, uploaded once for both parts; Mesa's three DCDs (pre frame 0 and
/// 1, post frame), the unused ones left zero; then the depth/stencil part
/// into DCD 1 and the colour part into DCD 0, in Mesa's order. Returns the
/// DCD array's address.
fn preload_dcds(
    desc: &mut Arena,
    t: &Target,
    zs: Option<(&ZsAttachment, ZsPreload)>,
    colour: Option<FrameShader>,
    store: Option<SampleStore>,
    tls: u64,
) -> Result<u64, FrameError> {
    if colour.is_some() {
        require(t.row_stride.is_multiple_of(64), NOT_BUILT_PRELOAD_STRIDE)?;
    }
    let (w, h) = (t.width as f32, t.height as f32);
    let rect = [0.0, 0.0, 0.0, 1.0, w, 0.0, 0.0, 1.0, 0.0, h, 0.0, 1.0, w, h, 0.0, 1.0f32];
    let mut bytes = [0u8; 64];
    for (k, v) in rect.iter().enumerate() {
        bytes[4 * k..4 * k + 4].copy_from_slice(&v.to_le_bytes());
    }
    let coords = desc.upload(&bytes, 64)?;
    let dcds = desc.alloc(3 * Draw::LENGTH, Draw::ALIGN as u64)?;
    if let Some((z, p)) = zs {
        let draw = zs_preload_dcd(desc, t, z, coords.gpu, tls, p)?;
        desc.write(dcds, Draw::LENGTH, &draw.pack()?)?;
    }
    if let Some(p) = colour {
        let planes = zs.map_or((false, false), |(_, p)| (p.z, p.s));
        let draw = colour_preload_dcd(desc, t, planes, coords.gpu, tls, p, store)?;
        desc.write(dcds, 0, &draw.pack()?)?;
    }
    Ok(dcds.gpu)
}

/// The inputs both preload parts read (`pan_preload_emit_sampler`,
/// `_varying`, `_varying_buffer`, pan_fb_preload.c:762-842): a nearest
/// sampler that does not normalise the coordinate, the corners at `coords`
/// as an RGB32F attribute of 16 bytes a vertex and its buffer. Returns the
/// sampler, the attribute and the buffer.
fn preload_inputs(desc: &mut Arena, coords: u64) -> Result<(Block, Block, Block), FrameError> {
    let sampler = desc.alloc(Sampler::LENGTH, Sampler::ALIGN as u64)?;
    let s = Sampler {
        seamless_cube_map: false,
        normalized_coordinates: false,
        minify_nearest: true,
        magnify_nearest: true,
        ..Default::default()
    };
    desc.write(sampler, 0, &s.pack()?)?;
    let attribute = desc.alloc(Attribute::LENGTH, Attribute::ALIGN as u64)?;
    let attr = Attribute {
        attribute_type: AttributeType::_1D,
        offset_enable: false,
        format: Format::RGB32F.0 << 12 | RgbComponentOrder::RGB1.0,
        table: 1,
        frequency: AttributeFrequency::VERTEX,
        buffer_index: 0,
        stride: 16,
        ..Default::default()
    };
    desc.write(attribute, 0, &attr.pack()?)?;
    let buffer = desc.alloc(Buffer::LENGTH, Buffer::ALIGN as u64)?;
    desc.write(buffer, 0, &Buffer { address: coords, size: 64, ..Default::default() }.pack()?)?;
    Ok((sampler, attribute, buffer))
}

/// A preload part's resource tables (`PAN_BLIT_TABLE_*`: attribute,
/// attribute buffer, sampler, `textures` textures from `texture`) and its
/// SPD (pan_fb_preload.c:1063-1101). Returns the table and the SPD.
fn preload_tables(
    desc: &mut Arena,
    inputs: (Block, Block, Block),
    texture: Block,
    textures: u64,
    p: FrameShader,
) -> Result<(Block, Block), FrameError> {
    let (sampler, attribute, buffer) = inputs;
    let table = desc.alloc(PRELOAD_TABLES * Resource::LENGTH, 64)?;
    for (k, (b, n)) in [(attribute, 1u64), (buffer, 1), (sampler, 1), (texture, textures)].into_iter().enumerate() {
        resource(desc, table, k, b.gpu, n)?;
    }
    let spd = desc.alloc(ShaderProgram::LENGTH, ShaderProgram::ALIGN as u64)?;
    let sp = ShaderProgram {
        stage: ShaderStage::FRAGMENT,
        fragment_coverage_bitmask_type: CoverageBitmaskType::GL,
        register_allocation: ShaderRegisterAllocation::_32_PER_THREAD,
        binary: p.binary,
        preload: Preload { r48_r63: p.preload, ..Default::default() },
        ..Default::default()
    };
    desc.write(spd, 0, &sp.pack()?)?;
    Ok((table, spd))
}

/// The ZSD of both preload parts (`pan_preload_emit_zs(pool, z, s)`,
/// pan_fb_preload.c:961-996, with the frame's `fb->zs.preload.z` and `.s`,
/// :1093-1094, 1142): always passing, replacing, culling nothing; the
/// depth (`z`) and the stencil (`s`) the frame preloads come from the
/// shader and are written, the others are not.
fn preload_zsd(desc: &mut Arena, (z, s): (bool, bool)) -> Result<Block, FrameError> {
    let zsd = desc.alloc(DepthStencil::LENGTH, 32)?;
    let ds = DepthStencil {
        front_compare_function: Func::ALWAYS,
        front_stencil_fail: StencilOp::REPLACE,
        front_depth_fail: StencilOp::REPLACE,
        front_depth_pass: StencilOp::REPLACE,
        back_compare_function: Func::ALWAYS,
        back_stencil_fail: StencilOp::REPLACE,
        back_depth_fail: StencilOp::REPLACE,
        back_depth_pass: StencilOp::REPLACE,
        stencil_from_shader: s,
        stencil_test_enable: s,
        front_write_mask: 0xff,
        back_write_mask: 0xff,
        front_value_mask: 0xff,
        back_value_mask: 0xff,
        depth_function: Func::ALWAYS,
        depth_write_enable: z,
        depth_clamp_mode: DepthClampMode::_0_1,
        depth_source: if z { DepthSource::SHADER } else { DepthSource::FIXED_FUNCTION },
        depth_cull_enable: false,
        ..Default::default()
    };
    desc.write(zsd, 0, &ds.pack()?)?;
    Ok(zsd)
}

/// The colour part (`pan_preload_emit_dcd`, pan_fb_preload.c:1019-1162, for
/// one single-sampled RGBA8 target): the target as a 2D RGBA8 texture of
/// one level, an opaque blend into target 0 (`pan_preload_emit_blend`,
/// :100-160), the preload ZSD
/// ([`preload_zsd`] of the planes the frame also preloads), and
/// the DCD that names them: early depth/stencil forced (no ATEST in the
/// program), forward pixel kill allowed, the tiles still clean.
///
/// With a sample store, the image read back is the store's four samples
/// (2026-10-28-splitref section 4.6): the texture has 4 samples, its plane
/// the store's four planes (size four planes, slice stride one), and the
/// draw is multisampled and run once a sample (`pan_preload_is_ms`,
/// pan_fb_preload.c:1095, 1144-1145), so `p` must be the program that
/// fetches the sample it runs for.
fn colour_preload_dcd(
    desc: &mut Arena,
    t: &Target,
    zs: (bool, bool),
    coords: u64,
    tls: u64,
    p: FrameShader,
    store: Option<SampleStore>,
) -> Result<Draw, FrameError> {
    let (format, clump_format) = t.format.preload_view();
    let plane = desc.alloc(GenericPlane::LENGTH, NullPlane::ALIGN as u64)?;
    /* a single-sampled 2D image has no slice stride in 26.2.1
     * (pan_texture.c:416-420); a multisampled one has its sample planes */
    let (pointer, size, slice_stride, sample_count) = match store {
        Some(s) => (s.gpu, 4 * s.surface_stride, s.surface_stride, 4),
        None => (t.gpu, t.row_stride * t.height, 0, 1),
    };
    let pl = GenericPlane {
        clump_ordering: ClumpOrdering::LINEAR,
        clump_format,
        size,
        pointer,
        row_stride: t.row_stride,
        slice_stride,
        width: t.width,
        height: t.height,
        ..Default::default()
    };
    desc.write(plane, 0, &pl.pack()?)?;
    let texture = desc.alloc(Texture::LENGTH, Texture::ALIGN as u64)?;
    let tex = Texture {
        dimension: TextureDimension::_2D,
        format,
        width: t.width,
        height: t.height,
        sample_count,
        swizzle: SWIZZLE_RGBA,
        levels: 1,
        array_size: 1,
        surfaces: plane.gpu,
        ..Default::default()
    };
    desc.write(texture, 0, &tex.pack()?)?;
    let inputs = preload_inputs(desc, coords)?;
    let (table, spd) = preload_tables(desc, inputs, texture, 1, p)?;
    let blend = desc.alloc(Blend::LENGTH, 64)?;
    let replace =
        BlendFunction { a: BlendOperandA::SRC, b: BlendOperandB::SRC, c: BlendOperandC::ZERO, ..Default::default() };
    let bd = Blend {
        enable: true,
        round_to_fb_precision: true,
        equation: BlendEquation { rgb: replace, alpha: replace, color_mask: 0xf },
        internal: InternalBlend {
            mode: BlendMode::OPAQUE,
            fixed_function: BlendFixedFunction {
                num_comps: 4,
                rt: 0,
                conversion: InternalConversion { memory_format: t.format.preload_blend_format(), raw: false },
            },
            ..Default::default()
        },
        ..Default::default()
    };
    desc.write(blend, 0, &bd.pack()?)?;
    let zsd = preload_zsd(desc, zs)?;
    Ok(Draw {
        flags_0: DcdFlags0 {
            zs_update_operation: PixelKill::FORCE_EARLY,
            pixel_kill_operation: PixelKill::FORCE_EARLY,
            allow_forward_pixel_to_kill: true,
            allow_forward_pixel_to_be_killed: true,
            clean_fragment_write: true,
            multisample_enable: store.is_some(),
            evaluate_per_sample: store.is_some(),
            ..Default::default()
        },
        flags_1: DcdFlags1 { sample_mask: 0xffff, render_target_mask: 1 },
        maximum_z: 1.0,
        depth_stencil: zsd.gpu,
        blend: blend.gpu,
        blend_count: 1,
        shader: ShaderEnvironment {
            resources: table.gpu | PRELOAD_TABLES as u64,
            shader: spd.gpu,
            thread_storage: tls,
            ..Default::default()
        },
        ..Default::default()
    })
}

/// The depth/stencil part (`pan_preload_emit_dcd` with `zs`, for a linear
/// D24S8 whose depth, stencil or both are preloaded): the buffer as a 2D
/// texture a preloaded plane, in that order (`pan_preload_emit_textures`,
/// :845-930), the depth as Z24X8 UNORM and the stencil as X24S8 UINT (S8
/// read from the X24S8 clump), each with the red channel replicated
/// (`pan_texture_swizzle_replicate_x`); a
/// ZSD that takes depth and stencil from the shader ([`preload_zsd`]); no
/// blend; and the
/// DCD: depth/stencil update and pixel kill forced late (ZS_EMIT, :1124-1128),
/// no forward pixel kill, no render target, the tiles still clean.
fn zs_preload_dcd(
    desc: &mut Arena,
    t: &Target,
    z: &ZsAttachment,
    coords: u64,
    tls: u64,
    p: ZsPreload,
) -> Result<Draw, FrameError> {
    let views = [
        (p.z, Format::Z24X8_UNORM.0 << 12 | RgbComponentOrder::RGBA.0, ClumpFormat::RAW32),
        (p.s, Format::S8.0 << 12 | RgbComponentOrder::GRBA.0, ClumpFormat::X24S8),
    ];
    let count = views.iter().filter(|v| v.0).count();
    let textures = desc.alloc(count * Texture::LENGTH, Texture::ALIGN as u64)?;
    for (k, (_, format, clump_format)) in views.into_iter().filter(|v| v.0).enumerate() {
        let plane = desc.alloc(GenericPlane::LENGTH, 64)?;
        let pl = GenericPlane {
            clump_ordering: ClumpOrdering::LINEAR,
            clump_format,
            size: z.row_stride * t.height,
            pointer: z.gpu,
            row_stride: z.row_stride,
            slice_stride: 0,
            width: t.width,
            height: t.height,
            ..Default::default()
        };
        desc.write(plane, 0, &pl.pack()?)?;
        let tex = Texture {
            dimension: TextureDimension::_2D,
            format,
            width: t.width,
            height: t.height,
            sample_count: 1,
            /* R R R R: every channel the red one */
            swizzle: 0,
            levels: 1,
            array_size: 1,
            surfaces: plane.gpu,
            ..Default::default()
        };
        desc.write(textures, k * Texture::LENGTH, &tex.pack()?)?;
    }
    let inputs = preload_inputs(desc, coords)?;
    let (table, spd) = preload_tables(desc, inputs, textures, count as u64, p.shader)?;
    let zsd = preload_zsd(desc, (p.z, p.s))?;
    Ok(Draw {
        flags_0: DcdFlags0 {
            zs_update_operation: PixelKill::FORCE_LATE,
            pixel_kill_operation: PixelKill::FORCE_LATE,
            allow_forward_pixel_to_kill: false,
            allow_forward_pixel_to_be_killed: true,
            clean_fragment_write: true,
            ..Default::default()
        },
        flags_1: DcdFlags1 { sample_mask: 0xffff, render_target_mask: 0 },
        maximum_z: 1.0,
        depth_stencil: zsd.gpu,
        shader: ShaderEnvironment {
            resources: table.gpu | PRELOAD_TABLES as u64,
            shader: spd.gpu,
            thread_storage: tls,
            ..Default::default()
        },
        ..Default::default()
    })
}

/// The value of one sysval component for this draw. `flip_height` is the
/// target's height when it is top-down ([`DrawCall::flip_y`]).
fn sysval(s: Sysval, component: usize, v: &Viewport, flip_height: Option<u32>, blend_color: &[f32; 4]) -> f32 {
    /* _mesa_get_viewport_xform (Mesa 26.2.1, src/mesa/main/viewport.c:535-561): GL's
     * lower-left origin, depth -1..1 */
    let mut half = [0.5 * v.width as f32, 0.5 * v.height as f32, 0.5 * (v.far - v.near)];
    let mut centre = [half[0] + v.x as f32, half[1] + v.y as f32, 0.5 * (v.near + v.far)];
    /* st_update_viewport for Y_0_TOP (st_atom_viewport.c:63-68) */
    if let Some(h) = flip_height {
        half[1] = -half[1];
        centre[1] = h as f32 - centre[1];
    }
    match (s, component) {
        (Sysval::ViewportScale, c @ 0..=2) => half[c],
        (Sysval::ViewportOffset, c @ 0..=2) => centre[c],
        (Sysval::BlendConstants, c @ 0..=3) => blend_color[c],
        /* the fourth lane, which Mesa leaves as its stack had it */
        _ => 0.0,
    }
}

/// One stage's samplers, sysval block, uniform block, UBO table and push
/// constants, in Mesa's order (`panfrost_update_shader_state`,
/// pan_cmdstream.c:2809-2871; `panfrost_emit_const_buf`, :1386-1470).
/// Returns the UBO table, its count, the sampler, the sysval block (length
/// 0 when the shader names no sysval) and the push block.
///
/// The UBO table: the sysval block alone as entry 0 (the board's Mesa
/// 25.0.7 puts the sysvals after the user UBOs, of which there are none
/// when every uniform is pushed), or, for a shader with a
/// [`ShaderMeta::ubo`] block, that block as entry 0 and the sysvals as
/// entry 1 (`PAN_UBO_SYSVALS`; the shim's uniidx_i trace: entry 0 of the
/// fragment resource table `Size: 0x40`, a `Buffer` of 0x24 bytes then one
/// of 0x10, tools/mali/testdata/uniidx_i.expect.json's draw). The block's
/// bytes are the uniform storage's dwords in Mesa's layout, uploaded once
/// a draw (`panfrost_map_constant_buffer_gpu`).
#[allow(clippy::type_complexity)]
fn emit_stage_uniforms(
    desc: &mut Arena,
    m: &ShaderMeta,
    d: &DrawCall,
    target_height: u32,
    uniforms: &[u32],
    textures: &[SampledTexture],
) -> Result<(Block, u32, Block, Block, Option<Block>), FrameError> {
    let (v, blend) = (&d.viewport, &d.blend_color);
    let flip = d.flip_y.then_some(target_height);
    /* the stage's samplers, sampler i of texture i, one array
     * (`panfrost_emit_sampler_descriptors`, Mesa 26.2.1
     * pan_cmdstream.c:2024-2043), or with none bound Mesa's one workaround
     * sampler at its defaults (`panfrost_upload_wa_sampler`, :2013-2020) */
    let sampler = desc.alloc(Sampler::LENGTH * textures.len().max(1), Sampler::ALIGN as u64)?;
    if textures.is_empty() {
        desc.write(sampler, 0, &Sampler::default().pack()?)?;
    }
    for (i, t) in textures.iter().enumerate() {
        desc.write(sampler, i * Sampler::LENGTH, &t.sampler().pack()?)?;
    }
    /* the sysval block, 16 bytes a slot, then the UBO table: user UBOs
     * (none) plus the sysval block as the last (pan_cmdstream.c:1397-1432) */
    let sysvals = desc.alloc(16 * m.sysvals.len(), 16)?;
    for (i, s) in m.sysvals.iter().enumerate() {
        for c in 0..4 {
            desc.write(sysvals, 16 * i + 4 * c, &sysval(*s, c, v, flip, blend).to_le_bytes())?;
        }
    }
    /* the uniform block the shader loads (`check_draw` checked its rows) */
    let block_bytes = ubo_block_bytes(m.ubo);
    let block = if block_bytes > 0 {
        let b = desc.alloc(block_bytes, 16)?;
        for r in m.ubo {
            for k in 0..r.words() {
                let value = if r.halves {
                    /* a half past the row (a float, a vec3's last word) is zero */
                    let h = |c: usize| if c < r.comps { f32_to_f16(uniforms[r.dword + c]) as u32 } else { 0 };
                    h(2 * k) | h(2 * k + 1) << 16
                } else {
                    uniforms[r.dword + k]
                };
                desc.write(b, 4 * (r.word + k), &value.to_le_bytes())?;
            }
        }
        Some(b)
    } else {
        None
    };
    let entries = usize::from(block.is_some()) + usize::from(!m.sysvals.is_empty());
    let ubos = desc.alloc(Buffer::LENGTH * entries.max(1), 32)?;
    let mut ubo_count = 0u32;
    if let Some(b) = block {
        desc.write(ubos, 0, &Buffer { size: block_bytes as u32, address: b.gpu, ..Default::default() }.pack()?)?;
        ubo_count += 1;
    }
    if !m.sysvals.is_empty() {
        /* at most 16 * MAX_SYSVALS bytes (`check_draw`) */
        let size = sysvals.len as u32;
        let at = ubo_count as usize * Buffer::LENGTH;
        desc.write(ubos, at, &Buffer { size, address: sysvals.gpu, ..Default::default() }.pack()?)?;
        ubo_count += 1;
    }
    /* the push words in FAU order, each a word of the sysval block or of
     * the uniform storage (bi_opt_push_ubo picks them and
     * bi_opt_reorder_push orders them) or a promoted immediate (va_lower_constants); Mesa uploads `fau->count`
     * words (Mesa 26.2.1 pan_cmdstream.c:1580-1651). The FAU is read in
     * 64-bit slots, so an odd count is padded with a zero word here; Mesa
     * allocates only the words and the hardware reads the slot's second
     * half from whatever follows */
    let push = if m.push.is_empty() {
        None
    } else {
        let b = desc.alloc(4 * m.push.len().div_ceil(2) * 2, 16)?;
        if b.gpu >> 56 != 0 {
            return Err(FrameError::Invalid(INVALID_FAU_ADDRESS));
        }
        for (i, w) in m.push.iter().enumerate() {
            let value = match *w {
                PushWord::Sysval { sysval: s, component } => sysval(m.sysvals[s], component, v, flip, blend).to_bits(),
                PushWord::Constant(c) => c,
                /* `check_draw` checked the dwords */
                PushWord::Uniform { dword } => uniforms[dword],
                PushWord::UniformHalves { lo, hi } => {
                    let h = |k: Option<usize>| k.map_or(0, |k| f32_to_f16(uniforms[k]) as u32);
                    h(Some(lo)) | h(hi) << 16
                }
                PushWord::FbWposYTransform { component } => {
                    fb_wpos_y_transform(d.flip_y, target_height, component).to_bits()
                }
            };
            desc.write(b, 4 * i, &value.to_le_bytes())?;
        }
        Some(b)
    };
    Ok((ubos, ubo_count, sampler, sysvals, push))
}

/// A stage's texture descriptors (`panfrost_emit_texture_descriptors`, Mesa
/// 26.2.1 pan_cmdstream.c:1934-1985: one array, texture i at index i, each
/// the sampler view's descriptor) and their payloads, each texture's planes
/// one a layer (or face) and level, the layer outermost ("faces as extra
/// layers", pan_texture.c:1074-1100: `plane = layer * levels + level`, the
/// board's cubemap dump tools/mali/board-dump/cube-2026-10-20.txt;
/// `pan_tex_emit_linear_payload_entry`, pan_texture.c:1003-1062): the
/// descriptors and every texture's planes, texture 0's first, back to back
/// (Mesa keeps a view's payload in the view's own buffer).
fn texture_descriptors(desc: &mut Arena, textures: &[SampledTexture]) -> Result<(Block, Block), FrameError> {
    let mut planes: Option<Block> = None;
    let mut surfaces = [0u64; MAX_STAGE_TEXTURES];
    for (tex, at) in textures.iter().zip(surfaces.iter_mut()) {
        let count = (tex.faces * tex.levels) as usize;
        let plane = desc.alloc(count * GenericPlane::LENGTH, NullPlane::ALIGN as u64)?;
        for f in 0..tex.faces {
            for l in 0..tex.levels {
                let k = (f * tex.levels + l) as usize;
                desc.write(plane, k * GenericPlane::LENGTH, &tex.plane(f, l).pack()?)?;
            }
        }
        *at = plane.gpu;
        /* 32-byte planes on 32 bytes (`NullPlane::ALIGN`): each texture's
         * follow the one before's in the arena */
        planes = Some(planes.map_or(plane, |p| Block { len: (plane.gpu + plane.len as u64 - p.gpu) as usize, ..p }));
    }
    let planes = planes.ok_or(FrameError::Invalid(INVALID_TEXTURE))?;
    let texture = desc.alloc(Texture::LENGTH * textures.len(), Texture::ALIGN as u64)?;
    for (i, (tex, at)) in textures.iter().zip(surfaces).enumerate() {
        desc.write(texture, i * Texture::LENGTH, &tex.texture(at).pack()?)?;
    }
    Ok((texture, planes))
}

/// One Resource entry (pan_encoder.h:232-243): nothing for a count of 0.
fn resource(desc: &mut Arena, table: Block, index: usize, address: u64, count: u64) -> Result<(), FrameError> {
    if count == 0 {
        return Ok(());
    }
    let r = Resource { address, size: count * RESOURCE_DESCRIPTOR_BYTES, ..Default::default() };
    desc.write(table, index * Resource::LENGTH, &r.pack()?)?;
    Ok(())
}

/// The draw: its descriptors in Mesa's order and the RUN_IDVS register
/// block (`panfrost_update_state_3d` and the two stages' state,
/// pan_cmdstream.c:2874-2920, 2809-2871; `csf_emit_draw_state` and
/// `csf_launch_draw`, pan_csf.c:1053-1316). The frame's first draw makes
/// the tiler context its draws share (`csf_get_tiler_desc`, once a batch)
/// and starts the vertex/tiler work.
#[allow(clippy::too_many_arguments)]
fn emit_draw(
    desc: &mut Arena,
    cs: &mut Stream,
    ctx: &Context,
    t: &Target,
    d: &DrawCall,
    sbox: [u32; 4],
    open: &mut OpenFrame,
) -> Result<(), FrameError> {
    let (vs, fs) = (d.vs, d.fs);
    let tls = open.layout.tls.ok_or(FrameError::Invalid(INVALID_OPEN_FRAME))?.gpu;
    /* the depth/stencil descriptor: the CSO's part
     * (`panfrost_create_depth_stencil_state`, Mesa 26.2.1
     * pan_cmdstream.c:4222-4296: a disabled face is NEVER and KEEP with
     * masks 0, depth ALWAYS when the test is off) and the draw's
     * (`panfrost_emit_depth_stencil`, :866-902: the reference values,
     * depth culling on, [0, 1] clamp, and the depth bias). The bias is
     * GL_POLYGON_OFFSET_FILL's (`offset_tri`), the factor as it is and the
     * units doubled for a fixed-point depth buffer or none
     * (`panfrost_z_depth_offset`, :443-453: "the minimum resolvable
     * difference scaling"; the frame's depth buffer is D24S8 or absent), no
     * clamp (ES 2.0 has no glPolygonOffsetClamp); st/mesa leaves factor and
     * units 0 while no offset is on (st_atom_rasterizer.c:154-163) */
    let ds = desc.alloc(DepthStencil::LENGTH, 32)?;
    let [front, back] = d.zs.faces;
    let zsa = DepthStencil {
        front_compare_function: front.func,
        front_stencil_fail: front.fail,
        front_depth_fail: front.depth_fail,
        front_depth_pass: front.depth_pass,
        back_compare_function: back.func,
        back_stencil_fail: back.fail,
        back_depth_fail: back.depth_fail,
        back_depth_pass: back.depth_pass,
        stencil_test_enable: d.zs.stencil_test,
        front_write_mask: u32::from(front.write_mask),
        back_write_mask: u32::from(back.write_mask),
        front_value_mask: u32::from(front.value_mask),
        back_value_mask: u32::from(back.value_mask),
        front_reference_value: u32::from(front.reference),
        back_reference_value: u32::from(back.reference),
        depth_write_enable: d.zs.depth_write,
        depth_function: d.zs.depth_func,
        depth_bias_enable: d.polygon_offset.is_some(),
        depth_factor: d.polygon_offset.map_or(0.0, |o| o[0]),
        depth_units: d.polygon_offset.map_or(0.0, |o| 2.0 * o[1]),
        depth_cull_enable: true,
        depth_clamp_mode: DepthClampMode::_0_1,
        depth_source: DepthSource::FIXED_FUNCTION,
        ..Default::default()
    };
    desc.write(ds, 0, &zsa.pack()?)?;
    /* the blend descriptor of target 0 (`panfrost_emit_blend`, Mesa 26.2.1
     * pan_cmdstream.c:286-420), its state as the blend CSO derives it
     * (`panfrost_create_blend_state`, :4364-4428): fixed function always
     * (`check_draw` refused what needs a blend shader), dithering on so
     * not rounded to the target's precision. Opaque when every channel is
     * written and the equation keeps the source (`pan_blend_is_opaque`),
     * the destination loaded when the equation or a partial mask reads it
     * (`pan_blend_reads_dest`); blending off is "replace" under the mask */
    let blend = desc.alloc(Blend::LENGTH, 64)?;
    let mask = u32::from(d.colour_mask & 0xf);
    /* no channel written: no fragment shader unless alpha-to-coverage is
     * on, which "can modify occlusion query results" (`panfrost_fs_required`,
     * Mesa 26.2.1 pan_cmdstream.h:200-235, 210-213), or the shader
     * discards (`sidefx`, :205-208: "This includes use of discard, which
     * can affect the results of an occlusion query"; femtovg's
     * colour-masked stencil pass with a discarding program, gl_test case
     * 38; `check_draw` refused every other side effect), and the target's
     * blend off either way (`panfrost_emit_blend`, pan_cmdstream.c:298-307:
     * a target no channel of which is written is not enabled) */
    let writes_colour = mask != 0;
    let fs_required = writes_colour || d.alpha_to_coverage || fs.meta.can_discard;
    let blending = d.blend.filter(|_| writes_colour);
    /* a fixed-function DST_ALPHA, INV_DST_ALPHA add on a target without
     * alpha is opaque (alpha is 1), which the hardware does not see by
     * itself (pan_cmdstream.c:313-325) */
    let dst_alpha_on_no_alpha = blending.is_some_and(|b| {
        use super::blend::{Base, Factor, Func};
        b.rgb.src == Factor { base: Base::DstAlpha, invert: false }
            && b.rgb.dst == Factor { base: Base::DstAlpha, invert: true }
            && b.rgb.func == Func::Add
            && !t.format.has_alpha()
    });
    let opaque = mask == 0xf && (blending.is_none_or(|b| super::blend::is_opaque(&b)) || dst_alpha_on_no_alpha);
    let load_dest = (mask != 0xf && writes_colour) || blending.is_some_and(|b| super::blend::reads_dest(&b));
    let replace =
        BlendFunction { a: BlendOperandA::SRC, b: BlendOperandB::SRC, c: BlendOperandC::ZERO, ..Default::default() };
    let bd = if writes_colour {
        let equation = match blending {
            Some(b) => super::blend::equation(&b, mask),
            None => BlendEquation { rgb: replace, alpha: replace, color_mask: mask },
        };
        /* one constant, the first channel the factors read
         * (`pan_blend_get_constant`), when any reads it */
        let constant_mask = blending.map_or(0, |b| super::blend::constant_mask(&b));
        let blend_constant = super::blend::constant(constant_mask, &d.blend_color)
            .filter(|_| constant_mask != 0)
            .map_or(0, |c| super::blend::pack_constant(c, t.format.chan_size()));
        let mode = if opaque { BlendMode::OPAQUE } else { BlendMode::FIXED_FUNCTION };
        /* fixed function does not take the RGB1 order: RGBA instead
         * (pan_cmdstream.c:393-405) */
        let mut memory_format = t.format.blend_memory_format();
        if mode == BlendMode::FIXED_FUNCTION && memory_format & 0xff == RgbComponentOrder::RGB1.0 {
            memory_format = memory_format & !0xff | RgbComponentOrder::RGBA.0;
        }
        Blend {
            load_destination: load_dest,
            enable: true,
            round_to_fb_precision: false,
            blend_constant,
            equation,
            internal: InternalBlend {
                mode,
                fixed_function: BlendFixedFunction {
                    num_comps: 4,
                    rt: 0,
                    conversion: InternalConversion { memory_format, raw: false },
                },
                ..Default::default()
            },
            ..Default::default()
        }
    } else {
        /* "Disable blending" for a target the draw does not write
         * (`panfrost_emit_blend`, pan_cmdstream.c:298-307): enable off,
         * mode Off. Mesa leaves the equation all zero there, whose operand
         * A v10.xml does not name, and the packer refuses an unnamed zero;
         * a disabled blend's equation is not read, so it is "replace" with
         * no channel (frame_zs_tests.rs names the word) */
        Blend {
            enable: false,
            equation: BlendEquation { rgb: replace, alpha: replace, color_mask: 0 },
            internal: InternalBlend {
                mode: BlendMode::OFF,
                fixed_function: BlendFixedFunction { num_comps: 1, ..Default::default() },
                ..Default::default()
            },
            ..Default::default()
        }
    };
    let mut packed = bd.pack()?;
    /* a blend shader's descriptor (`panfrost_emit_blend`, pan_cmdstream.c:
     * 329-377 with `blend_shaders[0]` set): word 0 as for fixed function
     * but no constant, word 1 (the equation) not written, the internal
     * word mode Shader with the program's address, low 32 bits (the
     * board's Mesa: `01 02 00 00 | 0 | 0 | 00 70 FC FF` for case 8's fifth
     * frame, 0dce654a8:tools/mali/board-ref/2026-10-20-mesa-refs) */
    if let (Some(pc), true) = (d.blend_shader, writes_colour) {
        if pc % 16 != 0 || pc >> 32 != fs.binary >> 32 {
            return Err(FrameError::Invalid(INVALID_BLEND_SHADER));
        }
        let shader = Blend {
            load_destination: load_dest,
            enable: true,
            round_to_fb_precision: false,
            equation: BlendEquation { rgb: replace, alpha: replace, color_mask: 0 },
            internal: InternalBlend {
                mode: BlendMode::SHADER,
                shader: BlendShader { pc: pc as u32 },
                fixed_function: BlendFixedFunction { num_comps: 1, ..Default::default() },
            },
            ..Default::default()
        };
        packed = shader.pack()?;
        /* the equation's word is Mesa's zero, which names no operand A
         * v10.xml has (the packer refuses it): written as such */
        packed[4..8].fill(0);
    }
    desc.write(blend, 0, &packed)?;
    /* the vertex attributes, packed when the vertex elements are created
     * (`panfrost_pack_attribute`, pan_cmdstream.c:3606-3639), and one
     * buffer each (`panfrost_emit_vertex_buffers`, :869-900), attribute n
     * in buffer n. A per-instance attribute (divisor 1) is the board's
     * instsplit dump (tools/mali/board-dump/instsplit-2026-10-24.txt):
     * `1D POT Divisor`, `Frequency: Instance`, `Offset enable` false,
     * `Divisor R/E/D` 0 and `Offset` 0, its offset folded into its Buffer
     * (base + offset, the size less it) */
    let n = d.attributes.len();
    let attributes = desc.alloc(n * Attribute::LENGTH, 32)?;
    for (i, a) in d.attributes.iter().enumerate() {
        /* R32_FLOAT .. R32G32B32A32_FLOAT: R32F R001, RG32F RG01, RGB32F
         * RGB1, RGBA32F RGBA (Mesa 26.2.1 pan_format.c:451-454); v9+ spells
         * R001 and RG01 as RGB1 (pan_format.h:306-307) */
        let (format, order) = match a.components {
            1 => (Format::R32F, RgbComponentOrder::RGB1),
            2 => (Format::RG32F, RgbComponentOrder::RGB1),
            3 => (Format::RGB32F, RgbComponentOrder::RGB1),
            _ => (Format::RGBA32F, RgbComponentOrder::RGBA),
        };
        let per_instance = a.instance_divisor != 0;
        let attr = Attribute {
            table: TABLE_ATTRIBUTE_BUFFER as u32,
            frequency: if per_instance { AttributeFrequency::INSTANCE } else { AttributeFrequency::VERTEX },
            format: format.0 << 12 | order.0,
            /* at most i32::MAX (`check_draw`) */
            offset: if per_instance { 0 } else { a.offset as i32 },
            buffer_index: i as u32,
            stride: a.stride,
            attribute_type: if per_instance { AttributeType::_1D_POT_DIVISOR } else { AttributeType::_1D },
            offset_enable: !per_instance,
            ..Default::default()
        };
        desc.write(attributes, i * Attribute::LENGTH, &attr.pack()?)?;
    }
    let buffers = desc.alloc(n * Buffer::LENGTH, 32)?;
    for (i, a) in d.attributes.iter().enumerate() {
        let b = if a.instance_divisor != 0 {
            Buffer {
                address: a.buffer.gpu + u64::from(a.offset),
                size: a.buffer.size.saturating_sub(a.offset),
                ..Default::default()
            }
        } else {
            Buffer { address: a.buffer.gpu, size: a.buffer.size, ..Default::default() }
        };
        desc.write(buffers, i * Buffer::LENGTH, &b.pack()?)?;
    }

    /* the vertex stage's texture first, then its sampler and uniforms, in
     * `panfrost_update_shader_state`'s order (the board's vtxgrid dump puts
     * the texture descriptor before the sampler) */
    let vs_texture = match d.vs_texture {
        Some(tex) => Some(texture_descriptors(desc, &[tex])?),
        None => None,
    };
    let (vs_ubos, vs_ubo_count, vs_sampler, vs_sysvals, vs_push) =
        emit_stage_uniforms(desc, &vs.meta, d, t.height, d.vs_uniforms, d.vs_texture.as_slice())?;
    let fs_state = if fs_required {
        Some(emit_stage_uniforms(desc, &fs.meta, d, t.height, d.fs_uniforms, d.textures)?)
    } else {
        None
    };
    let fs_texture = match fs_state {
        Some(_) if !d.textures.is_empty() => Some(texture_descriptors(desc, d.textures)?),
        _ => None,
    };
    /* the fragment tables' counts: Mesa's `sampler_count` and
     * `sampler_view_count` (pan_cmdstream.c:1940-1942, 2029-2034), the
     * stage's bound samplers, at least the workaround sampler */
    let fs_count = d.textures.len().max(1) as u64;

    /* csf_get_tiler_desc (pan_csf.c:654-665), filled by
     * csf_emit_tiler_desc (:668-697): once a frame, by its first draw */
    let first = open.layout.tiler.is_none();
    let tiler = match open.layout.tiler {
        Some(b) => b,
        None => tiler_context(desc, ctx, t)?,
    };

    /* the resource tables (`panfrost_emit_resources`, pan_cmdstream.h:279-323) */
    let vs_res = desc.alloc(RESOURCE_TABLES * Resource::LENGTH, 64)?;
    resource(desc, vs_res, TABLE_UBO, vs_ubos.gpu, u64::from(vs_ubo_count))?;
    resource(desc, vs_res, TABLE_ATTRIBUTE, attributes.gpu, n as u64)?;
    resource(desc, vs_res, TABLE_ATTRIBUTE_BUFFER, buffers.gpu, n as u64)?;
    resource(desc, vs_res, TABLE_SAMPLER, vs_sampler.gpu, 1)?;
    if let Some((texture, _)) = vs_texture {
        resource(desc, vs_res, TABLE_TEXTURE, texture.gpu, 1)?;
    }
    let fs_res = match fs_state {
        Some((ubos, count, sampler, _, _)) => {
            let r = desc.alloc(RESOURCE_TABLES * Resource::LENGTH, 64)?;
            resource(desc, r, TABLE_UBO, ubos.gpu, u64::from(count))?;
            resource(desc, r, TABLE_SAMPLER, sampler.gpu, fs_count)?;
            if let Some((texture, _)) = fs_texture {
                resource(desc, r, TABLE_TEXTURE, texture.gpu, fs_count)?;
            }
            Some(r)
        }
        None => None,
    };

    open.layout = Layout {
        depth_stencil: Some(ds),
        blend: Some(blend),
        attributes: Some(attributes),
        buffers: Some(buffers),
        vs_sampler: Some(vs_sampler),
        vs_sysvals: Some(vs_sysvals),
        vs_ubos: Some(vs_ubos),
        vs_push,
        vs_texture: vs_texture.map(|t| t.0),
        vs_plane: vs_texture.map(|t| t.1),
        fs_sampler: fs_state.map(|f| f.2),
        fs_texture: fs_texture.map(|t| t.0),
        fs_plane: fs_texture.map(|t| t.1),
        fs_sysvals: fs_state.map(|f| f.3),
        fs_ubos: fs_state.map(|f| f.0),
        fs_push: fs_state.and_then(|f| f.4),
        tiler: Some(tiler),
        vs_resources: Some(vs_res),
        fs_resources: fs_res,
        ..open.layout
    };

    /* a stage's SRT register: the table and its count in the low bits
     * (pan_cmdstream.h:322); its FAU register: see `fau` */
    let srt = |res: Block| tagged(res.gpu, RESOURCE_TABLES as u64, INVALID_SRT_ALIGN);

    /* csf_emit_draw_state (pan_csf.c:1053-1270); the vertex/tiler work
     * starts once, before the frame's first draw (the board's trace of two
     * draws, 0dce654a8:tools/mali/board-ref/2026-10-19-frame-depth/mesa/mesa-depth.pandecode
     * lines 49 and 567) */
    if first {
        cs.push(Instr::HeapOperation(CsHeapOperation {
            operation: CsHeapOperationEnum::VERTEX_TILER_STARTED,
            ..Default::default()
        }))?;
    }
    cs.move64(reg::SRT_VERTEX, srt(vs_res)?)?;
    cs.move64(reg::FAU_VERTEX, fau(vs_push, vs.meta.push.len()))?;
    /* `panfrost_get_position_shader` (pan_cmdstream.h:239-250) */
    let points = d.primitive == Topology::Points;
    cs.move64(reg::SPD_VERTEX, if points { vs.points_spd } else { vs.spd })?;
    /* no fragment shader: its three registers 0 (Mesa 26.2.1
     * pan_csf.c:1457-1464) */
    match (fs_res, fs_state) {
        (Some(res), Some(st)) => {
            cs.move64(reg::SRT_FRAGMENT, srt(res)?)?;
            cs.move64(reg::FAU_FRAGMENT, fau(st.4, fs.meta.push.len()))?;
            cs.move64(reg::SPD_FRAGMENT, fs.spd)?;
        }
        _ => {
            cs.move64(reg::SRT_FRAGMENT, 0)?;
            cs.move64(reg::FAU_FRAGMENT, 0)?;
            cs.move64(reg::SPD_FRAGMENT, 0)?;
        }
    }
    /* the varying program, run when the fragment shader is (Mesa 26.2.1
     * pan_csf.c:1443, 1470-1473) */
    let secondary = vs.varying_spd.filter(|_| fs_required);
    if let Some(spd) = secondary {
        cs.move64(reg::VARY_SPD, spd)?;
    }
    /* Mesa 25.0.7 sets TSD_0 and TSD_3 (pan_csf.c:1090-1091); 26.2.1
     * sets only TSD_0 for arch 10 (its pan_csf.c:1475) */
    cs.move64(reg::TSD_0, tls)?;
    cs.move64(reg::TSD_3, tls)?;
    cs.move32(reg::GLOBAL_ATTRIBUTE_OFFSET, 0)?;
    cs.move32(reg::INSTANCE_OFFSET, 0)?;
    cs.move32(reg::DCD2, 0)?;
    cs.move64(reg::TILER_CTX, tiler.gpu)?;
    let scissor = Scissor {
        scissor_minimum_x: sbox[0],
        scissor_minimum_y: sbox[1],
        scissor_maximum_x: sbox[2] - 1,
        scissor_maximum_y: sbox[3] - 1,
    };
    cs.move64(reg::SCISSOR_BOX, pack_u64(scissor.pack()?))?;
    /* util_viewport_zmin_zmax for GL's depth (clip_halfz off) */
    let v = &d.viewport;
    cs.move32(reg::LOW_DEPTH_CLAMP, v.near.min(v.far).to_bits())?;
    cs.move32(reg::HIGH_DEPTH_CLAMP, v.near.max(v.far).to_bits())?;
    /* the occlusion counter of a counted draw, here as Mesa writes it
     * (`panfrost_occlusion_query_active`, Mesa 26.2.1 pan_csf.c:1500-1504);
     * an uncounted draw leaves the register as it was, its DCD0 flag off */
    if let Some(oq) = d.occlusion_query {
        if !oq.is_multiple_of(8) {
            return Err(FrameError::Invalid(INVALID_OQ_ALIGN));
        }
        cs.move64(reg::OQ, oq)?;
    }
    /* the bytes of one vertex's generic varyings (Mesa 26.2.1
     * pan_csf.c:1506-1507, `generic_size_B`) */
    cs.move32(reg::VARY_SIZE, vs.meta.varying_bytes)?;
    cs.move64(reg::BLEND_DESC, tagged(blend.gpu, 1, INVALID_BLEND_ALIGN)?)?;
    cs.move64(reg::ZSD, ds.gpu)?;
    /* the primitive flags: rotating allowed unless the
     * draw makes lines (no flat shading; `allow_rotating_primitives`,
     * pan_cmdstream.h:312-317; the board's Mesa writes r56 0x430000 for
     * lines, strips and loops, 0x438000 for triangles), the basic position FIFO (pan_csf.c:1123-1140), and
     * GL's depth clipping on at both planes, written here and not left to
     * the descriptor's defaults. Mesa 26.2.1 takes both bits from the
     * rasterizer state, `depth_clip_near` and `depth_clip_far`
     * (pan_csf.c:1525-1526), which GL leaves on unless GL_DEPTH_CLAMP-style
     * clamping is asked for (ES 2.0 has none) */
    /* `panfrost_writes_point_size` (pan_context.c:83-89): a points draw
     * of a shader that writes gl_PointSize reads the point size array, an
     * fp16 per vertex in the extended position FIFO (pan_csf.c:1520-1535;
     * the board's Mesa writes r56 0x839000, d60 0,
     * 0dce654a8:tools/mali/board-ref/2026-10-19-points) */
    let psiz = points && vs.meta.no_psiz_offset.is_some();
    let flags = PrimitiveFlags {
        allow_rotating_primitives: !d.primitive.lines(),
        point_size_array_format: if psiz { PointSizeArrayFormat::FP16 } else { PointSizeArrayFormat::NONE },
        position_fifo_format: if psiz { FifoFormat::EXTENDED } else { FifoFormat::BASIC },
        low_depth_cull: true,
        high_depth_cull: true,
        ..Default::default()
    };
    cs.move32(reg::TILER_FLAGS, pack_u32(flags.pack()?))?;
    /* DCD flags 0 for an FS that only writes colour, blending off
     * (Mesa 26.2.1 pan_csf.c:1555-1620). Early pixel kill and ZS update
     * (`pan_earlyzs_get`): nothing makes them late but alpha-to-coverage
     * (below; no discard, no depth or stencil from the shader, `check_draw`), so
     * "weak early" when the depth/stencil test always passes and "force
     * early" otherwise (`best_early_mode`, Mesa 26.2.1
     * src/panfrost/lib/pan_earlyzs.c:30-37). Forward pixel kill only when
     * every channel is written: a colour mask that leaves one out reads
     * the destination (`pan_allow_forward_pixel_to_kill`,
     * pan_cmdstream.h:98-112). The overdraw flags of an enabled target
     * (`panfrost_overdraw_alpha`, :124-139). The front face as st/mesa
     * gives it for an FBO: GL's winding inverted, so GL's default CCW is
     * `false` (st_atom_rasterizer.c:77, 90-93); it decides which stencil
     * face a triangle tests and which face culling takes as the front. Aligned line ends
     * (`!line_rectangular`). A draw with no fragment shader is force
     * early with forward pixel kill on (pan_csf.c:1621-1637: "These
     * operations need to be FORCE to benefit from the depth-only pass
     * optimizations"). Alpha-to-coverage makes the ZS update force late
     * and leaves the kill as it was (`analyze`, pan_earlyzs.c:41-50: "we
     * observe this requirement on Valhall"), turns forward pixel kill off
     * and sets `Shader Modifies Coverage` with `Alpha To Coverage` ("Unlike
     * Bifrost, alpha-to-coverage must be included in this identically-named
     * flag", pan_csf.c:1607-1614); `Alpha To Coverage Invert` is never set
     * by the gallium driver (the board's Mesa: r57 0x5901BA for case 198's
     * draws, 0dce654a8:tools/mali/board-ref/2026-10-23-a2c). A discarding shader is
     * a late coverage update too (`late_coverage`, pan_earlyzs.c:57-62:
     * "Discards are implemented as coverage mask updates"), which delays
     * the ZS update only when the draw writes depth or stencil or an
     * occlusion query counts it (:64-76; the kill never, :78-81): forward
     * pixel kill off (`can_fpk` false, pan_compiler.c:269-271), `Shader
     * Modifies Coverage` set (pan_csf.c:1607-1612); docs/notes/
     * mali-discard.md section 2.2, pinned to the board's Mesa in
     * 0dce654a8:tools/mali/board-ref/2026-10-23-s6-discard (frame_discard_tests.rs). */
    let a2c = d.alpha_to_coverage;
    let discard = fs.meta.can_discard;
    let late_coverage = a2c || discard;
    let late_update = a2c || (late_coverage && (d.zs.writes_zs() || d.occlusion_query.is_some()));
    let early = if fs_required && d.zs.always_passes() { PixelKill::WEAK_EARLY } else { PixelKill::FORCE_EARLY };
    let culls = d.primitive.polygon() && d.cull != Cull::None;
    let cull_front = culls && ((d.cull == Cull::Cw) == d.front_cw);
    let cull_back = culls && !cull_front;
    let dcd0 = DcdFlags0 {
        /* `pan_allow_forward_pixel_to_kill` (pan_cmdstream.h:98-111): not
         * while the blend reads the destination, alpha-to-coverage is on or
         * the shader discards */
        allow_forward_pixel_to_kill: (!load_dest && !late_coverage) || !fs_required,
        allow_forward_pixel_to_be_killed: true,
        pixel_kill_operation: early,
        zs_update_operation: if late_update { PixelKill::FORCE_LATE } else { early },
        shader_modifies_coverage: late_coverage,
        alpha_to_coverage: a2c,
        overdraw_alpha0: true,
        overdraw_alpha1: true,
        /* rast->multisample, which ES 2.0 cannot turn off for a 4x target,
         * and aligned line ends unless lines are rectangular, which st/mesa
         * makes them when multisampling (pan_csf.c:1561, 1572;
         * st_atom_rasterizer.c:254) */
        multisample_enable: t.samples > 1,
        /* `rast->multisample && (min_samples > 1 ||
         * valhall_has_blend_shader)` (pan_csf.c:1568-1570): a blend shader
         * of a 4x target runs once a sample, each reading and writing its
         * own (r61's sample id; the board's Mesa: r57 0x809AA for case
         * 211's blended draw, docs/notes/mali-blend-shaders.md section 3) */
        evaluate_per_sample: t.samples > 1 && writes_colour && d.blend_shader.is_some(),
        aligned_line_ends: t.samples == 1,
        /* set before Mesa's `fs_required` branch, so a draw without a
         * fragment shader counts too (pan_csf.c:1577-1583); the early-z
         * modes move with it only for a shader that discards (above:
         * `writes_zs || has_oq`, pan_earlyzs.c:71) */
        occlusion_query: if d.occlusion_query.is_some() { OcclusionMode::PREDICATE } else { OcclusionMode::DISABLED },
        /* gallium's front_ccw: GL's for a window, inverted for an FBO
         * (st_atom_rasterizer.c:83-93); the board's Mesa writes r57 bit 16
         * set for a CCW triangle in a pbuffer, clear in an FBO */
        front_face_ccw: d.front_cw != d.flip_y,
        /* `cull_front_face = polygon && (cull_face & FRONT)`, `cull_back_face`
         * likewise (pan_csf.c:1544-1559): the culled winding is the front
         * face when it is the `glFrontFace` winding, else the back face;
         * points and lines are drawn with every face culled */
        cull_front_face: cull_front,
        cull_back_face: cull_back,
        ..Default::default()
    };
    cs.move32(reg::DCD0, pack_u32(dcd0.pack()?))?;
    /* the targets the shader writes, none without one (pan_csf.c:1641-1648) */
    let rt_mask = if fs_required { fs.meta.colour_outputs & 1 } else { 0 };
    /* `rast->multisample ? ctx->sample_mask : 0xFFFF` (pan_csf.c:1639).
     * st/mesa's mask is all ones cut by glSampleCoverage, its invert over
     * all 16 bits (the board's Mesa: coverage off 0xFFFF, 0.5 0x0003, 0.5
     * inverted 0xFFFC). The draw's mask has the 4 samples only, so the
     * bits above them are set: Mesa's bytes but for a non-inverted
     * coverage (0xFFF3 for its 0x0003), the same samples on a 4x target */
    let sample_mask = if t.samples > 1 { 0xfff0 | u32::from(d.sample_mask & 0xf) } else { 0xffff };
    let dcd1 = DcdFlags1 { sample_mask, render_target_mask: rt_mask };
    cs.move32(reg::DCD1, pack_u32(dcd1.pack()?))?;
    /* `panfrost_emit_primitive_size` (pan_cmdstream.h:144-157): the size
     * array's address, 0 on CSF (pan_csf.c:1656), or the fixed size */
    let size = match (psiz, points) {
        (true, _) => PrimitiveSize::default(),
        (false, true) => PrimitiveSize { fixed_sized: FIXED_POINT_SIZE, ..Default::default() },
        (false, false) => PrimitiveSize { fixed_sized: d.line_width, ..Default::default() },
    };
    cs.move64(reg::PRIMITIVE_SIZE, pack_u64(size.pack()?))?;
    /* csf_launch_draw (pan_csf.c:1289-1316); Mesa 26.2.1 writes
     * INSTANCE_OFFSET here a second time (its pan_csf.c:1708) */
    cs.move32(reg::INDEX_COUNT, d.vertex_count)?;
    cs.move32(reg::INSTANCE_COUNT, d.instance_count.max(1))?;
    cs.move32(reg::INDEX_OFFSET, 0)?;
    /* an indexed draw's VERTEX_OFFSET is the index bias, 0 in ES 2.0
     * (`check_draw`), its INDEX_BUFFER_SIZE the indices' bytes, and its
     * INDEX_BUFFER the indices (pan_csf.c:1512-1513, 1713-1720); the board's
     * Mesa writes d54, r39 = 2 * count and the override's index type U16
     * (0dce654a8:tools/mali/board-ref/2026-10-19-indexed) */
    match d.indices {
        Some(i) => {
            cs.move64(reg::INDEX_BUFFER, i.gpu)?;
            cs.move32(reg::VERTEX_OFFSET, 0)?;
            let bytes = d.vertex_count.checked_mul(2).ok_or(FrameError::Invalid(INVALID_INDEX_BYTES))?;
            cs.move32(reg::INDEX_BUFFER_SIZE, bytes)?;
        }
        None => {
            cs.move32(reg::VERTEX_OFFSET, d.first_vertex)?;
            cs.move32(reg::INDEX_BUFFER_SIZE, 0)?;
        }
    }
    /* the flags override, packed with no defaults: only the draw mode, the
     * index type and the secondary shader (pan_csf.c:1263-1269) */
    let over = PrimitiveFlags {
        draw_mode: d.primitive.draw_mode(),
        index_type: if d.indices.is_some() { IndexType::UINT16 } else { IndexType::NONE },
        secondary_shader: secondary.is_some(),
        low_depth_cull: false,
        high_depth_cull: false,
        ..Default::default()
    };
    /* cs_run_idvs(b, flags, false, true, cs_shader_res_sel(0, 0, 1, 0),
     * cs_shader_res_sel(2, 2, 2, 0), drawid): the varying stage's selects
     * 0, the fragment SRT from the fragment registers, no draw ID */
    cs.push(Instr::RunIdvs(CsRunIdvs {
        flags_override: pack_u32(over.pack()?),
        malloc_enable: true,
        fragment_srt_select: true,
        ..Default::default()
    }))?;
    Ok(())
}

/// The frame's tiler context (`csf_emit_tiler_desc`, pan_csf.c:668-697).
fn tiler_context(desc: &mut Arena, ctx: &Context, t: &Target) -> Result<Block, FrameError> {
    let tiler = desc.alloc(TilerContext::LENGTH, 64)?;
    let max_wh = t.width.max(t.height);
    /* pan_select_tiler_hierarchy_mask (pan_util.h:83-101), Mesa 25.0.7's
     * rule: all `max_levels` levels on, shifted up when the target needs more
     * than that, and the first level dropped when the effective tile is
     * larger than 16x16. Only max_levels 8 (the board's TILER_FEATURES 0x809)
     * is built (`check_draw`), and this rule is the one that reproduces the
     * trace's mask, so it is the one used.
     *
     * Mesa 26.2.1 chooses differently (read in its source, pan_desc.c:
     * 1746-1800): a lookup table for max_levels below 8, the levels below
     * the effective tile size dropped by a loop, and further levels dropped
     * until the bins fit the heap's budget, `chunk_size - 64` (pan_csf.c:
     * 981-983). For this 64x64 target at 8 levels, reading the code gives
     * the same mask as the rule below (0xfe); that is derived, not measured
     * on the board, and a larger target or a smaller heap can differ. */
    let last_bit = 32 - max_wh.div_ceil(16).leading_zeros();
    let mut hierarchy_mask = (1u32 << ctx.tiler_max_levels) - 1;
    if last_bit > ctx.tiler_max_levels {
        hierarchy_mask <<= last_bit - ctx.tiler_max_levels;
    }
    /* "For effective tile size larger than 16x16, disable first level" */
    if tile_size(t).0 > 16 * 16 {
        hierarchy_mask &= !1;
    }
    let tc = TilerContext {
        hierarchy_mask,
        fb_width: t.width,
        fb_height: t.height,
        heap: ctx.heap,
        sample_pattern: t.sample_pattern(),
        first_provoking_vertex: false,
        geometry_buffer: ctx.geometry_buffer,
        geometry_buffer_size: ctx.geometry_buffer_size,
        ..Default::default()
    };
    desc.write(tiler, 0, &tc.pack()?)?;
    Ok(tiler)
}
