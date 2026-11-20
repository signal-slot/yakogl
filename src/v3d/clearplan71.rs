// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.

//! How a `glClear` is carried out on V3D 7.1 (docs/notes/v3d71-gl-table.md,
//! the clear rows; Mesa's 7.1 choices as the `tools/v3d/pi5/gl_*.clif`
//! references show them).
//!
//! Pure over plain values, no `crate::` import. Two ways exist:
//!
//! * a **tile-buffer clear**: the one-byte `Clear Render Targets` packet 25
//!   (`frame71::OP_CLEAR_RT`, `rt71::CLEAR_RT_CODE`) in the render list's
//!   first tile and in the generic tile list, with the colour clear values in
//!   the `Render Target Part1/2/3` packets (`rt71::rt_format71` and
//!   `rt71::clear_words71` pack them per GL format) and the depth and stencil
//!   values in `Tile Rendering Mode Cfg (ZS Clear Values)`. The packet clears
//!   every render target and the depth/stencil buffer of the tile, so it is
//!   only right when the clear covers every buffer the job keeps;
//! * a **clear draw**: a quad with the clear program (`ours_clear`), the
//!   colour write masks (`COLOR_WMASKS`, one per target), the depth and
//!   stencil state and the scissor as the clip window. A masked or scissored
//!   clear, and a clear of a part of the buffers, is this (Mesa's blitter
//!   clear); the references' jobs show the extra draw and, for a scissor, the
//!   box as the `Clip Window`.
//!
//! [`plan_clear`] decides; a shape it does not carry is a named [`Refusal`].
//! The clear value packing is not here.

use core::fmt;

/// V3D 7.1's render-target count (`limits71`; pre-study section 3).
pub const MAX_TARGETS: usize = 8;
/// V3D 7.1's largest framebuffer side in pixels.
pub const MAX_FB_DIM: u32 = 7680;
/// The four colour channels written: bit 0 R, 1 G, 2 B, 3 A.
pub const MASK_RGBA: u8 = 0xf;

/// One colour target of the framebuffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TargetIn {
    /// `glClear(GL_COLOR_BUFFER_BIT)` reaches it: a live draw buffer.
    pub cleared: bool,
    /// The colour write mask, bit 0 R .. bit 3 A (set = written).
    pub mask: u8,
}

/// `glScissor`: the box in pixels, origin at the bottom left.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScissorIn {
    pub enabled: bool,
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

/// What the GL core knows when it issues a `glClear`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClearIn {
    pub fb_w: u32,
    pub fb_h: u32,
    /// Colour attachments, `targets[..n_targets]`.
    pub n_targets: usize,
    pub targets: [TargetIn; MAX_TARGETS],
    pub has_depth: bool,
    pub has_stencil: bool,
    /// `GL_DEPTH_BUFFER_BIT` / `GL_STENCIL_BUFFER_BIT` in the mask.
    pub clear_depth: bool,
    pub clear_stencil: bool,
    /// `glDepthMask`.
    pub depth_mask: bool,
    /// `glStencilMask` (front), 8 bits.
    pub stencil_mask: u8,
    pub scissor: ScissorIn,
    /// The attached depth/stencil buffer holds content that a clear which
    /// does not touch it must keep (written or loaded in this job).
    pub ds_valid: bool,
    /// The job already holds draws: a tile clear then needs a fresh job.
    pub draws_queued: bool,
}

/// The pixel rectangle of a clear draw, clipped to the framebuffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

/// A clear draw.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClearDraw {
    /// Colour targets the quad writes, bit n = target n.
    pub targets: u8,
    /// Per target, the channels written (bit 0 R .. bit 3 A); 0 for a target
    /// the quad does not write. `COLOR_WMASKS` is the complement.
    pub color_masks: [u8; MAX_TARGETS],
    /// The quad writes depth (depth test always, depth write on).
    pub depth: bool,
    /// The quad writes stencil (replace) through `stencil_mask`.
    pub stencil: bool,
    pub stencil_mask: u8,
    /// The quad's area: the clipped scissor, or the framebuffer.
    pub rect: Rect,
}

/// The way a clear is carried out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClearPlan {
    /// Every mask or box leaves nothing to write.
    Nothing,
    /// `Clear Render Targets`; `targets` cleared (all of them), `depth` and
    /// `stencil` set the ZS clear values' use. `new_job`: draws are queued, so
    /// the job is flushed first and the clear opens the next one.
    TileClear {
        targets: u8,
        depth: bool,
        stencil: bool,
        new_job: bool,
    },
    Draw(ClearDraw),
}

/// A shape the plan does not carry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    TooManyTargets {
        n: usize,
    },
    FramebufferSize {
        w: u32,
        h: u32,
    },
    /// A clear of a target the framebuffer does not have.
    TargetOutOfRange {
        index: usize,
        n: usize,
    },
    NegativeScissor {
        w: i32,
        h: i32,
    },
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Refusal::TooManyTargets { n } => {
                write!(f, "clear: {n} colour targets, V3D 7.1 has {MAX_TARGETS}")
            }
            Refusal::FramebufferSize { w, h } => {
                write!(f, "clear: framebuffer {w}x{h}, V3D 7.1 is 1..={MAX_FB_DIM} a side")
            }
            Refusal::TargetOutOfRange { index, n } => {
                write!(f, "clear: colour target {index} is cleared but the framebuffer has {n}")
            }
            Refusal::NegativeScissor { w, h } => {
                write!(f, "clear: scissor {w}x{h} has a negative side")
            }
        }
    }
}

/// The scissor clipped to the framebuffer; `None` when it is empty.
fn clip(sc: &ScissorIn, fb_w: u32, fb_h: u32) -> Option<Rect> {
    let (x0, y0) = (i64::from(sc.x).max(0), i64::from(sc.y).max(0));
    let x1 = (i64::from(sc.x) + i64::from(sc.w)).min(i64::from(fb_w));
    let y1 = (i64::from(sc.y) + i64::from(sc.h)).min(i64::from(fb_h));
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    Some(Rect { x: x0 as u32, y: y0 as u32, w: (x1 - x0) as u32, h: (y1 - y0) as u32 })
}

/// Decide how `glClear` is carried out. See the module docs.
pub fn plan_clear(c: &ClearIn) -> Result<ClearPlan, Refusal> {
    if c.n_targets > MAX_TARGETS {
        return Err(Refusal::TooManyTargets { n: c.n_targets });
    }
    if c.fb_w == 0 || c.fb_h == 0 || c.fb_w > MAX_FB_DIM || c.fb_h > MAX_FB_DIM {
        return Err(Refusal::FramebufferSize { w: c.fb_w, h: c.fb_h });
    }
    if c.scissor.enabled && (c.scissor.w < 0 || c.scissor.h < 0) {
        return Err(Refusal::NegativeScissor { w: c.scissor.w, h: c.scissor.h });
    }
    for (i, t) in c.targets.iter().enumerate().skip(c.n_targets) {
        if t.cleared {
            return Err(Refusal::TargetOutOfRange { index: i, n: c.n_targets });
        }
    }
    let full = Rect { x: 0, y: 0, w: c.fb_w, h: c.fb_h };
    let rect = if c.scissor.enabled {
        match clip(&c.scissor, c.fb_w, c.fb_h) {
            Some(r) => r,
            None => return Ok(ClearPlan::Nothing),
        }
    } else {
        full
    };

    // What the clear writes once the masks are applied.
    let mut targets = 0u8;
    let mut masks = [0u8; MAX_TARGETS];
    for (i, t) in c.targets.iter().enumerate().take(c.n_targets) {
        if t.cleared && t.mask & MASK_RGBA != 0 {
            targets |= 1 << i;
            masks[i] = t.mask & MASK_RGBA;
        }
    }
    let depth = c.has_depth && c.clear_depth && c.depth_mask;
    let stencil = c.has_stencil && c.clear_stencil && c.stencil_mask != 0;
    if targets == 0 && !depth && !stencil {
        return Ok(ClearPlan::Nothing);
    }

    // Mesa's tile clear: covers the buffers, writes every bit of them, and
    // leaves nothing the job keeps.
    let all_targets = if c.n_targets == 0 { 0 } else { ((1u16 << c.n_targets) - 1) as u8 };
    let colour_full = c.targets.iter().take(c.n_targets).all(|t| t.cleared && t.mask & MASK_RGBA == MASK_RGBA);
    let zs_ok = |has: bool, asked: bool, full: bool| !has || (asked && full) || (!asked && !c.ds_valid);
    let depth_ok = zs_ok(c.has_depth, c.clear_depth, c.depth_mask);
    let stencil_ok = zs_ok(c.has_stencil, c.clear_stencil, c.stencil_mask == 0xff);
    let covers = !c.scissor.enabled || rect == full;
    if covers && colour_full && depth_ok && stencil_ok {
        return Ok(ClearPlan::TileClear {
            targets: all_targets,
            depth: depth && c.has_depth,
            stencil: stencil && c.has_stencil,
            new_job: c.draws_queued,
        });
    }
    Ok(ClearPlan::Draw(ClearDraw {
        targets,
        color_masks: masks,
        depth,
        stencil,
        stencil_mask: if stencil { c.stencil_mask } else { 0 },
        rect,
    }))
}
