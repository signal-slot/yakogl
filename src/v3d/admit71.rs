// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.

//! The 7.1 table's admission: one place that accepts a GL frame or draw, or
//! refuses it by name (docs/notes/v3d71-gl-table.md sections 2 to 6,
//! docs/notes/v3d71-record-u11.md section 4).
//!
//! Pure: no FFI, no feature cfg, no `crate::` path. It reaches the sibling
//! modules as `super::{ident71, limits71, rt71, attr71, record71, format71}`
//! (the same names when a test includes them with `#[path]` at its crate
//! root). It re-derives no figure and invents no refusal where a module has
//! one: every [`Refusal`] is one of those modules' own errors, tagged with
//! the module's name and its own text, except two: limits71 answers `None`
//! for a tile size that does not fit (named `limits71` here), and a colour
//! format given as the depth/stencil buffer is named `admit71`.
//!
//! What the modules do not cover is not checked here: the primitive (prim71),
//! the depth and stencil write state (draw71, record71's `writes_z` is
//! carried by the program description) and the clear plan (clearplan71).

use super::{attr71, format71, ident71, limits71, record71, rt71};

/// A named refusal: the module whose rule fired and its reason text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refusal {
    pub module: &'static str,
    pub reason: String,
}

impl core::fmt::Display for Refusal {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}: {}", self.module, self.reason)
    }
}

fn from_rt(r: rt71::Refusal71) -> Refusal {
    Refusal { module: "rt71", reason: r.as_str().to_string() }
}

fn from_attr(r: attr71::Refusal71) -> Refusal {
    Refusal { module: "attr71", reason: r.message().to_string() }
}

fn from_record(r: record71::Refusal) -> Refusal {
    Refusal { module: "record71", reason: r.message() }
}

fn from_ident(r: ident71::Refusal) -> Refusal {
    Refusal { module: "ident71", reason: r.to_string() }
}

fn from_format(r: format71::Refusal71) -> Refusal {
    Refusal { module: "format71", reason: format!("{r:?}") }
}

/// The hub identity (`HUB_IDENT1`, `HUB_IDENT3`): 7.1 with IPREV >= 10.
pub fn admit_ident(hub_ident1: u32, hub_ident3: u32) -> Result<ident71::HubIdent, Refusal> {
    ident71::check_words(hub_ident1, hub_ident3).map_err(from_ident)
}

/// A texture's GL format and type: format71 must have a TMU answer.
pub fn admit_texture(internal: u32, ty: u32) -> Result<format71::Row71, Refusal> {
    format71::tmu_format71(internal, ty).map_err(from_format)
}

/// What a buffer of a job does: loaded from memory at the start, stored at
/// the end, cleared at the start.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BufferUse {
    pub load: bool,
    pub store: bool,
    pub clear: bool,
}

/// One colour target of a frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColorTarget {
    /// GL sized internal format (`GL_RGBA8` = 0x8058).
    pub gl_format: u32,
    pub uses: BufferUse,
    /// The clear colour, when `uses.clear`; checked against the format's kind.
    pub clear_color: Option<rt71::ClearColor>,
    /// The target is a texture the program samples later: format71 must
    /// have a TMU answer for it too.
    pub sampled: bool,
}

/// The depth/stencil buffer of a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DepthStencil {
    /// GL sized internal format (`GL_DEPTH24_STENCIL8`, `GL_DEPTH_COMPONENT16`,
    /// `GL_STENCIL_INDEX8`, ...).
    pub gl_format: u32,
    pub depth: BufferUse,
    pub stencil: BufferUse,
}

/// A frame (one job's target set).
#[derive(Clone, Debug, PartialEq)]
pub struct Frame<'a> {
    pub width: u32,
    pub height: u32,
    pub colors: &'a [ColorTarget],
    pub depth_stencil: Option<DepthStencil>,
    pub msaa: bool,
    pub double_buffer: bool,
}

/// What an admitted frame needs from the job builder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameAdmitted {
    /// limits71's tile size for the targets' summed bytes per pixel.
    pub tile: (u32, u32),
    /// bytes per pixel the colour targets take in the tile buffer.
    pub color_bytes: u32,
}

/// Accept a frame or name the first refusal. Order: size, target count,
/// each colour target (rt71 format, format71 for a sampled one, rt71 clear
/// kind), the depth/stencil format, the tile size.
pub fn admit_frame(f: &Frame<'_>) -> Result<FrameAdmitted, Refusal> {
    rt71::check_size(f.width, f.height).map_err(from_rt)?;
    rt71::check_targets(f.colors.len() as u32).map_err(from_rt)?;
    let mut color_bytes = 0;
    for c in f.colors {
        let rt = rt71::rt_format71(c.gl_format).map_err(from_rt)?;
        if c.sampled {
            admit_texture(c.gl_format, 0)?;
        }
        if let (true, Some(color)) = (c.uses.clear, c.clear_color) {
            rt71::clear_words71(&rt, color).map_err(from_rt)?;
        }
        color_bytes += 4 * rt71::bpp_words(rt.bpp);
    }
    if let Some(ds) = &f.depth_stencil {
        match rt71::rt_format71(ds.gl_format) {
            Err(rt71::Refusal71::NotAColourFormat) => {}
            Err(e) => return Err(from_rt(e)),
            Ok(_) => {
                return Err(Refusal {
                    module: "admit71",
                    reason: "a colour format is not a depth or stencil format".to_string(),
                })
            }
        }
    }
    let tile = limits71::choose_tile_size71_total(color_bytes, f.msaa, f.double_buffer).ok_or_else(|| Refusal {
        module: "limits71",
        reason: format!("no tile size fits {color_bytes} colour bytes per pixel"),
    })?;
    Ok(FrameAdmitted { tile, color_bytes })
}

/// A draw: its attributes and programs.
#[derive(Clone, Debug)]
pub struct Draw<'a> {
    pub attrs: &'a [attr71::Attr],
    /// the coordinate and vertex programs' `vattr_sizes`
    pub cs_reads: [u8; attr71::MAX_ATTRS],
    pub vs_reads: [u8; attr71::MAX_ATTRS],
    pub cs_builtins: bool,
    pub dummy_addr: u32,
    /// the programs need separate input and output VPM segments
    pub separate_segments: bool,
    /// The programs, including their draw-index / base-vertex use.
    pub programs: record71::DrawDesc,
}

/// What an admitted draw needs from the job builder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DrawAdmitted {
    pub attrs: attr71::Plan71,
    pub record: record71::Record,
}

/// Accept a draw on `rev` or name the first refusal: attr71's plan (and its
/// VPM segment rule), then record71's description.
pub fn admit_draw(rev: record71::Revision, d: &Draw<'_>) -> Result<DrawAdmitted, Refusal> {
    attr71::vpm_input_size(d.separate_segments).map_err(from_attr)?;
    let attrs = attr71::plan(d.attrs, &d.cs_reads, &d.vs_reads, d.cs_builtins, d.dummy_addr).map_err(from_attr)?;
    let record = record71::describe(rev, &d.programs).map_err(from_record)?;
    Ok(DrawAdmitted { attrs, record })
}
