// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! V3D 7.1 (Raspberry Pi 5, V3D 7.1.10): what a job does with depth and
//! stencil kept between jobs (roadmap P9, map71's gap G3).
//!
//! Pure functions over plain values; no `crate::` import, so a test includes
//! this file with `#[path]` (tests/v3d71_zskeep.rs). Nothing hooks it into
//! `src/v3d/mod.rs` yet: the 7.1 table (backend71) does that.
//!
//! The V3D 4.2 table keeps a history per depth/stencil buffer
//! (`ZsKnown`, src/v3d/backend.rs) and decides each job's load and store in
//! `flush_job`: one load of both planes when either holds stored contents
//! the job did not clear, one store when either was written. A whole clear of
//! one plane while the other is loaded is drawn as a quad instead (GFXH-1461).
//! This module gives the same decision for 7.1, per plane:
//!
//! * [`route_clear`]: where a glClear of depth and stencil goes: a TLB clear
//!   (the job's start value), a quad (a scissored or write-masked clear), or
//!   nothing (a plane the framebuffer does not have, ES 2.0 4.2.3), and
//!   whether the open job must end first;
//! * [`decide`]: at the end of a job, from each plane's history
//!   ([`Known71`]) and what the job did with it ([`PlaneUse71`]): how the
//!   plane starts (load, TLB clear, or undefined), whether it is stored, and
//!   the history the next job starts from;
//! * [`Decision71::frame`]: the src/v3d/frame71.rs inputs the decision fills.
//!
//! # GFXH-1461 does not apply at 7.1
//!
//! Mesa 26.2.1 (the tarball `third_party/mesa-26.2.1/UPSTREAM` pins; the
//! files below are not vendored, cited from the full tree):
//!
//! * `src/gallium/drivers/v3d/v3dx_draw.c:1676-1687`, `v3d_tlb_clear`: "If we
//!   were to emit a load of just depth or just stencil, then the clear for
//!   the other may get lost. [...] This issue is fixed since V3D 4.3.18."
//!   The guard that turns a one-plane TLB clear of a combined depth/stencil
//!   buffer into a draw is `if (devinfo->ver == 42 && ...)` (`:1681`): at 7.1
//!   the TLB clears one plane while the job loads the other.
//! * `src/broadcom/vulkan/v3dv_pass.c:133-151`,
//!   `check_do_depth_stencil_clear_with_draw`: the same erratum, "fixed since
//!   V3D 4.3.18", returns at once for `device->devinfo.ver > 42` (`:145`).
//! * `src/gallium/drivers/v3d/v3dx_rcl.c:325-341` ("GFXH-1461/GFXH-1689: The
//!   per-buffer store command's clear buffer bit is broken for
//!   depth/stencil"): the store's clear bit is never set in either generation
//!   (`:140`); 4.2 clears with `CLEAR_TILE_BUFFERS`, 7.1 with
//!   `CLEAR_RENDER_TARGETS` (`:337-339`). That half of the label is the
//!   packet frame71 already emits (`Op71::ClearRt`), not a decision here.
//!
//! So [`gfxh1461_applies`] is Mesa's `ver == 42`, false for 7.1, and
//! [`route_clear`] never sends a whole clear to the quad because of the
//! other plane: a 7.1 job TLB-clears one plane and loads the other.
//!
//! # Per-plane loads and stores (Mesa's 7.1 rule)
//!
//! Mesa loads and stores the two planes of one surface separately:
//! `v3d_update_job_tlb_load_store` (`v3dx_draw.c:1063-1096`) adds
//! `PIPE_CLEAR_DEPTH` to the job's loads when the depth test is on and to its
//! stores when depth is written, and `PIPE_CLEAR_STENCIL` likewise for the
//! stencil test and stencil writes, minus what the job clears
//! (`no_load_mask`, `:1070-1071`); `v3d_rcl_emit_loads`/`_stores`
//! (`v3dx_rcl.c:188-239`, `:242-341`) turn the bits into buffer Z, Stencil
//! or Z+Stencil (`zs_buffer_from_pipe_bits`, `:172-185`), a separate stencil
//! surface on its own. The Mesa 7.1 reference tools/v3d/pi5/gl_fbods.clif
//! shows it: a stencil-only surface (D24S8) stores and loads buffer Stencil
//! alone, a D16 buffer Z alone. Hence:
//!
//! * load a plane when its history is [`Known71::Stored`], the job does not
//!   TLB-clear it, and the job uses it (reads, writes or draws a partial
//!   clear into it, [`PlaneUse71::needs`]); a plane the job leaves alone is
//!   neither loaded nor stored, so the buffer keeps what it held;
//! * store a plane when the job wrote it (a draw or a partial-clear quad) and
//!   the contents are not discarded;
//! * a plane only TLB-cleared is not stored: its history is
//!   [`Known71::Cleared`] and the next job clears it again (the 4.2 table's
//!   combined-buffer rule; Mesa stores it, `v3dx_draw.c:1791-1792`, which
//!   the 4.2 split path copies only so that no buffer holds a cleared plane
//!   that a load of the other plane would lose: GFXH-1461, gone at 7.1).
//!   Whoever reads the buffer's memory otherwise (a depth texture) writes a
//!   `Cleared` plane back first, as on 4.2 (src/ds_retention.rs).
//!
//! A buffer the job never wrote and never cleared starts undefined: Mesa
//! TLB-clears it (`v3d_job.c:457-466`, `!rsc->writes`), with its job's clear
//! value 0.0 in gl_fbods.clif's fifth job. This module starts an undefined
//! depth at [`UNDEFINED_Z_BITS`] (1.0, the far plane), what the 4.2 table
//! starts from (`Job::ZERO`'s `z: 1.0`, src/v3d/backend.rs), so case 67's
//! frame 5 draws its far quad into a fresh buffer.
//!
//! # The frame71 inputs
//!
//! [`Decision71::frame`] gives [`FrameZs71`]; each field names the
//! src/v3d/frame71.rs input it is:
//!
//! | output | frame71 input |
//! |---|---|
//! | `Start71::Load` of depth / stencil, combined buffer | `Frame71::zs` `ZsAttach71::load_z` / `load_s` (frame71 picks buffer Z, Stencil or Z+Stencil) |
//! | store of depth / stencil, combined buffer | `ZsAttach71::store_z` / `store_s` |
//! | depth of a split buffer | `ZsAttach71::load_z` / `store_z` (`load_s`, `store_s` false) |
//! | stencil of a split buffer | `Frame71::stencil` `Attach71::load`, `end` `EndOp71::Store` or `Invalidate` |
//! | `Start71::Clear(v)` / undefined start | `Frame71::z_clear_bits`, `stencil_clear`, and `clear_tile_buffers` (Mesa's `clear_tlb`) |
//! | early clear | `Frame71::early_ds_clear` (Mesa's `early_zs_clear`, `v3dx_rcl.c:752-756`) |
//!
//! A tile-only target (the GL depth/stencil lives in the tile buffer, no
//! memory keeps it) has neither `zs` nor `stencil`. The surfaces' formats,
//! addresses and the `depth_kind` are the buffer's own answer, not here.
//!
//! A combination the module does not support is refused by a [`ZsRefusal71`]
//! variant, never by a default answer.

use core::fmt;

/// Mesa's GFXH-1461 condition (`v3dx_draw.c:1681`: `devinfo->ver == 42`):
/// whether a TLB clear of one plane of a combined depth/stencil buffer is
/// lost when the job loads the other. `ver` is Mesa's `devinfo->ver` (42,
/// 71). False for 7.1.
pub const fn gfxh1461_applies(ver: u32) -> bool {
    ver == 42
}

/// The Mesa citation of the 7.1 answer.
pub const GFXH1461_MESA: &str = "src/gallium/drivers/v3d/v3dx_draw.c:1676-1687";

/// The start value of an undefined depth plane: 1.0 as f32 bits.
pub const UNDEFINED_Z_BITS: u32 = 0x3f80_0000;
/// The start value of an undefined stencil plane.
pub const UNDEFINED_STENCIL: u32 = 0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Plane71 {
    Depth,
    Stencil,
}

/// What a plane is known to hold when no job is open (the 4.2 `ZsKnown`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Known71 {
    /// Never cleared or written, or discarded since: undefined in GL.
    Untouched,
    /// Cleared whole, not drawn into since: the value (depth as f32 bits,
    /// stencil in the low 8 bits). The buffer's memory may not hold it.
    Cleared(u32),
    /// Written and not kept anywhere (a tile-only target): a later read
    /// before a clear is refused.
    Written,
    /// Written and stored in the target's buffer: the next job that uses it
    /// loads it.
    Stored,
}

/// Where the target keeps depth and stencil between jobs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout71 {
    /// No memory: the planes live in the tile buffer for one job only.
    TileOnly,
    /// One surface for the planes the target has (D16, D24, D32F, D24S8,
    /// and a stencil-only buffer kept as D24S8 or S8): `Frame71::zs`.
    Combined,
    /// A depth surface and a separate stencil surface: `Frame71::zs` (depth
    /// only) and `Frame71::stencil`.
    Split,
}

/// The target: its layout and the planes GL says it has.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Target71 {
    pub layout: Layout71,
    pub depth: bool,
    pub stencil: bool,
}

impl Target71 {
    fn has(&self, p: Plane71) -> bool {
        match p {
            Plane71::Depth => self.depth,
            Plane71::Stencil => self.stencil,
        }
    }

    fn kept(&self) -> bool {
        self.layout != Layout71::TileOnly
    }

    fn check(&self) -> Result<(), ZsRefusal71> {
        match self.layout {
            Layout71::Split if !(self.depth && self.stencil) => Err(ZsRefusal71::SplitNeedsBoth),
            Layout71::Combined if !(self.depth || self.stencil) => Err(ZsRefusal71::EmptyBuffer),
            _ => Ok(()),
        }
    }
}

/// What a glClear asks of one plane, after the GL core's masks: a clear
/// with `glDepthMask(GL_FALSE)` asks nothing; a scissored one, or a stencil
/// clear under a partial `glStencilMask`, is `Partial`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClearKind71 {
    None,
    /// The whole plane, to this value (depth as f32 bits, stencil value).
    Full(u32),
    Partial,
}

/// Where a plane's clear goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Route71 {
    None,
    /// The job's start value: the TLB clears the plane.
    Tlb(u32),
    /// Drawn as the clear quad: a write of the plane.
    Quad,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClearRoute71 {
    /// The open job has draws and something is TLB-cleared: it ends first
    /// and the clear opens a new one (the 4.2 table's rule; Mesa's
    /// `v3d_tlb_clear` keeps a buffer the queued draws used off the TLB,
    /// `v3dx_draw.c:1667-1674`, and draws its clear instead).
    pub new_job: bool,
    pub z: Route71,
    pub s: Route71,
}

/// Route a glClear's depth and stencil parts. `job_draws`: the open job has
/// draws queued; `colour_tlb`: the clear also TLB-clears a colour target.
/// At 7.1 a whole clear is a TLB clear whatever the other plane does
/// ([`gfxh1461_applies`]); a plane the target lacks is not cleared.
pub fn route_clear(
    t: &Target71,
    job_draws: bool,
    colour_tlb: bool,
    z: ClearKind71,
    s: ClearKind71,
) -> Result<ClearRoute71, ZsRefusal71> {
    t.check()?;
    let route = |p: Plane71, k: ClearKind71| match k {
        _ if !t.has(p) => Route71::None,
        ClearKind71::None => Route71::None,
        ClearKind71::Full(v) => Route71::Tlb(v),
        ClearKind71::Partial => Route71::Quad,
    };
    let (z, s) = (route(Plane71::Depth, z), route(Plane71::Stencil, s));
    let tlb = colour_tlb || matches!(z, Route71::Tlb(_)) || matches!(s, Route71::Tlb(_));
    Ok(ClearRoute71 { new_job: job_draws && tlb, z, s })
}

/// What one job did with one plane.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlaneUse71 {
    /// The TLB clears it at the job's start, to this value.
    pub tlb_clear: Option<u32>,
    /// A partial clear was drawn into it (the quad).
    pub quad: bool,
    /// A draw tests it (depth test or stencil test on).
    pub read: bool,
    /// A draw writes it (depth writes, a stencil op that writes).
    pub written: bool,
}

impl PlaneUse71 {
    pub const NONE: PlaneUse71 = PlaneUse71 { tlb_clear: None, quad: false, read: false, written: false };

    /// Add a clear route to the job's use.
    pub fn apply(&mut self, r: Route71) {
        match r {
            Route71::None => {}
            Route71::Tlb(v) => self.tlb_clear = Some(v),
            Route71::Quad => self.quad = true,
        }
    }

    /// Add a draw's use.
    pub fn draw(&mut self, read: bool, written: bool) {
        self.read |= read;
        self.written |= written;
    }

    /// The job needs the plane's contents: it reads it or writes part of it.
    pub fn needs(&self) -> bool {
        self.read || self.written || self.quad
    }

    fn touched(&self) -> bool {
        self.needs() || self.tlb_clear.is_some()
    }
}

/// One job's depth and stencil use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Job71 {
    pub z: PlaneUse71,
    pub s: PlaneUse71,
    /// The contents are dropped when the job ends (eglSwapBuffers, EGL 1.5
    /// 3.10.1; glInvalidateFramebuffer of depth and stencil): nothing is
    /// stored and both planes become [`Known71::Untouched`] (Mesa's
    /// `v3d_invalidate_resource` drops the store, `v3d_context.c:117-133`).
    pub discard: bool,
}

impl Job71 {
    pub const NONE: Job71 = Job71 { z: PlaneUse71::NONE, s: PlaneUse71::NONE, discard: false };
}

/// How a plane starts in the tile buffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Start71 {
    /// Loaded from the buffer.
    Load,
    /// TLB-cleared to this value.
    Clear(u32),
    /// Anything: the job neither needs nor keeps it, or GL leaves it
    /// undefined. The tile buffer clears it to the undefined values.
    Undefined,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlaneOut71 {
    pub start: Start71,
    pub store: bool,
    pub next: Known71,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Decision71 {
    pub z: PlaneOut71,
    pub s: PlaneOut71,
    /// `Early Depth/Stencil Clear`: a plane is cleared, none is loaded or
    /// stored (Mesa `v3dx_rcl.c:752-754`).
    pub early_ds_clear: bool,
    /// The tile buffer must be cleared for every tile (a plane starts from a
    /// clear value or undefined and the job uses it): Mesa's `clear_tlb`.
    pub clear_tile_buffers: bool,
    target: Target71,
}

/// The frame71 inputs of one decision (see the module docs' table).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrameZs71 {
    /// `Frame71::zs` is `Some` (the combined surface, or a split target's
    /// depth surface)
    pub zs_surface: bool,
    pub zs_load_z: bool,
    pub zs_load_s: bool,
    pub zs_store_z: bool,
    pub zs_store_s: bool,
    /// `Frame71::stencil` is `Some` (a split target's stencil surface)
    pub stencil_surface: bool,
    /// its `Attach71::load`
    pub stencil_load: bool,
    /// its `Attach71::end` is `EndOp71::Store` (else `Invalidate`)
    pub stencil_store: bool,
    /// `Frame71::early_ds_clear`
    pub early_ds_clear: bool,
    /// asked of `Frame71::clear_tile_buffers` (the colour side may ask too)
    pub clear_tile_buffers: bool,
    /// `Frame71::z_clear_bits`
    pub z_clear_bits: u32,
    /// `Frame71::stencil_clear`
    pub stencil_clear: u8,
}

impl Decision71 {
    pub fn frame(&self) -> FrameZs71 {
        let t = self.target;
        let load = |o: &PlaneOut71| o.start == Start71::Load;
        let mut f = FrameZs71 {
            early_ds_clear: self.early_ds_clear,
            clear_tile_buffers: self.clear_tile_buffers,
            z_clear_bits: match self.z.start {
                Start71::Clear(v) => v,
                _ if t.depth => UNDEFINED_Z_BITS,
                _ => 0,
            },
            stencil_clear: match self.s.start {
                Start71::Clear(v) => v as u8,
                _ => UNDEFINED_STENCIL as u8,
            },
            ..FrameZs71::default()
        };
        match t.layout {
            Layout71::TileOnly => {}
            Layout71::Combined => {
                f.zs_surface = true;
                (f.zs_load_z, f.zs_store_z) = (load(&self.z), self.z.store);
                (f.zs_load_s, f.zs_store_s) = (load(&self.s), self.s.store);
            }
            Layout71::Split => {
                f.zs_surface = true;
                (f.zs_load_z, f.zs_store_z) = (load(&self.z), self.z.store);
                f.stencil_surface = true;
                (f.stencil_load, f.stencil_store) = (load(&self.s), self.s.store);
            }
        }
        f
    }
}

fn plane(t: &Target71, p: Plane71, k: Known71, u: &PlaneUse71, discard: bool) -> Result<PlaneOut71, ZsRefusal71> {
    if !t.has(p) {
        if u.touched() || k != Known71::Untouched {
            return Err(ZsRefusal71::AbsentPlane(p));
        }
        return Ok(PlaneOut71 { start: Start71::Undefined, store: false, next: Known71::Untouched });
    }
    let kept = t.kept();
    match k {
        Known71::Written if kept => return Err(ZsRefusal71::WrittenWithBuffer(p)),
        Known71::Stored if !kept => return Err(ZsRefusal71::StoredWithoutBuffer(p)),
        _ => {}
    }
    let start = match (u.tlb_clear, k) {
        (Some(v), _) => Start71::Clear(v),
        (None, Known71::Stored) if u.needs() => Start71::Load,
        (None, Known71::Stored) => Start71::Undefined,
        (None, Known71::Cleared(v)) => Start71::Clear(v),
        (None, Known71::Untouched) => Start71::Undefined,
        (None, Known71::Written) if u.read => return Err(ZsRefusal71::ReadLost(p)),
        (None, Known71::Written) => Start71::Undefined,
    };
    let wrote = u.written || u.quad;
    let store = kept && !discard && wrote;
    let next = if discard {
        Known71::Untouched
    } else if wrote {
        if kept {
            Known71::Stored
        } else {
            Known71::Written
        }
    } else if let Some(v) = u.tlb_clear {
        Known71::Cleared(v)
    } else {
        k
    };
    Ok(PlaneOut71 { start, store, next })
}

/// The end-of-job decision for a target whose planes hold `z` and `s`.
pub fn decide(t: &Target71, z: Known71, s: Known71, job: &Job71) -> Result<Decision71, ZsRefusal71> {
    t.check()?;
    let zo = plane(t, Plane71::Depth, z, &job.z, job.discard)?;
    let so = plane(t, Plane71::Stencil, s, &job.s, job.discard)?;
    let cleared = |o: &PlaneOut71, u: &PlaneUse71| match o.start {
        Start71::Clear(_) => true,
        Start71::Undefined => u.needs(),
        Start71::Load => false,
    };
    let any_clear = cleared(&zo, &job.z) || cleared(&so, &job.s);
    let any_load = zo.start == Start71::Load || so.start == Start71::Load;
    let any_store = zo.store || so.store;
    Ok(Decision71 {
        z: zo,
        s: so,
        early_ds_clear: any_clear && !any_load && !any_store,
        clear_tile_buffers: any_clear,
        target: *t,
    })
}

/// A combination the 7.1 depth/stencil decision does not support.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ZsRefusal71 {
    /// A draw reads a plane an earlier job wrote into a tile-only target:
    /// the 4.2 table's strings (src/v3d/backend.rs, `record`), word for word.
    ReadLost(Plane71),
    /// The job uses, or a history names, a plane the framebuffer lacks (the
    /// GL core turns its test off, ES 2.0 4.1.4/4.1.6).
    AbsentPlane(Plane71),
    /// A split layout without both planes.
    SplitNeedsBoth,
    /// A combined buffer with neither plane.
    EmptyBuffer,
    /// A `Stored` history on a tile-only target.
    StoredWithoutBuffer(Plane71),
    /// A `Written` (not stored) history on a target with a buffer.
    WrittenWithBuffer(Plane71),
}

impl ZsRefusal71 {
    pub const fn message(self) -> &'static str {
        use Plane71::*;
        match self {
            ZsRefusal71::ReadLost(Depth) => {
                "the draw reads depth an earlier job wrote, and the target has no depth/stencil buffer to keep it in"
            }
            ZsRefusal71::ReadLost(Stencil) => {
                "the draw reads stencil an earlier job wrote, and the target has no depth/stencil buffer to keep it in"
            }
            ZsRefusal71::AbsentPlane(Depth) => "V3D 7.1: depth used or kept on a framebuffer without depth",
            ZsRefusal71::AbsentPlane(Stencil) => "V3D 7.1: stencil used or kept on a framebuffer without stencil",
            ZsRefusal71::SplitNeedsBoth => "V3D 7.1: split depth and stencil buffers without both planes",
            ZsRefusal71::EmptyBuffer => "V3D 7.1: a depth/stencil buffer with neither depth nor stencil",
            ZsRefusal71::StoredWithoutBuffer(Depth) => {
                "V3D 7.1: a stored depth history on a target with no depth/stencil buffer"
            }
            ZsRefusal71::StoredWithoutBuffer(Stencil) => {
                "V3D 7.1: a stored stencil history on a target with no depth/stencil buffer"
            }
            ZsRefusal71::WrittenWithBuffer(Depth) => {
                "V3D 7.1: an unstored depth history on a target with a depth/stencil buffer"
            }
            ZsRefusal71::WrittenWithBuffer(Stencil) => {
                "V3D 7.1: an unstored stencil history on a target with a depth/stencil buffer"
            }
        }
    }
}

impl fmt::Display for ZsRefusal71 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.message())
    }
}
