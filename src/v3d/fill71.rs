// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The V3D 7.1 table's fill: a GL-shaped frame and its draws into the
//! descriptors of the C job builder `driver/src/v3d/v3d_job71.{c,h}`
//! (`ffi71::V3dJob71Target`, `V3dJob71Draw` with its state and programs,
//! `V3dJob71Clear`), or a named [`Refusal`] (roadmap P9; the Pi 5's V3D
//! 7.1.10; docs/notes/v3d71-gl-table.md sections 4 to 6).
//!
//! Pure: no FFI call, no feature cfg, no `crate::` path; nothing hooks it
//! into `src/v3d/mod.rs` yet (backend71 does that). The siblings are reached
//! as `super::{admit71, attr71, clearplan71, draw71, ffi71, prim71,
//! record71, rt71, uniform71, zskeep71}` (and admit71's own `format71`,
//! `ident71`, `limits71`): the same names when a test includes them with
//! `#[path]` at its crate root (tests/v3d71_fill.rs).
//!
//! # What it does
//!
//! [`fill`] takes what the GL core holds between two flushes of one
//! framebuffer: the framebuffer ([`Framebuffer`]: size, colour attachments,
//! the depth/stencil attachment), what its buffers hold ([`History`]), and
//! the operations in GL order ([`Op`]: `glClear` requests and draws). It
//! returns one [`FilledJob`] per job the builder is to run (a flush is one
//! job unless a whole clear follows queued draws: clearplan71's `new_job`),
//! each with the history the next flush starts from. Every decision is a
//! merged module's:
//!
//! | decision | module |
//! |---|---|
//! | the frame's size, target count, colour and depth/stencil formats, clear kinds | `admit71::admit_frame` (rt71, format71, limits71) |
//! | a `glClear`: `Clear Render Targets` or a clear quad, or nothing | `clearplan71::plan_clear` |
//! | where a clear's depth and stencil go, and whether a new job opens | `zskeep71::route_clear` |
//! | each job's depth/stencil load, store and start value, and the next history | `zskeep71::decide` |
//! | the clear colour as the tile buffer holds it | `rt71::clear_words71` |
//! | a draw's state (viewport, scissor, clip, depth, stencil, blend, refusals) | `draw71::translate`, `viewport_xform`, `scissor_box` |
//! | the clear quad's state | `draw71::DrawState::blitter_clear` |
//! | the attribute records (no default-attribute BO) and the shader record | `admit71::admit_draw` (attr71, record71) |
//! | the uniform streams | `uniform71::fill` (checked; v3d_job71 writes the words) |
//! | the primitive, index type and counts | `prim71` |
//!
//! What this module adds is only the mapping onto the C structs and the
//! rules the C builder imposes that no module states (each a [`Refusal`]
//! named `fill71`, [`FILL71_REFUSALS`]):
//!
//! * the output image format of a GL colour format: RGBA8, RGB8 (stored as
//!   RGBA8 with no alpha) and SRGB8_ALPHA8 only for now ([`color_format`]);
//! * the depth/stencil image of a GL depth or stencil format
//!   ([`ds_format`]); `GL_STENCIL_INDEX8` is a D24S8 image moved as its
//!   stencil, Mesa's choice (tools/v3d/pi5/gl_fbods.clif, frames 6 to 9);
//!   `GL_DEPTH32F_STENCIL8` (a separate stencil image) is refused for now;
//! * v3d_job71 has one colour write mask and one blend state for every
//!   render target (RT 0's, v3d_job71.c `emit_state`), float attribute
//!   records only, no base vertex, base instance or primitive restart, and
//!   fixed record fields (Min input segments 1, output 0, no NaN
//!   propagation, no sample-rate shading).
//!
//! # Choices this module makes (documented, tested)
//!
//! * A format with no alpha (GL_RGB) keeps alpha 1 as the 4.2 table does
//!   (src/v3d/state.rs, "A format with no alpha"): its loads read alpha 1
//!   (`force_alpha_1`), no draw writes alpha (the draw's mask & 0x7), and its
//!   clears write alpha 1. For the clear plan, alpha is then a channel the
//!   mask does not need to hold (a clear of R, G and B is whole).
//! * A colour target is loaded when the job does not clear it with the tile
//!   buffer and it holds stored contents ([`History::colors`]); every
//!   target with a buffer is stored (v3d_job71 stores them all).
//! * The Z start value of a job is zskeep71's: the clear value, or 1.0 for an
//!   undefined plane the job uses (`UNDEFINED_Z_BITS`); a plane the job does
//!   not use, or loads, starts from 0.0, the value Mesa's zeroed job has.
//!   `zs_cleared` is set when the target has depth or stencil and loads
//!   neither (v3d_job71 then sets Early Depth/Stencil Clear unless it stores
//!   one, as Mesa does for every job with a Z/S buffer it does not keep).
//! * A clear quad (clearplan71's `Draw`) is Mesa's blitter clear: a
//!   TRIANGLE_FAN of 4 vertices over the target with the scissor off (the
//!   quad's vertices are the box, [`FilledDraw::quad`]), line width 0, the
//!   clear program the caller gives ([`ClearQuad`]) with the clear colour as
//!   its four GL uniform words, depth ALWAYS with writes for a depth clear,
//!   and for a stencil clear the REPLACE of the value through the mask.
//! * The depth and stencil tests of a draw are off on a framebuffer without
//!   that plane (ES 3.0 4.1.5, 4.1.4).
//!
//! # Using the result
//!
//! For each [`FilledJob`]: `v3d_job71_begin(&job, &filled.target, ...)`,
//! for each draw `v3d_job71_draw(&job, &draw.ffi(), layout)` (the pointers of
//! [`FilledDraw::ffi`] point into the `FilledDraw`, which must stay where it
//! is until the call returns), `v3d_job71_end(&job, &filled.clear, ...)`.
//! A clear quad's caller writes [`FilledDraw::quad`] where its
//! [`ClearQuad::attr`] reads before the job runs.

use super::{admit71, attr71, clearplan71, draw71, ffi71, prim71, record71, rt71, uniform71, zskeep71};

pub use admit71::Refusal;

use ffi71::{V3dFsProg, V3dJob71Clear, V3dJob71Draw, V3dJob71State, V3dJob71StencilFace, V3dJob71Target, V3dUniform};

/// The refusals this module names itself (module `fill71`): the rules of
/// v3d_job71 that no other module states.
pub const FILL71_REFUSALS: [&str; 12] = [
    REF_COLOR_FORMAT,
    REF_DS_FORMAT,
    REF_MEMORY_FORMAT,
    REF_ONE_MASK,
    REF_ONE_BLEND,
    REF_FLOAT_ATTRS,
    REF_BASE,
    REF_RECORD_FIXED,
    REF_BASE_INSTANCE,
    REF_SEPARATE_STENCIL,
    REF_COLOR_COUNT,
    REF_RGB_BLEND_ALPHA,
];

const REF_COLOR_FORMAT: &str =
    "V3D 7.1: no output image format is chosen for this colour format yet (RGBA8, RGB8 and SRGB8_ALPHA8 are filled)";
const REF_DS_FORMAT: &str = "V3D 7.1: this depth/stencil format has no depth/stencil image row";
const REF_MEMORY_FORMAT: &str = "V3D 7.1: a memory format above UIF_XOR";
const REF_ONE_MASK: &str =
    "V3D 7.1: v3d_job71 writes one colour write mask for every render target (the targets' masks differ)";
const REF_ONE_BLEND: &str = "V3D 7.1: v3d_job71 writes one blend state for every render target (independent blend)";
const REF_FLOAT_ATTRS: &str = "V3D 7.1: v3d_job71 writes float vertex attribute records only";
const REF_BASE: &str = "V3D 7.1: v3d_job71 does not write a base vertex, a base instance or primitive restart";
const REF_RECORD_FIXED: &str = "V3D 7.1: v3d_job71 writes Min input segments 1, Min output segments 0, no NaN propagation and no sample-rate shading";
const REF_BASE_INSTANCE: &str = "V3D 7.1: v3d_job71 does not write a program reading gl_BaseInstance";
const REF_SEPARATE_STENCIL: &str = "V3D 7.1: a DEPTH32F_STENCIL8 buffer (a separate stencil image) is not filled yet";
const REF_COLOR_COUNT: &str = "V3D 7.1: more colour attachments than render targets";
const REF_RGB_BLEND_ALPHA: &str =
    "V3D 7.1: a blend reading destination alpha on a format without alpha is not filled yet";

fn refuse(reason: &str) -> Refusal {
    Refusal { module: "fill71", reason: reason.to_string() }
}

fn named<E: core::fmt::Display>(module: &'static str) -> impl Fn(E) -> Refusal {
    move |e| Refusal { module, reason: e.to_string() }
}

/* ---- the framebuffer ------------------------------------------------------ */

/// GL sized formats this module maps (the rest are rt71's or admit71's).
pub const GL_RGBA8: u32 = 0x8058;
pub const GL_RGB8: u32 = 0x8051;
pub const GL_SRGB8_ALPHA8: u32 = 0x8C43;
pub const GL_DEPTH_COMPONENT16: u32 = 0x81A5;
pub const GL_DEPTH_COMPONENT24: u32 = 0x81A6;
pub const GL_DEPTH_COMPONENT32F: u32 = 0x8CAC;
pub const GL_DEPTH24_STENCIL8: u32 = 0x88F0;
pub const GL_DEPTH32F_STENCIL8: u32 = 0x8CAD;
pub const GL_STENCIL_INDEX8: u32 = 0x8D48;

/// One colour attachment: its GL format and its image in memory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ColorBuffer {
    /// GL sized internal format.
    pub gl_format: u32,
    /// GPU address of the level and layer drawn; 0: no buffer (GL_NONE).
    pub address: u32,
    /// `V3D_MEMORY_FORMAT_*`.
    pub memory_format: u32,
    /// Raster: the row stride in bytes; UIF: the padded height in UIF blocks.
    pub height_in_ub_or_stride: u32,
}

/// The depth/stencil attachment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DsBuffer {
    /// GL sized internal format (`GL_DEPTH24_STENCIL8`, `GL_DEPTH_COMPONENT16`,
    /// `GL_STENCIL_INDEX8`, ...).
    pub gl_format: u32,
    /// GPU address of its image; 0: the planes live in the tile buffer for
    /// one job only (zskeep71's `TileOnly`).
    pub address: u32,
    pub memory_format: u32,
    /// UIF: the padded height in UIF blocks; raster: the stride.
    pub height_in_ub: u32,
}

/// The framebuffer of a flush.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Framebuffer<'a> {
    pub width: u32,
    pub height: u32,
    /// GPU address of the job's tile allocation memory, 64-byte aligned.
    pub tile_alloc: u32,
    /// A window whose first row in memory is the top of the picture; a
    /// framebuffer object is not flipped.
    pub flip_y: bool,
    pub colors: &'a [ColorBuffer],
    pub ds: Option<DsBuffer>,
}

/// What the framebuffer's buffers hold when no job is open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct History {
    /// Colour attachment n holds stored contents (a job wrote it).
    pub colors: [bool; ffi71::V3D_JOB71_MAX_RTS],
    pub z: zskeep71::Known71,
    pub s: zskeep71::Known71,
}

impl History {
    /// Fresh buffers: nothing written.
    pub const NEW: History = History {
        colors: [false; ffi71::V3D_JOB71_MAX_RTS],
        z: zskeep71::Known71::Untouched,
        s: zskeep71::Known71::Untouched,
    };
}

/// A colour format's row: the output image format and whether it has alpha.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ColorRow {
    pub output_format: u32,
    pub has_alpha: bool,
}

/// The output image format of a GL colour format, or the `fill71` refusal.
pub fn color_format(gl: u32) -> Result<ColorRow, Refusal> {
    Ok(match gl {
        GL_RGBA8 => ColorRow { output_format: ffi71::V3D_OUTPUT_IMAGE_FORMAT_RGBA8, has_alpha: true },
        GL_RGB8 => ColorRow { output_format: ffi71::V3D_OUTPUT_IMAGE_FORMAT_RGBA8, has_alpha: false },
        GL_SRGB8_ALPHA8 => ColorRow { output_format: ffi71::V3D_OUTPUT_IMAGE_FORMAT_SRGB8_ALPHA8, has_alpha: true },
        _ => return Err(refuse(REF_COLOR_FORMAT)),
    })
}

/// A depth/stencil format's row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DsRow {
    /// `V3D_OUTPUT_IMAGE_FORMAT_D16`, `_D24S8` or `_D32F`.
    pub output_format: u32,
    /// `V3D_INTERNAL_TYPE_DEPTH_*`.
    pub depth_type: u32,
    pub depth: bool,
    pub stencil: bool,
}

/// The depth/stencil image of a GL depth or stencil format. A format admit71
/// already refuses (a colour one, an unknown one) is not asked here.
pub fn ds_format(gl: u32) -> Result<DsRow, Refusal> {
    use ffi71::*;
    let row = |output_format, depth_type, depth, stencil| DsRow { output_format, depth_type, depth, stencil };
    Ok(match gl {
        GL_DEPTH_COMPONENT16 => row(V3D_OUTPUT_IMAGE_FORMAT_D16, V3D_INTERNAL_TYPE_DEPTH_16, true, false),
        GL_DEPTH_COMPONENT24 => row(V3D_OUTPUT_IMAGE_FORMAT_D24S8, V3D_INTERNAL_TYPE_DEPTH_24, true, false),
        GL_DEPTH24_STENCIL8 => row(V3D_OUTPUT_IMAGE_FORMAT_D24S8, V3D_INTERNAL_TYPE_DEPTH_24, true, true),
        GL_DEPTH_COMPONENT32F => row(V3D_OUTPUT_IMAGE_FORMAT_D32F, V3D_INTERNAL_TYPE_DEPTH_32F, true, false),
        /* Mesa keeps a stencil-only buffer as a D24S8 image moved as its
         * stencil (gl_fbods.clif: Internal Depth Type depth_24, LOAD/STORE
         * buffer Stencil of a D24S8) */
        GL_STENCIL_INDEX8 => row(V3D_OUTPUT_IMAGE_FORMAT_D24S8, V3D_INTERNAL_TYPE_DEPTH_24, false, true),
        GL_DEPTH32F_STENCIL8 => return Err(refuse(REF_SEPARATE_STENCIL)),
        _ => return Err(refuse(REF_DS_FORMAT)),
    })
}

/* ---- programs and operations ---------------------------------------------- */

/// The varying flags of the fragment program (`v3d_fs_prog`'s six words).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VaryingFlags {
    pub flat: [u32; 2],
    pub noperspective: [u32; 2],
    pub centroid: [u32; 2],
}

/// A linked program: the record's description and what the C program
/// structs carry besides.
#[derive(Clone, Copy, Debug)]
pub struct Program<'a> {
    /// The shader record's view of the three programs (record71).
    pub desc: record71::DrawDesc,
    /// The coordinate and vertex programs' `vattr_sizes`.
    pub cs_reads: [u8; attr71::MAX_ATTRS],
    pub vs_reads: [u8; attr71::MAX_ATTRS],
    pub cs_vcm_cache_size: u32,
    pub vs_vcm_cache_size: u32,
    /// The programs need separate VPM segments (refused by attr71).
    pub separate_segments: bool,
    pub varyings: VaryingFlags,
    /// The fragment program can discard (draw71's early-Z answer).
    pub discards: bool,
    /// The uniform streams, `(contents, data)` in the order the code reads.
    pub fs_uniforms: &'a [uniform71::Word],
    pub vs_uniforms: &'a [uniform71::Word],
    pub cs_uniforms: &'a [uniform71::Word],
}

/// A `glClear` with the state it reads.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClearReq {
    /// `GL_COLOR_BUFFER_BIT`, with `glClearColor`.
    pub color: Option<[f32; 4]>,
    /// Bit n: colour attachment n is a draw buffer (glDrawBuffers).
    pub draw_buffers: u8,
    /// glColorMask, bit 0 R .. bit 3 A.
    pub color_mask: u8,
    /// `GL_DEPTH_BUFFER_BIT`, with `glClearDepthf` (0..1).
    pub depth: Option<f32>,
    pub depth_mask: bool,
    /// `GL_STENCIL_BUFFER_BIT`, with `glClearStencil`.
    pub stencil: Option<u32>,
    /// glStencilMask (front), 8 bits.
    pub stencil_mask: u8,
    /// glScissor x, y, w, h with the scissor test on.
    pub scissor: Option<[i32; 4]>,
}

impl ClearReq {
    /// `glClear(GL_COLOR_BUFFER_BIT)` of draw buffer 0 with GL's other state
    /// at its start values.
    pub const fn color(c: [f32; 4]) -> ClearReq {
        ClearReq {
            color: Some(c),
            draw_buffers: 1,
            color_mask: 0xf,
            depth: None,
            depth_mask: true,
            stencil: None,
            stencil_mask: 0xff,
            scissor: None,
        }
    }
}

/// The program and vertex array a clear quad is drawn with.
#[derive(Clone, Copy, Debug)]
pub struct ClearQuad<'a> {
    /// The clear program: its fragment stream reads the colour as GL uniform
    /// words 0 to 3 (`ours_clear`'s `u_color`).
    pub program: &'a Program<'a>,
    /// Attribute 0, the quad's positions; the caller writes
    /// [`FilledDraw::quad`] where it points.
    pub attr: attr71::Attr,
    /// A valid address of unused data (attr71's GFXH-930 dummy).
    pub dummy_addr: u32,
}

/// One GL draw call with its state.
#[derive(Clone, Copy, Debug)]
pub struct DrawIn<'a> {
    /// The GL state (draw71). The target fields (`target_w`, `target_h`,
    /// `flip_y`, `nr_targets`, each target's `has_alpha`) and `polygon` are
    /// the framebuffer's and the primitive's: [`fill`] overwrites them.
    pub state: draw71::DrawState,
    pub line_width: f32,
    /// glBlendColor, already clamped to 0..1.
    pub blend_color: [f32; 4],
    pub program: &'a Program<'a>,
    /// The GL vertex attributes, index = attribute location (attr71).
    pub attrs: &'a [attr71::Attr],
    pub dummy_addr: u32,
    /// The draw call (prim71).
    pub prim: prim71::Draw,
    /// The stages' GL uniform blocks (the VS block serves the CS too).
    pub vs_words: &'a [u32],
    pub fs_words: &'a [u32],
    /// The occlusion query counter's address, 0 for none.
    pub oq_addr: u32,
}

/// One operation of a flush, in GL order (a draw is boxed: it is many
/// times a clear's size).
#[derive(Clone, Debug)]
pub enum Op<'a> {
    Clear(ClearReq),
    Draw(Box<DrawIn<'a>>),
}

/* ---- the result ------------------------------------------------------------ */

/// One draw's descriptors. Not `Clone`: the program structs point at the
/// uniform lists this value owns.
#[derive(Debug)]
pub struct FilledDraw {
    pub state: V3dJob71State,
    pub fs: V3dFsProg,
    pub vs: ffi71::V3dVsProg,
    pub cs: ffi71::V3dVsProg,
    pub fs_uniforms: Vec<V3dUniform>,
    pub vs_uniforms: Vec<V3dUniform>,
    pub cs_uniforms: Vec<V3dUniform>,
    /// The VS (and CS) and FS GL uniform blocks.
    pub vs_words: Vec<u32>,
    pub fs_words: Vec<u32>,
    pub attrs: Vec<ffi71::V3dVertexAttr>,
    /// `V3D_PRIM_*`.
    pub prim: u32,
    pub count: u32,
    pub first: u32,
    pub indexed: bool,
    pub index_type: u32,
    pub index_buffer: u32,
    pub index_buffer_bytes: u32,
    pub index_offset: u32,
    pub instances: u32,
    pub vs_writes_psiz: bool,
    /// A clear quad's four vertices (x, y, z, w in clip space, TRIANGLE_FAN
    /// order), for the caller to write where [`ClearQuad::attr`] reads.
    pub quad: Option<[[f32; 4]; 4]>,
    /// The quad of a clear (not a GL draw).
    pub is_clear: bool,
}

impl FilledDraw {
    /// The `struct v3d_job71_draw` for `v3d_job71_draw`, pointing into `self`.
    pub fn ffi(&self) -> V3dJob71Draw {
        let words = |w: &Vec<u32>| ffi71::V3dUniformStorage {
            words: if w.is_empty() { core::ptr::null() } else { w.as_ptr() },
            count: w.len() as u32,
        };
        V3dJob71Draw {
            st: &self.state,
            fs: &self.fs,
            vs: &self.vs,
            cs: &self.cs,
            vs_storage: words(&self.vs_words),
            fs_storage: words(&self.fs_words),
            attrs: if self.attrs.is_empty() { core::ptr::null() } else { self.attrs.as_ptr() },
            num_attrs: self.attrs.len() as u32,
            prim: self.prim,
            count: self.count,
            first: self.first,
            indexed: self.indexed,
            index_type: self.index_type,
            index_buffer: self.index_buffer,
            index_buffer_bytes: self.index_buffer_bytes,
            index_offset: self.index_offset,
            tmu: core::ptr::null(),
            num_tmu: 0,
            vs_tmu: core::ptr::null(),
            num_vs_tmu: 0,
            vs_writes_psiz: self.vs_writes_psiz,
            instances: self.instances,
        }
    }
}

/// One job's descriptors.
#[derive(Debug)]
pub struct FilledJob {
    pub target: V3dJob71Target,
    pub draws: Vec<FilledDraw>,
    pub clear: V3dJob71Clear,
    /// What the buffers hold after the job: the next job's [`History`].
    pub after: History,
}

/* ---- the fill ---------------------------------------------------------------- */

struct Frame {
    target: V3dJob71Target,
    rows: Vec<(rt71::Rt71, ColorRow)>,
    zs: zskeep71::Target71,
}

fn frame(fb: &Framebuffer<'_>) -> Result<Frame, Refusal> {
    if fb.colors.len() > ffi71::V3D_JOB71_MAX_RTS {
        return Err(refuse(REF_COLOR_COUNT));
    }
    let colors: Vec<admit71::ColorTarget> = fb
        .colors
        .iter()
        .map(|c| admit71::ColorTarget {
            gl_format: c.gl_format,
            uses: admit71::BufferUse::default(),
            clear_color: None,
            sampled: false,
        })
        .collect();
    let ds = fb.ds.map(|d| admit71::DepthStencil {
        gl_format: d.gl_format,
        depth: admit71::BufferUse::default(),
        stencil: admit71::BufferUse::default(),
    });
    admit71::admit_frame(&admit71::Frame {
        width: fb.width,
        height: fb.height,
        colors: &colors,
        depth_stencil: ds,
        msaa: false,
        double_buffer: false,
    })?;

    let mut t = V3dJob71Target {
        width: fb.width,
        height: fb.height,
        tile_alloc: fb.tile_alloc,
        rts: fb.colors.len() as u32,
        depth_type: ffi71::V3D_INTERNAL_TYPE_DEPTH_32F,
        ..V3dJob71Target::default()
    };
    let mut rows = Vec::with_capacity(fb.colors.len());
    for (n, c) in fb.colors.iter().enumerate() {
        let rt = rt71::rt_format71(c.gl_format).map_err(|e| Refusal { module: "rt71", reason: e.as_str().into() })?;
        let row = color_format(c.gl_format)?;
        if c.memory_format > ffi71::V3D_MEMORY_FORMAT_UIF_XOR {
            return Err(refuse(REF_MEMORY_FORMAT));
        }
        if c.address != 0 {
            t.rt[n] = ffi71::V3dJob71Rt {
                address: c.address,
                format: row.output_format,
                memory_format: c.memory_format,
                height_in_ub_or_stride: c.height_in_ub_or_stride,
                r_b_swap: false,
                force_alpha_1: !row.has_alpha,
                pad: [0; 2],
            };
        }
        rows.push((rt, row));
    }
    let mut zs = zskeep71::Target71 { layout: zskeep71::Layout71::TileOnly, depth: false, stencil: false };
    if let Some(d) = fb.ds {
        let row = ds_format(d.gl_format)?;
        if d.memory_format > ffi71::V3D_MEMORY_FORMAT_UIF_XOR {
            return Err(refuse(REF_MEMORY_FORMAT));
        }
        t.depth_type = row.depth_type;
        if d.address != 0 {
            t.zs = ffi71::V3dJob71Zs {
                address: d.address,
                format: row.output_format,
                memory_format: d.memory_format,
                height_in_ub: d.height_in_ub,
            };
        }
        zs = zskeep71::Target71 {
            layout: if d.address != 0 { zskeep71::Layout71::Combined } else { zskeep71::Layout71::TileOnly },
            depth: row.depth,
            stencil: row.stencil,
        };
    }
    Ok(Frame { target: t, rows, zs })
}

/// The open job.
struct Open {
    colors: [Option<[u32; 4]>; ffi71::V3D_JOB71_MAX_RTS],
    z: zskeep71::PlaneUse71,
    s: zskeep71::PlaneUse71,
    draws: Vec<FilledDraw>,
    touched: bool,
}

impl Open {
    fn new() -> Open {
        Open {
            colors: [None; ffi71::V3D_JOB71_MAX_RTS],
            z: zskeep71::PlaneUse71::NONE,
            s: zskeep71::PlaneUse71::NONE,
            draws: Vec::new(),
            touched: false,
        }
    }
}

fn zs_err(e: zskeep71::ZsRefusal71) -> Refusal {
    Refusal { module: "zskeep71", reason: e.message().to_string() }
}

/// The clear colour a target is cleared to: alpha 1 on a format without one.
fn clear_rgba(c: [f32; 4], has_alpha: bool) -> [f32; 4] {
    if has_alpha {
        c
    } else {
        [c[0], c[1], c[2], 1.0]
    }
}

/// The colour mask the clear plan sees for a target: alpha is free on a
/// format without it (writing 1 is what it holds).
fn plan_mask(mask: u8, has_alpha: bool) -> u8 {
    let m = mask & 0xf;
    if has_alpha {
        m
    } else if m & 0x7 == 0 {
        0
    } else {
        m | 0x8
    }
}

/// The one colour mask v3d_job71 writes for every target: the targets' own
/// masks must agree (a target without a buffer does not count).
fn one_mask(fb: &Framebuffer<'_>, masks: impl Iterator<Item = u8>) -> Result<u32, Refusal> {
    let mut one: Option<u8> = None;
    for (c, m) in fb.colors.iter().zip(masks) {
        if c.address == 0 {
            continue;
        }
        match one {
            None => one = Some(m),
            Some(o) if o != m => return Err(refuse(REF_ONE_MASK)),
            _ => {}
        }
    }
    Ok(u32::from(one.unwrap_or(0xf)))
}

fn compare_code(c: draw71::Compare) -> u32 {
    c as u8 as u32
}

fn face(f: &draw71::StencilFace) -> V3dJob71StencilFace {
    V3dJob71StencilFace {
        func: compare_code(f.func),
        sfail_op: f.sfail as u8 as u32,
        zfail_op: f.zfail as u8 as u32,
        zpass_op: f.zpass as u8 as u32,
        ref_: f.reference.min(0xff),
        value_mask: f.value_mask & 0xff,
        write_mask: f.write_mask & 0xff,
    }
}

fn face_writes(f: &draw71::StencilFace) -> bool {
    use draw71::StencilOp::Keep;
    f.write_mask & 0xff != 0 && (f.sfail != Keep || f.zfail != Keep || f.zpass != Keep)
}

/// The blend colour as IEEE halves: rt71's RGBA16F clear packing is exactly
/// that conversion (round to nearest even, no clamp).
fn halves(c: [f32; 4]) -> Result<[u16; 4], Refusal> {
    let row = rt71::rt_format71(0x881A).map_err(|e| Refusal { module: "rt71", reason: e.as_str().into() })?;
    let w = rt71::clear_words71(&row, rt71::ClearColor::Float(c))
        .map_err(|e| Refusal { module: "rt71", reason: e.as_str().into() })?
        .words;
    Ok([w[0] as u16, (w[0] >> 16) as u16, w[1] as u16, (w[1] >> 16) as u16])
}

fn uniforms(list: &[uniform71::Word]) -> Vec<V3dUniform> {
    list.iter().map(|w| V3dUniform { contents: w.contents, data: w.data }).collect()
}

/// The draw's state, programs and arrays (a GL draw, or a clear quad).
fn build_draw(
    fr: &Frame,
    fb: &Framebuffer<'_>,
    rev: record71::Revision,
    d: &DrawIn<'_>,
    open: &mut Open,
    is_clear: bool,
) -> Result<FilledDraw, Refusal> {
    let mode = prim71::prim_mode(d.prim.mode).map_err(named("prim71"))?;
    prim71::pack_draw(&d.prim).map_err(named("prim71"))?;
    if d.prim.base_vertex != 0 || d.prim.base_instance != 0 || d.prim.indices.is_some_and(|i| i.restart) {
        return Err(refuse(REF_BASE));
    }
    let polygon = u32::from(mode) >= ffi71::V3D_PRIM_TRIANGLES;

    /* the GL state on this framebuffer */
    let mut st = d.state;
    st.target_w = fb.width;
    st.target_h = fb.height;
    st.flip_y = fb.flip_y;
    st.nr_targets = fb.colors.len() as u32;
    st.polygon = polygon;
    for (i, (_, row)) in fr.rows.iter().enumerate() {
        st.targets[i].has_alpha = row.has_alpha;
    }
    st.depth_test &= fr.zs.depth;
    st.stencil_test &= fr.zs.stencil;
    st.frag = draw71::FragProgram { writes_z: d.program.desc.fs.writes_z, discards: d.program.discards };
    let pk = draw71::translate(&st).map_err(|e| Refusal { module: "draw71", reason: e.message().into() })?;
    if st.independent_blend {
        let n = st.nr_targets as usize;
        if (1..n).any(|i| {
            st.targets[i].blend_enable != st.targets[0].blend_enable
                || (st.targets[0].blend_enable && st.targets[i].blend != st.targets[0].blend)
        }) {
            return Err(refuse(REF_ONE_BLEND));
        }
    }
    if st.targets[0].blend_enable && fr.rows.iter().any(|(_, r)| !r.has_alpha) {
        let e = &st.targets[0].blend;
        use draw71::BlendFactor::*;
        if [e.rgb_src, e.rgb_dst, e.alpha_src, e.alpha_dst]
            .iter()
            .any(|f| matches!(f, DstAlpha | InvDstAlpha | SrcAlphaSaturate))
        {
            return Err(refuse(REF_RGB_BLEND_ALPHA));
        }
    }
    let eff = |i: usize| if st.independent_blend { st.targets[i] } else { st.targets[0] };
    let colormask = one_mask(
        fb,
        fr.rows.iter().enumerate().map(|(i, (_, row))| eff(i).color_mask & if row.has_alpha { 0xf } else { 0x7 }),
    )?;

    let (scale, translate) = draw71::viewport_xform(&st);
    let mut s = V3dJob71State {
        vp: ffi71::V3dViewport { scale, translate },
        front_ccw: pk.cfg.clockwise_primitives,
        cull_front: polygon && matches!(st.cull, draw71::Cull::Front | draw71::Cull::FrontAndBack),
        cull_back: polygon && matches!(st.cull, draw71::Cull::Back | draw71::Cull::FrontAndBack),
        offset_tri: st.polygon_offset && polygon,
        offset_units: st.offset_units,
        offset_scale: st.offset_factor,
        offset_clamp: st.offset_clamp,
        line_width: d.line_width,
        depth_enabled: st.depth_test,
        depth_writemask: st.depth_test && st.depth_write,
        depth_func: compare_code(st.depth_func),
        stencil_enabled: st.stencil_test,
        stencil_back_enabled: st.stencil_test && st.stencil_two_sided,
        colormask,
        blend_color_f16: halves(d.blend_color)?,
        sample_mask: 0xf,
        oq_addr: d.oq_addr,
        ..V3dJob71State::default()
    };
    if let Some([minx, miny, maxx, maxy]) = draw71::scissor_box(&st) {
        (s.scissor, s.scissor_minx, s.scissor_miny, s.scissor_maxx, s.scissor_maxy) = (true, minx, miny, maxx, maxy);
    }
    if st.stencil_test {
        s.stencil_front = face(&st.stencil_front);
        if st.stencil_two_sided {
            s.stencil_back = face(&st.stencil_back);
        }
    }
    if let Some(b) = pk.blend_cfg[0] {
        s.blend_enabled = true;
        (s.rgb_func, s.rgb_src, s.rgb_dst) =
            (b.color_blend_mode as u32, b.color_blend_src_factor as u32, b.color_blend_dst_factor as u32);
        (s.alpha_func, s.alpha_src, s.alpha_dst) =
            (b.alpha_blend_mode as u32, b.alpha_blend_src_factor as u32, b.alpha_blend_dst_factor as u32);
    }

    /* the programs: attr71's records and record71's description */
    let p = d.program;
    let desc = &p.desc;
    let adm = admit71::admit_draw(
        rev,
        &admit71::Draw {
            attrs: d.attrs,
            cs_reads: p.cs_reads,
            vs_reads: p.vs_reads,
            cs_builtins: desc.cs.uses_vid || desc.cs.uses_iid,
            dummy_addr: d.dummy_addr,
            separate_segments: p.separate_segments,
            programs: *desc,
        },
    )?;
    if [&desc.vs, &desc.cs].iter().any(|v| v.min_input_segments != 1 || v.min_output_segments != 0)
        || [desc.fs.prog, desc.vs.prog, desc.cs.prog].iter().any(|g| g.propagate_nans)
        || desc.fs.sample_rate_shading
    {
        return Err(refuse(REF_RECORD_FIXED));
    }
    if desc.vs.uses_base_instance || desc.cs.uses_base_instance {
        return Err(refuse(REF_BASE_INSTANCE));
    }
    let mut attrs = Vec::with_capacity(d.attrs.len());
    for r in adm.attrs.records.iter().take(d.attrs.len()) {
        if r.ty != attr71::TYPE_FLOAT || r.signed || r.normalized || r.read_as_int {
            return Err(refuse(REF_FLOAT_ATTRS));
        }
        attrs.push(ffi71::V3dVertexAttr {
            address: r.address,
            stride: r.stride,
            components: if r.vec_size == 0 { 4 } else { u32::from(r.vec_size) },
            divisor: u32::from(r.divisor),
        });
    }

    /* the uniform streams: every word must be one 7.1 fills */
    let vp = uniform71::Viewport { scale, translate };
    for (list, words) in [(p.fs_uniforms, d.fs_words), (p.vs_uniforms, d.vs_words), (p.cs_uniforms, d.vs_words)] {
        let mut out = vec![0u32; list.len()];
        uniform71::fill(list, &vp, words, &mut out).map_err(named("uniform71"))?;
    }

    let fs_uniforms = uniforms(p.fs_uniforms);
    let vs_uniforms = uniforms(p.vs_uniforms);
    let cs_uniforms = uniforms(p.cs_uniforms);
    let ptr = |v: &Vec<V3dUniform>| if v.is_empty() { core::ptr::null() } else { v.as_ptr() };
    let fs = V3dFsProg {
        code: desc.fs.prog.code,
        threads: desc.fs.prog.threads,
        single_seg: desc.fs.prog.single_seg,
        num_inputs: desc.fs.num_varyings,
        flat_shade_flags: p.varyings.flat[0],
        noperspective_flags: p.varyings.noperspective[0],
        centroid_flags: p.varyings.centroid[0],
        flat_shade_flags_hi: p.varyings.flat[1],
        noperspective_flags_hi: p.varyings.noperspective[1],
        centroid_flags_hi: p.varyings.centroid[1],
        writes_z: desc.fs.writes_z,
        disable_ez: desc.fs.disable_ez,
        uses_center_w: desc.fs.uses_center_w,
        lock_scoreboard_on_first_thrsw: desc.fs.lock_scoreboard_on_first_thrsw,
        uses_implicit_point_line_varyings: desc.fs.uses_implicit_point_line_varyings,
        uses_pid: desc.fs.uses_pid,
        uniforms: ptr(&fs_uniforms),
        num_uniforms: fs_uniforms.len() as u32,
    };
    let vsp =
        |v: &record71::VertexDesc, reads: [u8; attr71::MAX_ATTRS], vcm: u32, u: &Vec<V3dUniform>| ffi71::V3dVsProg {
            code: v.prog.code,
            threads: v.prog.threads,
            single_seg: v.prog.single_seg,
            vpm_input_size: v.vpm_input_size,
            vpm_output_size: v.vpm_output_size,
            separate_segments: p.separate_segments,
            vcm_cache_size: vcm,
            uses_vid: v.uses_vid,
            uses_iid: v.uses_iid,
            uses_biid: v.uses_base_instance,
            vattr_sizes: reads,
            uniforms: ptr(u),
            num_uniforms: u.len() as u32,
        };
    let vs = vsp(&desc.vs, p.vs_reads, p.vs_vcm_cache_size, &vs_uniforms);
    let cs = vsp(&desc.cs, p.cs_reads, p.cs_vcm_cache_size, &cs_uniforms);

    /* the planes and targets the draw uses */
    open.z.draw(st.depth_test, st.depth_test && st.depth_write);
    let s_writes = face_writes(&st.stencil_front) || (st.stencil_two_sided && face_writes(&st.stencil_back));
    open.s.draw(st.stencil_test, st.stencil_test && s_writes);
    open.touched = true;

    let (indexed, index_type, index_buffer, index_buffer_bytes, index_offset) = match d.prim.indices {
        Some(i) => {
            let (ty, _) = prim71::index_type(i.gl_type).map_err(named("prim71"))?;
            (true, u32::from(ty), i.addr, i.size as u32, i.offset as u32)
        }
        None => (false, 0, 0, 0, 0),
    };
    Ok(FilledDraw {
        state: s,
        fs,
        vs,
        cs,
        fs_uniforms,
        vs_uniforms,
        cs_uniforms,
        vs_words: d.vs_words.to_vec(),
        fs_words: d.fs_words.to_vec(),
        attrs,
        prim: u32::from(mode),
        count: d.prim.count as u32,
        first: if indexed { 0 } else { d.prim.first as u32 },
        indexed,
        index_type,
        index_buffer,
        index_buffer_bytes,
        index_offset,
        instances: d.prim.instances as u32,
        vs_writes_psiz: desc.point_size,
        quad: None,
        is_clear,
    })
}

const GL_TRIANGLE_FAN: u32 = 6;

/// The clear quad of clearplan71's `Draw`.
fn clear_quad(
    fr: &Frame,
    fb: &Framebuffer<'_>,
    rev: record71::Revision,
    q: &ClearQuad<'_>,
    req: &ClearReq,
    cd: &clearplan71::ClearDraw,
    open: &mut Open,
) -> Result<FilledDraw, Refusal> {
    let nt = fb.colors.len();
    /* the one mask: what the quad writes of each target with a buffer */
    let masks: Vec<u8> = (0..nt)
        .map(|i| {
            let has_alpha = fr.rows[i].1.has_alpha;
            if cd.targets & (1 << i) != 0 {
                cd.color_masks[i] & if has_alpha { 0xf } else { 0x7 }
            } else {
                0
            }
        })
        .collect();
    let mask = one_mask(fb, masks.iter().copied())? as u8;
    let stencil = cd.stencil.then(|| (req.stencil.unwrap_or(0) & 0xff, u32::from(cd.stencil_mask)));
    let mut st = draw71::DrawState::blitter_clear(fb.width, fb.height, fb.flip_y, nt as u32, mask, stencil);
    if cd.depth {
        st.depth_test = true;
        st.depth_func = draw71::Compare::Always;
        st.depth_write = true;
    }
    let z = if cd.depth { 2.0 * req.depth.unwrap_or(1.0) - 1.0 } else { 0.0 };
    let r = cd.rect;
    let (w, h) = (fb.width as f32, fb.height as f32);
    let x0 = 2.0 * r.x as f32 / w - 1.0;
    let x1 = 2.0 * (r.x + r.w) as f32 / w - 1.0;
    let y0 = 2.0 * r.y as f32 / h - 1.0;
    let y1 = 2.0 * (r.y + r.h) as f32 / h - 1.0;
    let color = req.color.unwrap_or([0.0; 4]);
    /* one colour for the quad: alpha 1 where a target has none (its mask
     * leaves alpha alone anyway) */
    let all_alpha = fr.rows.iter().all(|(_, r)| r.has_alpha);
    let fs_words: Vec<u32> = clear_rgba(color, all_alpha).iter().map(|c| c.to_bits()).collect();
    let attrs = [q.attr];
    let d = DrawIn {
        state: st,
        line_width: 0.0,
        blend_color: [0.0; 4],
        program: q.program,
        attrs: &attrs,
        dummy_addr: q.dummy_addr,
        prim: prim71::Draw::arrays(GL_TRIANGLE_FAN, 0, 4),
        vs_words: &[],
        fs_words: &fs_words,
        oq_addr: 0,
    };
    let mut fd = build_draw(fr, fb, rev, &d, open, true)?;
    fd.quad = Some([[x0, y0, z, 1.0], [x1, y0, z, 1.0], [x1, y1, z, 1.0], [x0, y1, z, 1.0]]);
    Ok(fd)
}

fn end_job(fr: &Frame, hist: &History, open: Open) -> Result<FilledJob, Refusal> {
    let job = zskeep71::Job71 { z: open.z, s: open.s, discard: false };
    let dec = zskeep71::decide(&fr.zs, hist.z, hist.s, &job).map_err(zs_err)?;
    let f = dec.frame();
    let mut c = V3dJob71Clear::default();
    let mut after = *hist;
    for (n, rt) in fr.target.rt.iter().enumerate().take(fr.target.rts as usize) {
        if rt.address == 0 {
            continue;
        }
        match open.colors[n] {
            Some(w) => c.color[n] = w,
            None if hist.colors[n] => c.load_rts |= 1 << n,
            None => {}
        }
        after.colors[n] = true;
    }
    if f.zs_surface {
        let aspects = |z: bool, s: bool| {
            (if z { ffi71::V3D_JOB71_ZS_DEPTH } else { 0 }) | (if s { ffi71::V3D_JOB71_ZS_STENCIL } else { 0 })
        };
        c.zs_load = f.zs_load_z || f.zs_load_s;
        c.zs_store = f.zs_store_z || f.zs_store_s;
        c.zs_load_aspects = if c.zs_load { aspects(f.zs_load_z, f.zs_load_s) } else { 0 };
        c.zs_store_aspects = if c.zs_store { aspects(f.zs_store_z, f.zs_store_s) } else { 0 };
    }
    /* the start values: a clear's, zskeep71's 1.0 for an undefined depth the
     * job uses, 0 (Mesa's zeroed job) for a plane loaded or unused */
    c.z = match dec.z.start {
        zskeep71::Start71::Clear(v) => f32::from_bits(v),
        zskeep71::Start71::Undefined if open.z.needs() => f32::from_bits(f.z_clear_bits),
        _ => 0.0,
    };
    c.stencil = match dec.s.start {
        zskeep71::Start71::Clear(v) => v & 0xff,
        _ => 0,
    };
    c.zs_cleared = (fr.zs.depth || fr.zs.stencil) && !c.zs_load;
    after.z = dec.z.next;
    after.s = dec.s.next;
    Ok(FilledJob { target: fr.target, draws: open.draws, clear: c, after })
}

/// Fill the jobs of one flush of `fb` (see the module docs). `hist` is what
/// the buffers hold before it; each job's [`FilledJob::after`] is the next
/// job's. `quad` draws the clears the tile buffer cannot do. An empty result
/// is a flush with nothing to do.
pub fn fill(
    fb: &Framebuffer<'_>,
    rev: record71::Revision,
    hist: &History,
    quad: &ClearQuad<'_>,
    ops: &[Op<'_>],
) -> Result<Vec<FilledJob>, Refusal> {
    let fr = frame(fb)?;
    let nt = fb.colors.len();
    let mut jobs = Vec::new();
    let mut hist = *hist;
    let mut open = Open::new();
    for op in ops {
        match op {
            Op::Draw(d) => {
                let fd = build_draw(&fr, fb, rev, d, &mut open, false)?;
                open.draws.push(fd);
            }
            Op::Clear(req) => {
                let mut targets = [clearplan71::TargetIn { cleared: false, mask: 0 }; clearplan71::MAX_TARGETS];
                for (i, c) in fb.colors.iter().enumerate() {
                    targets[i] = clearplan71::TargetIn {
                        cleared: req.color.is_some() && req.draw_buffers & (1 << i) != 0 && c.address != 0,
                        mask: plan_mask(req.color_mask, fr.rows[i].1.has_alpha),
                    };
                }
                let (x, y, w, h) = req.scissor.map_or((0, 0, 0, 0), |s| (s[0], s[1], s[2], s[3]));
                let plan = clearplan71::plan_clear(&clearplan71::ClearIn {
                    fb_w: fb.width,
                    fb_h: fb.height,
                    n_targets: nt,
                    targets,
                    has_depth: fr.zs.depth,
                    has_stencil: fr.zs.stencil,
                    clear_depth: req.depth.is_some(),
                    clear_stencil: req.stencil.is_some(),
                    depth_mask: req.depth_mask,
                    stencil_mask: req.stencil_mask,
                    scissor: clearplan71::ScissorIn { enabled: req.scissor.is_some(), x, y, w, h },
                    ds_valid: open.z.needs() || open.s.needs(),
                    draws_queued: !open.draws.is_empty(),
                })
                .map_err(named("clearplan71"))?;
                match plan {
                    clearplan71::ClearPlan::Nothing => {}
                    clearplan71::ClearPlan::TileClear { targets: tt, depth, stencil, new_job } => {
                        if new_job {
                            let done = end_job(&fr, &hist, core::mem::replace(&mut open, Open::new()))?;
                            hist = done.after;
                            jobs.push(done);
                        }
                        let zk = |on: bool, v: u32| {
                            if on {
                                zskeep71::ClearKind71::Full(v)
                            } else {
                                zskeep71::ClearKind71::None
                            }
                        };
                        let route = zskeep71::route_clear(
                            &fr.zs,
                            !open.draws.is_empty(),
                            tt != 0,
                            zk(depth, req.depth.unwrap_or(1.0).clamp(0.0, 1.0).to_bits()),
                            zk(stencil, req.stencil.unwrap_or(0) & 0xff),
                        )
                        .map_err(zs_err)?;
                        open.z.apply(route.z);
                        open.s.apply(route.s);
                        for i in 0..nt {
                            if tt & (1 << i) == 0 || fb.colors[i].address == 0 {
                                continue;
                            }
                            let (rt, row) = &fr.rows[i];
                            let words = rt71::clear_words71(
                                rt,
                                rt71::ClearColor::Float(clear_rgba(req.color.unwrap_or([0.0; 4]), row.has_alpha)),
                            )
                            .map_err(|e| Refusal { module: "rt71", reason: e.as_str().into() })?;
                            open.colors[i] = Some(words.words);
                        }
                        open.touched = true;
                    }
                    clearplan71::ClearPlan::Draw(cd) => {
                        let kind =
                            |on: bool| if on { zskeep71::ClearKind71::Partial } else { zskeep71::ClearKind71::None };
                        let route = zskeep71::route_clear(
                            &fr.zs,
                            !open.draws.is_empty(),
                            false,
                            kind(cd.depth),
                            kind(cd.stencil),
                        )
                        .map_err(zs_err)?;
                        let fd = clear_quad(&fr, fb, rev, quad, req, &cd, &mut open)?;
                        /* the quad's own draw state already marked a depth
                         * write and a stencil write; the routes add the
                         * partial-clear use */
                        open.z.apply(route.z);
                        open.s.apply(route.s);
                        open.draws.push(fd);
                    }
                }
            }
        }
    }
    if open.touched {
        jobs.push(end_job(&fr, &hist, open)?);
    }
    Ok(jobs)
}
