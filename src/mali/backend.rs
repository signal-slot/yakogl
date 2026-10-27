// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The Mali-G610 backend table: the GL core's capability answers, the frames
//! its draws and clears build, and their submits through the OSAL
//! (milestones L2 and L3 of driver/docs/nanopc-t6-mali.md).
//!
//! `init` asks the OSAL for the device's counts and the tiler heap
//! ([`crate::osal::mali`]), takes its buffers (each a CPU mapping at a GPU
//! VA), builds the per-context objects once (the heap descriptor and the
//! sample positions, [`frame::build_context`]), runs the queue-init stream
//! (`HEAP_SET`) and compiles the clear program ([`CLEAR_VS`], [`CLEAR_FS`]).
//!
//! A frame stays open from the draw or clear that opens it to the GL core's
//! `flush` (or `finish`, or a draw into another target), as Mesa's batch
//! does: every draw into the same target is added to it
//! (src/mali/frame.rs: [`frame::begin_frame`], [`frame::add_draw`],
//! [`frame::end_frame`]), so its draws are depth-tested against each other
//! in the tile buffer. The frame is built in place in one of two frame
//! slots, every block at its buffer's GPU VA, and submitted when it ends. A
//! slot is built into again only after its last frame's fence: the GPU may
//! still read it until then. A wait that times out or fails leaves the slot
//! in flight, and a frame that needs it is refused (`-EIO`,
//! [`SLOT_STILL_IN_FLIGHT`], counted as lost work) until the OSAL's
//! `completed` reaches that fence. The shader programs are the Valhall back
//! end's (src/shaderc/online/mali/), taken from the draw's shader records
//! ([`super::program`]).
//!
//! Clears, as Mesa does them (`panfrost_clear`, and st/mesa's
//! `clear_with_quad`, Mesa 26.2.1 src/mesa/state_tracker/st_cb_clear.c:
//! 194-309, 406-521): a whole colour clear opens a frame and is its tile
//! buffer's start value. A whole depth or stencil clear is the frame's
//! start value while no draw of the frame has used the attachment, and is
//! kept as the plane's value when no frame is open
//! ([`crate::ds_retention::Plane`]). Any other clear (masked, scissored, or
//! after a draw) is drawn: a quad of the clear program with Mesa's state.
//! A frame that opens without a whole colour clear starts from the
//! target's old image: its pre-frame draw runs the colour preload program
//! (compiled at `init`, src/shaderc/online/mali/preload.rs), which reads
//! the target back into the tile buffer (`frame::FrameShader`).
//!
//! Depth and stencil: each attachment pair has a linear D24S8 buffer the
//! frame writes back to ([`frame::ZsAttachment`]); each GL object's plane
//! says what it holds, and a live plane which pair's buffer holds it. A
//! frame starts each plane from a clear value, from nothing GL keeps, or,
//! when a past frame drew it, from the buffer: the frame's first draw that
//! uses the attachment starts the Z/S preload (pre frame 1, one of three
//! programs for the depth, the stencil or both). A plane held in another
//! pair's buffer is refused ([`NOT_BUILT_ZS_ELSEWHERE`]).
//!
//! What the frame builder does not build is refused before any of that,
//! each cause with its own line (the `NOT_BUILT_*` of `frame.rs` and of this
//! file), counted by [`backend::note_draw_refused`] and returned as
//! `-ENOTSUP`, which the GL core reports as a backend gap
//! (`GL_INVALID_OPERATION`), not as lost work. A refused draw leaves the open
//! frame as it was. A draw whose data does not fit grows the frame's data
//! pool ([`grow_data`]); one that does not fit the other arenas ends the
//! frame and goes on in the next. A draw that fits no frame is `-ENOMEM`, a
//! value a packer refused `-EINVAL`, a submit or a wait that failed `-EIO`
//! (or the wait's own error), and a backend with no Mali OSAL `-ENODEV`.
//!
//! The capability answers say what this backend does today: it samples no
//! texel format, renders no integer or depth-only target and writes one
//! colour target. Where the GL core needs a limit that is the hardware's,
//! the value is Mesa's for this GPU, cited.

use super::arena::{Arena, ArenaError, Pool, PoolTop, POOL_CHUNKS};
use super::blend;
use super::cs::Stream;
use super::frame::{self, DepthStencilState, FrameError, ShaderMeta, StencilFace};
use super::genxml::v10::{ClumpFormat, Format, Func, MipmapMode, RgbComponentOrder, StencilOp, WrapMode};
use super::program::{self, StageMeta};
use crate::backend::{self, gles_attribf_bit, GlesAttrib, GlesDraw, YakoglBackend, GLES_ATTRIBF_FLOAT32};
use crate::ds_retention::{self, Plane};
use crate::osal::mali::{OsalMaliGpu, OsalMaliHeap, OsalMaliKick, OSAL_QUERY_MALI_GPU, OSAL_QUERY_MALI_TILER_HEAP};
use crate::osal::{self, OsalGpuMem, OSAL_MEM_DATA, OSAL_MEM_GPU_EXEC};
use crate::shaderc::online::mali::MaliShader;
use crate::shaderc::{ShcShader, SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX};
use crate::util::{Global, EBUSY, EINVAL, EIO, ENODEV, ENOMEM, ENOTSUP};
use core::ffi::c_void;
use core::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};

/// A framebuffer with no colour attachment: not drawn (nothing is), so the
/// GL core reports it GL_FRAMEBUFFER_UNSUPPORTED (`backend::DEPTH_ONLY_TARGETS`).
pub const DRAWS_WITHOUT_COLOUR: bool = false;

/// The texel formats this backend samples (`backend::sampled_texfmts`):
/// none, until the texture descriptor (`genxml::v10::Texture`, `Plane`) is
/// written by a draw.
pub const SAMPLED_TEXFMTS: u32 = 0;

/// The integer colour surfaces it renders (`backend::INTEGER_RT_FORMATS`):
/// none.
pub const INTEGER_RT_FORMATS: u32 = 0;

/// Colour targets one draw writes (`backend::DRAW_TARGETS`): one, the
/// least the GL core takes. The hardware has eight
/// (`MALI_BIFROST_BLEND_MAX_RT`, v10.xml:1034; Mesa 26.2.1's `pan_get_max_cbufs`,
/// src/gallium/drivers/panfrost/pan_screen.c:700-701); none is written yet.
pub const DRAW_TARGETS: usize = 1;

/// Vertex-stage texture units (`backend::GLES_VS_TEX_UNITS`): 16, the most
/// the GL core binds; a vertex shader reads one of them (its look-up's
/// [`MaliShader::sampler`], bound in the vertex resource table). Mesa
/// 26.2.1 gives every stage PIPE_MAX_SAMPLERS samplers
/// (src/gallium/drivers/panfrost/pan_screen.c:593;
/// src/gallium/include/pipe/p_state.h:68, 32).
pub const VS_TEX_UNITS: usize = 16;

/// The largest point size (`gl::state` POINT_SIZE_MAX): Mesa's
/// `max_point_size` for this GPU, 4095.9375 (Mesa 26.2.1,
/// src/gallium/drivers/panfrost/pan_screen.c:913-916, one assignment chain
/// that ends in `max_point_size` and `max_point_size_aa`; the
/// Primitive Size descriptor's float, v10.xml:2166).
pub const POINT_SIZE_MAX: f32 = 4095.9375;

/// The log line of an `init` whose OSAL cannot run a Mali queue, and of a
/// draw before `init` succeeded.
pub const NO_MALI_OSAL: &str = "mali: the OSAL has no Mali queue (gpu_alloc, submit, wait, completed and the OSAL_QUERY_MALI_* answers: the panthor development OSAL tools/mali/panthor, or the kernel side driver/rust/src/gpu/mali, milestone Z1)";
/// The OSAL answered the device query with a register count that leaves no
/// user register, or a tiler heap descriptor the frame cannot name.
pub const INVALID_OSAL_ANSWER: &str = "mali: the OSAL's device or tiler heap answer is outside what the frame can use";
/// A buffer the OSAL gave without a CPU mapping, or one whose mapping is not
/// 8-byte aligned (the command stream is written as 64-bit words).
pub const INVALID_OSAL_BUFFER: &str = "mali: an OSAL buffer has no 8-byte-aligned CPU mapping";
/// A draw whose shaders carry no Mali program: a record of another origin
/// (the GL core's internal programs, the offline table), not the Valhall
/// back end's.
pub const NO_MALI_PROGRAM: &str =
    "mali: the draw's shaders carry no Mali program (a record the Valhall back end did not make)";
/// A fence wait ran out of time ([`WAIT_US`]): the frame's slot stays in
/// flight ([`SLOT_STILL_IN_FLIGHT`]).
pub const WAIT_TIMED_OUT: &str = "mali: a frame's fence wait timed out; its slot stays in flight";
/// The OSAL's wait failed (a fault, or the OSAL lost the fence): the slot
/// stays in flight until the OSAL reports the fence complete.
pub const WAIT_FAILED: &str = "mali: a frame's fence wait failed; its slot stays in flight";
/// A frame needs a frame slot whose last frame's wait timed out or failed,
/// and the OSAL has not reported that frame complete since: the GPU may
/// still read the slot, so it is not written. Counted as lost work
/// (`yakogl_gpu_work_lost`), every draw.
pub const SLOT_STILL_IN_FLIGHT: &str =
    "mali: a draw needs a frame slot whose last frame's wait failed and which the OSAL has not reported complete; the draw is refused";
/// An open frame whose end the builder refused or whose submit failed: its
/// draws were taken and will never run. Counted as lost work.
pub const FRAME_LOST: &str = "mali: an open frame could not be ended or submitted; its draws are lost";
/// The draw names no colour surface. The contract of `backend::draw` makes
/// that `-EINVAL` (a core bug), not a gap.
pub const INVALID_NO_TARGET: &str = "mali: the draw has no colour target (target_gpu is 0)";
pub const NOT_BUILT_TARGET_COUNT: &str = "mali: a draw with more than one colour target is not built yet";
/// glDrawBuffers({GL_NONE, ATT1}): the list starts past fragment output 0.
pub const NOT_BUILT_OUTPUT_SHIFT: &str = "mali: a draw whose first draw buffer is GL_NONE is not built yet";
pub const NOT_BUILT_TARGET_FORMAT: &str =
    "mali: a colour target other than RGBA8888, XRGB8888, RGB565 or RGBA16F is not built yet";
/// The row pitch in bytes does not fit the Render Target descriptor's
/// 32-bit row stride.
pub const INVALID_ROW_STRIDE: &str = "mali: the target's row pitch in bytes does not fit 32 bits";
/* The draw state the frame builder does not build, one line per cause
 * (`check`, in this order). */
/// A blend factor or equation code outside `shc_blend_factor` /
/// `shc_blend_func`: the GL core's defect.
pub const INVALID_BLEND_CODE: &str = "mali: a blend factor or equation code that is not the core's";
/// A cull code outside `GLES_CULL_*`: the GL core's defect.
pub const INVALID_CULL: &str = "mali: the draw's cull winding is not the core's";
pub const INVALID_PRIMITIVE: &str = "mali: the draw's primitive is not a GL primitive";
/// The GL core hands u16 indices only (`GlesDraw::index_size`), `index_bytes`
/// of them staged; anything else is the core's defect.
pub const INVALID_INDEX_SIZE: &str =
    "mali: an indexed draw whose indices are not the core's u16 (index_size 2, 2 bytes each)";
/// `GlesDraw::occlusion_query` past the handles the GL core hands
/// (1 .. `GLES_QUERY_HANDLES`): the core's defect.
pub const INVALID_QUERY_HANDLE: &str =
    "mali: a draw counted on an occlusion query handle outside 1 .. GLES_QUERY_HANDLES";
/// A depth/stencil attachment smaller than the colour target: the frame
/// writes it back over the whole target (ES 3.0's render area is the
/// intersection; the builder has no such area).
pub const NOT_BUILT_ZS_SIZE: &str = "mali: a depth/stencil attachment smaller than the colour target is not built yet";
/// A frame would use a depth/stencil attachment one of whose planes a past
/// frame drew into and nothing cleared since, and the depth/stencil preload
/// program did not build at `init` (its line was logged then).
pub const NOT_BUILT_ZS_PRELOAD: &str =
    "mali: a frame that uses depth or stencil drawn by an earlier frame and not cleared since, and no Z/S preload program (see its log line at init)";
/// A plane a past frame drew holds its contents in the buffer of another
/// attachment pair (the object was drawn in another framebuffer): this
/// pair's buffer does not hold them, so a preload of it would read the
/// wrong memory.
pub const NOT_BUILT_ZS_ELSEWHERE: &str =
    "mali: a depth or stencil object drawn in another framebuffer's depth/stencil pair is not built yet";
/// The clear program did not compile or link at `init` (its line was
/// logged then): a clear that needs the quad is refused.
pub const NOT_BUILT_CLEAR_PROGRAM: &str =
    "mali: a clear that needs the clear quad, whose program did not build at init (see its log line)";
pub const NOT_BUILT_ATTRIBUTE: &str =
    "mali: a vertex attribute other than float32 data (staged by the GL core, or in a buffer object) is not built yet";
/// A buffer object above the Buffer descriptor's 32-bit size.
pub const NOT_BUILT_ATTRIBUTE_BUFFER_SIZE: &str =
    "mali: a vertex attribute in a buffer object of 4 GiB or more (the Buffer descriptor's size is 32 bits) is not built yet";
/// The vertex shader reads an attribute the draw does not carry, or its
/// attribute records are not 0..n (the frame's attribute table is indexed by
/// record).
pub const NOT_BUILT_ATTRIBUTE_RECORDS: &str =
    "mali: a vertex shader whose attribute records are not 0..n of the draw's attributes is not built yet";

pub const NOT_BUILT_TEXTURE_FORMAT: &str =
    "mali: a texture stored other than RGBA8, sRGB8_A8, R32F or ETC2 (RGB8, RGBA8, RGB8A1) is not built yet";
pub const NOT_BUILT_TEXTURE_TYPE: &str =
    "mali: a texture other than one 2D image or a cube map of six faces (an array, a 3D texture) is not built yet";
/// A mip chain of a non-power-of-two image: the GL core lays none out for
/// this backend (`npot_mip_chains`), so one would not be where
/// [`chain_level`] looks.
pub const NOT_BUILT_TEXTURE_NPOT_CHAIN: &str = "mali: a mip chain of a non-power-of-two texture is not built yet";
/// An imported image's texture is copied (its rows are not
/// [`TEXTURE_ROW_ALIGN`] bytes) and the GL core gave no CPU view of it, or
/// one shorter than its rows.
pub const INVALID_TEXTURE_STORAGE: &str = "mali: a texture to copy without a CPU view of its rows";
/// The imported image to copy is the open frame's own target: sampling
/// what the draw renders is undefined in GL (ES 2.0 4.4.3, a feedback loop).
pub const NOT_BUILT_TEXTURE_FEEDBACK: &str =
    "mali: a draw that samples the texture its own frame renders into is not built yet";
/// A frame starts from the old image and the preload program did not build
/// at init (its log line says why).
pub const NOT_BUILT_PRELOAD_PROGRAM: &str =
    "mali: a frame without a colour clear, and no preload program to draw the old image back, is not built yet";

/// The texture unit reads a linear row stride that is a multiple of 64
/// bytes: measured on the NanoPC-T6 (0dce654a8:tools/mali/board-ref/2026-10-19-texture,
/// `stride/`: W x 4 textures of 64- and 128-byte rows sample right, of 32,
/// 80 and 96 read back as zeros, with no fault). Mesa lays its linear images
/// out so (`pan_linear_or_tiled_row_align_req` is 64 for these formats on
/// v7+, Mesa 26.2.1 src/panfrost/lib/pan_layout.h:164-230) and refuses an
/// imported one that is not (pan_mod.c:741-746). The GL core lays every
/// row it allocates out so (`YakoglBackend::linear_row_align`). An image
/// imported at its producer's pitch (a native buffer) may not be: this
/// backend does not refuse it as Mesa does, it samples a copy with
/// 64-byte rows from the frame's data arena, and counts every copy
/// (`MaliState::row_copies`, printed at shutdown).
pub const TEXTURE_ROW_ALIGN: u32 = 64;

/// The GL core's mip chain layout for this backend
/// (`YakoglBackend::mip_level_align`): one row pitch, level 0's, every level
/// on a 64-byte boundary, the GC7000's layout, at a level 0 pitch of whole
/// [`TEXTURE_ROW_ALIGN`] bytes: every level is sampled in place (Mesa's
/// linear levels each start on 64 bytes, `pan_image_slice_align`,
/// pan_layout.h:155-161).
pub const MIP_LEVEL_ALIGN: u32 = 64;

/// Initialise the GPU through the OSAL (see the module documentation).
pub extern "C" fn mali_init() -> i32 {
    // SAFETY: the backend entry points run one at a time (the GL core's
    // single-threaded contract), and each takes the state once.
    let g = unsafe { STATE.get() };
    if g.up {
        return 0;
    }
    match bring_up(g) {
        Ok(()) => {
            g.up = true;
            0
        }
        Err(rc) => {
            release(g);
            rc
        }
    }
}

/// Submit the open frame, wait for every frame, then give every buffer back.
pub extern "C" fn mali_shutdown() {
    // SAFETY: as in `mali_init`.
    let g = unsafe { STATE.get() };
    if g.up {
        let _ = end_open(g);
        let _ = wait_all(g);
    }
    release(g);
    g.up = false;
}

/// A shader as the frame builder takes it: the code and its meta.
#[derive(Clone, Copy, Debug)]
pub struct ShaderInput<'a> {
    pub code: &'a [u8],
    pub meta: ShaderMeta<'a>,
}

/* The frame memory: one slot holds the open frame while the other may still
 * be read by the GPU. A frame holds every draw from the clear that opens it
 * to the flush, each with its own descriptors, vertex data and program
 * copies, so the sizes hold a few hundred of the measured draws (about
 * 1.6 KiB of descriptors, 0.7 KiB of program and 70 stream words each);
 * each region's high-water is logged when it rises. A 4x frame must not
 * split (the next frame's preload has the resolved colour only, in every
 * sample), and gl_test case 198 draws 1024 times into one, as Mesa runs it
 * (one frame a picture, 0dce654a8:tools/mali/board-ref/2026-10-20-mesa-refs): its
 * draws take 640 bytes of descriptors, 29 stream words and 96 bytes of
 * vertex data each (programs are cached), so the descriptors and the
 * stream hold about 1600 of them. The data region is the first chunk of
 * the frame's data pool: a draw whose data does not fit what is left grows
 * the pool by a chunk of `MAX2(DATA_BYTES, block)` bytes ([`grow_data`]),
 * as Mesa's pool and stream uploader do (gl_test case 197 stages 516,128
 * bytes of client arrays for one draw); a frame whose data fits is built
 * as before, in the first chunk alone. */
const DESC_BYTES: usize = 1024 * 1024;
const DATA_BYTES: usize = 256 * 1024;
const CODE_BYTES: usize = 128 * 1024;
const SPD_BYTES: usize = 32 * 1024;
const CS_WORDS: usize = 48 * 1024;
const CONTEXT_BYTES: usize = 4096;
const INIT_WORDS: usize = 16;
const SLOTS: usize = 2;
/// The bound on one fence wait: a frame of this backend takes well under a
/// millisecond on the board (L1); a wait that runs out is lost work.
const WAIT_US: i64 = 5_000_000;

/// One occlusion query handle's counter: a qword for each of the 64 core ids
/// a shader core mask can name. Mesa sizes it `8 * core_id_range`
/// (`panfrost_create_query`, Mesa 26.2.1 pan_context.c:689; the T6's
/// shader_present 0x50005 gives 19), from a query the OSAL does not answer
/// here (`OsalMaliGpu`); a qword no core writes stays 0, so the sum of all
/// of them is the sum of the written ones.
const QUERY_BLOCK_BYTES: usize = 8 * 64;
/// Every handle's counter, `GLES_QUERY_HANDLES` blocks (handle 0, no query,
/// unused).
const QUERY_BYTES: usize = QUERY_BLOCK_BYTES * backend::GLES_QUERY_HANDLES as usize;

/// The regions of one slot, in this order: desc, data, code, spds, cs.
const REGIONS: [(&str, usize, u32); 5] = [
    ("desc", DESC_BYTES, OSAL_MEM_DATA),
    ("data", DATA_BYTES, OSAL_MEM_DATA),
    ("code", CODE_BYTES, OSAL_MEM_GPU_EXEC),
    ("spds", SPD_BYTES, OSAL_MEM_DATA),
    ("cs", 8 * CS_WORDS, OSAL_MEM_DATA),
];

/// The frame a slot last held, until the OSAL said it completed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct InFlight {
    kick: u32,
    fence: i64,
    /// A wait for it timed out or failed. The GPU may still read the slot,
    /// so the slot stays in flight; from now on its completion is read from
    /// the OSAL's `completed` (never waited for again), and a draw that
    /// needs the slot is refused ([`SLOT_STILL_IN_FLIGHT`]) until then.
    wait_failed: bool,
}

#[derive(Clone, Copy)]
struct Slot {
    mem: [OsalGpuMem; 5],
    /// The data chunks grown after `mem[1]` ([`grow_data`]), kept for the
    /// slot's later frames; given back at shutdown, or when one that holds
    /// nothing of the open frame is too small for a block.
    data_chunks: [OsalGpuMem; POOL_CHUNKS - 1],
    in_flight: Option<InFlight>,
}

/// What one draw is drawn into: the colour target and the depth/stencil
/// objects. A draw with another key ends the open frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TargetKey {
    gpu: u64,
    w: u32,
    h: u32,
    row_stride: u32,
    /// The same memory in another format, or rasterised at another sample
    /// count, is another target.
    format: frame::TargetFormat,
    samples: u8,
    ds_id: u32,
    /// The depth and the stencil object (`GlesDraw::depth_id`,
    /// `stencil_id`) and their storage generations.
    objects: [(u32, u32); 2],
    ds_w: u32,
    ds_h: u32,
}

/// Where one plane of the open frame's attachment starts.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Start {
    /// No object on this attachment point.
    Absent,
    /// The object holds nothing GL keeps: any value will do.
    Undefined,
    /// Cleared to this value (depth as `f32` bits, the stencil value), by
    /// this frame or before it.
    Value(u32),
    /// A past frame drew into it: this pair's buffer holds its contents,
    /// which a frame that uses the attachment preloads.
    Unloaded,
    /// A past frame drew into it in another pair's buffer
    /// ([`NOT_BUILT_ZS_ELSEWHERE`]).
    Elsewhere,
}

/// The open frame's depth/stencil attachment.
#[derive(Clone, Copy, Debug)]
struct OpenZs {
    /// Its buffer, an index of `MaliState::zsbufs`.
    buf: usize,
    start: [Start; 2],
    /// A draw of the frame wrote the plane.
    written: [bool; 2],
}

/// The frame being recorded.
#[derive(Clone, Copy)]
struct OpenFrame {
    slot: usize,
    /// The tops of the desc, data, code and spds arenas, and the stream's
    /// length.
    tops: Tops,
    cs_len: usize,
    frame: frame::OpenFrame,
    key: TargetKey,
    target: frame::Target,
    clear: [f32; 4],
    zs: Option<OpenZs>,
}

/// One depth/stencil buffer: the attachment pair it is for (its `ds_id`; 0:
/// free), its size and its memory. Memory is given back only at shutdown:
/// a frame in flight may still write it back, and the queue runs frames in
/// order, so a buffer given to another pair is written by its later frames
/// after the earlier frame's write-back.
#[derive(Clone, Copy)]
struct ZsBuf {
    ds_id: u32,
    w: u32,
    h: u32,
    mem: OsalGpuMem,
}

/// One GL object's plane: (object, its storage generation, depth 0 or
/// stencil 1) and what the plane holds. Id 0 is a free entry; a plane not
/// listed is [`Plane::UNDEFINED`].
#[derive(Clone, Copy)]
struct PlaneEntry {
    id: u32,
    gen: u32,
    aspect: usize,
    plane: Plane,
    /// The attachment pair (`ds_id`) whose buffer holds a live plane.
    ds_id: u32,
}

const NO_PLANE: PlaneEntry = PlaneEntry { id: 0, gen: 0, aspect: 0, plane: Plane::UNDEFINED, ds_id: 0 };
/// The planes kept: a depth and a stencil for each depth/stencil buffer.
const PLANES: usize = 2 * ds_retention::DS_SLOTS;
const NO_ZSBUF: ZsBuf = ZsBuf { ds_id: 0, w: 0, h: 0, mem: OsalGpuMem::ZERO };

/// The clear program: the two stages' records, compiled and linked at init.
#[derive(Clone, Copy)]
struct ClearProgram {
    built: bool,
    vs: ShcShader,
    fs: ShcShader,
    /// The GL location of `a_pos`.
    location: u8,
}

/// The longest preload program kept: Mesa's is 7 instructions, padded to 16
/// words.
const PRELOAD_WORDS: usize = 32;

#[derive(Clone, Copy)]
struct PreloadProgram {
    /// 0: not built.
    n: usize,
    words: [u64; PRELOAD_WORDS],
    /// Its preloaded r48-r63.
    preload: u32,
}

const NO_PRELOAD_PROGRAM: PreloadProgram = PreloadProgram { n: 0, words: [0; PRELOAD_WORDS], preload: 0 };

/// Compile a preload program (once, at init) with its builder
/// (src/shaderc/online/mali/preload.rs): `what` names it in the log.
fn build_preload_program<F>(what: &str, build: F) -> PreloadProgram
where
    F: for<'s> FnOnce(
        &'s mut [u64],
        &crate::shaderc::online::arena::Arena<'_>,
    ) -> Result<
        crate::shaderc::online::mali::preload::Preload<'s>,
        crate::shaderc::online::mali::MaliError,
    >,
{
    let mut out = [0u64; PRELOAD_WORDS];
    let r = crate::shaderc::online::store::with_scratch(|s| build(&mut out, s).map(|p| (p.code.len(), p.preload)));
    match r {
        Some(Ok((n, preload))) => {
            glog!("mali: {} preload program built: {} bytes of code", what, 8 * n);
            PreloadProgram { n, words: out, preload: (preload >> 48) as u32 }
        }
        Some(Err(e)) => {
            glog!("mali: {} preload program did not build: {:?}", what, e);
            NO_PRELOAD_PROGRAM
        }
        None => {
            glog!("mali: {} preload program did not build: the compiler's scratch arena is held", what);
            NO_PRELOAD_PROGRAM
        }
    }
}

/// The longest blend shader kept: Mesa's for case 211's equation is 22
/// instructions, padded to 32 words.
const BLEND_SHADER_WORDS: usize = 32;

/// The blend shaders compiled so far, one per equation (Mesa caches them on
/// the equation without its constants, pan_blend_cso.c:59-98: one serves
/// every glBlendColor).
const BLEND_SHADERS: usize = 8;

#[derive(Clone, Copy)]
struct BlendShaderProgram {
    eq: crate::shaderc::online::mali::blend::Equation,
    /// 0: a free entry.
    n: usize,
    words: [u64; BLEND_SHADER_WORDS],
}

const NO_BLEND_SHADER: BlendShaderProgram = BlendShaderProgram {
    eq: crate::shaderc::online::mali::blend::Equation::both(
        crate::shaderc::online::mali::blend::Factor::Zero,
        crate::shaderc::online::mali::blend::Factor::Zero,
    ),
    n: 0,
    words: [0; BLEND_SHADER_WORDS],
};

/// The memory format of a blend shader's RGBA8 target, the conversion word
/// of its internal descriptor (`pan_dithered_format_from_pipe_format`,
/// not dithered: RGBA8_TB in RGBA order, the shim's `#0xED000`).
const BLEND_SHADER_RGBA8: u32 = 0xed000;

/// The blend shader of `eq`, from the cache or compiled now
/// (src/shaderc/online/mali/blend.rs); None when it does not compile, which
/// the frame builder then refuses by name.
fn blend_shader(g: &mut MaliState, eq: crate::shaderc::online::mali::blend::Equation) -> Option<BlendShaderProgram> {
    if let Some(p) = g.blend_shaders.iter().find(|p| p.n != 0 && p.eq == eq) {
        return Some(*p);
    }
    let mut out = [0u64; 2 * BLEND_SHADER_WORDS];
    let r = crate::shaderc::online::store::with_scratch(|sc| {
        crate::shaderc::online::mali::blend::shader(&mut out, sc, eq, BLEND_SHADER_RGBA8).map(|b| b.code.len())
    });
    let n = match r {
        Some(Ok(n)) if n <= BLEND_SHADER_WORDS => n,
        Some(Ok(n)) => {
            glog!("mali: the blend shader of {:?} is {} words, more than the {} kept", eq, n, BLEND_SHADER_WORDS);
            return None;
        }
        Some(Err(e)) => {
            glog!("mali: the blend shader of {:?} did not build: {:?}", eq, e);
            return None;
        }
        None => {
            glog!("mali: the blend shader of {:?} did not build: the compiler's scratch arena is held", eq);
            return None;
        }
    };
    let mut p = BlendShaderProgram { eq, n, words: [0; BLEND_SHADER_WORDS] };
    p.words[..n].copy_from_slice(&out[..n]);
    glog!("mali: blend shader of {:?} built: {} bytes of code", eq, 8 * n);
    /* a full table drops its oldest entry */
    g.blend_shaders.copy_within(0..BLEND_SHADERS - 1, 1);
    g.blend_shaders[0] = p;
    Some(p)
}

/// A draw's blend shader: the words [`blend_shader`] compiled.
#[derive(Clone, Copy)]
struct DrawBlendShader {
    n: usize,
    words: [u64; BLEND_SHADER_WORDS],
}

impl DrawBlendShader {
    fn of(words: &[u64]) -> DrawBlendShader {
        let mut s = DrawBlendShader { n: words.len(), words: [0; BLEND_SHADER_WORDS] };
        s.words[..words.len()].copy_from_slice(words);
        s
    }

    fn words(&self) -> &[u64] {
        &self.words[..self.n]
    }
}

/// The blend shader a draw needs (`blend::needs_shader`) and that is built
/// here (`blend::shader_equation`: the constant colour's equations into an
/// RGBA8 target written whole, single-sampled or case 211's at 4x, and case
/// 190's columns 3 and 4), compiled. None for a draw that needs none, or one
/// not built (the frame builder refuses that draw by name).
fn draw_blend_shader(
    g: &mut MaliState,
    d: &GlesDraw,
    target: &frame::Target,
) -> Result<Option<DrawBlendShader>, FrameError> {
    let mask = if d.color_write { d.color_mask & 0xf } else { 0 };
    if !d.blend || mask == 0 {
        return Ok(None);
    }
    let b = blending(d)?;
    if !blend::needs_shader(&b, &d.blend_color) {
        return Ok(None);
    }
    let eq = blend::shader_equation(&b, u32::from(mask), target.samples, target.format);
    Ok(eq.and_then(|eq| blend_shader(g, eq)).map(|p| DrawBlendShader::of(&p.words[..p.n])))
}

const NO_CLEAR_PROGRAM: ClearProgram =
    ClearProgram { built: false, vs: ShcShader::ZERO, fs: ShcShader::ZERO, location: 0 };

struct MaliState {
    up: bool,
    user_regs: u32,
    ctx: Option<frame::Context>,
    context: OsalGpuMem,
    geometry: OsalGpuMem,
    init: OsalGpuMem,
    /// The occlusion counters ([`QUERY_BYTES`]): `query_begin` zeroes a
    /// handle's block, a counted draw names it (`OQ`), `query_result` sums it.
    queries: OsalGpuMem,
    slots: [Slot; SLOTS],
    /// The slot the next frame is built in (the open frame's, while one is
    /// open).
    next: usize,
    open: Option<OpenFrame>,
    zsbufs: [ZsBuf; ds_retention::DS_SLOTS],
    planes: [PlaneEntry; PLANES],
    clear_program: ClearProgram,
    /// The colour preload program's words (src/shaderc/online/mali/
    /// preload.rs), compiled at init: a frame without a colour clear draws
    /// the old image back with it.
    preload: PreloadProgram,
    /// The depth/stencil preload programs (`preload::zs`), for the depth
    /// alone, the stencil alone and both ([`zs_variant`]): a frame that
    /// uses depth or stencil a past frame drew draws them back with one.
    zs_preload: [PreloadProgram; 3],
    /// The blend shaders compiled so far ([`blend_shader`]).
    blend_shaders: [BlendShaderProgram; BLEND_SHADERS],
    /// Kicks submitted, and waits made.
    kicks: u32,
    waits: u32,
    /// The most each region held in any frame, in bytes (desc, data, code,
    /// spds, cs), and the context arena.
    high_water: [usize; 6],
    /// The most depth/stencil buffers held at once.
    zs_high_water: usize,
    /// Draws that sampled a copy of an imported image whose rows are not
    /// [`TEXTURE_ROW_ALIGN`] bytes ([`needs_copy`]). Printed with the
    /// arenas' high-water lines when a frame ends and the count has reached
    /// a new power of two (1, 2, 4, ...), and at shutdown: a product run
    /// shows whether the copy ever ran, and how often.
    row_copies: u32,
    /// The count last printed.
    row_copies_noted: u32,
}

const NO_SLOT: Slot =
    Slot { mem: [OsalGpuMem::ZERO; 5], data_chunks: [OsalGpuMem::ZERO; POOL_CHUNKS - 1], in_flight: None };

static STATE: Global<MaliState> = Global::new(MaliState {
    up: false,
    user_regs: 0,
    ctx: None,
    context: OsalGpuMem::ZERO,
    geometry: OsalGpuMem::ZERO,
    init: OsalGpuMem::ZERO,
    queries: OsalGpuMem::ZERO,
    slots: [NO_SLOT; SLOTS],
    next: 0,
    open: None,
    zsbufs: [NO_ZSBUF; ds_retention::DS_SLOTS],
    planes: [NO_PLANE; PLANES],
    clear_program: NO_CLEAR_PROGRAM,
    preload: NO_PRELOAD_PROGRAM,
    zs_preload: [NO_PRELOAD_PROGRAM; 3],
    blend_shaders: [NO_BLEND_SHADER; BLEND_SHADERS],
    kicks: 0,
    waits: 0,
    high_water: [0; 6],
    zs_high_water: 0,
    row_copies: 0,
    row_copies_noted: 0,
});

fn note_high_water(slot: &mut usize, label: &str, now: usize, capacity: usize) {
    if now > *slot {
        *slot = now;
        glog!("mali arena '{}': high-water {} of {} bytes", label, now, capacity);
    }
}

/// The first reason of the process's, whole. The log line of a refused
/// draw is cut at the OSAL's line limit (`OSAL_LOG_LINE_MAX`) and is
/// written for the first refusal only; a harness that groups by the reason
/// (tools/mali/panthor `panthor-gltest`) reads it here instead.
pub struct FirstRefusal {
    ptr: AtomicPtr<u8>,
    len: AtomicUsize,
}

impl FirstRefusal {
    pub const fn new() -> Self {
        FirstRefusal { ptr: AtomicPtr::new(core::ptr::null_mut()), len: AtomicUsize::new(0) }
    }

    /// Keep `why` when no reason is kept yet.
    pub fn note(&self, why: &'static str) {
        if self.ptr.load(Ordering::Acquire).is_null() {
            self.len.store(why.len(), Ordering::Relaxed);
            self.ptr.store(why.as_ptr().cast_mut(), Ordering::Release);
        }
    }

    pub fn get(&self) -> Option<&'static str> {
        let p = self.ptr.load(Ordering::Acquire);
        if p.is_null() {
            return None;
        }
        let n = self.len.load(Ordering::Relaxed);
        // SAFETY: `p` and `n` are a `&'static str`'s, stored by `note` (the
        // length before the pointer, read after it).
        Some(unsafe { core::str::from_utf8_unchecked(core::slice::from_raw_parts(p, n)) })
    }
}

impl Default for FirstRefusal {
    fn default() -> Self {
        Self::new()
    }
}

static FIRST_REFUSAL: FirstRefusal = FirstRefusal::new();

/// The reason of the first draw this backend refused in the process, whole.
pub fn first_refusal() -> Option<&'static str> {
    FIRST_REFUSAL.get()
}

fn refuse(why: &'static str) -> i32 {
    FIRST_REFUSAL.note(why);
    backend::note_draw_refused(why, 0);
    -ENOTSUP
}

/// Why a draw or a clear was not recorded: a frame error (its errno and log
/// line from [`frame_error`]), or an errno whose cause was counted and
/// logged where it happened (a lost slot, a depth/stencil buffer memory
/// could not give). The cause travels to the entry point, so a test reads
/// it whole.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Fail {
    Frame(FrameError),
    Errno(i32),
}

impl From<FrameError> for Fail {
    fn from(e: FrameError) -> Self {
        Fail::Frame(e)
    }
}

fn fail_errno(f: Fail) -> i32 {
    match f {
        Fail::Frame(e) => frame_error(e),
        Fail::Errno(rc) => rc,
    }
}

fn frame_error(e: FrameError) -> i32 {
    match e {
        FrameError::NotBuilt(why) => refuse(why),
        FrameError::Arena(a) => {
            glog_once!("{}", a);
            -ENOMEM
        }
        /* a value the packer or the encoder refused, or an input outside the
         * hardware's range: a builder or caller defect, never masked */
        other => {
            glog_once!("{}", other);
            -EINVAL
        }
    }
}

/// One OSAL query into `out`.
fn query<T>(what: u32, out: &mut T) -> Result<(), i32> {
    let q = osal::ops().and_then(|o| o.query).ok_or(-ENODEV)?;
    // SAFETY: `out` is a valid exclusive reference of the size passed.
    let rc = unsafe { q(what, (out as *mut T).cast::<c_void>(), core::mem::size_of::<T>()) };
    if rc != 0 {
        glog!("{} (query {} returned {})", NO_MALI_OSAL, what, rc);
        return Err(-ENODEV);
    }
    Ok(())
}

/// A buffer of the OSAL's, with an 8-byte-aligned CPU mapping.
fn alloc(size: usize, flags: u32, out: &mut OsalGpuMem) -> Result<(), i32> {
    let rc = osal::gpu_alloc(0, size, flags, out);
    if rc != 0 {
        glog!("mali: the OSAL refused a buffer of {} bytes ({})", size, rc);
        return Err(if rc < 0 { rc } else { -ENOMEM });
    }
    if out.cpu.is_null() || !(out.cpu as usize).is_multiple_of(8) || out.size < size {
        glog!("{}", INVALID_OSAL_BUFFER);
        return Err(-EINVAL);
    }
    Ok(())
}

/// The bytes of a buffer the backend owns.
///
/// # Safety
/// `m` is a live allocation of this backend with a CPU mapping, and the GPU
/// does not read or write it while the slice is used.
unsafe fn bytes_of<'a>(m: &OsalGpuMem) -> &'a mut [u8] {
    // SAFETY: the caller's contract.
    unsafe { core::slice::from_raw_parts_mut(m.cpu.cast::<u8>(), m.size) }
}

/// The words of a stream buffer.
///
/// # Safety
/// As [`bytes_of`]; `alloc` checked the mapping's alignment.
unsafe fn words_of<'a>(m: &OsalGpuMem) -> &'a mut [u64] {
    // SAFETY: the caller's contract; 8-byte aligned (checked by `alloc`).
    unsafe { core::slice::from_raw_parts_mut(m.cpu.cast::<u64>(), m.size / 8) }
}

fn submit(stream_gpu: u64, words: usize) -> Result<i64, i32> {
    let s = osal::ops().and_then(|o| o.submit).ok_or(-ENODEV)?;
    let kick = OsalMaliKick { stream_gpu, stream_bytes: (8 * words) as u32, reserved: 0 };
    // SAFETY: `kick` is a live OsalMaliKick of the size passed.
    let fence = unsafe { s((&kick as *const OsalMaliKick).cast::<c_void>(), core::mem::size_of::<OsalMaliKick>()) };
    if fence < 0 {
        glog_once!("mali: submit of {} words at {:#x} failed ({})", words, stream_gpu, fence);
        return Err(-EIO);
    }
    Ok(fence)
}

/// Why a frame slot could not be had: the errno and the line logged.
type SlotError = (i32, &'static str);

fn wait(g: &mut MaliState, kick: u32, fence: i64) -> Result<(), SlotError> {
    let w = osal::ops().and_then(|o| o.wait).ok_or((-ENODEV, NO_MALI_OSAL))?;
    // SAFETY: a wait on a fence the OSAL returned.
    let rc = unsafe { w(fence, WAIT_US) };
    g.waits += 1;
    if rc > 0 {
        glog_once!("{} (kick {}, fence {}, {} us)", WAIT_TIMED_OUT, kick, fence, WAIT_US);
        return Err((-EIO, WAIT_TIMED_OUT));
    }
    if rc < 0 {
        glog_once!("{} (kick {}, fence {}, OSAL wait {})", WAIT_FAILED, kick, fence, rc);
        return Err((rc, WAIT_FAILED));
    }
    Ok(())
}

/// The OSAL's last completed fence.
fn completed() -> Option<i64> {
    let c = osal::ops().and_then(|o| o.completed)?;
    // SAFETY: a plain query of the OSAL.
    Some(unsafe { c() })
}

/// Free slot `i` of the frame it holds, if any. The slot is free only when
/// the OSAL said that frame completed: a wait that returned 0, or, after a
/// wait for it failed, `completed` at or past its fence. A wait that times
/// out or fails keeps the slot in flight (the GPU may still read it) and
/// returns the wait's error; a slot whose wait failed before is not waited
/// for again and is `-EIO` ([`SLOT_STILL_IN_FLIGHT`]) until it completes.
fn wait_slot(g: &mut MaliState, i: usize) -> Result<(), SlotError> {
    let Some(f) = g.slots[i].in_flight else {
        return Ok(());
    };
    if f.wait_failed {
        if completed().is_some_and(|c| c >= f.fence) {
            g.slots[i].in_flight = None;
            return Ok(());
        }
        glog_once!("{} (kick {}, fence {})", SLOT_STILL_IN_FLIGHT, f.kick, f.fence);
        return Err((-EIO, SLOT_STILL_IN_FLIGHT));
    }
    match wait(g, f.kick, f.fence) {
        Ok(()) => {
            g.slots[i].in_flight = None;
            Ok(())
        }
        Err(e) => {
            g.slots[i].in_flight = Some(InFlight { wait_failed: true, ..f });
            Err(e)
        }
    }
}

/// Wait for every frame, oldest first; the first error.
fn wait_all(g: &mut MaliState) -> Result<(), SlotError> {
    let mut order = [0usize, 1];
    let kick = |s: &Slot| s.in_flight.map_or(0, |f| f.kick);
    if kick(&g.slots[1]) < kick(&g.slots[0]) {
        order = [1, 0];
    }
    let mut first = Ok(());
    for i in order {
        let r = wait_slot(g, i);
        if first.is_ok() {
            first = r;
        }
    }
    first
}

fn bring_up(g: &mut MaliState) -> Result<(), i32> {
    let o = osal::ops();
    if !o.is_some_and(|o| {
        o.gpu_alloc.is_some() && o.submit.is_some() && o.wait.is_some() && o.completed.is_some() && o.query.is_some()
    }) {
        glog!("{}", NO_MALI_OSAL);
        return Err(-ENODEV);
    }
    let mut gpu = OsalMaliGpu::default();
    query(OSAL_QUERY_MALI_GPU, &mut gpu)?;
    let mut heap = OsalMaliHeap::default();
    query(OSAL_QUERY_MALI_TILER_HEAP, &mut heap)?;
    let Some(user_regs) = frame::user_regs(gpu.cs_reg_count, gpu.unpreserved_cs_reg_count) else {
        glog!(
            "{} (cs_reg_count {}, unpreserved {})",
            INVALID_OSAL_ANSWER,
            gpu.cs_reg_count,
            gpu.unpreserved_cs_reg_count
        );
        return Err(-EINVAL);
    };
    if heap.ctx_gpu == 0 || heap.first_chunk_gpu == 0 || heap.chunk_size <= 64 {
        glog!(
            "{} (heap ctx {:#x}, chunk {:#x} of {})",
            INVALID_OSAL_ANSWER,
            heap.ctx_gpu,
            heap.first_chunk_gpu,
            heap.chunk_size
        );
        return Err(-EINVAL);
    }
    g.user_regs = user_regs;
    glog!(
        "mali: gpu_id {:#010x}, tiler_features {:#x}, {} user registers; tiler heap ctx {:#x}, first chunk {:#x}, chunk {:#x}",
        gpu.gpu_id,
        gpu.tiler_features,
        user_regs,
        heap.ctx_gpu,
        heap.first_chunk_gpu,
        heap.chunk_size
    );
    alloc(CONTEXT_BYTES, OSAL_MEM_DATA, &mut g.context)?;
    alloc(frame::GEOMETRY_BUFFER_BYTES as usize, OSAL_MEM_DATA, &mut g.geometry)?;
    alloc(8 * INIT_WORDS, OSAL_MEM_DATA, &mut g.init)?;
    for s in 0..SLOTS {
        for (r, &(_, size, flags)) in REGIONS.iter().enumerate() {
            alloc(size, flags, &mut g.slots[s].mem[r])?;
        }
    }
    alloc(QUERY_BYTES, OSAL_MEM_DATA, &mut g.queries)?;
    /* the per-context objects, once */
    // SAFETY: just allocated; nothing has been submitted.
    let mut ctx_arena = Arena::new("context", g.context.gpu, unsafe { bytes_of(&g.context) }).map_err(|e| {
        glog!("{}", e);
        -EINVAL
    })?;
    let ctx = frame::build_context(
        &mut ctx_arena,
        heap.first_chunk_gpu,
        heap.chunk_size,
        g.geometry.gpu,
        frame::tiler_max_levels(gpu.tiler_features),
    )
    .map_err(frame_error)?;
    note_high_water(&mut g.high_water[5], "context", ctx_arena.high_water(), ctx_arena.capacity());
    g.ctx = Some(ctx);
    /* the queue-init stream: HEAP_SET of the OSAL's heap context */
    // SAFETY: as above.
    let mut init = Stream::new(unsafe { words_of(&g.init) }, user_regs);
    frame::queue_init(&mut init, heap.ctx_gpu).map_err(frame_error)?;
    let n = init.words().len();
    // SAFETY: both buffers are this backend's, written above.
    unsafe {
        osal::cache_flush(g.context.cpu, g.context.size);
        osal::cache_flush(g.init.cpu, 8 * n);
    }
    let fence = submit(g.init.gpu, n)?;
    g.kicks += 1;
    let kick = g.kicks;
    wait(g, kick, fence).map_err(|(rc, _)| rc)?;
    g.clear_program = build_clear_program();
    g.preload = build_preload_program("colour", crate::shaderc::online::mali::preload::colour);
    for (k, (z, s, what)) in
        [(true, false, "depth"), (false, true, "stencil"), (true, true, "depth/stencil")].into_iter().enumerate()
    {
        g.zs_preload[k] =
            build_preload_program(what, |out, sc| crate::shaderc::online::mali::preload::zs(out, sc, z, s));
    }
    Ok(())
}

/// Give every buffer back (nothing may be in flight).
fn release(g: &mut MaliState) {
    for s in &mut g.slots {
        for m in s.mem.iter_mut().chain(&mut s.data_chunks) {
            osal::gpu_free(m);
        }
        s.in_flight = None;
    }
    for m in [&mut g.context, &mut g.geometry, &mut g.init, &mut g.queries] {
        osal::gpu_free(m);
    }
    for z in &mut g.zsbufs {
        osal::gpu_free(&mut z.mem);
        *z = NO_ZSBUF;
    }
    if g.zs_high_water > 0 {
        glog!("mali: depth/stencil buffers: high-water {} of {}", g.zs_high_water, ds_retention::DS_SLOTS);
    }
    glog!("mali: draws that sampled a copy of an imported image's rows: {}", g.row_copies);
    (g.row_copies, g.row_copies_noted) = (0, 0);
    g.planes = [NO_PLANE; PLANES];
    if g.clear_program.built {
        crate::shaderc::release_shader(&g.clear_program.vs);
        crate::shaderc::release_shader(&g.clear_program.fs);
    }
    g.clear_program = NO_CLEAR_PROGRAM;
    g.open = None;
    g.ctx = None;
    g.next = 0;
}

/* ---- the clear program --------------------------------------------------- */

/// The clear quad's vertex shader: the position passes through (the quad's
/// vertices are in clip space, at the clear depth, [`clear_quad_vertices`]).
/// The V3D's and the GC7000's clear shader (src/v3d/backend.rs `CLEAR_VS`,
/// tools/shaders/internal/clear.vert), in GLSL ES 1.00.
pub const CLEAR_VS: &[u8] = b"attribute vec4 a_pos;\nvoid main() { gl_Position = a_pos; }\n\0";
/// The clear quad's fragment shader: the clear colour, from its one uniform.
pub const CLEAR_FS: &[u8] =
    b"precision mediump float;\nuniform vec4 u_color;\nvoid main() { gl_FragColor = u_color; }\n\0";

/// The text of a compiler log (NUL-terminated in `log`).
fn log_text(log: &[core::ffi::c_char]) -> &str {
    // SAFETY: c_char and u8 have the same size and alignment.
    let bytes = unsafe { core::slice::from_raw_parts(log.as_ptr().cast::<u8>(), log.len()) };
    let n = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    core::str::from_utf8(&bytes[..n]).unwrap_or("(the log is not UTF-8)")
}

/// Compile and link the clear program through the shader back end, as the
/// GL core does an application's, and say once what happened.
fn build_clear_program() -> ClearProgram {
    let mut log = [0 as core::ffi::c_char; 256];
    match compile_clear_program(&mut log) {
        Ok(p) => {
            glog!("mali: clear program built: {} + {} bytes of code", 4 * p.vs.usc_dw, 4 * p.fs.usc_dw);
            p
        }
        Err(what) => {
            glog!("mali: clear program did not build: {}: {}", what, log_text(&log));
            NO_CLEAR_PROGRAM
        }
    }
}

fn compile_clear_program(log: &mut [core::ffi::c_char; 256]) -> Result<ClearProgram, &'static str> {
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
    // SAFETY: two compiled shaders.
    let rc = unsafe { link(&vs, &mut fs, log.as_mut_ptr(), log.len()) };
    if rc == 0 {
        log[0] = 0; /* what fails below is ours, not the compiler's */
    }
    let give_back = |vs: &ShcShader, fs: &ShcShader| {
        crate::shaderc::release_shader(vs);
        crate::shaderc::release_shader(fs);
    };
    if rc != 0 {
        give_back(&vs, &fs);
        return Err("the program did not link");
    }
    /* the quad hands the colour as the fragment stage's four uniform
     * dwords, so the program must read `u_color` from dwords 0..3 */
    let at_zero = fs
        .uniform_table()
        .iter()
        .any(|u| crate::util::name_bytes(&u.name) == b"u_color" && u.dword_off == 0 && u.dwords == 4);
    if !at_zero {
        give_back(&vs, &fs);
        return Err("u_color is not the fragment stage's dwords 0..3");
    }
    let location = match MaliShader::of(&vs).map(|r| r.attributes) {
        Some(&[(loc, 0)]) => loc,
        _ => {
            give_back(&vs, &fs);
            return Err("the vertex shader does not read one attribute as its record 0");
        }
    };
    Ok(ClearProgram { built: true, vs, fs, location })
}

/// The quad's six vertices (two triangles over the whole clip-space square),
/// every one at the clear depth: z = 2 * depth - 1 with the depth range
/// (0, 1), as st/mesa passes `ctx->Depth.Clear * 2.0f - 1.0f`
/// (st_cb_clear.c:309).
pub fn clear_quad_vertices(depth: f32) -> [[f32; 4]; 6] {
    let z = depth * 2.0 - 1.0;
    [
        [-1.0, -1.0, z, 1.0],
        [1.0, -1.0, z, 1.0],
        [1.0, 1.0, z, 1.0],
        [-1.0, -1.0, z, 1.0],
        [1.0, 1.0, z, 1.0],
        [-1.0, 1.0, z, 1.0],
    ]
}

/// Which buffers one clear quad writes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct QuadBuffers {
    pub colour: bool,
    pub depth: bool,
    pub stencil: bool,
}

/// A clear as a draw of the clear program, with the state of st/mesa's
/// `clear_with_quad` (st_cb_clear.c:194-309): the colour write mask the
/// clear's (or none when the quad does not clear colour); depth ALWAYS with
/// the write on when it clears depth; stencil ALWAYS, REPLACE on fail,
/// depth fail and pass, value mask 0xff, the clear's write mask and value,
/// on both faces; the viewport the whole target, depth range (0, 1), no
/// blend, no cull, no polygon offset. The quad covers the target and keeps
/// the clear's scissor (Mesa's covers the scissor rectangle, :300-308; the
/// pixels are the same), when the clear has one (`clear_quad`).
///
/// Only the target and the clear's values are taken from `d`; everything
/// else starts from zero, because `d` may be an application's draw. The
/// returned draw points at `p`, `vertices` and `colour`.
fn clear_quad_draw(
    d: &GlesDraw,
    b: QuadBuffers,
    p: &ClearProgram,
    vertices: &[[f32; 4]; 6],
    colour: &[f32; 4],
) -> GlesDraw {
    let mut q = GlesDraw::ZERO;
    (q.target_gpu, q.target_w, q.target_h, q.target_format) = (d.target_gpu, d.target_w, d.target_h, d.target_format);
    (q.target_alpha_bits, q.samples, q.num_targets, q.flip_y) =
        (d.target_alpha_bits, d.samples, d.num_targets, d.flip_y);
    q.targets = d.targets;
    (q.ds_id, q.depth_id, q.depth_gen, q.stencil_id, q.stencil_gen) =
        (d.ds_id, d.depth_id, d.depth_gen, d.stencil_id, d.stencil_gen);
    (q.ds_w, q.ds_h) = (d.ds_w, d.ds_h);
    q.prim = backend::GLES_PRIM_TRIANGLES;
    q.index_count = 6;
    (q.vs, q.fs) = (&p.vs, &p.fs);
    q.attribs[0] = GlesAttrib {
        vi_reg: p.location,
        components: 4,
        format: GLES_ATTRIBF_FLOAT32,
        stride: 16,
        cpu: vertices.as_ptr() as *const c_void,
        cpu_bytes: core::mem::size_of_val(vertices) as u32,
        ..GlesAttrib::ZERO
    };
    q.num_attribs = 1;
    q.fs_uniform_cpu = colour.as_ptr() as *const c_void;
    q.fs_uniform_dwords = 4;
    q.viewport = [0, 0, d.target_w as i32, d.target_h as i32];
    q.depth_range = [0.0, 1.0];
    q.line_width = 1.0;
    q.cull = backend::GLES_CULL_NONE;
    q.color_write = b.colour;
    q.color_mask = if b.colour { colour_clear_mask(d) } else { 0 };
    q.depth_test = b.depth;
    q.depth_write = b.depth;
    q.depth_func = backend::GLES_CMP_ALWAYS;
    q.stencil = b.stencil;
    if b.stencil {
        use backend::{GLES_CMP_ALWAYS, GLES_SOP_REPLACE};
        (q.stencil_func, q.stencil_back_func) = (GLES_CMP_ALWAYS, GLES_CMP_ALWAYS);
        (q.stencil_sfail, q.stencil_dpfail, q.stencil_dppass) = (GLES_SOP_REPLACE, GLES_SOP_REPLACE, GLES_SOP_REPLACE);
        (q.stencil_back_sfail, q.stencil_back_dpfail, q.stencil_back_dppass) =
            (GLES_SOP_REPLACE, GLES_SOP_REPLACE, GLES_SOP_REPLACE);
        (q.stencil_ref, q.stencil_back_ref) = (d.clear_stencil, d.clear_stencil);
        (q.stencil_cmp_mask, q.stencil_back_cmp_mask) = (0xff, 0xff);
        /* the glClear's own mask (`clear_stencil_write_mask`, 0 = not
         * given), on both faces (st_cb_clear.c:261-268) */
        let mask = if d.clear_stencil_write_mask != 0 { d.clear_stencil_write_mask } else { d.stencil_write_mask };
        (q.stencil_write_mask, q.stencil_back_write_mask) = (mask, mask);
    }
    if d.clear_quad {
        (q.scissor, q.scissor_rect) = (d.scissor, d.scissor_rect);
    }
    q
}

/// The channels the colour clear writes (`clear_color_mask`, 0 = all).
fn colour_clear_mask(d: &GlesDraw) -> u8 {
    if d.clear_color_mask == 0 {
        0xf
    } else {
        d.clear_color_mask & 0xf
    }
}

/* ---- the draw's state ---------------------------------------------------- */

/// The GL state this backend does not build, refused by name before any
/// memory is touched. A draw with no target is `Invalid`; the rest is
/// `NotBuilt`. A window (top-down) target is drawn by the frame builder
/// with the flipped viewport ([`frame::DrawCall::flip_y`]).
fn check(d: &GlesDraw) -> Result<(), FrameError> {
    let not_built = |why| Err(FrameError::NotBuilt(why));
    if d.target_gpu == 0 {
        return Err(FrameError::Invalid(INVALID_NO_TARGET));
    }
    if d.num_targets > 1 {
        return not_built(NOT_BUILT_TARGET_COUNT);
    }
    if d.targets[0].out_shift != 0 {
        return not_built(NOT_BUILT_OUTPUT_SHIFT);
    }
    target_format(d)?;
    let touches_zs = d.index_count != 0 || d.do_clear_depth || d.do_clear_stencil;
    if touches_zs && (d.depth_id != 0 || d.stencil_id != 0) && (d.ds_w < d.target_w || d.ds_h < d.target_h) {
        return not_built(NOT_BUILT_ZS_SIZE);
    }
    if d.index_count == 0 {
        return Ok(());
    }
    topology(d.prim)?;
    if d.occlusion_query >= backend::GLES_QUERY_HANDLES {
        return Err(FrameError::Invalid(INVALID_QUERY_HANDLE));
    }
    cull(d.cull)?;
    Ok(())
}

/// The GL core's colour surface format (`GLES_SURF_*`) as the frame's
/// target format; the others are refused by name.
fn target_format(d: &GlesDraw) -> Result<frame::TargetFormat, FrameError> {
    use frame::TargetFormat as F;
    match d.target_format {
        backend::GLES_SURF_RGBA8888 => Ok(F::Rgba8888),
        backend::GLES_SURF_XRGB8888 => Ok(F::Xrgb8888),
        backend::GLES_SURF_RGB565 => Ok(F::Rgb565),
        backend::GLES_SURF_RGBA16F => Ok(F::Rgba16f),
        _ => Err(FrameError::NotBuilt(NOT_BUILT_TARGET_FORMAT)),
    }
}

/// The GL core's blend state (`shc_blend_factor` / `shc_blend_func` codes:
/// a base factor and the invert bit, gallium's own pair) as the frame's.
fn blending(d: &GlesDraw) -> Result<blend::Blending, FrameError> {
    use crate::shaderc as s;
    use blend::{Base, Channel, Factor, Func};
    let factor = |code: u8| {
        let code = u32::from(code);
        let base = match code & !s::SHC_BF_INVERT {
            s::SHC_BF_ONE => Base::One,
            s::SHC_BF_SRC_COLOR => Base::SrcColor,
            s::SHC_BF_SRC_ALPHA => Base::SrcAlpha,
            s::SHC_BF_DST_ALPHA => Base::DstAlpha,
            s::SHC_BF_DST_COLOR => Base::DstColor,
            s::SHC_BF_SRC_ALPHA_SATURATE => Base::SrcAlphaSaturate,
            s::SHC_BF_CONST_COLOR => Base::ConstColor,
            s::SHC_BF_CONST_ALPHA => Base::ConstAlpha,
            _ => return Err(FrameError::Invalid(INVALID_BLEND_CODE)),
        };
        Ok(Factor { base, invert: code & s::SHC_BF_INVERT != 0 })
    };
    let func = |code: u8| match u32::from(code) {
        s::SHC_BLEND_ADD => Ok(Func::Add),
        s::SHC_BLEND_SUBTRACT => Ok(Func::Subtract),
        s::SHC_BLEND_REVERSE_SUBTRACT => Ok(Func::ReverseSubtract),
        s::SHC_BLEND_MIN => Ok(Func::Min),
        s::SHC_BLEND_MAX => Ok(Func::Max),
        _ => Err(FrameError::Invalid(INVALID_BLEND_CODE)),
    };
    Ok(blend::Blending {
        rgb: Channel { func: func(d.blend_eq_rgb)?, src: factor(d.blend_src_rgb)?, dst: factor(d.blend_dst_rgb)? },
        alpha: Channel {
            func: func(d.blend_eq_alpha)?,
            src: factor(d.blend_src_alpha)?,
            dst: factor(d.blend_dst_alpha)?,
        },
    })
}

/// The GL core's cull winding (`GLES_CULL_*`) as the frame's.
fn cull(c: backend::GlesCull) -> Result<frame::Cull, FrameError> {
    match c {
        backend::GLES_CULL_NONE => Ok(frame::Cull::None),
        backend::GLES_CULL_CW => Ok(frame::Cull::Cw),
        backend::GLES_CULL_CCW => Ok(frame::Cull::Ccw),
        _ => Err(FrameError::Invalid(INVALID_CULL)),
    }
}

/// The GL core's primitive code (`GLES_PRIM_*`) as the frame's topology.
fn topology(prim: backend::GlesPrim) -> Result<frame::Topology, FrameError> {
    use frame::Topology as T;
    Ok(match prim {
        backend::GLES_PRIM_TRIANGLES => T::Triangles,
        backend::GLES_PRIM_TRI_STRIP => T::TriangleStrip,
        backend::GLES_PRIM_TRI_FAN => T::TriangleFan,
        backend::GLES_PRIM_LINES => T::Lines,
        backend::GLES_PRIM_LINE_STRIP => T::LineStrip,
        backend::GLES_PRIM_LINE_LOOP => T::LineLoop,
        backend::GLES_PRIM_POINTS => T::Points,
        _ => return Err(FrameError::Invalid(INVALID_PRIMITIVE)),
    })
}

/// `pan_pipe_to_stencil_op` (Mesa 26.2.1 pan_cmdstream.c:4167-4190) from
/// the GL core's codes (`GLES_SOP_*`, the gallium order).
fn stencil_op(op: backend::GlesStencilOp) -> StencilOp {
    match op {
        backend::GLES_SOP_ZERO => StencilOp::ZERO,
        backend::GLES_SOP_REPLACE => StencilOp::REPLACE,
        backend::GLES_SOP_INCR_SAT => StencilOp::INCR_SAT,
        backend::GLES_SOP_DECR_SAT => StencilOp::DECR_SAT,
        backend::GLES_SOP_INVERT => StencilOp::INVERT,
        backend::GLES_SOP_INCR_WRAP => StencilOp::INCR_WRAP,
        backend::GLES_SOP_DECR_WRAP => StencilOp::DECR_WRAP,
        _ => StencilOp::KEEP,
    }
}

/// The depth and stencil state of a GL draw as Mesa builds it from st/mesa's
/// ZSA (`panfrost_create_depth_stencil_state`, Mesa 26.2.1
/// pan_cmdstream.c:4222-4296): the depth function ALWAYS and no write
/// without the test (ES 2.0 4.1.6), a disabled face all zero, and the
/// reference values as `panfrost_emit_depth_stencil` writes them (:879-882).
/// GL's compare codes (`GLES_CMP_*`, NEVER .. ALWAYS) are the Mali's `Func`
/// values. The back face is GL's back state: st/mesa enables `stencil[1]`
/// only when the two differ, and Mesa then copies the front, which is the
/// same value.
fn depth_stencil_state(d: &GlesDraw) -> DepthStencilState {
    let face = |func, sfail, dpfail, dppass, vmask, wmask, reference| StencilFace {
        func: Func(func),
        fail: stencil_op(sfail),
        depth_fail: stencil_op(dpfail),
        depth_pass: stencil_op(dppass),
        value_mask: vmask,
        write_mask: wmask,
        reference,
    };
    let faces = if d.stencil {
        [
            face(
                d.stencil_func,
                d.stencil_sfail,
                d.stencil_dpfail,
                d.stencil_dppass,
                d.stencil_cmp_mask,
                d.stencil_write_mask,
                d.stencil_ref,
            ),
            face(
                d.stencil_back_func,
                d.stencil_back_sfail,
                d.stencil_back_dpfail,
                d.stencil_back_dppass,
                d.stencil_back_cmp_mask,
                d.stencil_back_write_mask,
                d.stencil_back_ref,
            ),
        ]
    } else {
        [StencilFace::off(d.stencil_ref), StencilFace::off(d.stencil_ref)]
    };
    DepthStencilState {
        depth_func: if d.depth_test { Func(d.depth_func) } else { Func::ALWAYS },
        depth_write: d.depth_test && d.depth_write,
        stencil_test: d.stencil,
        faces,
    }
}

/// Which planes a draw of this state writes: the depth with the test and
/// writes on, the stencil with the test on and a write mask on either face
/// (as `ds_retention::draw_writes`).
fn writes(zs: &DepthStencilState) -> [bool; 2] {
    [zs.depth_write, zs.stencil_test && zs.faces.iter().any(|f| f.write_mask != 0)]
}

/// The two stages' metas of a GL draw, from its shader records.
fn stage_metas(d: &GlesDraw) -> Result<(StageMeta, StageMeta), FrameError> {
    // SAFETY: the GL core's contract: null or the program's live records.
    let (vs, fs) = unsafe { (d.vs.as_ref(), d.fs.as_ref()) };
    let (Some(vs), Some(fs)) = (vs.and_then(MaliShader::of), fs.and_then(MaliShader::of)) else {
        return Err(FrameError::NotBuilt(NO_MALI_PROGRAM));
    };

    Ok((StageMeta::of(vs)?, StageMeta::of(fs)?))
}

/// The arenas of one slot, each at its buffer's GPU VA; the data pool
/// over the slot's data buffer and the chunks grown after it.
struct Arenas<'m> {
    desc: Arena<'m>,
    data: Pool<'m>,
    code: Arena<'m>,
    spds: Arena<'m>,
}

/// Where the arenas of a slot stand.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Tops {
    desc: usize,
    data: PoolTop,
    code: usize,
    spds: usize,
}

impl Tops {
    const ZERO: Tops = Tops { desc: 0, data: PoolTop::ZERO, code: 0, spds: 0 };
}

impl Arenas<'_> {
    fn tops(&self) -> Tops {
        Tops { desc: self.desc.top(), data: self.data.top(), code: self.code.top(), spds: self.spds.top() }
    }

    fn rewind(&mut self, tops: Tops) {
        self.desc.rewind(tops.desc);
        self.data.rewind(tops.data);
        self.code.rewind(tops.code);
        self.spds.rewind(tops.spds);
    }
}

/// The arenas and the stream of slot `slot`, resumed at `tops` and `cs_len`.
///
/// # Safety
/// The slot is the open frame's (or a free one): the GPU does not read it.
unsafe fn slot_arenas<'m>(
    slot: &Slot,
    user_regs: u32,
    tops: Tops,
    cs_len: usize,
) -> Result<(Arenas<'m>, Stream<'m>), FrameError> {
    let mk = |r: usize, top: usize| {
        let m = &slot.mem[r];
        // SAFETY: the caller's contract.
        Arena::resume(REGIONS[r].0, m.gpu, unsafe { bytes_of(m) }, top)
    };
    let chunks = core::array::from_fn(|k| {
        let m = if k == 0 { &slot.mem[1] } else { &slot.data_chunks[k - 1] };
        // SAFETY: the caller's contract; a chunk not grown has no memory.
        (!m.cpu.is_null()).then(|| (m.gpu, unsafe { bytes_of(m) }))
    });
    let a = Arenas {
        desc: mk(0, tops.desc)?,
        data: Pool::resume(REGIONS[1].0, chunks, tops.data)?,
        code: mk(2, tops.code)?,
        spds: mk(3, tops.spds)?,
    };
    // SAFETY: the caller's contract.
    let cs = Stream::resume(unsafe { words_of(&slot.mem[4]) }, cs_len, user_regs)
        .ok_or(FrameError::Invalid(frame::INVALID_OPEN_FRAME))?;
    Ok((a, cs))
}

/// The frame builder's draw of a GL draw: its programs and attributes
/// uploaded into `a`. `shaders` are the draw's vertex and fragment
/// programs; `vs_attributes` the vertex shader's (GL location, record)
/// pairs; `occlusion` the counter of the query that counts it;
/// `blend_shader` the address of target 0's blend shader, when its blend
/// needs one.
#[allow(clippy::too_many_arguments)]
fn add_gl_draw(
    open: &mut frame::OpenFrame,
    d: &GlesDraw,
    shaders: (ShaderInput, ShaderInput),
    vs_attributes: &[(u8, u8)],
    ctx: &frame::Context,
    target: &frame::Target,
    has_zs: bool,
    occlusion: Option<u64>,
    blend_shader: Option<u64>,
    a: &mut Arenas,
    cs: &mut Stream,
) -> Result<(), FrameError> {
    let mut attributes = [frame::VertexAttribute {
        buffer: frame::VertexBuffer { gpu: 0, size: 0 },
        offset: 0,
        stride: 0,
        components: 0,
        instance_divisor: 0,
    }; 16];
    let (vs, fs) = shaders;
    let n = vertex_attributes(d, vs_attributes, &mut a.data, &mut attributes)?;
    let programs = (
        frame::upload_program_cached(&mut open.programs, &mut a.code, &mut a.spds, vs.code, vs.meta)?,
        frame::upload_program_cached(&mut open.programs, &mut a.code, &mut a.spds, fs.code, fs.meta)?,
    );
    let draw = frame::DrawCall {
        vs: &programs.0,
        fs: &programs.1,
        primitive: topology(d.prim)?,
        first_vertex: 0,
        vertex_count: d.index_count,
        instance_count: d.instance_count,
        indices: indices(d, &mut a.data)?,
        attributes: &attributes[..n],
        viewport: frame::Viewport {
            x: d.viewport[0],
            y: d.viewport[1],
            width: d.viewport[2],
            height: d.viewport[3],
            near: d.depth_range[0],
            far: d.depth_range[1],
        },
        scissor: d.scissor.then_some(d.scissor_rect),
        line_width: d.line_width,
        flip_y: d.flip_y,
        blend_color: d.blend_color,
        blend: d.blend.then(|| blending(d)).transpose()?,
        polygon_offset: d.polygon_offset.then_some([d.polygon_offset_factor, d.polygon_offset_units]),
        sample_mask: d.sample_mask,
        vs_uniforms: uniform_words(d.vs_uniform_cpu, d.vs_uniform_dwords),
        fs_uniforms: uniform_words(d.fs_uniform_cpu, d.fs_uniform_dwords),
        zs: depth_stencil_state(d),
        colour_mask: if d.color_write { d.color_mask & 0xf } else { 0 },
        front_cw: d.front_cw,
        cull: cull(d.cull)?,
        texture: sampled_texture(d, false, &mut a.data)?,
        vs_texture: sampled_texture(d, true, &mut a.data)?,
        occlusion_query: occlusion,
        blend_shader,
        /* the GL core's state is already "on and the target multisampled"
         * (backend.h), st/mesa's condition (st_atom_blend.c:339-347) */
        alpha_to_coverage: d.alpha_to_coverage,
    };
    frame::add_draw(open, &mut a.desc, cs, ctx, target, has_zs, &draw)
}

/// The GL core's binding of the sampler the draw's fragment shader (or,
/// `vertex`, its vertex shader) reads ([`MaliShader::sampler`];
/// `GlesTextureBind::unit` is the sampler binding, the stage's own binds
/// `textures` or `vs_textures`): None for a shader that samples nothing,
/// Some(None) when the GL core handed no texture for it (no texture
/// object's storage on the unit: GL's texture with no texels, the
/// backend's to bind).
fn sampled_bind(d: &GlesDraw, vertex: bool) -> Option<Option<&backend::GlesTextureBind>> {
    let (sh, binds, n) =
        if vertex { (d.vs, &d.vs_textures[..], d.num_vs_textures) } else { (d.fs, &d.textures[..], d.num_textures) };
    // SAFETY: the draw's records are null or live past the draw
    // (`programs`).
    let binding = unsafe { sh.as_ref() }.and_then(MaliShader::of).and_then(|r| r.sampler)?;
    let binds = &binds[..usize::from(n).min(binds.len())];
    Some(binds.iter().find(|t| t.unit == binding))
}

/// The binds both stages read, the vertex stage's first.
fn sampled_binds(d: &GlesDraw) -> impl Iterator<Item = &backend::GlesTextureBind> {
    [true, false].into_iter().filter_map(|v| sampled_bind(d, v).flatten())
}

/// How the texture unit reads one of the GL core's texel formats
/// (`pan_format.c`, Mesa 26.2.1: on Valhall a block format is the
/// interchange format in the texture descriptor and its compression in the
/// plane's clump format, :162-180, pan_texture.c:191-245).
#[derive(Clone, Copy)]
struct TexelFormat {
    /// The texture descriptor's format word (`MALI_PACK_FMT`: the format,
    /// the RGB component order, sRGB at bit 20).
    format: u32,
    clump: ClumpFormat,
    /// The swizzle of the view (`pan_translate_swizzle_4` of the format's).
    swizzle: u32,
    /// 4x4 blocks of this many bytes; 0: 4-byte texels.
    block_bytes: u32,
}

/// The formats this backend samples (Mesa 26.2.1 pan_format.c:231-241, 508,
/// 528); None for any other.
fn texel_format(texfmt: u8) -> Option<TexelFormat> {
    use backend::{GLES_TEXFMT_ETC2_RGB, GLES_TEXFMT_ETC2_RGBA, GLES_TEXFMT_ETC2_RGB_PT, GLES_TEXFMT_RGBA8};
    const SRGB: u32 = 1 << 20;
    let rgba8 = |order: RgbComponentOrder| Format::RGBA8_UNORM.0 << 12 | order.0;
    let raw = |srgb: u32| TexelFormat {
        format: rgba8(RgbComponentOrder::RGBA) | srgb,
        clump: ClumpFormat::RAW32,
        swizzle: frame::SWIZZLE_RGBA,
        block_bytes: 0,
    };
    Some(match texfmt {
        GLES_TEXFMT_RGBA8 => raw(0),
        backend::GLES_TEXFMT_SRGB8_A8 => raw(SRGB),
        /* the GL core's storage of every depth texture (texture.rs: R32F
         * texels the CPU resolve writes, `mali_depth_resolve`), and ES
         * 3.0's GL_R32F: Mesa's R32_FLOAT, `R32F` in the R001 order, which
         * v9+ spells RGB1 (pan_format.c:451, pan_format.h:306-307); the
         * sampler's view then reads (r, 0, 0, 1) through the identity
         * swizzle, or (d, d, d, 1) through [`frame::SWIZZLE_RRR1`] */
        backend::GLES_TEXFMT_R32F => TexelFormat {
            format: Format::R32F.0 << 12 | RgbComponentOrder::RGB1.0,
            clump: ClumpFormat::RAW32,
            swizzle: frame::SWIZZLE_RGBA,
            block_bytes: 0,
        },
        /* ETC1 is ETC2 RGB8's subset: the GL core keeps its blocks
         * (`etc1_decoded_at_upload` false) */
        GLES_TEXFMT_ETC2_RGB => TexelFormat {
            format: rgba8(RgbComponentOrder::RGB1),
            clump: ClumpFormat::ETC2_RGB8,
            swizzle: frame::SWIZZLE_RGB1,
            block_bytes: 8,
        },
        GLES_TEXFMT_ETC2_RGBA => TexelFormat {
            format: rgba8(RgbComponentOrder::RGBA),
            clump: ClumpFormat::ETC2_RGBA8,
            swizzle: frame::SWIZZLE_RGBA,
            block_bytes: 16,
        },
        GLES_TEXFMT_ETC2_RGB_PT => TexelFormat {
            format: rgba8(RgbComponentOrder::RGBA),
            clump: ClumpFormat::ETC2_RGB8A1,
            swizzle: frame::SWIZZLE_RGBA,
            block_bytes: 8,
        },
        _ => return None,
    })
}

/// Where level `l` lies in the GL core's storage: its offset, its row pitch,
/// its rows (of texels, or of 4x4 blocks) per face and the bytes of one row
/// it uses. With the Mali's `mip_level_align` (64) the GL core lays a chain
/// out with every level on a 64-byte boundary and allocates the whole
/// chain: a power-of-two chain of texels with one row pitch, level 0's
/// (`chain_layout_npot`), a block chain as raster rows of its own blocks a
/// level (`compressed_blocks_in_rows`, `compressed_chain_layout`; src/gl/
/// texture.rs). A cube map's level is its six faces stacked, `rows` each,
/// on that pitch (level-major, face-minor: `chain_layout_npot`'s "level i
/// is `layers` faces", src/gl/texture.rs; docs/notes/mali-cube-textures.md
/// (c)), so a level takes `pitch * rows * layers` bytes.
#[derive(Clone, Copy)]
struct LevelAt {
    off: usize,
    pitch: usize,
    rows: usize,
    /// The bytes of a row the level's texels (or blocks) take.
    row_bytes: usize,
}

/// The images stacked in a binding: 6 for a cube, 1 otherwise.
fn layers_of(t: &backend::GlesTextureBind) -> usize {
    usize::from(t.layers.max(1))
}

fn level_at(t: &backend::GlesTextureBind, f: &TexelFormat, l: u32) -> Option<LevelAt> {
    let mut off = 0usize;
    let layers = layers_of(t);
    /* the GL core hands the stack as one image: `height` is every face's */
    let face_h = (t.height as usize).checked_div(layers)? as u32;
    for k in 0..=l {
        off = off.checked_next_multiple_of(MIP_LEVEL_ALIGN as usize)?;
        let (w, h) = ((t.width >> k).max(1) as usize, (face_h >> k).max(1) as usize);
        let at = if f.block_bytes == 0 {
            let pitch = (t.stride_texels as usize).checked_mul(4)?;
            LevelAt { off, pitch, rows: h, row_bytes: w.checked_mul(4)? }
        } else {
            /* the GL core's raster rows of blocks, each on the linear row
             * pitch (`block_grid`, src/gl/texture.rs) */
            let blocks = crate::gl::context::row_aligned(w.div_ceil(4) as u32, f.block_bytes, TEXTURE_ROW_ALIGN);
            let pitch = (blocks as usize).checked_mul(f.block_bytes as usize)?;
            let row_bytes = w.div_ceil(4).checked_mul(f.block_bytes as usize)?;
            LevelAt { off, pitch, rows: h.div_ceil(4), row_bytes }
        };
        if k == l {
            return Some(at);
        }
        off = off.checked_add(at.pitch.checked_mul(at.rows)?.checked_mul(layers)?)?;
    }
    None
}

/// The levels a binding's descriptor names.
fn levels_of(t: &backend::GlesTextureBind) -> u32 {
    u32::from(t.mip_levels.max(1))
}

/// Whether the texture unit cannot read the texture's storage in place: a
/// level's rows are not a multiple of [`TEXTURE_ROW_ALIGN`] bytes, which
/// only an imported image's can be (the GL core lays its own storage out on
/// them). A format this backend does not sample is never copied (it is
/// refused).
fn needs_copy(t: &backend::GlesTextureBind) -> bool {
    let Some(f) = texel_format(t.format) else { return false };
    (0..levels_of(t)).any(|l| level_at(t, &f, l).is_none_or(|a| a.pitch % TEXTURE_ROW_ALIGN as usize != 0))
}

/// The end of the storage a copy reads: the last level's (every face's).
fn storage_end(t: &backend::GlesTextureBind) -> Option<usize> {
    let f = texel_format(t.format)?;
    let a = level_at(t, &f, levels_of(t) - 1)?;
    a.off.checked_add(a.pitch.checked_mul(a.rows)?.checked_mul(layers_of(t))?)
}

/// Before a texture is copied, the GPU is done writing it: a render into it
/// in a kick still running is waited for (the open frame's own target is a
/// feedback loop, refused), and the CPU's view is invalidated.
fn texture_ready(g: &mut MaliState, d: &GlesDraw) -> Result<(), Fail> {
    for t in sampled_binds(d).filter(|t| t.incomplete == 0 && needs_copy(t)) {
        texture_ready_one(g, t)?;
    }
    Ok(())
}

fn texture_ready_one(g: &mut MaliState, t: &backend::GlesTextureBind) -> Result<(), Fail> {
    if t.target_kick == 0 {
        return Ok(());
    }
    if t.target_kick == mali_kick_id() {
        return Err(FrameError::NotBuilt(NOT_BUILT_TEXTURE_FEEDBACK).into());
    }
    if let Some(i) = g.slots.iter().position(|s| s.in_flight.is_some_and(|f| f.kick == t.target_kick)) {
        wait_slot(g, i).map_err(|(rc, _)| Fail::Errno(rc))?;
    }
    /* the whole chain the copy reads (`sampled_texture`), level 0 at least */
    let end = storage_end(t).unwrap_or(0).max(t.bytes as usize);
    // SAFETY: the GL core's CPU view of the texture: level 0's `bytes` and
    // the chain laid out after it (`level_at`).
    unsafe { osal::cache_invd(t.cpu.cast_mut(), end) };
    Ok(())
}

/// GL's texture with no texels (ES 2.0 3.8.2: a sampler of an incomplete
/// texture, or of a unit with no texture object's storage, reads (0, 0, 0,
/// 1)): one opaque black texel this backend binds instead, as st/mesa binds
/// its own fallback texture.
const BLACK_TEXEL: [u8; 4] = [0, 0, 0, 0xff];

/// The texture the fragment (or, `vertex`, the vertex) shader's look-up
/// reads ([`sampled_bind`]): its
/// levels in place, at the GL core's linear row pitch, or, for an imported
/// image whose rows are not [`TEXTURE_ROW_ALIGN`] bytes, each copied with
/// such rows into the frame's `data` arena ([`texture_ready`] made the
/// storage current); a sampler with no texels reads [`BLACK_TEXEL`]. What
/// this backend does not sample yet is refused by name.
fn sampled_texture(d: &GlesDraw, vertex: bool, data: &mut Pool) -> Result<Option<frame::SampledTexture>, FrameError> {
    let not_built = |why| Err(FrameError::NotBuilt(why));
    let wrap = |w: backend::GlesWrap| match w {
        backend::GLES_WRAP_REPEAT => WrapMode::REPEAT,
        backend::GLES_WRAP_MIRRORED_REPEAT => WrapMode::MIRRORED_REPEAT,
        _ => WrapMode::CLAMP_TO_EDGE,
    };
    let Some(bind) = sampled_bind(d, vertex) else { return Ok(None) };
    let t = match bind {
        Some(t) if t.incomplete == 0 => t,
        /* no texels: one black texel, the GL state's wraps and filters */
        _ => {
            let b = data.upload(&BLACK_TEXEL, u64::from(TEXTURE_ROW_ALIGN))?;
            let mut level = [frame::TextureLevel::default(); frame::MAX_TEXTURE_LEVELS];
            level[0] = frame::TextureLevel { gpu: b.gpu, row_stride: TEXTURE_ROW_ALIGN };
            let f = texel_format(backend::GLES_TEXFMT_RGBA8).ok_or(FrameError::Invalid(INVALID_TEXTURE_STORAGE))?;
            return Ok(Some(frame::SampledTexture {
                width: 1,
                height: 1,
                level,
                levels: 1,
                faces: 1,
                format: f.format,
                clump: f.clump,
                block_bytes: 0,
                swizzle: f.swizzle,
                min_nearest: bind.is_none_or(|t| t.min_linear == 0),
                mag_nearest: bind.is_none_or(|t| t.linear_filter == 0),
                mipmap: MipmapMode::NONE,
                wrap_s: bind.map_or(WrapMode::REPEAT, |t| wrap(t.wrap_s)),
                wrap_t: bind.map_or(WrapMode::REPEAT, |t| wrap(t.wrap_t)),
            }));
        }
    };
    let levels = levels_of(t);
    /* a cube map is six faces stacked in one image, `height` every face's
     * (`native_layered_textures` is false here, src/gl/draw.rs); a 2D array
     * and a 3D texture come the same way and are not described yet */
    let layers = layers_of(t);
    let cube = t.type_ == backend::GLES_TEXTYPE_CUBE && layers == 6;
    let face_h = t.height / layers as u32;
    let pot = t.width.is_power_of_two() && face_h.is_power_of_two();
    let f = texel_format(t.format);
    let checks = [
        (!(t.type_ == backend::GLES_TEXTYPE_2D && layers == 1 || cube), NOT_BUILT_TEXTURE_TYPE),
        (f.is_none(), NOT_BUILT_TEXTURE_FORMAT),
        (levels > 1 && !pot && f.is_some_and(|f| f.block_bytes == 0), NOT_BUILT_TEXTURE_NPOT_CHAIN),
        (levels as usize > frame::MAX_TEXTURE_LEVELS, frame::NOT_BUILT_TEXTURE_SIZE),
    ];
    if let Some(&(_, why)) = checks.iter().find(|(on, _)| *on) {
        return not_built(why);
    }
    if t.height % layers as u32 != 0 {
        return Err(FrameError::Invalid(INVALID_TEXTURE_STORAGE));
    }
    let f = f.ok_or(FrameError::NotBuilt(NOT_BUILT_TEXTURE_FORMAT))?;
    let too_big = FrameError::NotBuilt(frame::NOT_BUILT_TEXTURE_SIZE);
    let mut level = [frame::TextureLevel::default(); frame::MAX_TEXTURE_LEVELS];
    if !needs_copy(t) {
        for (l, lv) in level.iter_mut().enumerate().take(levels as usize) {
            let a = level_at(t, &f, l as u32).ok_or(too_big)?;
            *lv = frame::TextureLevel { gpu: t.base_gpu + a.off as u64, row_stride: a.pitch as u32 };
        }
    } else {
        let end = storage_end(t).ok_or(too_big)?;
        /* level 0 is what `bytes` names; the levels after it are the chain
         * the GL core allocated with it (`level_at`) */
        let a0 = level_at(t, &f, 0).ok_or(too_big)?;
        if t.cpu.is_null() || (t.bytes as usize) < a0.pitch * a0.rows * layers || a0.row_bytes > a0.pitch {
            return Err(FrameError::Invalid(INVALID_TEXTURE_STORAGE));
        }
        // SAFETY: the GL core's CPU view of the storage: level 0's `bytes`
        // and, for a chain, the levels laid out after it in the same
        // allocation (`level_at`), live until the draw returns.
        let texels = unsafe { core::slice::from_raw_parts(t.cpu.cast::<u8>(), end) };
        for (l, lv) in level.iter_mut().enumerate().take(levels as usize) {
            let a = level_at(t, &f, l as u32).ok_or(too_big)?;
            let dst = a.row_bytes.next_multiple_of(TEXTURE_ROW_ALIGN as usize);
            /* the level's faces, each its `rows` on the new pitch, so a
             * face still starts at `face * rows` rows (`face_gpu`) */
            let rows = a.rows * layers;
            let b = data.alloc(dst * rows, u64::from(TEXTURE_ROW_ALIGN))?;
            for y in 0..rows {
                let at = a.off + y * a.pitch;
                data.write(b, y * dst, &texels[at..at + a.row_bytes])?;
            }
            *lv = frame::TextureLevel { gpu: b.gpu, row_stride: dst as u32 };
        }
    }
    /* a view of storage without alpha, or in BGRA order, reads it through
     * the format's own swizzle composed with these; a depth texture of the
     * ES 2.0 surface reads (d, d, d, 1), GL_OES_depth_texture's luminance
     * form, which Mesa composes over .XXXX for a depth view (pan_cmdstream.c
     * `panfrost_create_sampler_view_bo`; the board's `Swizzle: 2560 (RRR1)`
     * for case 169, tools/mali/board-dump/dstex-2026-10-20.txt) */
    let swizzle = match (f.block_bytes != 0, t.depth_luminance != 0, t.bgra != 0, t.no_alpha != 0) {
        (true, _, _, _) => f.swizzle,
        (false, true, _, _) => frame::SWIZZLE_RRR1,
        (false, false, false, false) => frame::SWIZZLE_RGBA,
        (false, false, false, true) => frame::SWIZZLE_RGB1,
        (false, false, true, false) => frame::SWIZZLE_BGRA,
        (false, false, true, true) => frame::SWIZZLE_BGR1,
    };
    let mipmap = match t.mip_filter {
        0 => MipmapMode::NONE,
        1 => MipmapMode::NEAREST,
        _ => MipmapMode::TRILINEAR,
    };
    Ok(Some(frame::SampledTexture {
        width: t.width,
        height: face_h,
        level,
        levels,
        faces: layers as u32,
        format: f.format,
        clump: f.clump,
        block_bytes: f.block_bytes,
        swizzle,
        min_nearest: t.min_linear == 0,
        mag_nearest: t.linear_filter == 0,
        mipmap,
        wrap_s: wrap(t.wrap_s),
        wrap_t: wrap(t.wrap_t),
    }))
}

/// An indexed draw's u16 indices where the frame reads them: the GL core's
/// staging copy (`index_cpu`, one scratch every draw reuses) copied into
/// the frame's data arena, as Mesa copies user indices into the batch's
/// pool (`panfrost_get_index_buffer`, Mesa 26.2.1 pan_helpers.c:92-99), or
/// an element buffer read in place (pan_helpers.c:88-91; the core keeps it
/// until the kick that reads it completes, `last_use`). None for a
/// non-indexed draw.
fn indices(d: &GlesDraw, data: &mut Pool) -> Result<Option<frame::Indices>, FrameError> {
    if d.index_buf_gpu == 0 && d.index_cpu.is_null() {
        return Ok(None);
    }
    if d.index_size != 2 {
        return Err(FrameError::Invalid(INVALID_INDEX_SIZE));
    }
    if !d.index_cpu.is_null() {
        if u64::from(d.index_bytes) != 2 * u64::from(d.index_count) {
            return Err(FrameError::Invalid(INVALID_INDEX_SIZE));
        }
        // SAFETY: the GL core's contract for `index_cpu`: `index_bytes`
        // readable bytes that live until the draw returns.
        let bytes = unsafe { core::slice::from_raw_parts(d.index_cpu.cast::<u8>(), d.index_bytes as usize) };
        let b = data.upload(bytes, 16)?;
        return Ok(Some(frame::Indices { gpu: b.gpu }));
    }
    Ok(Some(frame::Indices { gpu: d.index_buf_gpu }))
}

/// One stage's GL uniform storage as the draw hands it: `dwords` words at
/// `cpu`, or none.
fn uniform_words<'a>(cpu: *const c_void, dwords: u16) -> &'a [u32] {
    if cpu.is_null() || dwords == 0 {
        return &[];
    }
    // SAFETY: the GL core's contract for the draw's uniform blocks:
    // `dwords` readable words that live until the draw returns.
    unsafe { core::slice::from_raw_parts(cpu.cast::<u32>(), usize::from(dwords)) }
}

/// The draw's attributes in the vertex shader's record order, float32
/// each: data the GL core staged for this draw (`GlesAttrib::cpu`), copied
/// into the frame's data arena; or a buffer object fetched in place, the
/// Buffer descriptor its whole store (`buffer_gpu`, `buffer_bytes`: Mesa's
/// address and `width0 - buffer_offset`, `panfrost_emit_vertex_buffers`,
/// Mesa 26.2.1 pan_cmdstream.c:869-900) and the attribute its offset in it
/// (the GL core folds the first vertex in). The OSAL keeps a deleted
/// buffer's store until the kick being recorded has completed
/// (`gpu_free`'s contract), so the frame may read it in place. Returns how
/// many.
fn vertex_attributes(
    d: &GlesDraw,
    vs_attributes: &[(u8, u8)],
    data: &mut Pool,
    out: &mut [frame::VertexAttribute; 16],
) -> Result<usize, FrameError> {
    let n = vs_attributes.len();
    if n == 0 || n > out.len() {
        return Err(FrameError::NotBuilt(frame::NOT_BUILT_ATTRIBUTES));
    }
    let staged = &d.attribs[..usize::from(d.num_attribs).min(d.attribs.len())];
    for rec in 0..n {
        /* record `rec` is read by exactly one location */
        let mut it = vs_attributes.iter().filter(|(_, r)| usize::from(*r) == rec);
        let (Some(&(loc, _)), None) = (it.next(), it.next()) else {
            return Err(FrameError::NotBuilt(NOT_BUILT_ATTRIBUTE_RECORDS));
        };
        let a = staged.iter().find(|a| a.vi_reg == loc).ok_or(FrameError::NotBuilt(NOT_BUILT_ATTRIBUTE_RECORDS))?;
        if a.format != GLES_ATTRIBF_FLOAT32 {
            return Err(FrameError::NotBuilt(NOT_BUILT_ATTRIBUTE));
        }
        let (buffer, offset) = if !a.cpu.is_null() && a.cpu_bytes != 0 {
            // SAFETY: the GL core's contract for `cpu`: `cpu_bytes` readable
            // bytes that live until the draw returns.
            let bytes = unsafe { core::slice::from_raw_parts(a.cpu.cast::<u8>(), a.cpu_bytes as usize) };
            let b = data.upload(bytes, 16)?;
            (frame::VertexBuffer { gpu: b.gpu, size: a.cpu_bytes }, 0)
        } else if a.buffer_gpu != 0 && a.buffer_bytes != 0 {
            let size =
                u32::try_from(a.buffer_bytes).map_err(|_| FrameError::NotBuilt(NOT_BUILT_ATTRIBUTE_BUFFER_SIZE))?;
            (frame::VertexBuffer { gpu: a.buffer_gpu, size }, a.offset)
        } else {
            return Err(FrameError::NotBuilt(NOT_BUILT_ATTRIBUTE));
        };
        /* one element per `divisor` instances, 0 read as 1 (`GlesAttrib`);
         * the frame builds divisor 1, the one the core hands the Mali
         * (`max_instance_divisor` 0) */
        let instance_divisor = if a.per_instance != 0 { u32::from(a.divisor.max(1)) } else { 0 };
        out[rec] = frame::VertexAttribute {
            buffer,
            offset,
            stride: a.stride,
            components: u32::from(a.components),
            instance_divisor,
        };
    }
    Ok(n)
}

/// A draw's two programs as the frame builder takes them, and the vertex
/// shader's (GL location, attribute record) pairs.
type Programs<'a> = ((ShaderInput<'a>, ShaderInput<'a>), &'a [(u8, u8)]);

/// The programs of a GL draw (or of the clear quad), from its shader records.
fn programs<'a>(d: &GlesDraw, metas: &'a mut Option<(StageMeta, StageMeta)>) -> Result<Programs<'a>, FrameError> {
    let (vm, fm) = metas.insert(stage_metas(d)?);
    // SAFETY: stage_metas found both records non-null; their code and
    // their records live in the compiler's keep arena while the program
    // holds them, which is past this draw.
    let (vc, fc, attrs) = unsafe {
        let attrs = MaliShader::of(&*d.vs).map_or(&[][..], |r| r.attributes);
        (program::code_bytes(&*d.vs)?, program::code_bytes(&*d.fs)?, attrs)
    };
    Ok(((ShaderInput { code: vc, meta: vm.meta() }, ShaderInput { code: fc, meta: fm.meta() }), attrs))
}

/* ---- depth/stencil buffers and planes ------------------------------------ */

/// Record what object `id`'s `aspect` plane holds. An undefined plane is
/// no entry. `Err` when every entry holds a plane GL keeps.
fn set_plane(g: &mut MaliState, id: u32, gen: u32, aspect: usize, plane: Plane, ds_id: u32) -> Result<(), ()> {
    let at = g.planes.iter().position(|p| p.id == id && p.aspect == aspect);
    if plane == Plane::UNDEFINED {
        if let Some(i) = at {
            g.planes[i] = NO_PLANE;
        }
        return Ok(());
    }
    let i = at.or_else(|| g.planes.iter().position(|p| p.id == 0)).ok_or(())?;
    g.planes[i] = PlaneEntry { id, gen, aspect, plane, ds_id };
    Ok(())
}

/// Where the plane of the object at attachment point `aspect` of `key`
/// starts in a frame.
fn start_of(g: &MaliState, key: &TargetKey, aspect: usize) -> Start {
    let (id, gen) = key.objects[aspect];
    if id == 0 {
        return Start::Absent;
    }
    let Some(e) = g.planes.iter().find(|p| p.id == id && p.aspect == aspect && p.gen == gen) else {
        return Start::Undefined;
    };
    match e.plane.cleared_value() {
        Some(v) => Start::Value(v),
        None if e.plane.is_live() && e.ds_id == key.ds_id => Start::Unloaded,
        None if e.plane.is_live() => Start::Elsewhere,
        None => Start::Undefined,
    }
}

/// The depth/stencil buffer of the attachment pair of `key`, made on first
/// use: `Err` is the draw's errno (`ds_retention`'s refusals, `-ENOMEM`).
fn zs_buffer(g: &mut MaliState, key: &TargetKey) -> Result<usize, Fail> {
    let who = key.objects[0].0 | key.objects[1].0;
    if let Some(i) = g.zsbufs.iter().position(|z| z.ds_id == key.ds_id && z.w == key.ds_w && z.h == key.ds_h) {
        return Ok(i);
    }
    let bytes = 4 * key.ds_w as usize * key.ds_h as usize;
    /* a free entry whose memory holds it, else one with none. An entry is
     * free when it holds nothing GL keeps (ds_retention): no live plane is
     * in its pair's buffer and the open frame does not use it, so a pair
     * whose objects are gone gives its buffer back */
    let open_pair = g.open.map(|o| o.key.ds_id);
    let planes = g.planes;
    let free = |z: &ZsBuf| {
        z.ds_id == 0 || (Some(z.ds_id) != open_pair && !planes.iter().any(|p| p.ds_id == z.ds_id && p.plane.is_live()))
    };
    let i = g
        .zsbufs
        .iter()
        .position(|z| free(z) && !z.mem.cpu.is_null() && z.mem.size >= bytes)
        .or_else(|| g.zsbufs.iter().position(|z| free(z) && z.mem.cpu.is_null()))
        .ok_or_else(|| Fail::Errno(ds_retention::refuse("mali", who, ds_retention::DS_SLOTS)))?;
    if g.zsbufs[i].mem.cpu.is_null() {
        let mut mem = OsalGpuMem::ZERO;
        if alloc(bytes, OSAL_MEM_DATA, &mut mem).is_err() {
            return Err(Fail::Errno(ds_retention::refuse_no_memory("mali", who, bytes)));
        }
        g.zsbufs[i].mem = mem;
    }
    /* the old pair, if the size changed, gives its entry up */
    for z in g.zsbufs.iter_mut().filter(|z| z.ds_id == key.ds_id) {
        z.ds_id = 0;
    }
    g.zsbufs[i] = ZsBuf { ds_id: key.ds_id, w: key.ds_w, h: key.ds_h, mem: g.zsbufs[i].mem };
    let held = g.zsbufs.iter().filter(|z| !free(z) || z.ds_id == key.ds_id).count();
    if held > g.zs_high_water {
        g.zs_high_water = held;
        glog!("mali: depth/stencil buffers: high-water {} of {}", held, ds_retention::DS_SLOTS);
    }
    ds_retention::admitted();
    Ok(i)
}

/// Write depth texels `t` (a draw's `GlesDraw::depth_texels`: the CPU wrote
/// the depth texture since its plane matched it) into the depth/stencil
/// buffer of `key`'s attachment pair, and record the depth plane as live
/// in it: the next frame of the pair preloads it (`Start::Unloaded`), so
/// the draw tests against the texels. No frame is open (the caller ended
/// it).
///
/// The layout is the one `mali_depth_resolve` reads, written the other
/// way: linear D24S8 at `4 * w` bytes a row, depth in the low 24 bits
/// ([`ds_retention::depth_unorm24`]), each word's stencil byte kept, row 0
/// at GL's y 0. Every submitted frame is waited for first: the pair may be
/// given a buffer whose earlier pair's frame is still writing it back
/// (`ZsBuf`: memory moves between pairs while frames are in flight).
///
/// `Err` is the draw's errno: the buffer's (`zs_buffer`), a wait that
/// failed, `-EINVAL` for a buffer shorter than `4 * w * h` bytes or with no
/// CPU view, or every plane entry holding a plane GL keeps.
fn seed_depth(g: &mut MaliState, key: &TargetKey, t: ds_retention::DepthTexels) -> Result<(), Fail> {
    let (id, gen) = key.objects[ds_retention::DEPTH];
    if let Err((rc, why)) = wait_all(g) {
        backend::note_work_lost(why, rc);
        return Err(Fail::Errno(rc));
    }
    let buf = zs_buffer(g, key)?;
    let z = g.zsbufs[buf];
    let (zw, zh) = (z.w as usize, z.h as usize);
    let bytes = zw.saturating_mul(zh).saturating_mul(4);
    if z.mem.cpu.is_null() || bytes == 0 || bytes > z.mem.size {
        return Err(Fail::Errno(-EINVAL));
    }
    // SAFETY: a live, CPU-mapped allocation of at least `bytes` (checked
    // above), which no frame writes (every one was waited for, none open).
    let words = unsafe {
        osal::cache_invd(z.mem.cpu, bytes);
        core::slice::from_raw_parts_mut(z.mem.cpu as *mut u32, zw * zh)
    };
    let cols = zw.min(t.w as usize);
    for (y, row) in words.chunks_exact_mut(zw).take(t.h as usize).enumerate() {
        for (x, w) in row[..cols].iter_mut().enumerate() {
            *w = (*w & 0xff00_0000) | t.unorm24(x as u32, y as u32);
        }
    }
    // SAFETY: the words just written, inside the allocation.
    unsafe { osal::cache_flush(z.mem.cpu, bytes) };
    set_plane(g, id, gen, ds_retention::DEPTH, Plane::LIVE, key.ds_id)
        .map_err(|()| Fail::Errno(ds_retention::refuse("mali", id, PLANES)))
}

/* ---- the open frame ------------------------------------------------------ */

impl TargetKey {
    fn of(d: &GlesDraw, row_stride: u32, format: frame::TargetFormat) -> TargetKey {
        TargetKey {
            gpu: d.target_gpu,
            w: d.target_w,
            h: d.target_h,
            row_stride,
            format,
            samples: d.samples.max(1),
            ds_id: d.ds_id,
            objects: [(d.depth_id, d.depth_gen), (d.stencil_id, d.stencil_gen)],
            ds_w: d.ds_w,
            ds_h: d.ds_h,
        }
    }

    fn has_zs(&self) -> bool {
        self.objects[0].0 != 0 || self.objects[1].0 != 0
    }
}

/// The index in `MaliState::zs_preload` of the program that preloads the
/// depth (`z`), the stencil (`s`) or both; None for neither.
fn zs_variant(z: bool, s: bool) -> Option<usize> {
    match (z, s) {
        (true, false) => Some(0),
        (false, true) => Some(1),
        (true, true) => Some(2),
        (false, false) => None,
    }
}

/// A preload program in the frame's code arena, on 128 bytes as every
/// program starts. Returns its address.
fn upload_preload(code: &mut Arena, p: &PreloadProgram) -> Result<u64, FrameError> {
    let mut bytes = [0u8; 8 * PRELOAD_WORDS];
    for (k, w) in p.words[..p.n].iter().enumerate() {
        bytes[8 * k..8 * k + 8].copy_from_slice(&w.to_le_bytes());
    }
    Ok(code.upload(&bytes[..8 * p.n], 128)?.gpu)
}

/// Open a frame in the next slot for `target`: cleared to `clear`, or (None) starting from its
/// old image, which the colour preload program draws back.
fn open_frame(g: &mut MaliState, key: TargetKey, target: frame::Target, clear: Option<[f32; 4]>) -> Result<(), Fail> {
    if clear.is_none() && g.preload.n == 0 {
        return Err(FrameError::NotBuilt(NOT_BUILT_PRELOAD_PROGRAM).into());
    }
    /* refused here, at the draw, rather than when the frame ends */
    if clear.is_none() && !target.row_stride.is_multiple_of(64) {
        return Err(FrameError::NotBuilt(frame::NOT_BUILT_PRELOAD_STRIDE).into());
    }
    let i = g.next;
    if let Err((rc, why)) = wait_slot(g, i) {
        backend::note_work_lost(why, rc);
        return Err(Fail::Errno(rc));
    }
    // SAFETY: the slot is free of the GPU since the wait above.
    let (mut a, mut cs) = unsafe { slot_arenas(&g.slots[i], g.user_regs, Tops::ZERO, 0) }?;
    let mut f = frame::begin_frame(&mut a.desc, &mut cs)?;
    if clear.is_none() {
        let bin = upload_preload(&mut a.code, &g.preload)?;
        f.colour_preload = Some(frame::FrameShader { binary: bin, preload: g.preload.preload });
    }
    let clear = clear.unwrap_or([0.0; 4]);
    let zs = if key.has_zs() {
        Some(OpenZs { buf: usize::MAX, start: [start_of(g, &key, 0), start_of(g, &key, 1)], written: [false; 2] })
    } else {
        None
    };
    g.open = Some(OpenFrame { slot: i, tops: a.tops(), cs_len: cs.words().len(), frame: f, key, target, clear, zs });
    Ok(())
}

/// End the open frame, if any, and submit it. A frame that cannot be ended
/// or submitted loses its draws (counted, [`FRAME_LOST`]).
fn end_open(g: &mut MaliState) -> i32 {
    let Some(o) = g.open.take() else {
        return 0;
    };
    let Some(ctx) = g.ctx else {
        return -ENODEV;
    };
    let lost = |rc: i32| {
        backend::note_work_lost(FRAME_LOST, rc);
        rc
    };
    let zs = match o.zs {
        Some(z) if o.frame.uses_zs => {
            let buf = &g.zsbufs[z.buf];
            /* a preloaded frame was never cleared: Mesa's batch clear
             * values stay 0 (the board's preload_zs.c dump: Z Clear 0, S
             * Clear 0) */
            let preloaded = o.frame.zs_preload.is_some();
            let value = |s: Start, default: u32| match s {
                Start::Value(v) => v,
                _ if preloaded => 0,
                _ => default,
            };
            Some(frame::ZsAttachment {
                gpu: buf.mem.gpu,
                row_stride: 4 * buf.w,
                clear_depth: f32::from_bits(value(z.start[0], 1.0f32.to_bits())),
                clear_stencil: value(z.start[1], 0) as u8,
            })
        }
        _ => None,
    };
    let slot = g.slots[o.slot];
    // SAFETY: the open frame's slot, which the GPU does not read until the
    // submit below.
    let built = unsafe { slot_arenas(&slot, g.user_regs, o.tops, o.cs_len) }.and_then(|(mut a, mut cs)| {
        frame::end_frame(&o.frame, &mut a.desc, &mut cs, &ctx, &o.target, o.clear, zs)?;
        Ok((a.tops(), cs.words().len()))
    });
    let (tops, words) = match built {
        Ok(t) => t,
        Err(e) => return lost(frame_error(e)),
    };
    let used = [tops.desc, tops.data.tops[0], tops.code, tops.spds, 8 * words];
    if g.row_copies.is_power_of_two() && g.row_copies != g.row_copies_noted {
        g.row_copies_noted = g.row_copies;
        glog!("mali: draws that sampled a copy of an imported image's rows: {}", g.row_copies);
    }
    for (r, &(label, size, _)) in REGIONS.iter().enumerate() {
        /* the data region's line counts its grown chunks too */
        let (now, capacity) = match r {
            1 => (tops.data.used(), size + slot.data_chunks.iter().map(|m| m.size).sum::<usize>()),
            _ => (used[r], size),
        };
        note_high_water(&mut g.high_water[r], label, now, capacity);
        // SAFETY: the slot's buffer; `used` is within it.
        unsafe { osal::cache_flush(slot.mem[r].cpu, used[r]) };
    }
    for (m, &n) in slot.data_chunks.iter().zip(&tops.data.tops[1..]) {
        if n != 0 {
            // SAFETY: a grown data chunk of the slot; its top is within it.
            unsafe { osal::cache_flush(m.cpu, n) };
        }
    }
    let fence = match submit(slot.mem[4].gpu, words) {
        Ok(f) => f,
        Err(rc) => return lost(rc),
    };
    g.kicks += 1;
    g.slots[o.slot].in_flight = Some(InFlight { kick: g.kicks, fence, wait_failed: false });
    g.next = (o.slot + 1) % SLOTS;
    /* what each plane holds after the frame: drawn into, live; otherwise
     * what it started from (a clear's value, or nothing GL keeps) */
    if let Some(z) = o.zs {
        for aspect in 0..2 {
            let (id, gen) = o.key.objects[aspect];
            let plane = match (z.written[aspect], z.start[aspect]) {
                (_, Start::Absent) | (false, Start::Elsewhere) => continue,
                (true, _) | (false, Start::Unloaded) => Plane::LIVE,
                (false, Start::Value(v)) => Plane::cleared(v),
                (false, Start::Undefined) => Plane::UNDEFINED,
            };
            if set_plane(g, id, gen, aspect, plane, o.key.ds_id).is_err() {
                /* the table is full of planes GL keeps: this one's contents
                 * are in its buffer, but which frame may read them is no
                 * longer known */
                glog_once!(
                    "yakogl/mali: {} for object 0x{:x}: all {} plane entries hold contents GL keeps",
                    ds_retention::REFUSED,
                    id,
                    PLANES
                );
            }
        }
    }
    0
}

/// Run `f` over the open frame's arenas and stream; a refusal or an error
/// puts every top back, so the frame is as it was.
fn with_open<R>(
    g: &mut MaliState,
    f: impl FnOnce(&mut frame::OpenFrame, &mut Arenas, &mut Stream) -> Result<R, FrameError>,
) -> Result<R, FrameError> {
    let o = g.open.as_mut().ok_or(FrameError::Invalid(frame::INVALID_OPEN_FRAME))?;
    let slot = g.slots[o.slot];
    // SAFETY: the open frame's slot: the GPU does not read it until it is
    // submitted.
    let (mut a, mut cs) = unsafe { slot_arenas(&slot, g.user_regs, o.tops, o.cs_len) }?;
    let mut fr = o.frame;
    match f(&mut fr, &mut a, &mut cs) {
        Ok(r) => {
            (o.tops, o.cs_len, o.frame) = (a.tops(), cs.words().len(), fr);
            Ok(r)
        }
        Err(e) => {
            a.rewind(o.tops);
            Err(e)
        }
    }
}

/// Add one GL draw (or a clear quad) to the open frame: its depth/stencil
/// needs first, then the draw.
fn add_to_open(g: &mut MaliState, d: &GlesDraw, ctx: &frame::Context) -> Result<(), Fail> {
    let mut metas = None;
    let (shaders, vs_attributes) = programs(d, &mut metas)?;
    let zs = depth_stencil_state(d);
    let o = g.open.ok_or(FrameError::Invalid(frame::INVALID_OPEN_FRAME))?;
    /* the Z/S preload this draw starts, when it is the frame's first use of
     * an attachment a past frame drew */
    let mut zs_preload = None;
    if zs.uses_attachment() {
        let z = o.zs.ok_or(FrameError::Invalid(frame::INVALID_NO_ZS))?;
        /* the write-back writes both planes of the D24S8 buffer: neither may
         * hold contents the tile buffer did not load */
        if z.start.contains(&Start::Elsewhere) {
            return Err(FrameError::NotBuilt(NOT_BUILT_ZS_ELSEWHERE).into());
        }
        if o.frame.zs_preload.is_none() {
            /* a plane a past frame drew is preloaded; one cleared for the
             * frame starts from its value, one GL keeps nothing of from
             * anything (Mesa preloads what the batch does not clear,
             * pan_fb_preload.c:746-760) */
            let [pz, ps] = z.start.map(|s| s == Start::Unloaded);
            if let Some(k) = zs_variant(pz, ps) {
                if g.zs_preload[k].n == 0 {
                    return Err(FrameError::NotBuilt(NOT_BUILT_ZS_PRELOAD).into());
                }
                let cleared = z.start.iter().any(|s| matches!(s, Start::Value(_)));
                zs_preload = Some((g.zs_preload[k], pz, ps, cleared));
            }
        }
        if z.buf == usize::MAX {
            let buf = zs_buffer(g, &o.key)?;
            if let Some(oz) = g.open.as_mut().and_then(|o| o.zs.as_mut()) {
                oz.buf = buf;
            }
        }
    }
    let has_zs = o.zs.is_some();
    let target = o.target;
    /* `check` refused a handle past the counters */
    let occlusion = query_counter(g, d.occlusion_query);
    texture_ready(g, d)?;
    let copied = sampled_binds(d).any(|t| t.incomplete == 0 && needs_copy(t));
    let blend = draw_blend_shader(g, d, &target)?;
    let draw = |fr: &mut frame::OpenFrame, a: &mut Arenas, cs: &mut Stream| {
        /* with the draw, so that a refused draw leaves no preload behind */
        if let Some((p, z, s, cleared)) = zs_preload {
            let bin = upload_preload(&mut a.code, &p)?;
            fr.zs_preload = Some(frame::ZsPreload {
                shader: frame::FrameShader { binary: bin, preload: p.preload },
                z,
                s,
                cleared,
            });
        }
        /* the blend shader in the frame's code arena, with the draw: in
         * the fragment program's 4 GiB window, as the descriptor holds the
         * low 32 bits of its address */
        let blend_shader = match &blend {
            Some(b) => {
                let words = b.words();
                let mut bytes = [0u8; 8 * BLEND_SHADER_WORDS];
                for (k, w) in words.iter().enumerate() {
                    bytes[8 * k..8 * k + 8].copy_from_slice(&w.to_le_bytes());
                }
                Some(a.code.upload(&bytes[..8 * words.len()], 128)?.gpu)
            }
            None => None,
        };
        add_gl_draw(fr, d, shaders, vs_attributes, ctx, &target, has_zs, occlusion, blend_shader, a, cs)
    };
    match with_open(g, draw) {
        Ok(()) => {
            if copied {
                g.row_copies = g.row_copies.saturating_add(1);
            }
        }
        /* the draw's data does not fit the frame's data chunks: the pool
         * grows by one that holds the block, and the draw is made again
         * (every block of the frame stays where it is) */
        Err(FrameError::Arena(ArenaError::Full { label, need, align, .. }))
            if label == REGIONS[1].0 && grow_data(g, need, align) =>
        {
            return add_to_open(g, d, ctx);
        }
        /* the frame's room is spent by its earlier draws: submit it and go
         * on in a second frame that starts from its image (the colour
         * preload); a draw that does not fit an empty frame is refused as
         * it is */
        Err(e) if o.frame.draws > 0 && no_room(&e) => {
            let rc = end_open(g);
            if rc != 0 {
                return Err(Fail::Errno(rc));
            }
            open_frame(g, o.key, target, None)?;
            return add_to_open(g, d, ctx);
        }
        Err(e) => return Err(e.into()),
    }
    if let Some(z) = g.open.as_mut().and_then(|o| o.zs.as_mut()) {
        let w = writes(&zs);
        z.written = [z.written[0] || w[0], z.written[1] || w[1]];
    }
    Ok(())
}

/// Grow the open frame's data pool by a chunk that holds a block of `need`
/// bytes aligned to `align`: `ALIGN_POT(MAX2(slab_size, sz), 4096)` bytes,
/// as Mesa's pool takes a new BO (pan_mempool.c:162-164), the first data
/// chunk's size the slab. It takes the first place after the frame's
/// current chunk that has no chunk, or one smaller than that, which is then
/// given back: the places after the current chunk hold nothing of the frame
/// (the draw that did not fit was undone), and the GPU does not read the
/// open frame's slot. False when every place holds a chunk at least that
/// big (the frame's room is spent, [`no_room`]) or the OSAL gave no memory
/// (its line is logged by [`alloc`]).
fn grow_data(g: &mut MaliState, need: usize, align: u64) -> bool {
    let Some(o) = g.open else { return false };
    let len = usize::try_from(align)
        .ok()
        .and_then(|a| need.checked_add(a - 1))
        .and_then(|n| n.max(DATA_BYTES).checked_next_multiple_of(4096));
    let Some(len) = len else { return false };
    let cur = o.tops.data.chunk;
    let chunks = &mut g.slots[o.slot].data_chunks;
    let Some(m) = chunks.iter_mut().skip(cur).find(|m| m.cpu.is_null() || m.size < len) else {
        return false;
    };
    osal::gpu_free(m);
    if alloc(len, OSAL_MEM_DATA, m).is_err() {
        osal::gpu_free(m);
        return false;
    }
    glog!("mali arena 'data': a chunk of {} bytes for a block of {}", m.size, need);
    true
}

/// Whether a draw failed only for the open frame's room: an arena or the
/// stream full, as the frame builder or a program upload reports it.
fn no_room(e: &FrameError) -> bool {
    matches!(
        e,
        FrameError::NotBuilt(frame::NOT_BUILT_FRAME_ROOM)
            | FrameError::Arena(crate::mali::arena::ArenaError::Full { .. })
            | FrameError::Cs(crate::mali::cs::CsError::StreamFull { .. })
    )
}

/// Record one draw's or clear's part of the open frame (see the module
/// documentation).
fn record_in(g: &mut MaliState, d: &GlesDraw) -> Result<(), Fail> {
    check(d)?;
    /* a draw's programs are checked before the frame is touched */
    if d.index_count != 0 {
        let mut metas = None;
        programs(d, &mut metas)?;
    }
    let Some(ctx) = g.ctx.filter(|_| g.up) else {
        glog_once!("{}", NO_MALI_OSAL);
        return Err(Fail::Errno(-ENODEV));
    };
    let format = target_format(d)?;
    let row_stride =
        backend::draw_pitch(d).checked_mul(format.bytes()).ok_or(FrameError::Invalid(INVALID_ROW_STRIDE))?;
    let key = TargetKey::of(d, row_stride, format);
    if g.open.is_some_and(|o| o.key != key) {
        let rc = end_open(g);
        if rc != 0 {
            return Err(Fail::Errno(rc));
        }
    }
    /* depth texels the CPU wrote go into the buffer before a frame
     * preloads it: the frame open now (if any) ends first */
    // SAFETY: the GL core's contract for the draw it hands over.
    if let Some(t) = unsafe { ds_retention::DepthTexels::of(d) } {
        let rc = end_open(g);
        if rc != 0 {
            return Err(Fail::Errno(rc));
        }
        seed_depth(g, &key, t)?;
    }
    /* the clear's parts: a whole one is a start value while nothing of the
     * frame has used its buffer; any other is drawn */
    let colour_whole = d.do_clear && !d.clear_quad && colour_clear_mask(d) == 0xf;
    let ds_whole = [d.do_clear_depth && !d.clear_quad, d.do_clear_stencil && !d.clear_quad];
    let values = [d.clear_depth.to_bits(), u32::from(d.clear_stencil)];
    let mut quad = QuadBuffers { colour: false, depth: false, stencil: false };
    if let Some(o) = g.open.as_mut() {
        if d.do_clear {
            if colour_whole && o.frame.draws == 0 {
                /* the frame's start: nothing of the old image is kept, so
                 * nothing is preloaded (`pan_preload_needed`,
                 * pan_fb_preload.c:746-760) */
                o.clear = d.clear_rgba;
                o.frame.colour_preload = None;
            } else {
                quad.colour = true;
            }
        }
    } else if colour_whole {
        let target = frame::Target {
            gpu: d.target_gpu,
            width: d.target_w,
            height: d.target_h,
            row_stride,
            format,
            /* `GlesDraw::samples` is 1 or 4; GlesDraw::ZERO's 0 is single
             * sampled, as every backend reads it */
            samples: u32::from(d.samples.max(1)),
        };
        open_frame(g, key, target, Some(d.clear_rgba))?;
    } else if d.index_count == 0 && !d.do_clear && !d.clear_quad {
        /* a whole depth/stencil clear and no frame: the plane's value */
        for (aspect, whole) in ds_whole.iter().enumerate() {
            let (id, gen) = key.objects[aspect];
            if *whole && id != 0 && set_plane(g, id, gen, aspect, Plane::cleared(values[aspect]), 0).is_err() {
                return Err(Fail::Errno(ds_retention::refuse("mali", id, PLANES)));
            }
        }
        return Ok(());
    } else {
        /* no whole colour clear: the frame starts from the old image (Mesa's
         * preload, pan_fb_preload.c), and a part clear is drawn into it */
        let target = frame::Target {
            gpu: d.target_gpu,
            width: d.target_w,
            height: d.target_h,
            row_stride,
            format,
            samples: u32::from(d.samples.max(1)),
        };
        open_frame(g, key, target, None)?;
        quad.colour = d.do_clear || d.clear_quad;
    }
    let o = g.open.as_mut().ok_or(FrameError::Invalid(frame::INVALID_OPEN_FRAME))?;
    for aspect in 0..2 {
        let cleared = [d.do_clear_depth, d.do_clear_stencil][aspect];
        let Some(z) = o.zs.as_mut().filter(|_| cleared && o.key.objects[aspect].0 != 0) else {
            continue;
        };
        if ds_whole[aspect] && !o.frame.uses_zs {
            z.start[aspect] = Start::Value(values[aspect]);
        } else if aspect == 0 {
            quad.depth = true;
        } else {
            quad.stencil = true;
        }
    }
    if quad.colour || quad.depth || quad.stencil {
        if !g.clear_program.built {
            return Err(FrameError::NotBuilt(NOT_BUILT_CLEAR_PROGRAM).into());
        }
        let vertices = clear_quad_vertices(d.clear_depth);
        let colour = d.clear_rgba;
        let p = g.clear_program;
        let q = clear_quad_draw(d, quad, &p, &vertices, &colour);
        add_to_open(g, &q, &ctx)?;
    }
    if d.index_count != 0 {
        add_to_open(g, d, &ctx)?;
    }
    Ok(())
}

/// Record one draw's or clear's part of the open frame (see
/// [`crate::backend::draw`] for the contract).
///
/// # Safety
/// `draw` is null or points at a valid `GlesDraw`.
pub unsafe extern "C" fn mali_draw(draw: *const GlesDraw) -> i32 {
    // SAFETY: the caller's contract: null or a valid draw.
    let Some(d) = (unsafe { draw.as_ref() }) else {
        return -EINVAL;
    };
    // SAFETY: as in `mali_init`.
    record_in(unsafe { STATE.get() }, d).err().map_or(0, fail_errno)
}

/// Submit the open frame.
pub extern "C" fn mali_flush() -> i32 {
    // SAFETY: as in `mali_init`.
    end_open(unsafe { STATE.get() })
}

/// Submit the open frame and wait for every submitted one.
pub extern "C" fn mali_finish() -> i32 {
    // SAFETY: as in `mali_init`.
    let g = unsafe { STATE.get() };
    let rc = end_open(g);
    let waited = wait_all(g).err().map_or(0, |(rc, _)| rc);
    if rc != 0 {
        rc
    } else {
        waited
    }
}

/// Wait for the frame before the last submitted one.
pub extern "C" fn mali_finish_previous() -> i32 {
    // SAFETY: as in `mali_init`.
    let g = unsafe { STATE.get() };
    /* `next` is the slot after the last submitted one, which with two slots
     * is the one before it (or the open frame's, which holds nothing in
     * flight) */
    let i = g.next;
    wait_slot(g, i).err().map_or(0, |(rc, _)| rc)
}

/// The kick being recorded: the open frame, or the next one (each frame is
/// one kick).
pub extern "C" fn mali_kick_id() -> u32 {
    // SAFETY: as in `mali_init`.
    unsafe { STATE.get() }.kicks.wrapping_add(1)
}

/// Whether kick `id` may still be running or is not submitted yet: the
/// kick being recorded, or one a slot holds that no wait has seen complete.
/// A kick whose slot was reused was waited for.
pub extern "C" fn mali_kick_pending(id: u32) -> bool {
    // SAFETY: as in `mali_init`.
    let g = unsafe { STATE.get() };
    if id == 0 {
        return false;
    }
    if id > g.kicks {
        return true;
    }
    let Some(f) = g.slots.iter().filter_map(|s| s.in_flight).find(|f| f.kick == id) else {
        return false;
    };
    completed().is_none_or(|c| c < f.fence)
}

/// Kicks submitted and waits made, for the GL core's counters.
///
/// # Safety
/// Each pointer is null or writable.
pub unsafe extern "C" fn mali_counters(kicks: *mut u32, waits: *mut u32) {
    // SAFETY: as in `mali_init`.
    let g = unsafe { STATE.get() };
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

/// End the open frame so that its draws run: the GL core is about to free
/// something it draws into. The memory itself is the OSAL's to keep until
/// the kick that may read it has completed (`gpu_free`'s contract,
/// include/gles_osal.h; tools/mali/panthor's OSAL holds it to the kick
/// being recorded, as zephyr.c does), so nothing is waited for here, as
/// the V3D and the PowerVR do not wait (`v3d_attachment_gone`,
/// `pvr_backend_attachment_gone`). A frame that cannot be ended is lost
/// work (counted in `end_open`) and logged.
fn end_open_for(g: &mut MaliState, why: &str) {
    let rc = end_open(g);
    if rc != 0 {
        glog_once!("mali: the open frame did not submit before {} ({})", why, rc);
    }
}

/// A depth or stencil object is gone (deleted and held by no framebuffer,
/// or its storage specified again): its planes hold nothing GL keeps
/// (`ds_retention`). An open frame that uses it is ended first, so its
/// draws run and its planes are recorded before they are forgotten.
pub extern "C" fn mali_attachment_gone(id: u32) {
    // SAFETY: as in `mali_init`.
    let g = unsafe { STATE.get() };
    if id == 0 {
        return;
    }
    if g.open.is_some_and(|o| o.key.objects.iter().any(|&(obj, _)| obj == id)) {
        end_open_for(g, "its depth/stencil object went");
    }
    for p in g.planes.iter_mut().filter(|p| p.id == id) {
        *p = NO_PLANE;
    }
}

/// Write the depth plane of GL object `id` (a depth texture) into `w` x `h`
/// floats at `dst`, `pitch` a row (`YakoglBackend::depth_resolve`; the GL
/// core calls it before a draw samples the texture or the CPU writes it,
/// after every kick that rendered the depth completed).
///
/// The plane is in the buffer of the attachment pair whose frame last drew
/// it (`PlaneEntry::ds_id`, its `ZsBuf`): the frame's Z/S write-back,
/// linear D24S8 at `4 * w` bytes a row, the depth in the low 24 bits of
/// each word and the stencil in the high byte (`frame::ZsAttachment`;
/// Mesa's buffer for case 169 has the same layout, `ZS Write Format: D24S8`,
/// `Row Stride: 256` for 64 wide, row 0 holding GL's bottom row,
/// 0dce654a8:tools/mali/board-ref/2026-10-20-mesa-refs/mesa/mesa-dstex.pandecode:3672-3690 and
/// 4963-4981; docs/notes/mali-texture-refusals.md (c)). The frame renders a
/// texture target unflipped, so buffer row `y` is texel row `y`. Each word
/// goes through [`ds_retention::unorm24_depth`]; a plane that holds a whole
/// clear only is that value everywhere; texels past the buffer are left.
/// The CPU's view is invalidated first (the GPU wrote the buffer).
///
/// 1 when nothing is held for `id` (never drawn, undefined, or drawn into a
/// buffer since given to another pair: the texels stay as they were);
/// -EBUSY when the open frame still draws into it (the GL core submits and
/// waits first, so this is a defect check); -EINVAL for a buffer shorter
/// than its pair's `4 * w * h` bytes. No sample count is involved: a depth
/// texture is a single-sampled attachment, and the 4x frame's Z/S is
/// written back averaged into the same single-sampled layout.
///
/// # Safety
/// `dst` holds `h` rows of `pitch` floats, `w <= pitch`.
pub unsafe extern "C" fn mali_depth_resolve(id: u32, dst: *mut f32, w: u32, h: u32, pitch: u32) -> i32 {
    // SAFETY: as in `mali_init`.
    let g = unsafe { STATE.get() };
    if !g.up || id == 0 || dst.is_null() || w > pitch {
        return 1;
    }
    if g.open.is_some_and(|o| o.key.objects[ds_retention::DEPTH].0 == id) {
        return -EBUSY;
    }
    let Some(e) = g.planes.iter().find(|p| p.id == id && p.aspect == ds_retention::DEPTH) else {
        return 1;
    };
    if let Some(v) = e.plane.cleared_value() {
        // SAFETY: the caller's contract.
        unsafe { ds_retention::write_depth_texels(dst, w, h, pitch, |_, _| Some(f32::from_bits(v))) };
        return 0;
    }
    if !e.plane.is_live() {
        return 1;
    }
    let Some(z) = g.zsbufs.iter().find(|z| z.ds_id == e.ds_id && !z.mem.cpu.is_null()) else {
        return 1;
    };
    let (zw, zh) = (z.w as usize, z.h as usize);
    let bytes = zw.saturating_mul(zh).saturating_mul(4);
    if bytes == 0 || bytes > z.mem.size {
        return -EINVAL;
    }
    // SAFETY: a live, CPU-mapped allocation of at least `bytes` (checked
    // above), which no pending kick writes (the GL core waited).
    let words = unsafe {
        osal::cache_invd(z.mem.cpu, bytes);
        core::slice::from_raw_parts(z.mem.cpu as *const u32, zw * zh)
    };
    // SAFETY: the caller's contract.
    unsafe {
        ds_retention::write_depth_texels(dst, w, h, pitch, |x, y| {
            ((x as usize) < zw && (y as usize) < zh)
                .then(|| ds_retention::unorm24_depth(words[y as usize * zw + x as usize]))
        })
    };
    0
}

/// The GL core is about to free the storage `[base_gpu, + bytes)` of a
/// texture or renderbuffer (`YakoglBackend::texture_released`): an open
/// frame that draws into it is ended, so its draws run (the OSAL then holds
/// the storage until that kick has completed). Anything else is not this
/// backend's: it keeps no copy of texel storage.
pub extern "C" fn mali_texture_released(base_gpu: u64, bytes: u64) {
    // SAFETY: as in `mali_init`.
    let g = unsafe { STATE.get() };
    let end = base_gpu.saturating_add(bytes);
    if g.open.is_some_and(|o| {
        let s = o.target.gpu;
        s < end && base_gpu < s + u64::from(o.target.row_stride) * u64::from(o.target.height)
    }) {
        end_open_for(g, "its colour target was released");
    }
}

/* ---- occlusion queries --------------------------------------------------- */

/// The GPU address of `handle`'s counter: None for 0 (no query) and past the
/// handles, or before init.
fn query_counter(g: &MaliState, handle: u32) -> Option<u64> {
    (g.up && handle != 0 && handle < backend::GLES_QUERY_HANDLES)
        .then(|| g.queries.gpu + (QUERY_BLOCK_BYTES as u64) * u64::from(handle))
}

/// The qwords of `handle`'s counter (checked by [`query_counter`]).
///
/// # Safety
/// No kick that may write the block is in flight (the GL core's contract
/// for `query_begin` and `query_result`).
unsafe fn query_block<'a>(g: &MaliState, handle: u32) -> &'a mut [u64] {
    let at = handle as usize * QUERY_BLOCK_BYTES / 8;
    // SAFETY: the caller's contract; `queries` is this backend's.
    unsafe { &mut words_of(&g.queries)[at..at + QUERY_BLOCK_BYTES / 8] }
}

/// An occlusion query starts counting on `handle`: its counter zeroed, as
/// Mesa's `panfrost_begin_query` writes zeroes into the query's buffer
/// (Mesa 26.2.1 pan_context.c:753-758). Nothing goes into the command
/// stream and the open frame goes on: a draw recorded from now on names the
/// counter (`GlesDraw::occlusion_query`).
pub extern "C" fn mali_query_begin(handle: u32) -> i32 {
    // SAFETY: as in `mali_init`.
    let g = unsafe { STATE.get() };
    if !g.up {
        return -ENODEV;
    }
    if query_counter(g, handle).is_none() {
        return -EINVAL;
    }
    // SAFETY: every kick that counted on the handle has completed (the GL
    // core waits before query_begin).
    let block = unsafe { query_block(g, handle) };
    block.fill(0);
    // SAFETY: the block lies in `queries`, this backend's mapping.
    unsafe { osal::cache_flush(block.as_mut_ptr().cast::<c_void>(), QUERY_BLOCK_BYTES) };
    0
}

/// The samples counted on `handle`: the sum of its counter's qwords. The
/// hardware sets the first qword to 1 in PREDICATE mode (the board's Mesa,
/// 0dce654a8:tools/mali/board-ref/2026-10-20-query); Mesa reads `result[0]` for a
/// predicate (pan_context.c:868), the sum of all of them is the same answer.
///
/// # Safety
/// `samples` is null or writable.
pub unsafe extern "C" fn mali_query_result(handle: u32, samples: *mut u64) -> i32 {
    // SAFETY: as in `mali_init`.
    let g = unsafe { STATE.get() };
    if !g.up {
        return -ENODEV;
    }
    if query_counter(g, handle).is_none() {
        return -EINVAL;
    }
    // SAFETY: every kick holding a counted draw has completed (the GL core
    // calls it then).
    let block = unsafe { query_block(g, handle) };
    // SAFETY: as in `mali_query_begin`.
    unsafe { osal::cache_invd(block.as_mut_ptr().cast::<c_void>(), QUERY_BLOCK_BYTES) };
    let sum = block.iter().fold(0u64, |s, &w| s.saturating_add(u64::from_le(w)));
    // SAFETY: the caller's contract.
    if let Some(out) = unsafe { samples.as_mut() } {
        *out = sum;
    }
    0
}

/// The backend table of a `gpu-mali` build.
pub static yakogl_backend_mali: YakoglBackend = YakoglBackend {
    name: c"Arm Mali-G610 (NanoPC-T6)".as_ptr(),
    /* float32 is always included (the field's contract); the GL core
     * converts the rest, as for the other GPUs before their fetch formats */
    native_attrib_formats: gles_attribf_bit(GLES_ATTRIBF_FLOAT32),
    rt_rgba8888_is_bgra: false,
    mip_level_align: MIP_LEVEL_ALIGN,
    /* a block format's level is raster rows of its 4x4 blocks: Mesa's
     * linear plane of ETC2 (the board's dump of fmt.c, a 16x8 ETC1 texture:
     * Clump format ETC2 RGB8, one row of blocks a row stride) */
    compressed_blocks_in_rows: true,
    native_layered_textures: false,
    front_facing_shader_variant: false,
    /* the GPU reads ETC1 as ETC2 RGB8 (Mesa 26.2.1's src/panfrost/lib/pan_format.c:231; v10.xml
     * Format "ETC2 RGB8", :102), so the GL core keeps the blocks */
    etc1_decoded_at_upload: false,
    init: Some(mali_init),
    shutdown: Some(mali_shutdown),
    draw: Some(mali_draw),
    flush: Some(mali_flush),
    finish: Some(mali_finish),
    finish_previous: Some(mali_finish_previous),
    kick_id: Some(mali_kick_id),
    kick_pending: Some(mali_kick_pending),
    heap_base: None,
    counters: Some(mali_counters),
    texture_released: Some(mali_texture_released),
    surface_swapped: None,
    texture_uploaded: None,
    /* divisor 1 only (the field's 0) */
    max_instance_divisor: 0,
    wait_kick: None,
    samples_texel_copies: false,
    attachment_gone: Some(mali_attachment_gone),
    depth_resolve: Some(mali_depth_resolve),
    query_begin: Some(mali_query_begin),
    /* Mesa's end only stops naming the buffer (pan_context.c:798-810) and
     * never ends the batch: on the board's Mesa, case 196's q4 (a quad
     * counted, then covered by a later draw of the same frame) reads 1
     * (0dce654a8:tools/mali/board-ref/2026-10-20-query) */
    query_end: None,
    query_result: Some(mali_query_result),
    generate_mipmap: None,
    return_mipmap_chain: None,
    /* the texture unit samples a linear image only at a row stride that is a
     * multiple of 64 bytes (0dce654a8:tools/mali/board-ref/2026-10-19-texture), as
     * Mesa lays them out */
    linear_row_align: TEXTURE_ROW_ALIGN,
    uniforms_host_only: false,
    ds_write: None,
    stencil_resolve: None,
};

#[cfg(test)]
#[path = "backend_tests.rs"]
mod tests;
