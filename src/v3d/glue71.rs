// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The V3D 7.1 table's glue: the GL core's resolved draw ([`GlesDraw`],
//! src/backend.rs) into fill71's inputs ([`fill71::Framebuffer`]'s parts,
//! [`fill71::ClearReq`], [`fill71::DrawIn`], [`fill71::Op`]), or a named
//! [`Refusal`] (roadmap P9; the Pi 5's V3D 7.1.10; docs/notes/v3d71-gl-table.md).
//!
//! Pure: no FFI call, no feature cfg, no OS. Nothing hooks it into
//! `src/v3d/mod.rs` yet: the 7.1 table (`yakogl_backend_v3d71`, item
//! table71-draw) does. The siblings are reached as `super::{attr71, draw71,
//! ffi71, fill71, prim71}` (and fill71's own siblings): the same names when a
//! test includes them with `#[path]` at its crate root (tests/v3d71_glue.rs).
//!
//! # The GL core's types
//!
//! The input is the GL core's own [`GlesDraw`], read through
//! `crate::backend` (plain `#[repr(C)]` data; only its value fields are read,
//! never a pointer it holds). In the library that is src/backend.rs itself.
//! A host test reaches the same definitions the way every other test does,
//! through the library crate: its crate root has
//! `mod backend { pub use yakogl::backend::*; }` (tests/v3d71_glue.rs), so
//! no mirror struct exists that could drift from the real one. What the
//! GlesDraw points at (the uniform blocks, the staged arrays and indices)
//! and what the table itself owns (where it copied those, its depth/stencil
//! images, the tile allocation, the programs it linked) come in as plain
//! values: [`DsPlace`] and [`DrawInputs`].
//!
//! # What it does
//!
//! | function | from the GlesDraw | into |
//! |---|---|---|
//! | [`frame`] | the colour attachments and formats, the depth/stencil attachment, the size, the flip | [`Frame71`] (a [`fill71::Framebuffer`] with [`Frame71::framebuffer`]) |
//! | [`clear`] | the clear fields (`do_clear*`, the clear masks, `clear_quad` with its scissor box) | [`fill71::ClearReq`] |
//! | [`draw_state`] | viewport, depth range, scissor, colour masks, depth, stencil, blend, cull, offset | [`draw71::DrawState`] |
//! | [`prim`] | primitive, counts, instances, indices | [`prim71::Draw`] |
//! | [`attrs`] | the vertex arrays, by GL location | [`attr71::Attr`]s |
//! | [`ops`] | the whole draw: its fused clear first, then its geometry | [`fill71::Op`]s |
//!
//! [`refusal`] checks what neither fill71 nor its modules can take before
//! anything is mapped; every other value is decided where it already is
//! (fill71, draw71, clearplan71, zskeep71, admit71). One GlesDraw is one
//! `glClear` (index_count 0), one draw, or a draw with the clear the GL core
//! fused into it (src/gl/draw.rs:4385-4418, never a quad clear: that one is
//! its own GlesDraw before the draw, `settle_quad_clear`). [`ops`] gives
//! them in GL order, the clear before the draw (st_cb_clear.c's order).
//!
//! What the table keeps between draws is not here: which framebuffer the
//! open job is ([`Frame71`] is `PartialEq` for that), what each buffer holds
//! ([`fill71::History`], by the GL object ids `GlesDraw::depth_id`,
//! `stencil_id` and the colour addresses), the clear quad's program
//! ([`fill71::ClearQuad`]) and where it copies staged data.
//!
//! # Duplicated from src/v3d/state.rs (4.2)
//!
//! state.rs decides several of these values for the 4.2 table, but it reads
//! `crate::shaderc` and `super::ffi` (the 4.2 structs) and returns them in
//! 4.2 terms, so it cannot be called here without editing it. Each rule
//! below is the same rule, stated again, and named at its use:
//!
//! * the primitive (`GLES_PRIM_*` to the GL mode): state.rs `prim`;
//! * the stencil operations' order (`GLES_SOP_*` is KEEP first, the V3D's
//!   ZERO first): state.rs `stencil_op`;
//! * the cull face from the culled winding and glFrontFace: state.rs
//!   `job_state` (`GLES_CULL_CW` == `front_cw` culls the front);
//! * the two-sided stencil only when the faces differ (st/mesa
//!   `_TestTwoSide`): state.rs `job_state`;
//! * a blend factor reading destination alpha on a target without alpha
//!   reads 1 (`v3d_factor`'s `dst_alpha_one`): state.rs `blend_factor`;
//! * `color_write` false writes no channel: state.rs `job_state`;
//! * the refusals of alpha-to-coverage and of a draw list starting past
//!   attachment 0: state.rs `unsupported`.
//!
//! The viewport transform and the scissor box are not duplicated: the
//! GlesDraw's viewport and scissor go into [`draw71::DrawState`] as GL
//! values, and draw71 (`viewport_xform`, `scissor_box`) does what state.rs
//! `viewport` and `scissor` do for 4.2.
//!
//! # Choices (documented, tested)
//!
//! * The colour images default to what the GL core's surfaces are: raster
//!   rows at the target's address, attachment 0 at [`draw_pitch`] pixels,
//!   the others at the pair-padded width, 4 bytes a pixel. A table that
//!   renders into images of its own layout (a UIF render image) edits
//!   [`Frame71::colors`] after [`frame`]; nothing else reads the layout.
//! * The depth/stencil image is the table's ([`DsPlace`]): the GL core
//!   gives the planes (`depth_id`, `stencil_id`) and not the format. Without
//!   a format from the table, the planes pick it: both D24S8, depth alone
//!   `GL_DEPTH_COMPONENT24` (a D24S8 image moved as Z), stencil alone
//!   `GL_STENCIL_INDEX8` (fill71's D24S8 moved as stencil), the 4.2 table's
//!   one D24S8 per object. Address 0 is a depth/stencil the table keeps in
//!   the tile buffer only (zskeep71's `TileOnly`): a draw that needs what an
//!   earlier job left there is then zskeep71's refusal (gl_test case 59 on
//!   such a window).
//! * The frame is the attachments' intersection (the 4.2 table's
//!   `extra_targets`; the GL core lets the depth/stencil size differ in ES
//!   3.0, `GlesDraw::ds_w`).
//! * A format with fewer than 8 alpha bits (RGBA4, RGB5_A1) is drawn as
//!   its RGBA8 storage with alpha (the 4.2 table does the same: state.rs
//!   `job_state`, "rendered as RGBA8"; the GL core quantises its clear);
//!   with none (`target_alpha_bits` 0) it is `GL_RGB8` (fill71's alpha-1
//!   rule).
//! * A non-indexed draw starts at vertex 0: the GL core puts the first
//!   vertex into the array offsets (the 4.2 table's `first: 0`).
//! * glBlendColor is clamped to 0..1 here (fill71 takes it clamped).

use super::{attr71, draw71, ffi71, fill71, prim71};
use crate::backend::{
    draw_pitch, GlesAttrib, GlesDraw, GLES_ATTRIBF_BYTE, GLES_ATTRIBF_FIXED, GLES_ATTRIBF_FLOAT32, GLES_ATTRIBF_SHORT,
    GLES_ATTRIBF_UBYTE, GLES_ATTRIBF_USHORT, GLES_CULL_CCW, GLES_CULL_CW, GLES_CULL_NONE, GLES_MAX_TARGETS,
    GLES_PRIM_LINES, GLES_PRIM_LINE_LOOP, GLES_PRIM_LINE_STRIP, GLES_PRIM_POINTS, GLES_PRIM_TRIANGLES,
    GLES_PRIM_TRI_FAN, GLES_PRIM_TRI_STRIP, GLES_SOP_DECR_SAT, GLES_SOP_DECR_WRAP, GLES_SOP_INCR_SAT,
    GLES_SOP_INCR_WRAP, GLES_SOP_INVERT, GLES_SOP_KEEP, GLES_SOP_REPLACE, GLES_SOP_ZERO, GLES_SURF_RGBA8888,
    GLES_SURF_SRGB8_A8, GLES_SURF_XRGB8888,
};

pub use fill71::Refusal;

/// The refusals this module names itself (module `glue71`): what a GlesDraw
/// can carry that no 7.1 module takes yet, or a GlesDraw that is malformed.
pub const GLUE71_REFUSALS: [&str; 16] = [
    REF_SURFACE,
    REF_BGRX,
    REF_NO_TARGET,
    REF_ADDRESS,
    REF_TEXTURES,
    REF_ALPHA_TO_COVERAGE,
    REF_OUT_SHIFT,
    REF_SECOND_INDICES,
    REF_INSTANCE_ID,
    REF_DEPTH_TEXELS,
    REF_PRIM,
    REF_CULL,
    REF_COMPARE,
    REF_STENCIL_OP,
    REF_BLEND,
    REF_NO_PROGRAM,
];

const REF_SURFACE: &str =
    "V3D 7.1: no GL format is glued for this colour surface yet (RGBA8, RGB8 and SRGB8_ALPHA8 are)";
const REF_BGRX: &str = "V3D 7.1: an XRGB8888 surface (B,G,R in memory) needs the R/B swap fill71 does not write yet";
const REF_NO_TARGET: &str = "V3D 7.1: a draw with no colour target and no depth/stencil buffer";
const REF_ADDRESS: &str = "V3D 7.1: a buffer outside the V3D's 32-bit address space";
const REF_TEXTURES: &str = "V3D 7.1: a draw sampling textures (the 7.1 texture state is not glued yet)";
const REF_ALPHA_TO_COVERAGE: &str = "V3D 7.1: alpha-to-coverage is not implemented";
const REF_OUT_SHIFT: &str = "V3D 7.1: draw buffer 0 is GL_NONE while a later draw buffer is written";
const REF_SECOND_INDICES: &str = "V3D 7.1: a draw with a second index list";
const REF_INSTANCE_ID: &str = "V3D 7.1: an instance drawn on its own (gl_InstanceID given by the GL core)";
const REF_DEPTH_TEXELS: &str = "V3D 7.1: depth texels the CPU wrote are not seeded into the depth buffer yet";
const REF_PRIM: &str = "V3D 7.1: an unknown primitive";
const REF_CULL: &str = "V3D 7.1: an unknown cull mode";
const REF_COMPARE: &str = "V3D 7.1: an unknown compare function";
const REF_STENCIL_OP: &str = "V3D 7.1: an unknown stencil op";
const REF_BLEND: &str = "V3D 7.1: an unknown blend factor or equation";
const REF_NO_PROGRAM: &str = "V3D 7.1: a draw with no linked program";

fn refuse(reason: &str) -> Refusal {
    Refusal { module: "glue71", reason: reason.to_string() }
}

/* ---- the framebuffer --------------------------------------------------------- */

/// Where the table keeps the depth/stencil of `GlesDraw::ds_id`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DsPlace {
    /// The GL sized format of the image, when the table knows it; `None`:
    /// picked from the planes the GlesDraw names (module docs).
    pub gl_format: Option<u32>,
    /// GPU address of the image; 0: the tile buffer only.
    pub address: u32,
    /// `V3D_MEMORY_FORMAT_*`.
    pub memory_format: u32,
    /// UIF: the padded height in UIF blocks; raster: the stride.
    pub height_in_ub: u32,
}

/// One GlesDraw's framebuffer in fill71's terms.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame71 {
    pub width: u32,
    pub height: u32,
    pub flip_y: bool,
    /// `colors[..n]` are the colour attachments (fill71 takes `n` of them).
    pub colors: [fill71::ColorBuffer; ffi71::V3D_JOB71_MAX_RTS],
    pub n: usize,
    pub ds: Option<fill71::DsBuffer>,
}

impl Frame71 {
    /// fill71's framebuffer, with the table's tile allocation memory.
    pub fn framebuffer(&self, tile_alloc: u32) -> fill71::Framebuffer<'_> {
        fill71::Framebuffer {
            width: self.width,
            height: self.height,
            tile_alloc,
            flip_y: self.flip_y,
            colors: &self.colors[..self.n],
            ds: self.ds,
        }
    }
}

/// The GL sized format fill71 maps for a GL-core surface format, or the
/// `glue71` refusal. `alpha_bits` is `target_alpha_bits` (attachment 0) or 8.
pub fn gl_format(surf: u8, alpha_bits: u8) -> Result<u32, Refusal> {
    match surf {
        GLES_SURF_RGBA8888 if alpha_bits == 0 => Ok(fill71::GL_RGB8),
        GLES_SURF_RGBA8888 => Ok(fill71::GL_RGBA8),
        GLES_SURF_SRGB8_A8 => Ok(fill71::GL_SRGB8_ALPHA8),
        GLES_SURF_XRGB8888 => Err(refuse(REF_BGRX)),
        _ => Err(refuse(REF_SURFACE)),
    }
}

/// The depth/stencil format the planes pick when the table names none.
pub fn ds_gl_format(depth: bool, stencil: bool) -> Option<u32> {
    match (depth, stencil) {
        (true, true) => Some(fill71::GL_DEPTH24_STENCIL8),
        (true, false) => Some(fill71::GL_DEPTH_COMPONENT24),
        (false, true) => Some(fill71::GL_STENCIL_INDEX8),
        (false, false) => None,
    }
}

fn addr32(a: u64) -> Result<u32, Refusal> {
    u32::try_from(a).map_err(|_| refuse(REF_ADDRESS))
}

/// The framebuffer of `d`. `ds` is where the table keeps `d.ds_id`'s
/// depth/stencil; it is read only when the draw has one (`ds_persist`).
pub fn frame(d: &GlesDraw, ds: DsPlace) -> Result<Frame71, Refusal> {
    let mut f = Frame71 {
        width: u32::MAX,
        height: u32::MAX,
        flip_y: d.flip_y,
        colors: [fill71::ColorBuffer { gl_format: 0, address: 0, memory_format: 0, height_in_ub_or_stride: 0 };
            ffi71::V3D_JOB71_MAX_RTS],
        n: 0,
        ds: None,
    };
    if d.target_gpu != 0 {
        let n = (d.num_targets.max(1) as usize).min(GLES_MAX_TARGETS);
        for k in 0..n {
            /* targets[0] mirrors target_*; the frame target's own fields
             * carry its alpha bits and pitch */
            let (gpu, w, h, surf, alpha, pitch) = if k == 0 {
                (d.target_gpu, d.target_w, d.target_h, d.target_format, d.target_alpha_bits, draw_pitch(d))
            } else {
                let t = &d.targets[k];
                (t.gpu, t.w, t.h, t.format, 8, t.w.wrapping_add(1) & !1)
            };
            let gl = gl_format(surf, alpha)?;
            f.colors[k] = fill71::ColorBuffer {
                gl_format: gl,
                address: addr32(gpu)?,
                memory_format: ffi71::V3D_MEMORY_FORMAT_RASTER,
                height_in_ub_or_stride: pitch.saturating_mul(4),
            };
            if gpu != 0 {
                (f.width, f.height) = (f.width.min(w), f.height.min(h));
            }
        }
        f.n = n;
    }
    if d.ds_persist {
        let (depth, stencil) = (d.depth_id != 0, d.stencil_id != 0);
        if let Some(gl) = ds.gl_format.or(ds_gl_format(depth, stencil)) {
            f.ds = Some(fill71::DsBuffer {
                gl_format: gl,
                address: ds.address,
                memory_format: ds.memory_format,
                height_in_ub: ds.height_in_ub,
            });
            let (w, h) = if d.ds_w != 0 && d.ds_h != 0 { (d.ds_w, d.ds_h) } else { (d.target_w, d.target_h) };
            (f.width, f.height) = (f.width.min(w), f.height.min(h));
        }
    }
    if f.n == 0 && f.ds.is_none() {
        return Err(refuse(REF_NO_TARGET));
    }
    Ok(f)
}

/* ---- the clear ---------------------------------------------------------------- */

/// The `glClear` a GlesDraw carries (alone or fused into its draw), or
/// `None`. The GL core has applied glDepthMask and dropped what the
/// framebuffer lacks (src/gl/draw.rs `plan_clear`); a quad clear
/// (`clear_quad`) brings its scissor box, which clearplan71 turns into the
/// quad, as it does a partial colour or stencil mask.
pub fn clear(d: &GlesDraw) -> Option<fill71::ClearReq> {
    if !(d.do_clear || d.do_clear_depth || d.do_clear_stencil) {
        return None;
    }
    let n = (d.num_targets.max(1) as usize).min(GLES_MAX_TARGETS);
    let every = ((1u32 << n) - 1) as u8;
    /* 0 is "not given" for each of the three masks (src/backend.rs) */
    let stencil_mask = if d.clear_stencil_write_mask != 0 { d.clear_stencil_write_mask } else { d.stencil_write_mask };
    Some(fill71::ClearReq {
        color: d.do_clear.then_some(d.clear_rgba),
        draw_buffers: if d.clear_target_mask == 0 { every } else { d.clear_target_mask & every },
        color_mask: if d.clear_color_mask == 0 { 0xf } else { d.clear_color_mask & 0xf },
        depth: d.do_clear_depth.then_some(d.clear_depth),
        depth_mask: true,
        stencil: d.do_clear_stencil.then_some(u32::from(d.clear_stencil)),
        stencil_mask,
        scissor: (d.clear_quad && d.scissor).then_some(d.scissor_rect),
    })
}

/* ---- the draw's state ---------------------------------------------------------- */

/// What a GlesDraw carries that no 7.1 module takes yet (checked before
/// anything else, so a refused draw leaves its fused clear unrecorded too).
pub fn refusal(d: &GlesDraw) -> Option<Refusal> {
    if d.samples > 1 {
        return Some(Refusal { module: "draw71", reason: draw71::Refusal71::Multisample.message().into() });
    }
    if d.index_count == 0 {
        return None; /* a clear alone: its state is the clear quad's own */
    }
    if d.num_textures != 0 || d.num_vs_textures != 0 {
        return Some(refuse(REF_TEXTURES));
    }
    /* state.rs `unsupported` */
    if d.alpha_to_coverage {
        return Some(refuse(REF_ALPHA_TO_COVERAGE));
    }
    if d.targets[0].out_shift != 0 {
        return Some(refuse(REF_OUT_SHIFT));
    }
    if d.index_count2 != 0 {
        return Some(refuse(REF_SECOND_INDICES));
    }
    if d.instance_id != 0 {
        return Some(refuse(REF_INSTANCE_ID));
    }
    if !d.depth_texels.is_null() {
        return Some(refuse(REF_DEPTH_TEXELS));
    }
    None
}

fn compare(c: u32) -> Result<draw71::Compare, Refusal> {
    use draw71::Compare::*;
    /* GLES_CMP_* is NEVER .. ALWAYS in the V3D's order */
    [Never, Less, Equal, LEqual, Greater, NotEqual, GEqual, Always]
        .get(c as usize)
        .copied()
        .ok_or_else(|| refuse(REF_COMPARE))
}

/// state.rs `stencil_op`: `GLES_SOP_*` (KEEP first) to the V3D's op.
fn stencil_op(op: u32) -> Result<draw71::StencilOp, Refusal> {
    use draw71::StencilOp::*;
    Ok(match op {
        GLES_SOP_KEEP => Keep,
        GLES_SOP_ZERO => Zero,
        GLES_SOP_REPLACE => Replace,
        GLES_SOP_INCR_SAT => Incr,
        GLES_SOP_DECR_SAT => Decr,
        GLES_SOP_INVERT => Invert,
        GLES_SOP_INCR_WRAP => IncWrap,
        GLES_SOP_DECR_WRAP => DecWrap,
        _ => return Err(refuse(REF_STENCIL_OP)),
    })
}

/* `shc_blend_factor` (gallium's pipe_blendfactor; src/shaderc/mod.rs:59-74):
 * the codes the GlesDraw carries, stated here as numbers (the module reads
 * no crate::shaderc) */
const BF_ONE: u8 = 1;
const BF_SRC_COLOR: u8 = 2;
const BF_SRC_ALPHA: u8 = 3;
const BF_DST_ALPHA: u8 = 4;
const BF_DST_COLOR: u8 = 5;
const BF_SRC_ALPHA_SATURATE: u8 = 6;
const BF_CONST_COLOR: u8 = 7;
const BF_CONST_ALPHA: u8 = 8;
const BF_INVERT: u8 = 0x10;

/// state.rs `blend_factor`: a `shc_blend_factor` code, with destination
/// alpha read as 1 on a target without alpha (`v3d_factor`).
fn blend_factor(f: u8, dst_alpha_one: bool) -> Result<draw71::BlendFactor, Refusal> {
    use draw71::BlendFactor::*;
    Ok(match f {
        BF_ONE => One,
        x if x == BF_INVERT | BF_ONE => Zero,
        BF_SRC_COLOR => SrcColor,
        x if x == BF_INVERT | BF_SRC_COLOR => InvSrcColor,
        BF_DST_COLOR => DstColor,
        x if x == BF_INVERT | BF_DST_COLOR => InvDstColor,
        BF_SRC_ALPHA => SrcAlpha,
        x if x == BF_INVERT | BF_SRC_ALPHA => InvSrcAlpha,
        BF_DST_ALPHA if dst_alpha_one => One,
        BF_DST_ALPHA => DstAlpha,
        x if x == BF_INVERT | BF_DST_ALPHA && dst_alpha_one => Zero,
        x if x == BF_INVERT | BF_DST_ALPHA => InvDstAlpha,
        BF_CONST_COLOR => ConstColor,
        x if x == BF_INVERT | BF_CONST_COLOR => InvConstColor,
        BF_CONST_ALPHA => ConstAlpha,
        x if x == BF_INVERT | BF_CONST_ALPHA => InvConstAlpha,
        BF_SRC_ALPHA_SATURATE if dst_alpha_one => Zero,
        BF_SRC_ALPHA_SATURATE => SrcAlphaSaturate,
        _ => return Err(refuse(REF_BLEND)),
    })
}

/// `shc_blend_func` (pipe_blend_func: ADD, SUBTRACT, REVERSE_SUBTRACT, MIN,
/// MAX) is the V3D's Blend Mode order.
fn blend_mode(m: u8) -> Result<draw71::BlendMode, Refusal> {
    use draw71::BlendMode::*;
    [Add, Sub, RSub, Min, Max].get(m as usize).copied().ok_or_else(|| refuse(REF_BLEND))
}

/// The draw's GL state on `fr` (fill71 overwrites the target fields and
/// `polygon` from the framebuffer and the primitive itself).
pub fn draw_state(d: &GlesDraw, fr: &Frame71) -> Result<draw71::DrawState, Refusal> {
    let n = fr.n.max(1);
    let mut st = draw71::DrawState::gl_default(fr.width, fr.height, fr.flip_y, fr.n as u32);
    st.samples = u32::from(d.samples.max(1));
    st.viewport = d.viewport.map(|v| v as f32);
    st.depth_range = d.depth_range;
    st.scissor = d.scissor.then_some(d.scissor_rect);

    /* colour: color_write off writes nothing (state.rs `job_state`); a mask
     * per target where the GL core gives them */
    let mask = if d.color_write { d.color_mask & 0xf } else { 0 };
    let blend = if d.blend {
        let dst_alpha_one = d.target_alpha_bits == 0;
        Some(draw71::BlendEq {
            rgb_mode: blend_mode(d.blend_eq_rgb)?,
            rgb_src: blend_factor(d.blend_src_rgb, dst_alpha_one)?,
            rgb_dst: blend_factor(d.blend_dst_rgb, dst_alpha_one)?,
            alpha_mode: blend_mode(d.blend_eq_alpha)?,
            alpha_src: blend_factor(d.blend_src_alpha, dst_alpha_one)?,
            alpha_dst: blend_factor(d.blend_dst_alpha, dst_alpha_one)?,
        })
    } else {
        None
    };
    for (k, t) in st.targets.iter_mut().enumerate() {
        t.color_mask = if d.per_target_mask && d.color_write && k < n {
            d.target_color_mask[k.min(GLES_MAX_TARGETS - 1)] & 0xf
        } else {
            mask
        };
        if let Some(b) = blend {
            t.blend_enable = true;
            t.blend = b;
        }
    }
    if d.per_target_mask && d.color_write {
        /* fill71 reads each target's mask (and refuses them when they
         * differ: v3d_job71 writes one) */
        st.independent_blend = true;
        st.targets[0].color_mask = mask;
    }

    st.depth_test = d.depth_test;
    st.depth_func = compare(d.depth_func)?;
    st.depth_write = d.depth_write;

    st.stencil_test = d.stencil;
    let front = draw71::StencilFace {
        func: compare(d.stencil_func)?,
        sfail: stencil_op(d.stencil_sfail)?,
        zfail: stencil_op(d.stencil_dpfail)?,
        zpass: stencil_op(d.stencil_dppass)?,
        reference: u32::from(d.stencil_ref),
        value_mask: u32::from(d.stencil_cmp_mask),
        write_mask: u32::from(d.stencil_write_mask),
    };
    let back = draw71::StencilFace {
        func: compare(d.stencil_back_func)?,
        sfail: stencil_op(d.stencil_back_sfail)?,
        zfail: stencil_op(d.stencil_back_dpfail)?,
        zpass: stencil_op(d.stencil_back_dppass)?,
        reference: u32::from(d.stencil_back_ref),
        value_mask: u32::from(d.stencil_back_cmp_mask),
        write_mask: u32::from(d.stencil_back_write_mask),
    };
    st.stencil_front = front;
    /* st/mesa's _TestTwoSide: the back state only when the faces differ */
    st.stencil_two_sided = back != front;
    st.stencil_back = if st.stencil_two_sided { back } else { front };

    st.polygon_offset = d.polygon_offset;
    st.offset_factor = d.polygon_offset_factor;
    st.offset_units = d.polygon_offset_units;
    st.offset_clamp = 0.0;

    st.front_cw = d.front_cw;
    /* the GL core names the winding it culls (draw.rs map_cull); the front
     * face's winding is glFrontFace's (state.rs `job_state`) */
    st.cull = match d.cull {
        GLES_CULL_NONE => draw71::Cull::None,
        GLES_CULL_CW | GLES_CULL_CCW if (d.cull == GLES_CULL_CW) == d.front_cw => draw71::Cull::Front,
        GLES_CULL_CW | GLES_CULL_CCW => draw71::Cull::Back,
        _ => return Err(refuse(REF_CULL)),
    };
    Ok(st)
}

/* ---- the call --------------------------------------------------------------------- */

/// state.rs `prim`: `GLES_PRIM_*` to the GL mode prim71 takes.
pub fn gl_mode(p: u32) -> Result<u32, Refusal> {
    use prim71::{GL_LINES, GL_LINE_LOOP, GL_LINE_STRIP, GL_POINTS, GL_TRIANGLES, GL_TRIANGLE_FAN, GL_TRIANGLE_STRIP};
    Ok(match p {
        GLES_PRIM_TRIANGLES => GL_TRIANGLES,
        GLES_PRIM_TRI_STRIP => GL_TRIANGLE_STRIP,
        GLES_PRIM_TRI_FAN => GL_TRIANGLE_FAN,
        GLES_PRIM_LINES => GL_LINES,
        GLES_PRIM_LINE_STRIP => GL_LINE_STRIP,
        GLES_PRIM_LINE_LOOP => GL_LINE_LOOP,
        GLES_PRIM_POINTS => GL_POINTS,
        _ => return Err(refuse(REF_PRIM)),
    })
}

/// Where the table put the draw's indices: `GlesDraw::index_buf_gpu`, or
/// the copy of the staged `index_cpu` (`index_bytes` long).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IndexPlace {
    pub addr: u32,
    pub size: u64,
}

/// The draw call. `indices` is the table's placement of an indexed draw's
/// indices (ignored for a non-indexed one).
pub fn prim(d: &GlesDraw, indices: Option<IndexPlace>) -> Result<prim71::Draw, Refusal> {
    let mut p = prim71::Draw::arrays(gl_mode(d.prim)?, 0, u64::from(d.index_count));
    p.instances = u64::from(d.instance_count.max(1));
    if d.index_buf_gpu != 0 || !d.index_cpu.is_null() {
        let gl_type = match d.index_size {
            1 => prim71::GL_UNSIGNED_BYTE,
            2 => prim71::GL_UNSIGNED_SHORT,
            4 => prim71::GL_UNSIGNED_INT,
            t => {
                return Err(Refusal {
                    module: "prim71",
                    reason: prim71::Refusal71::IndexType(u32::from(t)).to_string(),
                })
            }
        };
        let at = match indices {
            Some(i) => i,
            None if d.index_cpu.is_null() => {
                IndexPlace { addr: addr32(d.index_buf_gpu)?, size: u64::from(d.index_count) * u64::from(d.index_size) }
            }
            None => return Err(refuse(REF_ADDRESS)),
        };
        p.indices = Some(prim71::Indices { gl_type, addr: at.addr, size: at.size, offset: 0, restart: false });
    }
    Ok(p)
}

/// The GL vertex arrays, index = attribute location (`vi_reg`), the
/// locations no array names disabled. `staged[k]` is where the table copied
/// `attribs[k]`'s draw-time bytes (`cpu`), read only for those.
pub fn attrs(d: &GlesDraw, staged: &[u32]) -> Result<Vec<attr71::Attr>, Refusal> {
    let list = &d.attribs[..(d.num_attribs as usize).min(d.attribs.len())];
    let len = list.iter().map(|a| a.vi_reg as usize + 1).max().unwrap_or(0);
    if len > attr71::MAX_ATTRS {
        return Err(Refusal { module: "attr71", reason: attr71::Refusal71::TooManyAttributes(len).message().into() });
    }
    let mut out = vec![attr71::Attr::disabled(); len];
    for (k, a) in list.iter().enumerate() {
        out[a.vi_reg as usize] = attr(a, if a.cpu.is_null() { None } else { staged.get(k).copied() })?;
    }
    Ok(out)
}

fn attr(a: &GlesAttrib, staged: Option<u32>) -> Result<attr71::Attr, Refusal> {
    let gl_type = match a.format {
        GLES_ATTRIBF_FLOAT32 => attr71::GL_FLOAT,
        GLES_ATTRIBF_BYTE => attr71::GL_BYTE,
        GLES_ATTRIBF_UBYTE => attr71::GL_UNSIGNED_BYTE,
        GLES_ATTRIBF_SHORT => attr71::GL_SHORT,
        GLES_ATTRIBF_USHORT => attr71::GL_UNSIGNED_SHORT,
        GLES_ATTRIBF_FIXED => attr71::GL_FIXED,
        f => return Err(Refusal { module: "attr71", reason: attr71::Refusal71::Type(u32::from(f)).message().into() }),
    };
    let buffer_addr = match (a.cpu.is_null(), staged) {
        (true, _) => addr32(a.buffer_gpu.checked_add(u64::from(a.offset)).ok_or_else(|| refuse(REF_ADDRESS))?)?,
        (false, Some(at)) => at,
        (false, None) => return Err(refuse(REF_ADDRESS)),
    };
    Ok(attr71::Attr {
        enabled: true,
        components: u32::from(a.components),
        gl_type,
        normalized: a.normalized != 0,
        integer: false,
        stride: u64::from(a.stride),
        offset: 0,
        buffer_addr,
        divisor: if a.per_instance != 0 { u32::from(a.divisor.max(1)) } else { 0 },
        generic: attr71::Attr::disabled().generic,
        constant_addr: None,
    })
}

/// What the table gives a draw besides the GlesDraw: the linked program,
/// the arrays ([`attrs`]), the uniform blocks (`vs_uniform_cpu`,
/// `fs_uniform_cpu` as words), the GFXH-930 dummy address, the occlusion
/// query's counter (0 for none) and the placed indices ([`prim`]).
#[derive(Clone, Copy, Debug)]
pub struct DrawInputs<'a> {
    pub program: &'a fill71::Program<'a>,
    pub attrs: &'a [attr71::Attr],
    pub vs_words: &'a [u32],
    pub fs_words: &'a [u32],
    pub dummy_addr: u32,
    pub oq_addr: u32,
    pub indices: Option<IndexPlace>,
}

/// One GlesDraw as fill71's operations, in GL order: its clear (fused or
/// alone), then its geometry when it draws (`index_count` > 0). `input` is
/// read only for a draw.
pub fn ops<'a>(d: &GlesDraw, fr: &Frame71, input: Option<&DrawInputs<'a>>) -> Result<Vec<fill71::Op<'a>>, Refusal> {
    if let Some(r) = refusal(d) {
        return Err(r);
    }
    let mut out = Vec::with_capacity(2);
    if let Some(c) = clear(d) {
        out.push(fill71::Op::Clear(c));
    }
    if d.index_count != 0 {
        let Some(i) = input else {
            return Err(refuse(REF_NO_PROGRAM));
        };
        out.push(fill71::Op::Draw(Box::new(fill71::DrawIn {
            state: draw_state(d, fr)?,
            line_width: d.line_width,
            blend_color: d.blend_color.map(|c| c.clamp(0.0, 1.0)),
            program: i.program,
            attrs: i.attrs,
            dummy_addr: i.dummy_addr,
            prim: prim(d, i.indices)?,
            vs_words: i.vs_words,
            fs_words: i.fs_words,
            oq_addr: i.oq_addr,
        })));
    }
    Ok(out)
}
