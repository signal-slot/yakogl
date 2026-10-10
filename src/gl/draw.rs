// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The draw path: resolve the current GL context into one [`GlesDraw`] and
//! hand it to the backend (`gl_draw.c`).
//!
//! This is where the recorded GL state machine collapses onto the
//! (silicon-proven) PowerVR backend boundary:
//! ```text
//!   - target      <- default window surface (or the bound FBO's colour buffer)
//!   - do_clear    <- pending glClear (color), fused into this kick's tile-load
//!   - geometry    <- prim + index_count (+ index buffer for glDrawElements)
//!   - shaders     <- linked program's rvs/rfs (USC uploaded once, cached per prog)
//!   - attributes  <- each enabled VS input, resolved via the VS shc_attrib map to
//!                    a gles_attrib (vi register + the bound VBO's GPU VA)
//!   - uniforms    <- program's packed dword block, uploaded and pointed at by both
//!                    VS and FS push-const
//!   - textures    <- the bound TEXTURE_2D wired to its sampler unit
//!   - fixed-func  <- depth / cull / scissor / stencil mapped to PVR enums; blend
//!                    selects a fragment-shader variant (fb-fetch + blend in the FS)
//! ```
#![allow(non_snake_case)]

use core::ffi::c_void;
use core::fmt::Write as _;
use core::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering};

use super::buffer;
use super::consts::*;
use super::context::{
    self, gl_trace, obj_index, tex_pitch, GlBuffer, GlContext, GlProgram, GlTexture, GlVertexAttrib,
    CAP_COLOR_ATTACHMENTS, CAP_SAMPLERS, MAX_BUFFERS, MAX_CONTEXTS, MAX_PROGRAMS, MAX_TEXTURES, MAX_UNIFORM_DWORDS,
    MAX_VERTEX_ATTRIBS, TEXTURE_SLOTS,
};
use super::framebuffer;
use super::tf;
use super::types::*;
use crate::backend::{self, *};
use crate::draw_steps::{self, Step};
use crate::osal::{self, OsalGpuMem, OSAL_MEM_DATA};
use crate::shaderc::{self, *};
use crate::util::{self, Bytes, FmtBuf, Global};

/// State of a glClear that has not been fused into a draw yet. glClear
/// records it (colour captured at glClear time, per GL); the next glDraw*
/// fuses it into `GlesDraw::do_clear` (one kick, clear-on-tile-load);
/// [`flush_pending_clear`] emits a clear-only kick for a glClear with no
/// following draw (called from glFlush/glFinish).
pub struct PendingClear {
    pub color: bool,
    pub rgba: [f32; 4],
    pub depth: bool,
    pub depth_value: f32,
    pub stencil: bool,
    pub stencil_value: u8,
    /// The framebuffer bound when the clear was recorded
    /// ([`pending_clear_for`]).
    pub fb: GLuint,
    /// The state that decides what a clear touches, AS IT STOOD AT THE CALL.
    ///
    /// GL ES 2.0 4.2.3: "the scissor test [and] the buffer writemasks affect
    /// the operation of glClear". They are the application's state at
    /// `glClear`, not whenever this library gets round to emitting the clear
    /// -- and this library defers it, either into the next draw or to a
    /// flush. Reading them late lost the colour clear of an application that
    /// restores its masks before drawing: `glClear(COLOR|STENCIL);
    /// glColorMask(0,0,0,0); draw; glColorMask(1,1,1,1); draw` is the
    /// stencil-then-cover fill FemtoVG and Slint are built on.
    ///
    /// A `glClear` issued under different state cannot join a pending one, so
    /// [`settle_clear_state`] emits the pending clear first rather than
    /// letting the newer state rewrite it.
    pub state: ClearState,
}

/// The write masks and scissor box in force at a `glClear`; see
/// [`PendingClear::state`].
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub struct ClearState {
    /// One bit per colour channel, as [`clear_color_mask`] packs them.
    pub color_mask: u8,
    pub depth_mask: bool,
    pub stencil_write_mask: u8,
    /// The scissor box, or `None` when the scissor test was off.
    pub scissor: Option<[GLint; 4]>,
    /// glClearBuffer(GL_COLOR, n): bit n; 0 for glClear, every target
    /// (`GlesDraw::clear_target_mask`).
    pub target_mask: u8,
}

impl ClearState {
    /// GL's initial state: every mask open, no scissor -- a clear that
    /// touches every pixel of every buffer it names.
    pub const CLEAR_ALL: ClearState =
        ClearState { color_mask: 0xf, depth_mask: true, stencil_write_mask: 0xff, scissor: None, target_mask: 0 };
}

impl PendingClear {
    /// No clear pending.
    const NONE: PendingClear = PendingClear {
        color: false,
        rgba: [0.0; 4],
        depth: false,
        depth_value: 1.0,
        stencil: false,
        stencil_value: 0,
        fb: 0,
        state: ClearState::CLEAR_ALL,
    };
}

/// The pending clear of each context slot. A clear is a command of the
/// context that issued it, so only that context's draws and flushes consume
/// it. It was one process-wide record until 2026-09-29 (review R-04): with
/// two contexts switched through the C API (`gl_context_set_current`, which
/// did not flush), B's next draw took A's clear -- A's colour in B's target
/// -- and A lost it. Kept here rather than in `GlContext` so that the
/// struct's C view does not change (as `UNIFORM_BUFFERS`), and because the
/// consumers below hold the record across calls that take the context.
static PENDING: Global<[PendingClear; MAX_CONTEXTS]> = Global::new([PendingClear::NONE; MAX_CONTEXTS]);

/// `c`'s pending clear (glClear records it; the next draw fuses it in).
pub fn pending_clear_of(c: &GlContext) -> &'static mut PendingClear {
    // SAFETY: single-threaded by contract; callers use the reference for a
    // few field accesses and do not hold it across calls into this module.
    unsafe { &mut PENDING.get()[c.index as usize % MAX_CONTEXTS] }
}

/// A context slot starts, or its context is destroyed: nothing is pending.
pub(super) fn reset_pending_clear(index: u8) {
    // SAFETY: single-threaded by contract; no other reference is live.
    unsafe { PENDING.get()[index as usize % MAX_CONTEXTS] = PendingClear::NONE };
}

/// Whether `c` has a clear pending (colour, depth or stencil).
pub fn has_pending_clear(c: &GlContext) -> bool {
    let pc = pending_clear_of(c);
    pc.color || pc.depth || pc.stencil
}

/// The current context's pending clear (tests).
#[cfg(test)]
pub fn pending_clear() -> &'static mut PendingClear {
    pending_clear_of(context::current().expect("a current context"))
}

/* Per-program GPU-side cache: USC uploads (fixed after link) + a reusable uniform
 * upload buffer. Keyed by program handle. Kept out of GlProgram so the public
 * object stays small. */
const PROG_MAX_FS_VARIANTS: usize = 4;

/// A fragment-shader variant compiled for one blend state (blending is lowered
/// into the FS on this GPU); its USC is uploaded once and reused.
struct ProgFsVariant {
    /// 0 = unused slot.
    blend_key: u32,
    /// The fragment half of the sampler-state variant it was compiled from
    /// ([`ProgLodVariant::key`]); 0 = the program's own fragment shader.
    lod_key: u64,
    /// Looked up and not available.
    missing: bool,
    /// [`ProgGpu::fs_clock`] at its last use.
    stamp: u32,
    sh: ShcShader,
    usc: OsalGpuMem,
}

struct ProgGpu {
    valid: bool,
    vs_usc: OsalGpuMem,
    fs_usc: OsalGpuMem,
    uniforms: OsalGpuMem,
    /// The frame that last read `uniforms` (see [`staging_reserve`]).
    uniforms_kick: u32,
    fs_var: [ProgFsVariant; PROG_MAX_FS_VARIANTS],
    /// Draws that asked for a blend variant: the variants' age.
    fs_clock: u32,
    /// Which texture bindings' filter state each stage's code depends on
    /// (`ShcBackend::lod_sensitivity`, `shaderc::LodKey` bits; [vertex,
    /// fragment]), asked once after the link ([`draw_lod_key`]).
    lod_sense: [u64; 2],
    lod_sense_known: bool,
    /// Whether the fragment shader has a sample-kill variant
    /// (`shaderc::LodKey::sample_kill`): 0 not asked yet, 1 no, 2 yes.
    /// Asked the first time a draw needs one ([`draw_lod_key`]), not after
    /// every link: most programs never draw with a partial sample mask.
    kill_sense: u8,
    /// The sampler-state variants of the program ([`lod_variant_for`]).
    lod_var: [ProgLodVariant; PROG_MAX_LOD_VARIANTS],
    /// Draws that asked for a variant: the variants' age.
    lod_clock: u32,
}

/// Sampler-state variants a program keeps beside its own two shaders, per
/// share group: a draw whose key has none takes the least recently used
/// slot. Two cover a program drawn with a mipmapped and a plain texture in
/// turn (Qt Quick's atlas and an image), and a program that cycles through
/// more recompiles from the keep region's cache, not from source.
const PROG_MAX_LOD_VARIANTS: usize = 2;

/// One sampler-state variant: the program's two stages compiled for one key
/// (`ShcBackend::sampler_variant`), the fragment shader linked to the
/// vertex shader, each USC uploaded once. A stage whose key is 0 is the
/// program's own shader and its USC (`own_*` false).
struct ProgLodVariant {
    used: bool,
    /// [vertex, fragment] `shaderc::LodKey` bits; never both 0 in a used slot.
    key: [u64; 2],
    /// Asked for and not available (the compile or the upload failed): its
    /// draws take the program's own shaders.
    missing: bool,
    /// [`ProgGpu::lod_clock`] at its last use.
    stamp: u32,
    vs: ShcShader,
    fs: ShcShader,
    vs_usc: OsalGpuMem,
    fs_usc: OsalGpuMem,
}

/// The file-static state of the draw path. Every field is valid when all-zero.
struct DrawState {
    pg: [[ProgGpu; MAX_PROGRAMS]; MAX_CONTEXTS],
    /// Reusable scratch for client-side (non-VBO) index arrays.
    index_scratch: OsalGpuMem,
    /// ... and for client-side vertex arrays, one per attribute (uploaded per
    /// draw, converted to tightly packed float32).
    attrib_scratch: [OsalGpuMem; MAX_VERTEX_ATTRIBS],
    /// Staging for a VS input whose attribute index another input of the
    /// same draw already staged into `attrib_scratch` (glBindAttribLocation
    /// may bind two inputs to one index; they must not share a buffer),
    /// by the input's slot in `GlesDraw::attribs`.
    dup_scratch: [OsalGpuMem; MAX_DRAW_ATTRIBS],
    /// Last-use stamps ([`staging_reserve`]) of the staging buffers.
    index_kick: u32,
    attrib_kick: [u32; MAX_VERTEX_ATTRIBS],
    dup_kick: [u32; MAX_DRAW_ATTRIBS],
    /// Draws the backend refused, reported so far (a budget of
    /// [`DRAW_FAIL_LOGS`] lines).
    draw_fail_reported: u32,
    /* One GlesDraw per user (~1 KB each): kept off small RTOS stacks (one
     * context, no reentrancy). */
    clear_draw: GlesDraw,
    arrays_draw: GlesDraw,
    elements_draw: GlesDraw,
    /// glDrawElements: "indices with no ELEMENT_ARRAY_BUFFER" reports so far.
    index_reported: i32,
    /// Bring-up: small fragment uniform blocks of large shaders still to be
    /// dumped; `!started` means the budget has not been handed out yet.
    fsdump_left: u32,
    fsdump_started: bool,
}

static STATE: Global<DrawState> = Global::zeroed();

/// The draw path's live pool memory for the owner report
/// ([`super::context::yakogl_pool_owners`]): every program's code and
/// uniforms with its variants, and the staging buffers.
pub(crate) fn pool_owners() -> (crate::osal::OwnerTally, crate::osal::OwnerTally) {
    use crate::osal::OwnerTally;
    // SAFETY: single-threaded by contract (the report runs between frames).
    let st = unsafe { STATE.get() };
    let (mut progs, mut staging) = (OwnerTally::default(), OwnerTally::default());
    for pg in st.pg.iter().flat_map(|g| g.iter()) {
        for m in [&pg.vs_usc, &pg.fs_usc, &pg.uniforms] {
            progs.add(m);
        }
        for v in &pg.fs_var {
            progs.add(&v.usc);
        }
        for v in &pg.lod_var {
            progs.add(&v.vs_usc);
            progs.add(&v.fs_usc);
        }
    }
    staging.add(&st.index_scratch);
    for m in st.attrib_scratch.iter().chain(st.dup_scratch.iter()) {
        staging.add(m);
    }
    (progs, staging)
}

/// The attribute slots of one GlesDraw.
const MAX_DRAW_ATTRIBS: usize = 16;

/// Whether the GPU's fetch of `pa` stays inside the `avail` bytes behind it:
/// a vertex element of one to four components whose last fetch -- `count`
/// vertices (or instances) at `pa.stride`, then `extent` bytes -- ends at or
/// before the end of its buffer or staged copy.
///
/// The one check every attribute passes before the backend sees it,
/// whichever way it was sourced. The GPU reading past the memory it was
/// given is how one client left the GC7000 drawing garbage for every client
/// after it (gl_test 63 on an etna_shc program, a mat4 recorded as 16
/// components: a 64-byte stride over a copy of 16-byte rows); the next
/// mismatch between a record and the draw path must be refused here, not
/// fetched.
fn fetch_fits(pa: &GlesAttrib, count: u32, extent: u64, avail: u64) -> bool {
    let last = u64::from(count.saturating_sub(1)) * u64::from(pa.stride);
    (1..=4).contains(&pa.components) && last.checked_add(extent).is_some_and(|end| end <= avail)
}

/// Refuse a draw whose attribute would be fetched out of range, naming it.
fn refuse_fetch(app: usize, pa: &GlesAttrib, count: u32, extent: u64, avail: u64) -> (GLenum, &'static str) {
    if log_enabled() {
        glog!(
            "yakogl: attribute {} would be fetched out of range: {} components, {} vertices at stride {} \
             and {} bytes each, from {} bytes; refusing the draw",
            app,
            pa.components,
            count,
            pa.stride,
            extent,
            avail
        );
    }
    (GL_INVALID_OPERATION, "glDraw*")
}

/// The vertex inputs a draw fetches, one per attribute location.
///
/// **A matrix attribute is one input per column** (ES 2.0 2.7: a matN takes
/// N consecutive locations, column k at location + k), and on the GC7000
/// each column is its own vertex element, read into register vi_reg + k --
/// both compilers read it there (matattr4.vert: etna_shc and ours read
/// t1..t4). The shader's record lists the matrix once, and not the same way
/// twice: etna_shc (the offline table) as rows x columns
/// components, 16 for a mat4, and ours as one column's, 4. Taken as it
/// stood, the 16 went into the element's byte extent and into a per-vertex
/// staging stride of 64 bytes over a copy the backend sized at 16, so the
/// GPU fetched past the end of it -- and gl_test 63 run on an etna_shc
/// program left the GPU drawing garbage for every later client until the
/// module was reloaded (imx8mp-board, 2026-09-23). The other columns were
/// never fetched either, so 63 drew the wrong picture on every back end.
///
/// The column count and height come from the GL type, never from
/// `components`, so both conventions give the same elements. The V3D build
/// takes the same split: its reflection (the online compiler's) lists a
/// matrix once, and its program fetches one attribute record per column
/// (`chain::attribute_locations`). Vivante and V3D only:
/// the PowerVR's `vi_reg` counts dwords rather than registers, and its table
/// carries no matrix attribute to measure a convention against.
fn vertex_inputs(sh: &ShcShader) -> ([ShcAttrib; MAX_DRAW_ATTRIBS], usize) {
    let mut out = [ShcAttrib::ZERO; MAX_DRAW_ATTRIBS];
    let mut n = 0usize;
    let rows = &sh.attribs[..(sh.num_attribs as usize).min(sh.attribs.len())];
    for whole in rows {
        /* A record that already lists the columns as rows of one name (the
         * PowerVR front end splits a matrix so pco_shc never sees one) is
         * taken as it is. */
        let split = rows.iter().filter(|r| r.name == whole.name).count() > 1;
        /* (columns, rows) of a matCxR (ES 3.00's non-square ones too): C
         * locations of R components each; (1, 1) for anything else */
        let (columns, rows): (u8, u8) = if cfg!(not(feature = "gpu-powervr")) && !split {
            let (c, r) = super::program::gl_type_shape(whole.gl_type);
            if c > 1 {
                (c as u8, r as u8)
            } else {
                (1, 1)
            }
        } else {
            (1, 1)
        };
        for k in 0..columns {
            if n == MAX_DRAW_ATTRIBS {
                return (out, n);
            }
            let mut col = *whole;
            if columns > 1 {
                col.location = whole.location.saturating_add(k);
                col.vi_reg = whole.vi_reg.saturating_add(k);
                col.components = rows;
            }
            out[n] = col;
            n += 1;
        }
    }
    (out, n)
}

/// The staging of one draw's attributes (split out of [`DrawState`] so that
/// build_common can hold it next to the draw being built).
struct AttribStaging<'a> {
    scratch: &'a mut [OsalGpuMem; MAX_VERTEX_ATTRIBS],
    kick: &'a mut [u32; MAX_VERTEX_ATTRIBS],
    dup: &'a mut [OsalGpuMem; MAX_DRAW_ATTRIBS],
    dup_kick: &'a mut [u32; MAX_DRAW_ATTRIBS],
}

/* --------------------------------------------------------------------------- */
/* the kick a draw's stamps name */

/// Where a stamp a draw took while it was being built lives
/// ([`draw_stamp`]): what it rendered into or read, and the staging blocks
/// it read.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum StampAt {
    /// The render-into stamp of texture `ti` in context slot `ctx`
    /// (`framebuffer::texture_target_use`).
    Target {
        ctx: u8,
        ti: u16,
    },
    /// `GlTexture::last_use` of texture slot `ti` of share group `group`.
    Texture {
        group: u8,
        ti: u16,
    },
    /// `GlBuffer::last_use` of buffer `bi` of share group `group`.
    Buffer {
        group: u8,
        bi: u16,
    },
    /// [`staging_reserve`]'s stamps, in `DrawState`: the index scratch, an
    /// attribute index's scratch, an input's second scratch, and the
    /// uniform block of program slot `prog` of share group `group`.
    IndexStaging,
    AttribStaging(u8),
    DupStaging(u8),
    Uniforms {
        group: u8,
        prog: u16,
    },
}

/// Distinct stamps one draw (and the pending clear it emits first) can
/// take: its targets, the textures of every unit of both stages, the vertex
/// and index buffers, and every staging block.
const DRAW_STAMPS_CAP: usize = (CAP_COLOR_ATTACHMENTS + 1)
    + 2 * CAP_SAMPLERS
    + (MAX_DRAW_ATTRIBS + 1)
    + (MAX_VERTEX_ATTRIBS + MAX_DRAW_ATTRIBS + 2);

/// The stamps the draw being built took ([`draw_stamp`]), so they can be
/// moved to the kick that holds the draw once the backend has recorded it
/// ([`restamp_draw`]). `depth` counts open [`DrawStampScope`]s: a pending
/// clear a draw emits first keeps the draw's own stamps.
struct DrawStamps {
    depth: u8,
    n: usize,
    /// A stamp did not fit (never, while the cap counts right): see
    /// [`restamp_draw`].
    lost: bool,
    at: [StampAt; DRAW_STAMPS_CAP],
}

static DRAW_STAMPS: Global<DrawStamps> =
    Global::new(DrawStamps { depth: 0, n: 0, lost: false, at: [StampAt::IndexStaging; DRAW_STAMPS_CAP] });

fn draw_stamps() -> &'static mut DrawStamps {
    // SAFETY: single-threaded by contract; callers hold no other reference.
    unsafe { DRAW_STAMPS.get() }
}

/// The value a draw being built stamps what it renders into or reads with:
/// `backend::kick_id()`, the kick being recorded. That is the kick holding
/// the draw only if the backend does not end the open frame for it: a draw
/// into another target (or one the open frame has no room for) makes the
/// backend submit that frame first, and the draw lands in the next kick.
/// So the place is noted, inside a [`DrawStampScope`], and the stamp moves
/// to the kick the backend actually put the draw in when it returns
/// ([`restamp_draw`]). Outside a scope (a target query, a completeness
/// check) the stamp is only taken.
pub(crate) fn draw_stamp(at: StampAt) -> u32 {
    let s = draw_stamps();
    if s.depth != 0 && !s.at[..s.n].contains(&at) {
        match s.at.get_mut(s.n) {
            Some(slot) => {
                *slot = at;
                s.n += 1;
            }
            None => s.lost = true,
        }
    }
    backend::kick_id()
}

/// A draw entry point (or a pending clear's emission) from its start to
/// its end: the stamps taken meanwhile are the ones [`restamp_draw`] moves.
/// Nested scopes share the outer one's stamps.
struct DrawStampScope;

impl DrawStampScope {
    fn open() -> Self {
        let s = draw_stamps();
        if s.depth == 0 {
            s.n = 0;
            s.lost = false;
        }
        s.depth = s.depth.saturating_add(1);
        DrawStampScope
    }
}

impl Drop for DrawStampScope {
    fn drop(&mut self) {
        let s = draw_stamps();
        s.depth = s.depth.saturating_sub(1);
        if s.depth == 0 {
            s.n = 0;
            s.lost = false;
        }
    }
}

/// `DrawState`'s staging stamps, for [`restamp_draw`]; a pending clear
/// stages nothing and passes none.
struct StagingKicks<'a> {
    index: &'a mut u32,
    attrib: &'a mut [u32; MAX_VERTEX_ATTRIBS],
    dup: &'a mut [u32; MAX_DRAW_ATTRIBS],
    pg: &'a mut [[ProgGpu; MAX_PROGRAMS]; MAX_CONTEXTS],
}

/// The backend recorded a draw: every stamp the draw took ([`draw_stamp`])
/// now names `backend::kick_id()`, the kick holding it. After a successful
/// draw that is the open frame the draw is in (or its last part is: the
/// parts before it are in earlier kicks, which complete first); the
/// backend contract says so (`YakoglBackend::kick_id`). Without this move
/// a draw that ended the open frame left stamps naming the kick just
/// submitted: glmark2 shadow's depth pass, after the window clear, was
/// stamped with the clear's kick, which had completed by the time the
/// ground sampled the depth texture, so `texture::resolve_depth` neither
/// submitted nor waited and the PowerVR answered -EBUSY (pvr-depth-resolve,
/// tests/depth_resolve_pvr.rs). Staging stamps without `kicks` are left for
/// the enclosing draw's own call.
fn restamp_draw(kicks: Option<&mut StagingKicks<'_>>) {
    let k = backend::kick_id();
    let s = draw_stamps();
    let mut kicks = kicks;
    for &at in &s.at[..s.n] {
        match (at, kicks.as_deref_mut()) {
            (StampAt::Target { ctx, ti }, _) => framebuffer::set_target_use(ctx as usize, ti as usize, k),
            (StampAt::Texture { group, ti }, _) => {
                if let Some(t) = context::objects(group).textures.get_mut(ti as usize) {
                    t.last_use = k;
                }
            }
            (StampAt::Buffer { group, bi }, _) => {
                if let Some(b) = context::objects(group).buffers.get_mut(bi as usize) {
                    b.last_use = k;
                }
            }
            (StampAt::IndexStaging, Some(sk)) => *sk.index = k,
            (StampAt::AttribStaging(i), Some(sk)) => sk.attrib[i as usize] = k,
            (StampAt::DupStaging(i), Some(sk)) => sk.dup[i as usize] = k,
            (StampAt::Uniforms { group, prog }, Some(sk)) => sk.pg[group as usize][prog as usize].uniforms_kick = k,
            (_, None) => {}
        }
    }
    if s.lost {
        /* a stamp that could not be noted may name a kick before the one
         * holding the draw: wait for the draw itself instead */
        glog_once!("yakogl: a draw took more stamps than the journal holds; waiting for its kick");
        if backend::flush() == 0 {
            let _ = backend::finish();
        }
    }
}

/// Log lines [`submit_draw`] spends on draws the backend refused.
const DRAW_FAIL_LOGS: u32 = 8;

/// Hand a finished draw to the backend. A negative result (the backend
/// could not record it: no room in its arenas, a vertex count beyond its
/// 16-bit index path, ...) means the draw is lost: GL_OUT_OF_MEMORY, the
/// only error GL allows for an implementation limit hit at draw time, and a
/// log line while the budget lasts -- a draw that disappears without a word
/// leaves a hole in the frame nobody can explain. One value is different:
/// `-ENOTSUP` says the GL state is legal but the backend cannot render it
/// yet (a backend gap the backend's own log names). That draw is lost too,
/// but it is reported as GL_INVALID_OPERATION: dEQP treats GL_OUT_OF_MEMORY
/// as a fatal resource error and ends the whole run, while any other error
/// fails only that case (`backend::draw` documents the contract).
///
/// A draw the backend recorded moves the stamps it took to the kick that
/// holds it ([`restamp_draw`]); a lost one leaves them.
fn submit_draw(
    c: &mut GlContext,
    reported: &mut u32,
    d: &GlesDraw,
    what: &'static str,
    kicks: Option<&mut StagingKicks<'_>>,
) {
    let rc = backend::draw(d);
    if rc >= 0 {
        restamp_draw(kicks);
    } else {
        if *reported < DRAW_FAIL_LOGS && log_enabled() {
            *reported += 1;
            glog!("yakogl: {} was not recorded ({}); the frame is missing it", what, rc);
        }
        let err = if rc == -crate::util::ENOTSUP { GL_INVALID_OPERATION } else { GL_OUT_OF_MEMORY };
        c.set_error(err, what);
    }
}

/// A texture's own sampling fields as a draw hands them to the backend
/// (`GlesTextureBind::linear_filter`, `min_linear`, `mip_filter`,
/// `mip_levels`): one function for the draw and for the upload hook
/// (`texture_bind_2d`), so both compute them the same way. The values can
/// still differ between an upload and a later draw (glTexParameteri writes
/// no texels), so a backend never keys on them.
pub(crate) fn sampling_fields(t: &GlTexture) -> (u8, u8, u8, u8) {
    let linear_filter = (t.mag_filter == GL_LINEAR) as u8;
    let min_linear = matches!(t.min_filter, GL_LINEAR | GL_LINEAR_MIPMAP_NEAREST | GL_LINEAR_MIPMAP_LINEAR) as u8;
    /* Only the levels the application filled, and only as far as its
     * GL_TEXTURE_MAX_LEVEL: a sampler allowed past either reads memory
     * nobody wrote (Qt declares a light probe's full chain and prefilters
     * six of its ten levels). */
    let mut lv = t.levels as u32;
    if t.levels_written != 0 && lv > t.levels_written as u32 {
        lv = t.levels_written as u32;
    }
    if lv > t.max_level as u32 + 1 {
        lv = t.max_level as u32 + 1;
    }
    let mip_levels = lv.max(1) as u8;
    /* a mipmap filter needs the chain; without one the texture is
     * incomplete in GL terms and is sampled from level 0 here */
    let mip_filter = if t.levels > 1 && matches!(t.min_filter, GL_NEAREST_MIPMAP_LINEAR | GL_LINEAR_MIPMAP_LINEAR) {
        2
    } else if t.levels > 1 && matches!(t.min_filter, GL_NEAREST_MIPMAP_NEAREST | GL_LINEAR_MIPMAP_NEAREST) {
        1
    } else {
        0
    };
    (linear_filter, min_linear, mip_filter, mip_levels)
}

/// The binding a draw would give a plain 2D texture (one layer, no 3D grid)
/// sampled through a sampler2D, without the draw's side effects (its kick
/// id, the incomplete warning): what the upload hook passes
/// (`backend::texture_uploaded`). The fields a draw also sets from the
/// program (`unit`) are 0.
/// The GL base format of `t` has no alpha channel (`GlesTextureBind::no_alpha`).
pub(crate) fn texture_has_no_alpha(t: &GlTexture) -> bool {
    t.bgra
        || matches!(
            t.format,
            GL_RGB | GL_LUMINANCE | GL_RED | GL_RG | GL_ETC1_RGB8_OES | GL_COMPRESSED_RGB8_ETC2 | GL_DEPTH_COMPONENT
        )
}

/// [`GlesTextureBind::depth_luminance`]: a depth texture on the surface that
/// claims GL_OES_depth_texture, which samples it as luminance, (d, d, d, 1)
/// ("depth value is returned as (d, d, d, 1.0) by GLSL texture* calls";
/// GL_OES_packed_depth_stencil: a depth-stencil texture samples "as if the
/// base internal format was DEPTH").
pub(crate) fn texture_depth_luminance(t: &GlTexture) -> bool {
    super::state::DEPTH_TEXTURE_EXTENSIONS && matches!(t.format, GL_DEPTH_COMPONENT | GL_DEPTH_STENCIL)
}

/// [`GlesTextureBind::compare_op`]: the compare function less GL_NEVER
/// (bit 0 less, bit 1 equal, bit 2 greater) when the compare mode is
/// GL_COMPARE_REF_TO_TEXTURE. With GL_NONE, bit 4 alone: no compare, the
/// look-up returns the depth itself ("If the value of TEXTURE_COMPARE_MODE
/// is NONE, then r = Dt", ES 3.0.6 3.8.15.1; GLSL ES 3.00 rev 6 8.8 calls a
/// shadow call with comparisons off undefined, which this satisfies too).
/// Bit 3: the texture is a fixed-point depth (DEPTH_COMPONENT16/24,
/// DEPTH24_STENCIL8): "If the texture's internal format indicates a
/// fixed-point depth texture, then Dt and Dref are clamped to the range
/// [0, 1]; otherwise no clamping is performed" (3.8.15.1).
fn compare_op(mode: GLenum, func: GLenum, t: &GlTexture) -> u8 {
    let op = if mode == GL_COMPARE_REF_TO_TEXTURE && (GL_NEVER..=GL_ALWAYS).contains(&func) {
        (func - GL_NEVER) as u8
    } else {
        16
    };
    let fixed_depth = matches!(t.format, GL_DEPTH_COMPONENT | GL_DEPTH_STENCIL)
        && !matches!(t.type_, GL_FLOAT | GL_FLOAT_32_UNSIGNED_INT_24_8_REV);
    op | if fixed_depth { 8 } else { 0 }
}

pub(crate) fn texture_bind_2d(t: &GlTexture) -> GlesTextureBind {
    let (linear_filter, min_linear, mip_filter, mip_levels) = sampling_fields(t);
    GlesTextureBind {
        format: t.texfmt,
        type_: GLES_TEXTYPE_2D,
        layers: 1,
        base_gpu: t.mem.gpu,
        content_gen: t.content_gen,
        target_kick: 0,
        cpu: t.mem.cpu as *const core::ffi::c_void,
        width: t.width,
        height: t.height,
        stride_texels: t.level_pitch(t.width),
        bytes: t.level_pitch(t.width).saturating_mul(t.height).saturating_mul(gles_texfmt_bytes(t.texfmt)),
        linear_filter,
        min_linear,
        mip_filter,
        mip_levels,
        wrap_s: map_wrap(t.wrap_s),
        wrap_t: map_wrap(t.wrap_t),
        /* B,G,R,A: an XRGB8888 image, or a texture a draw rendered into */
        bgra: (t.bgra || t.pe_order) as u8,
        incomplete: incomplete_reason(t).is_some() as u8,
        no_alpha: texture_has_no_alpha(t) as u8,
        depth_luminance: texture_depth_luminance(t) as u8,
        ..GlesTextureBind::ZERO
    }
}

/// Instances of the draw being built (draw_instanced sets it around one
/// glDrawArrays/glDrawElements): > 1 makes divisor-1 arrays per-instance
/// streams and the index list instanced.
static DRAW_INSTANCES: AtomicI32 = AtomicI32::new(1);

/// The instance the draw being built is, when draw_instanced draws an
/// instanced draw instance by instance (`GlesDraw::instance_id`); 0 otherwise.
static DRAW_INSTANCE_ID: AtomicU32 = AtomicU32::new(0);

/// A blend variant could not be uploaded (logged once).
static VARIANT_OOM_LOGGED: AtomicBool = AtomicBool::new(false);
/// A blend variant was refused for want of compiler room (logged once).
static VARIANT_NO_ROOM_LOGGED: AtomicBool = AtomicBool::new(false);

/// What a back end's variant or link call wrote into `log`, for a message.
fn log_text(log: &[core::ffi::c_char]) -> &str {
    let n = log.iter().position(|&c| c == 0).unwrap_or(log.len());
    // SAFETY: `log` is `n` initialised bytes before its NUL.
    let b = unsafe { core::slice::from_raw_parts(log.as_ptr() as *const u8, n) };
    core::str::from_utf8(b).unwrap_or("(a log that is not UTF-8)")
}

/// Log lines left for attribute ranges that leave their buffer.
static ATTRIB_RANGE_LOGS: AtomicI32 = AtomicI32::new(8);

/// A client pointer below this is taken for a byte offset meant for a buffer
/// that is not bound, and is refused rather than read (page 0 is unmapped on
/// the Linux harness and holds no application data on the boards). It is
/// what a vertex array reads after glDeleteBuffers reset its binding to zero
/// (ES 2.0.25 2.9): the pointer is still the old offset.
const CLIENT_POINTER_FLOOR: usize = 4096;

/// Log lines left for an enabled client array whose pointer is below
/// [`CLIENT_POINTER_FLOOR`].
static CLIENT_ARRAY_LOGS: AtomicI32 = AtomicI32::new(3);

/// The instance the draw being built is ([`DRAW_INSTANCE_ID`]): the first
/// instance a transform-feedback capture of this draw evaluates.
#[cfg(all(feature = "shaderc-online", any(test, feature = "host-test", not(feature = "report-es2"))))]
pub(super) fn draw_instance_id() -> u32 {
    DRAW_INSTANCE_ID.load(Ordering::Relaxed)
}

/// [`DRAW_INSTANCES`] as the count the draw carries (at least one).
fn draw_instances() -> u32 {
    let n = DRAW_INSTANCES.load(Ordering::Relaxed);
    if n > 1 {
        n as u32
    } else {
        1
    }
}

/// Debug: log the first vertices of every in-place attribute stream at draw
/// time (the backend's PVR_BACKEND_OPT_DEBUG dumps the converted ones).
#[no_mangle]
pub static mut gl_debug_attrib_dump: i32 = 0;

#[no_mangle]
pub unsafe extern "C" fn gl_debug_set_attrib_dump(on: i32) {
    gl_debug_attrib_dump = on;
}

fn attrib_dump_enabled() -> bool {
    // SAFETY: single-threaded by contract; a plain read of an int.
    unsafe { gl_debug_attrib_dump != 0 }
}

extern "C" {
    /// The C library's snprintf: the C printed this dump's floats with `%g`,
    /// which core::fmt has no equivalent of (the PowerVR backend's own
    /// attribute dump uses it the same way).
    fn snprintf(buf: *mut core::ffi::c_char, size: usize, fmt: *const core::ffi::c_char, ...) -> core::ffi::c_int;
}

/// gl_debug_attrib_dump: one vertex of an attribute stream fetched in place,
/// in the C's format ("  attrib vi0 (in place) v0: -1 -1 0 1").
fn dump_in_place_vertex(vi_reg: u8, v: u32, comps: &[f32]) {
    let mut line = [0 as core::ffi::c_char; 96];
    // SAFETY: snprintf writes at most `size` bytes into `line`; the varargs
    // match the format (two unsigned ints).
    let mut n =
        unsafe { snprintf(line.as_mut_ptr(), line.len(), c"  attrib vi%u (in place) v%u:".as_ptr(), vi_reg as u32, v) };
    for x in comps.iter().chain(core::iter::repeat(&0.0)).take(4) {
        if n < 0 || n as usize >= line.len() {
            break;
        }
        // SAFETY: the write stays inside `line` (n < len); `%g` takes the
        // double passed.
        n +=
            unsafe { snprintf(line.as_mut_ptr().add(n as usize), line.len() - n as usize, c" %g".as_ptr(), *x as f64) };
    }
    // SAFETY: snprintf always NUL-terminates inside `line`.
    osal::log_str(unsafe { core::ffi::CStr::from_ptr(line.as_ptr()) });
}

/// `g_osal->log` is set (the C code guards its debug loops with it).
fn log_enabled() -> bool {
    osal::ops().is_some_and(|o| o.log.is_some())
}

/// `if (g_osal->cache_flush) g_osal->cache_flush(cpu, n)`. Unlike
/// `osal::cache_flush` there is no null / n == 0 shortcut: the OSAL call
/// stream stays the one of the C code.
///
/// # Safety
/// `cpu..cpu+n` must be inside a live CPU mapping.
unsafe fn flush_cache(cpu: *mut c_void, n: usize) {
    if let Some(f) = osal::ops().and_then(|o| o.cache_flush) {
        f(cpu, n);
    }
}

/// `if (g_osal->cache_invd) g_osal->cache_invd(cpu, n)`; see [`flush_cache`].
///
/// # Safety
/// As [`flush_cache`].
unsafe fn invd_cache(cpu: *mut c_void, n: usize) {
    if let Some(f) = osal::ops().and_then(|o| o.cache_invd) {
        f(cpu, n);
    }
}

/// The raw bytes of a slice of plain dwords (f32 / u32).
fn dword_bytes<T: Copy>(s: &[T]) -> &[u8] {
    // SAFETY: only used with f32/u32 (no padding, every byte initialised);
    // the length covers exactly the slice.
    unsafe { core::slice::from_raw_parts(s.as_ptr() as *const u8, core::mem::size_of_val(s)) }
}

/// Upload host bytes into a GPU allocation, flushing so the GPU sees them.
/// (The C file's put_scratch() is this with off = 0.)
fn put(mem: &mut OsalGpuMem, off: usize, src: &[u8]) -> i32 {
    if !mem.cpu.is_null() {
        // SAFETY: the callers keep off + src.len() inside the allocation
        // `mem` describes (exactly as far as the C code did); a host slice
        // and a GPU mapping never overlap.
        unsafe {
            let dst = (mem.cpu as *mut u8).add(off);
            core::ptr::copy_nonoverlapping(src.as_ptr(), dst, src.len());
            flush_cache(dst as *mut c_void, src.len());
        }
        return 0;
    }
    match osal::ops().and_then(|o| o.gpu_write) {
        // SAFETY: `mem` is a live allocation, `src` a valid slice.
        Some(f) => unsafe { f(mem, off, src.as_ptr() as *const c_void, src.len()) },
        None => -1,
    }
}

/// Grow-only scratch: (re)allocate when too small; the bump allocator cannot
/// free, so the old block is simply left behind.
fn scratch_reserve(m: &mut OsalGpuMem, need: usize) -> i32 {
    if !m.impl_.is_null() && m.size >= need {
        return 0;
    }
    if !m.impl_.is_null() {
        osal::gpu_free(m);
    }
    *m = OsalGpuMem::ZERO;
    osal::gpu_alloc(backend::heap_base(GLES_HEAP_DATA), need, OSAL_MEM_DATA, m)
}

/// [`scratch_reserve`] for per-draw staging. With a CPU view the backend
/// keeps a per-draw copy of what the core staged (`GlesAttrib::cpu`,
/// `index_cpu`, `*_uniform_cpu`), so one reusable block is enough. Without
/// one the kick reads the block itself: a block a pending kick still reads
/// is released (the OSAL defers the free until that kick is done) and the
/// draw stages into fresh memory. `kick` stamps the block with the frame
/// that reads it; `at` is where that stamp lives ([`draw_stamp`]).
fn staging_reserve(m: &mut OsalGpuMem, kick: &mut u32, at: StampAt, need: usize) -> i32 {
    if !m.impl_.is_null() && m.cpu.is_null() && backend::kick_pending(*kick) {
        osal::gpu_free(m);
        *m = OsalGpuMem::ZERO;
    }
    let rc = scratch_reserve(m, need);
    if rc == 0 {
        *kick = draw_stamp(at);
    }
    rc
}

/// One client-array component -> float bits (GL normalisation rules, ES 3.0
/// 2.1.6 / 2.9). An integer attribute (glVertexAttribIPointer) is widened to
/// a 32-bit integer whose bits travel in the float slot: the vertex fetch
/// moves dwords as-is and the shader reads them as an ivec/uvec. The value is
/// returned as its bit pattern so that such integers never pass through a
/// float register. `src` holds the component's bytes (1, 2 or 4 by `type_`).
/// The packed 2_10_10_10 types are not per-component: see [`attrib_row`].
fn attrib_component(src: &[u8], type_: GLenum, normalized: GLboolean, integer: bool) -> u32 {
    let i16_at = |s: &[u8]| i16::from_ne_bytes([s[0], s[1]]);
    let u16_at = |s: &[u8]| u16::from_ne_bytes([s[0], s[1]]);
    let u32_at = |s: &[u8]| u32::from_ne_bytes([s[0], s[1], s[2], s[3]]);

    if integer {
        return match type_ {
            GL_BYTE => (src[0] as i8) as i32 as u32,
            GL_UNSIGNED_BYTE => src[0] as u32,
            GL_SHORT => i16_at(src) as i32 as u32,
            GL_UNSIGNED_SHORT => u16_at(src) as u32,
            _ => u32_at(src),
        };
    }
    let normalized = normalized != 0;
    let f: f32 = match type_ {
        GL_BYTE => signed_component(src[0] as i8 as i32, 8, normalized),
        GL_UNSIGNED_BYTE => unsigned_component(src[0] as u32, 8, normalized),
        GL_SHORT => signed_component(i16_at(src) as i32, 16, normalized),
        GL_UNSIGNED_SHORT => unsigned_component(u16_at(src) as u32, 16, normalized),
        /* glVertexAttribPointer integers are converted to float (ES 3.0 2.9);
         * only glVertexAttribIPointer keeps them (the `integer` path above) */
        GL_INT => signed_component(u32_at(src) as i32, 32, normalized),
        GL_UNSIGNED_INT => unsigned_component(u32_at(src), 32, normalized),
        GL_FIXED => (u32_at(src) as i32) as f32 / 65536.0,
        GL_HALF_FLOAT => half_to_float(u16_at(src)),
        _ => return u32_at(src),
    };
    f.to_bits()
}

/// A signed `bits`-wide component as float: its value, or normalised by
/// max(c / (2^(bits-1) - 1), -1) (ES 3.0 2.1.6.1). Up to 16 bits both
/// operands are exact in f32 and the f32 quotient is the one the C computed;
/// 32-bit values are divided in f64 so that the extremes land on +-1.
fn signed_component(v: i32, bits: u32, normalized: bool) -> f32 {
    if !normalized {
        return v as f32;
    }
    let max = (1u64 << (bits - 1)) - 1;
    if bits <= 16 {
        (v as f32 / max as f32).max(-1.0)
    } else {
        (v as f64 / max as f64).max(-1.0) as f32
    }
}

/// An unsigned `bits`-wide component as float: its value, or c / (2^bits - 1)
/// (in f32 up to 16 bits, as [`signed_component`]).
fn unsigned_component(v: u32, bits: u32, normalized: bool) -> f32 {
    if !normalized {
        return v as f32;
    }
    let max = (1u64 << bits) - 1;
    if bits <= 16 {
        v as f32 / max as f32
    } else {
        (v as f64 / max as f64) as f32
    }
}

/// IEEE 754 binary16 -> binary32 (GL_HALF_FLOAT arrays, half texels).
pub(crate) fn half_to_float(h: u16) -> f32 {
    let sign = ((h >> 15) as u32) << 31;
    let exp = ((h >> 10) & 0x1f) as u32;
    let mant = (h & 0x3ff) as u32;
    let bits = match (exp, mant) {
        (0, 0) => sign,
        (0, m) => {
            /* subnormal: renormalise */
            let mut m = m;
            let mut e = 1i32;
            while m & 0x400 == 0 {
                m <<= 1;
                e -= 1;
            }
            let m = m & 0x3ff;
            sign | (((e + 127 - 15) as u32) << 23) | (m << 13)
        }
        (0x1f, m) => sign | 0x7f80_0000 | (m << 13), /* inf / NaN */
        (e, m) => sign | ((e + 127 - 15) << 23) | (m << 13),
    };
    f32::from_bits(bits)
}

/// The packed ES 3.0 types: one 32-bit word holding x, y, z (10 bits each)
/// and w (2 bits), converted per ES 3.0 2.1.6 as their own bit widths.
fn packed_row(word: u32, signed: bool, normalized: bool) -> [u32; 4] {
    let field = |shift: u32, bits: u32| -> f32 {
        let raw = (word >> shift) & ((1u32 << bits) - 1);
        if signed {
            /* sign-extend the field */
            let v = ((raw << (32 - bits)) as i32) >> (32 - bits);
            signed_component(v, bits, normalized)
        } else {
            unsigned_component(raw, bits, normalized)
        }
    };
    [field(0, 10), field(10, 10), field(20, 10), field(30, 2)].map(f32::to_bits)
}

fn is_packed_type(type_: GLenum) -> bool {
    matches!(type_, GL_INT_2_10_10_10_REV | GL_UNSIGNED_INT_2_10_10_10_REV)
}

fn attrib_type_size(type_: GLenum) -> u32 {
    match type_ {
        GL_BYTE | GL_UNSIGNED_BYTE => 1,
        GL_SHORT | GL_UNSIGNED_SHORT | GL_HALF_FLOAT => 2,
        _ => 4,
    }
}

/// The bytes one array element occupies (the default stride): `size`
/// components of the type, or the one word of a packed type.
/// The attribute array's source format, as the backend contract names it.
fn map_attrib_format(type_: GLenum) -> GlesAttribFormat {
    match type_ {
        GL_BYTE => GLES_ATTRIBF_BYTE,
        GL_UNSIGNED_BYTE => GLES_ATTRIBF_UBYTE,
        GL_SHORT => GLES_ATTRIBF_SHORT,
        GL_UNSIGNED_SHORT => GLES_ATTRIBF_USHORT,
        GL_FIXED => GLES_ATTRIBF_FIXED,
        _ => GLES_ATTRIBF_FLOAT32,
    }
}

/// Can the backend fetch this array as it lies in the vertex buffer?
fn attrib_format_is_native(fmt: GlesAttribFormat) -> bool {
    fmt == GLES_ATTRIBF_FLOAT32 || backend::native_attrib_formats() & gles_attribf_bit(fmt) != 0
}

fn attrib_elem_bytes(ga: &GlVertexAttrib) -> u32 {
    if is_packed_type(ga.type_) {
        4
    } else {
        (ga.size as u32).min(4) * attrib_type_size(ga.type_)
    }
}

/// The array's stride: the explicit one, else one element.
fn attrib_stride(ga: &GlVertexAttrib) -> u32 {
    if ga.stride > 0 {
        ga.stride as u32
    } else {
        attrib_elem_bytes(ga)
    }
}

/// The four dwords a shader input reads from one array element: components
/// the array does not supply read as (0, 0, 0, 1), as GL fills a narrower
/// attribute into a wider shader input. `vsrc` holds one element
/// ([`attrib_elem_bytes`]).
fn attrib_row(vsrc: &[u8], ga: &GlVertexAttrib, comps: u32, tsz: u32, out_comps: u32) -> [u32; 4] {
    let mut row = [0u32; 4];
    let packed = is_packed_type(ga.type_).then(|| {
        let word = u32::from_ne_bytes([vsrc[0], vsrc[1], vsrc[2], vsrc[3]]);
        packed_row(word, ga.type_ == GL_INT_2_10_10_10_REV, ga.normalized != 0)
    });
    for c in 0..out_comps.min(4) {
        row[c as usize] = if let Some(p) = packed {
            p[c as usize]
        } else if c < comps {
            let at = (c * tsz) as usize;
            attrib_component(&vsrc[at..at + tsz as usize], ga.type_, ga.normalized, ga.integer)
        } else if c == 3 {
            1.0f32.to_bits()
        } else {
            0.0f32.to_bits()
        };
    }
    row
}

/// Converts vertices first..first+count-1 of an attribute array to packed
/// float32 in the attribute's scratch buffer `m` (vertex `first` lands at 0).
/// The vertex fetch DMAs dwords as-is, so every non-float32 format (and the
/// client-side arrays, which live in app memory) goes through this
/// conversion at draw time. `src` is the CPU view of the array: the client
/// pointer, or the VBO's CPU mapping; a VBO without one is read through the
/// OSAL (`vb` + `vb_off`). `kick` is the scratch's last-use stamp
/// ([`staging_reserve`]).
fn convert_attrib(
    m: &mut OsalGpuMem,
    kick: &mut u32,
    at: StampAt,
    ga: &GlVertexAttrib,
    src: *const u8,
    mut vb: Option<&mut OsalGpuMem>,
    vb_off: usize,
    first: u32,
    count: u32,
    out_comps: u32,
) -> i32 {
    let Some(ops) = osal::ops() else {
        return -1;
    };
    let comps = (ga.size as u32).min(4);
    let tsz = attrib_type_size(ga.type_);
    let stride = attrib_stride(ga) as u64;
    let out_comps = out_comps.min(4);
    let row_bytes = out_comps as usize * 4;
    let elem = attrib_elem_bytes(ga) as usize;
    /* an oversized request fails the allocation, it never wraps into a
     * small one */
    let Some(need) = (count as usize).checked_mul(row_bytes) else {
        return -1;
    };

    if (src.is_null() && elem != 0 && !(vb.is_some() && ops.gpu_read.is_some()))
        || staging_reserve(m, kick, at, need) != 0
    {
        return -1;
    }
    if m.cpu.is_null() && ops.gpu_write.is_none() {
        return -1;
    }
    for v in 0..count {
        let mut raw = [0u8; 16];
        /* u64: (u32 + u32) * stride (< 2^31) cannot overflow */
        let Ok(at) = usize::try_from((first as u64 + v as u64) * stride) else {
            return -1;
        };
        let vsrc: &[u8] = if elem == 0 {
            &[] /* an array whose pointer was never set (size 0): (0, 0, 0, 1) */
        } else if !src.is_null() {
            // SAFETY: the VBO mapping holds the range build_common() checked;
            // a client array is trusted to hold `elem` bytes per vertex of
            // the range drawn, as the C code did (its address is formed
            // without assuming it stays in bounds).
            unsafe { core::slice::from_raw_parts(src.wrapping_add(at), elem) }
        } else {
            let (Some(read), Some(vb)) = (ops.gpu_read, vb.as_deref_mut()) else {
                return -1;
            };
            let Some(off) = vb_off.checked_add(at) else {
                return -1;
            };
            // SAFETY: `vb` is a live allocation; `raw` has room for the at
            // most 16 bytes of one element.
            if unsafe { read(vb, off, raw.as_mut_ptr() as *mut c_void, elem) } != 0 {
                return -1;
            }
            &raw[..elem]
        };
        let row = attrib_row(vsrc, ga, comps, tsz, out_comps);
        let row = &dword_bytes(&row)[..row_bytes];
        if !m.cpu.is_null() {
            // SAFETY: staging_reserve() made `m` at least `need` bytes and
            // (v + 1) * row_bytes <= need.
            unsafe {
                let dst = (m.cpu as *mut u8).add(v as usize * row_bytes);
                core::ptr::copy_nonoverlapping(row.as_ptr(), dst, row_bytes);
            }
        } else if let Some(write) = ops.gpu_write {
            // SAFETY: `m` is a live allocation, `row` a valid slice.
            if unsafe { write(m, v as usize * row_bytes, row.as_ptr() as *const c_void, row_bytes) } != 0 {
                return -1;
            }
        }
    }
    if !m.cpu.is_null() {
        // SAFETY: the `need` bytes just written are inside the mapping.
        unsafe { flush_cache(m.cpu, need) };
    }
    0
}

/* --------------------------------------------------------------------------- */
/* enum mapping: GL -> PVR backend */

fn map_prim(mode: GLenum) -> Option<GlesPrim> {
    match mode {
        GL_TRIANGLES => Some(GLES_PRIM_TRIANGLES),
        GL_TRIANGLE_STRIP => Some(GLES_PRIM_TRI_STRIP),
        GL_TRIANGLE_FAN => Some(GLES_PRIM_TRI_FAN),
        GL_LINES => Some(GLES_PRIM_LINES),
        GL_LINE_STRIP => Some(GLES_PRIM_LINE_STRIP),
        GL_LINE_LOOP => Some(GLES_PRIM_LINE_LOOP),
        GL_POINTS => Some(GLES_PRIM_POINTS),
        _ => None,
    }
}

/// GL compare enums (0x200..0x207) are in the same order as `GlesCmp`.
fn map_cmp(f: GLenum) -> GlesCmp {
    if !(GL_NEVER..=GL_ALWAYS).contains(&f) {
        return GLES_CMP_ALWAYS;
    }
    f - GL_NEVER
}

/// GL blend factor -> the compiler's codes (gles_shaderc.h).
fn map_blend_factor(f: GLenum) -> u32 {
    match f {
        GL_ZERO => SHC_BF_ZERO,
        GL_ONE => SHC_BF_ONE,
        GL_SRC_COLOR => SHC_BF_SRC_COLOR,
        GL_ONE_MINUS_SRC_COLOR => SHC_BF_INV_SRC_COLOR,
        GL_SRC_ALPHA => SHC_BF_SRC_ALPHA,
        GL_ONE_MINUS_SRC_ALPHA => SHC_BF_INV_SRC_ALPHA,
        GL_DST_ALPHA => SHC_BF_DST_ALPHA,
        GL_ONE_MINUS_DST_ALPHA => SHC_BF_INV_DST_ALPHA,
        GL_DST_COLOR => SHC_BF_DST_COLOR,
        GL_ONE_MINUS_DST_COLOR => SHC_BF_INV_DST_COLOR,
        GL_SRC_ALPHA_SATURATE => SHC_BF_SRC_ALPHA_SATURATE,
        GL_CONSTANT_COLOR => SHC_BF_CONST_COLOR,
        GL_ONE_MINUS_CONSTANT_COLOR => SHC_BF_INV_CONST_COLOR,
        GL_CONSTANT_ALPHA => SHC_BF_CONST_ALPHA,
        GL_ONE_MINUS_CONSTANT_ALPHA => SHC_BF_INV_CONST_ALPHA,
        _ => SHC_BF_ONE,
    }
}

fn map_blend_eq(e: GLenum) -> u32 {
    match e {
        GL_FUNC_SUBTRACT => SHC_BLEND_SUBTRACT,
        GL_FUNC_REVERSE_SUBTRACT => SHC_BLEND_REVERSE_SUBTRACT,
        /* ES 3.0 blend equations glBlendEquation* accepts; the offline table
         * has no MIN/MAX variant, so such a draw takes the missing-variant
         * path (logged once, drawn unblended) instead of rendering as ADD. */
        GL_MIN => SHC_BLEND_MIN,
        GL_MAX => SHC_BLEND_MAX,
        _ => SHC_BLEND_ADD,
    }
}

/// The channels glColorMask disables, as `shc_blend_key_mask_off` bits.
/// Every channel when the draw buffers write no colour at all
/// ([`framebuffer::colour_writes_off`]).
fn disabled_channels(c: &GlContext) -> u32 {
    if framebuffer::colour_writes_off(c) {
        return 0xf;
    }
    let mut off = 0;
    for (ch, m) in c.color_writemask.iter().enumerate() {
        if *m == 0.0 {
            off |= 1u32 << ch;
        }
    }
    off
}

/// The blend state and colour mask of the context as a variant key; 0 when no
/// FS variant is needed (blending off or ONE/ZERO, which is a plain replace,
/// and every channel written). A mask with every channel off is handled by
/// the ISP (`GlesDraw::color_write`), not by a variant.
fn current_blend_key(c: &GlContext) -> u32 {
    let mut key = SHC_BLEND_KEY_REPLACE;
    let off = disabled_channels(c);

    /* an integer target: the variant that packs the integer output for its
     * layout (never blended, ES 3.0 4.1.7), with the mask of the channels it
     * has (a channel it lacks is never written, masked or not) */
    if let Some(l) = gles_surf_int_layout(framebuffer::fb_target_format(c)) {
        let rt = ShcIntRt { channels: l.channels, bits: l.bits, signed: l.signed };
        let own = (1u32 << l.channels) - 1;
        let off = off & own;
        /* every channel off is the ISP's (`GlesDraw::color_write`), as below */
        let mask = if off != 0 && disabled_channels(c) != 0xf { shc_blend_key_mask_off(off) } else { 0 };
        return shc_blend_key_int_rt(rt) | mask;
    }

    if c.blend && !framebuffer::fb_target_is_integer(c) {
        key = shc_blend_key(
            map_blend_factor(c.blend_src),
            map_blend_factor(c.blend_dst),
            map_blend_factor(c.blend_src_a),
            map_blend_factor(c.blend_dst_a),
            map_blend_eq(c.blend_eq),
            map_blend_eq(c.blend_eq_a),
        );
    }
    if off != 0xf {
        key |= shc_blend_key_mask_off(off);
    }
    /* a float render target takes the fragment shader variant that packs
     * its output for that class (a non-zero key even for plain replace) */
    key |= shc_blend_key_rt(gles_surf_rt_class(framebuffer::fb_target_format(c)));
    if key == SHC_BLEND_KEY_REPLACE {
        0
    } else {
        key
    }
}

fn map_stencil_op(op: GLenum) -> GlesStencilOp {
    match op {
        GL_ZERO => GLES_SOP_ZERO,
        GL_REPLACE => GLES_SOP_REPLACE,
        GL_INCR => GLES_SOP_INCR_SAT,
        GL_DECR => GLES_SOP_DECR_SAT,
        GL_INVERT => GLES_SOP_INVERT,
        GL_INCR_WRAP => GLES_SOP_INCR_WRAP,
        GL_DECR_WRAP => GLES_SOP_DECR_WRAP,
        _ => GLES_SOP_KEEP,
    }
}

/// Resolve cull enable/mode/front-face into the winding the backend should cull.
fn map_cull(c: &GlContext) -> GlesCull {
    if !c.cull_face || c.cull_mode == GL_FRONT_AND_BACK {
        /* FRONT_AND_BACK culls every polygon, which has no backend
         * equivalent: the draw entry points drop such triangle draws
         * (culls_everything), so what gets here is points and lines, which
         * culling never affects. */
        return GLES_CULL_NONE;
    }

    /* winding of the front face */
    let front_cw = c.front_face == GL_CW;
    /* winding of the faces being culled */
    let cull_front = c.cull_mode == GL_FRONT;
    let culled_cw = if cull_front { front_cw } else { !front_cw };
    if culled_cw {
        GLES_CULL_CW
    } else {
        GLES_CULL_CCW
    }
}

/// Culling is on with GL_FRONT_AND_BACK and the primitive is a polygon: no
/// fragment of the draw survives (ES 3.0 3.6.1), so there is nothing to
/// record (the pending clear stays pending for the next draw or flush).
fn culls_everything(c: &GlContext, prim: GlesPrim) -> bool {
    (c.cull_face
        && c.cull_mode == GL_FRONT_AND_BACK
        && matches!(prim, GLES_PRIM_TRIANGLES | GLES_PRIM_TRI_STRIP | GLES_PRIM_TRI_FAN))
        || covers_no_sample(c)
}

/// A draw into a multisampled default framebuffer whose sample coverage
/// mask is empty (glSampleCoverage(0, FALSE) or (1, TRUE)) changes no
/// sample, of colour, depth or stencil (ES 2.0 4.1.3): nothing to draw.
fn covers_no_sample(c: &GlContext) -> bool {
    let samples = if c.bound_framebuffer == 0 { c.default_samples } else { 1 };
    samples > 1 && c.sample_coverage && sample_coverage_mask(c, samples) == 0
}

/// The glSampleCoverage mask for `samples` samples (the value's share of
/// them, the low bits; inverted on request), as the backends apply it.
fn sample_coverage_mask(c: &GlContext, samples: u8) -> u8 {
    let full = (1u32 << samples.min(8)) - 1;
    let n = (c.sample_coverage_value * samples as f32) as u32;
    let mut m = (1u32 << n.min(8)) - 1;
    if c.sample_coverage_invert {
        m = !m;
    }
    (m & full) as u8
}

/* --------------------------------------------------------------------------- */
/* per-program GPU cache management */

/// Drop the cached USC/uniform GPU uploads of a (re)linked program.
pub fn invalidate_program(c: &mut GlContext, program: GLuint) {
    invalidate_program_in(c.group, program);
}

/// [`invalidate_program`] for a share group with no context left: the cache
/// keys on the group, because the program does (ES 3.0 2.6.1).
pub fn invalidate_program_in(group: u8, program: GLuint) {
    if obj_index(program) >= MAX_PROGRAMS {
        return;
    }
    // SAFETY: single-threaded by contract; the draw entry points never call
    // this, so no other reference to STATE is live.
    let pg = unsafe { &mut STATE.get().pg[group as usize][obj_index(program)] };
    if !pg.vs_usc.impl_.is_null() {
        osal::gpu_free(&mut pg.vs_usc);
    }
    if !pg.fs_usc.impl_.is_null() {
        osal::gpu_free(&mut pg.fs_usc);
    }
    if !pg.uniforms.impl_.is_null() {
        osal::gpu_free(&mut pg.uniforms);
    }
    for v in pg.fs_var.iter_mut() {
        if !v.usc.impl_.is_null() {
            osal::gpu_free(&mut v.usc);
        }
        shaderc::release_shader(&v.sh);
    }
    for i in 0..PROG_MAX_LOD_VARIANTS {
        free_lod_variant(pg, i);
    }
    // SAFETY: every field of ProgGpu is valid when all-zero.
    unsafe { core::ptr::write_bytes(pg as *mut ProgGpu, 0, 1) };
}

/// Release a program's GPU side-allocations (glDeleteProgram, and the last
/// holder of a share group letting its objects go).
pub fn release_program(c: &mut GlContext, program: GLuint) {
    invalidate_program_in(c.group, program);
}

/// [`release_program`] for a group with no context left.
pub fn release_program_in(group: u8, program: GLuint) {
    invalidate_program_in(group, program);
}

#[no_mangle]
pub unsafe extern "C" fn gl_draw_invalidate_program(program: GLuint) {
    if let Some(c) = context::current() {
        invalidate_program(c, program);
    }
}

#[no_mangle]
pub unsafe extern "C" fn gl_draw_release_program(program: GLuint) {
    if let Some(c) = context::current() {
        release_program(c, program);
    }
}

/// Bring-up debug: the GPU allocations holding a linked program's USC.
#[no_mangle]
pub unsafe extern "C" fn gl_draw_debug_usc(program: GLuint, fragment: i32) -> *const OsalGpuMem {
    let Some(c) = context::current() else {
        return core::ptr::null();
    };
    if obj_index(program) >= MAX_PROGRAMS {
        return core::ptr::null();
    }
    let pg = &STATE.get().pg[c.group as usize][obj_index(program)];
    if !pg.valid {
        return core::ptr::null();
    }
    if fragment != 0 {
        &pg.fs_usc
    } else {
        &pg.vs_usc
    }
}

/// The FS variant for blend key `key`: looked up in the shader backend once
/// per (program, key) and its USC uploaded once, in one of
/// [`PROG_MAX_FS_VARIANTS`] slots per program (a new key takes a free slot
/// or the least recently used one). Returns the slot in `pg.fs_var`,
/// `Ok(None)` when the draw needs none (key 0), and the reason when it needs
/// one it cannot have. The draw is then refused (`build_common`), never
/// drawn with the plain shader. A variant that does not exist, does not
/// build or does not link is remembered as missing; one refused for want of
/// room (the compiler's [`VARIANT_NO_ROOM`], or GPU memory for its USC)
/// leaves the slot free, so a later draw asks again.
///
/// `lod_key` is the fragment key of the sampler-state variant `rfs` is (0:
/// the program's own): a blend variant is of one fragment shader, so the
/// slots are keyed by both.
fn fs_variant_for(
    pg: &mut ProgGpu,
    rvs: &ShcShader,
    rfs: &ShcShader,
    key: u32,
    lod_key: u64,
) -> Result<Option<usize>, &'static str> {
    if key == 0 {
        return Ok(None);
    }
    pg.fs_clock = pg.fs_clock.wrapping_add(1);
    let now = pg.fs_clock;
    if let Some(i) = pg.fs_var.iter().position(|v| v.blend_key == key && v.lod_key == lod_key) {
        pg.fs_var[i].stamp = now;
        return if pg.fs_var[i].missing { Err("no fragment shader variant for its blend key") } else { Ok(Some(i)) };
    }
    /* A free slot, or the least recently used one. Its USC goes back through
     * the OSAL's deferred free, which holds it until the frames that may
     * still read it have completed. */
    let slot = pg.fs_var.iter().position(|v| v.blend_key == 0).unwrap_or_else(|| {
        (0..PROG_MAX_FS_VARIANTS).max_by_key(|&i| now.wrapping_sub(pg.fs_var[i].stamp)).unwrap_or(0)
    });
    let v = &mut pg.fs_var[slot];
    if !v.usc.impl_.is_null() {
        osal::gpu_free(&mut v.usc);
    }
    shaderc::release_shader(&v.sh);
    // SAFETY: every field of ProgFsVariant is valid when all-zero.
    unsafe { core::ptr::write_bytes(v as *mut ProgFsVariant, 0, 1) };
    v.blend_key = key;
    v.lod_key = lod_key;
    v.stamp = now;
    let mut log = [0 as core::ffi::c_char; 192];
    let rc = match shaderc::backend().and_then(|b| b.blend_variant) {
        // SAFETY: `rfs` and `v.sh` are valid for the call; the log is as long as said.
        Some(f) => unsafe { f(rfs, key, &mut v.sh, log.as_mut_ptr(), log.len()) },
        None => {
            // SAFETY: the log is valid for its length.
            let _ = unsafe { util::CStrWriter::new(log.as_mut_ptr(), log.len()) }
                .write_str("the shader back end has no blend variants");
            -util::ENOENT
        }
    };
    /* No room in the compiler now is not a property of the blend state: the
     * slot is freed, so a draw after something was freed tries again (the
     * back end refuses at once until then). This draw is refused. */
    let no_room = |v: &mut ProgFsVariant, log: &[core::ffi::c_char]| {
        if !VARIANT_NO_ROOM_LOGGED.swap(true, Ordering::Relaxed) {
            glog!("yakogl: no room for the blend key 0x{:x} variant ({}); the draw is refused", key, log_text(log));
        }
        shaderc::release_shader(&v.sh);
        // SAFETY: every field of ProgFsVariant is valid when all-zero.
        unsafe { core::ptr::write_bytes(v as *mut ProgFsVariant, 0, 1) };
        Err("no room for its blend key's fragment shader variant")
    };
    if rc == VARIANT_NO_ROOM {
        return no_room(v, &log);
    }
    if rc != 0 {
        glog_once!(
            "yakogl: no fragment shader variant for blend key 0x{:x} ({}); its draws are refused",
            key,
            log_text(&log)
        );
        v.missing = true;
        return Err("no fragment shader variant for its blend key");
    }
    /* The variant is a fresh copy from the shader back end, so its varyings
     * carry the offsets the fragment shader was compiled with -- its own
     * packing, which is not the vertex shader's whenever the fragment stage
     * reads fewer varyings than the vertex stage writes. Link it the same way
     * glLinkProgram linked p.rfs, or it iterates the wrong vertex outputs:
     * customeffect's floor read its transformed UV out of the slot holding
     * the tangent, and every draw into an RGBA16F target takes a variant. */
    if let Some(link) = shaderc::backend().and_then(|b| b.link) {
        // SAFETY: `rvs` and `v.sh` are valid shaders and distinct; the log is as long as said.
        let rc = unsafe { link(rvs, &mut v.sh, log.as_mut_ptr(), log.len()) };
        if rc == VARIANT_NO_ROOM {
            return no_room(v, &log);
        }
        if rc != 0 {
            glog_once!(
                "yakogl: fragment shader variant for key 0x{:x} does not link ({}); its draws are refused",
                key,
                log_text(&log)
            );
            v.missing = true;
            return Err("its blend key's fragment shader variant does not link");
        }
    }
    let bytes = v.sh.usc_dw as usize * 4;
    if osal::gpu_alloc(backend::heap_base(GLES_HEAP_SHADER), bytes, OSAL_MEM_DATA, &mut v.usc) != 0
        || put(&mut v.usc, 0, dword_bytes(v.sh.usc_words())) != 0
    {
        /* Out of memory is not a property of the blend state: free the slot
         * so that the next draw tries again (a `missing` mark would refuse
         * every later draw with that key). This draw is refused. */
        if !VARIANT_OOM_LOGGED.swap(true, Ordering::Relaxed) {
            glog!("yakogl: out of memory for the blend key 0x{:x} variant; the draw is refused", key);
        }
        if !v.usc.impl_.is_null() {
            osal::gpu_free(&mut v.usc);
        }
        shaderc::release_shader(&v.sh);
        // SAFETY: every field of ProgFsVariant is valid when all-zero.
        unsafe { core::ptr::write_bytes(v as *mut ProgFsVariant, 0, 1) };
        return Err("out of memory for its blend key's fragment shader variant");
    }
    Ok(Some(slot))
}

/* ---- sampler-state variants (shaderc::LodKey) ------------------------------
 *
 * A stage's code can depend on the filter state of the samplers it reads
 * (the online compiler says which bindings: ShcBackend::lod_sensitivity):
 * the PowerVR's exact gradients are not needed for a sampler without a
 * mipmap filter, and an explicit level has to be remapped for the c = 0.5
 * switch of mag LINEAR with min NEAREST_MIPMAP_*. Every draw computes the
 * key from the state bound NOW -- the units the sampler uniforms name, the
 * textures bound there, a sampler object's filters -- so glTexParameter,
 * glBindTexture, glBindSampler and glUniform1i need no hook: a changed state
 * is a different key at the next draw. A program with no sensitive binding
 * (every offline-table shader, most programs) costs one compare. */

/// Logged once: a variant a draw asked for could not be had.
static LOD_VARIANT_LOGGED: AtomicBool = AtomicBool::new(false);

/// The texture a stage's sampler binding reads at this draw: its minification
/// and magnification filters (a sampler object's in place of the texture's
/// own, ES 3.0 3.8.2), whether it has more than one level, and its storage
/// format (`GLES_TEXFMT_*`). `None`: no texture there.
fn binding_filters(c: &GlContext, unit: usize, gl_type: u16) -> Option<(GLenum, GLenum, bool, u8)> {
    let (bound, target) = match binding_type(gl_type) {
        GL_SAMPLER_CUBE_T | GL_SAMPLER_CUBE_SHADOW_T => (*c.bound_texture_cube.get(unit)?, GL_TEXTURE_CUBE_MAP),
        GL_SAMPLER_2D_ARRAY_T | GL_SAMPLER_2D_ARRAY_SHADOW_T => {
            (*c.bound_texture_2d_array.get(unit)?, GL_TEXTURE_2D_ARRAY)
        }
        GL_SAMPLER_3D_T => (*c.bound_texture_3d.get(unit)?, GL_TEXTURE_3D),
        _ => (*c.bound_texture_2d.get(unit)?, GL_TEXTURE_2D),
    };
    let slot = if bound == 0 { super::texture::default_slot(target)? } else { bound as usize };
    let t = c.objs_ref().textures.get(slot).filter(|t| slot != 0 && t.used)?;
    let (min, mag) = match super::es3::sampler_state(c.index, unit) {
        Some((min, mag, _, _)) => (min, mag),
        None => (t.min_filter, t.mag_filter),
    };
    Some((min, mag, t.levels > 1, t.texfmt))
}

/// Can a cube sampler with these filters read past a face's edge? A LINEAR
/// tap can (seamless on the V3D, per face in ES 2.0: `LodKey::cube_noseam`);
/// NEAREST and NEAREST_MIPMAP_NEAREST read the selected face's texel under
/// both rules.
fn cube_filters_see_seams(min: GLenum, mag: GLenum) -> bool {
    mag == GL_LINEAR || matches!(min, GL_LINEAR | GL_LINEAR_MIPMAP_NEAREST | GL_LINEAR_MIPMAP_LINEAR)
}

/// One stage's key for this draw: of the bindings in `sense`, those whose
/// bound sampler has no mipmap filter in use (`mip_free`: a NEAREST or
/// LINEAR minification, or a texture of one level, `sampling_fields`' rule),
/// those with mag LINEAR and min NEAREST_MIPMAP_* (`mag_switch`), those
/// whose texture is stored R32F (`ret32`: a depth texture, which the V3D's
/// TMU returns at 32 bits a channel), and the cube bindings whose filters
/// make the V3D's seamless filtering differ from ES 2.0's per-face rule
/// (`cube_noseam`: mag LINEAR, or min LINEAR / LINEAR_MIPMAP_*).
fn stage_lod_key(c: &GlContext, sense: u64, sh: &ShcShader, units: &[u8; CAP_SAMPLERS]) -> u64 {
    if sense == 0 {
        return 0;
    }
    let s = shaderc::LodKey::from_bits(sense);
    let mut k = shaderc::LodKey::NONE;
    for (binding, gl_type) in sh.sampler_bindings() {
        let bit = 1u32.checked_shl(u32::from(binding)).unwrap_or(0);
        if (s.mip_free | s.mag_switch | s.ret32 | s.cube_noseam) & bit == 0 || binding as usize >= CAP_SAMPLERS {
            continue;
        }
        let Some((min, mag, levels, texfmt)) = binding_filters(c, units[binding as usize] as usize, gl_type) else {
            continue;
        };
        if s.ret32 & bit != 0 && texfmt == GLES_TEXFMT_R32F {
            k.ret32 |= bit;
        }
        if s.cube_noseam & bit != 0 && binding_type(gl_type) == GL_SAMPLER_CUBE_T && cube_filters_see_seams(min, mag) {
            k.cube_noseam |= bit;
        }
        let mipmapped = levels
            && matches!(
                min,
                GL_NEAREST_MIPMAP_NEAREST
                    | GL_LINEAR_MIPMAP_NEAREST
                    | GL_NEAREST_MIPMAP_LINEAR
                    | GL_LINEAR_MIPMAP_LINEAR
            );
        if s.mip_free & bit != 0 && !mipmapped {
            k.mip_free |= bit;
        }
        if s.mag_switch & bit != 0
            && mag == GL_LINEAR
            && matches!(min, GL_NEAREST_MIPMAP_NEAREST | GL_NEAREST_MIPMAP_LINEAR)
        {
            k.mag_switch |= bit;
        }
    }
    k.bits()
}

/// The draw's sampler-state key, [vertex, fragment]; the program's
/// sensitivity is asked of the back end the first time (after a link, the
/// cache is new).
fn draw_lod_key(c: &GlContext, pg: &mut ProgGpu, p: &GlProgram, d: &GlesDraw) -> [u64; 2] {
    let ask = |sh: &ShcShader| match shaderc::backend().and_then(|b| b.lod_sensitivity) {
        // SAFETY: a linked program's shader, valid for the call.
        Some(f) => unsafe { f(sh) },
        None => 0,
    };
    if !pg.lod_sense_known {
        let sense = |sh: &ShcShader| if sh.sampler_bindings().next().is_some() { ask(sh) } else { 0 };
        /* the sample-kill bit is not a binding's: `kill_sense` keeps it */
        pg.lod_sense = [sense(&p.rvs), sense(&p.rfs) & !shaderc::LOD_KEY_SAMPLE_KILL];
        pg.lod_sense_known = true;
    }
    let kill = if needs_sample_kill(d, &p.rfs) {
        if pg.kill_sense == 0 {
            pg.kill_sense = if ask(&p.rfs) & shaderc::LOD_KEY_SAMPLE_KILL != 0 { 2 } else { 1 };
        }
        if pg.kill_sense == 2 {
            shaderc::LOD_KEY_SAMPLE_KILL
        } else {
            0
        }
    } else {
        0
    };
    if pg.lod_sense == [0, 0] {
        return [0, kill];
    }
    [
        stage_lod_key(c, pg.lod_sense[0], &p.rvs, &p.vs_sampler_unit),
        stage_lod_key(c, pg.lod_sense[1], &p.rfs, &p.fs_sampler_unit) | kill,
    ]
}

/// Does draw `d` need its fragment shader compiled for the sample state
/// (`shaderc::LodKey::sample_kill`)? On a multisampled target, when the
/// sample mask leaves a sample out (glSampleCoverage) or alpha to coverage
/// is on, and the shader does not already carry the epilogue that applies
/// them (a shader that discards does).
fn needs_sample_kill(d: &GlesDraw, fs: &ShcShader) -> bool {
    let full = ((1u32 << d.samples.min(8)) - 1) as u8;
    d.samples > 1 && (d.sample_mask & full != full || d.alpha_to_coverage) && fs.uses_discard == 0
}

/// Empty variant slot `i`: its uploads freed, its shaders released, and the
/// blend variants built on its fragment shader dropped (unless another slot
/// holds the same fragment key).
fn free_lod_variant(pg: &mut ProgGpu, i: usize) {
    let v = &mut pg.lod_var[i];
    if !v.used {
        return;
    }
    let fs_key = v.key[1];
    for m in [&mut v.vs_usc, &mut v.fs_usc] {
        if !m.impl_.is_null() {
            osal::gpu_free(m);
        }
    }
    shaderc::release_shader(&v.vs);
    shaderc::release_shader(&v.fs);
    // SAFETY: every field of ProgLodVariant is valid when all-zero.
    unsafe { core::ptr::write_bytes(v as *mut ProgLodVariant, 0, 1) };
    let shared = pg.lod_var.iter().any(|o| o.used && o.key[1] == fs_key);
    if fs_key != 0 && !shared {
        for b in pg.fs_var.iter_mut().filter(|b| b.blend_key != 0 && b.lod_key == fs_key) {
            if !b.usc.impl_.is_null() {
                osal::gpu_free(&mut b.usc);
            }
            shaderc::release_shader(&b.sh);
            // SAFETY: every field of ProgFsVariant is valid when all-zero.
            unsafe { core::ptr::write_bytes(b as *mut ProgFsVariant, 0, 1) };
        }
    }
}

/// Is the variant's fragment shader a program of its own (linked to the
/// variant's vertex shader, its code uploaded)? When the fragment key is set
/// it is; on the V3D the program holds both stages' code, so a vertex key
/// (`cube_noseam`, `mag_switch`) rebuilds it too.
fn fs_rebuilt(key: [u64; 2]) -> bool {
    key[1] != 0 || (cfg!(feature = "gpu-v3d") && key[0] != 0)
}

/// The variant slot for `key` (not [0, 0]): found, or compiled into a free
/// or the least recently used slot. `None` when it cannot be had -- no
/// such back end entry, a compile, link or upload failure: the draw then
/// takes the program's own shaders, which is what every draw did before
/// (the TPU's quad level, or the hardware's switch at 0), never a refusal.
/// A failure is remembered in the slot, so it is not retried every draw,
/// except a compile refused for want of room ([`VARIANT_NO_ROOM`]), which
/// frees the slot so a later draw asks again.
fn lod_variant_for(pg: &mut ProgGpu, rvs: &ShcShader, rfs: &ShcShader, key: [u64; 2]) -> Option<usize> {
    pg.lod_clock = pg.lod_clock.wrapping_add(1);
    let now = pg.lod_clock;
    if let Some(i) = pg.lod_var.iter().position(|v| v.used && v.key == key) {
        pg.lod_var[i].stamp = now;
        return (!pg.lod_var[i].missing).then_some(i);
    }
    let slot = pg.lod_var.iter().position(|v| !v.used).unwrap_or_else(|| {
        (0..PROG_MAX_LOD_VARIANTS).max_by_key(|&i| now.wrapping_sub(pg.lod_var[i].stamp)).unwrap_or(0)
    });
    free_lod_variant(pg, slot);
    let v = &mut pg.lod_var[slot];
    v.used = true;
    v.key = key;
    v.stamp = now;
    let mut log = [0 as core::ffi::c_char; 192];
    let fail = |v: &mut ProgLodVariant, why: &str, log: &[core::ffi::c_char], rc: i32| -> Option<usize> {
        if !LOD_VARIANT_LOGGED.swap(true, Ordering::Relaxed) {
            glog!(
                "yakogl: no sampler-state variant for key {:#x}/{:#x} ({}: {}); its draws take the program's own shaders",
                v.key[0],
                v.key[1],
                why,
                log_text(log)
            );
        }
        for m in [&mut v.vs_usc, &mut v.fs_usc] {
            if !m.impl_.is_null() {
                osal::gpu_free(m);
            }
        }
        shaderc::release_shader(&v.vs);
        shaderc::release_shader(&v.fs);
        v.vs = ShcShader::ZERO;
        v.fs = ShcShader::ZERO;
        if rc == VARIANT_NO_ROOM {
            /* not a property of the key: the slot is free again */
            // SAFETY: every field of ProgLodVariant is valid when all-zero.
            unsafe { core::ptr::write_bytes(v as *mut ProgLodVariant, 0, 1) };
        } else {
            v.missing = true;
        }
        None
    };
    let Some(variant) = shaderc::backend().and_then(|b| b.sampler_variant) else {
        return fail(v, "the shader back end has none", &log, -util::ENOENT);
    };
    // SAFETY: the program's shaders and the slot's outputs, all valid; the log is as long as said.
    let rc = unsafe { variant(rvs, key[0], &mut v.vs, log.as_mut_ptr(), log.len()) };
    if rc != 0 {
        return fail(v, "the vertex shader does not compile", &log, rc);
    }
    // SAFETY: as above.
    let rc = unsafe { variant(rfs, key[1], &mut v.fs, log.as_mut_ptr(), log.len()) };
    if rc != 0 {
        return fail(v, "the fragment shader does not compile", &log, rc);
    }
    /* A fragment variant is compiled from the source, with its own varying
     * packing: linked to the vertex shader as glLinkProgram linked the
     * program's (fs_variant_for, the same reason). Key 0 is the program's
     * own, linked already. */
    if fs_rebuilt(key) {
        if let Some(link) = shaderc::backend().and_then(|b| b.link) {
            // SAFETY: two distinct valid shaders; the log is as long as said.
            let rc = unsafe { link(&v.vs, &mut v.fs, log.as_mut_ptr(), log.len()) };
            if rc != 0 {
                return fail(v, "the fragment shader does not link", &log, rc);
            }
        }
    }
    let mut uploaded = true;
    for (k, sh, m) in [(key[0] != 0, &v.vs, &mut v.vs_usc), (fs_rebuilt(key), &v.fs, &mut v.fs_usc)] {
        if !k || sh.usc_dw == 0 || sh.usc.is_null() {
            continue;
        }
        if osal::gpu_alloc(backend::heap_base(GLES_HEAP_SHADER), sh.usc_dw as usize * 4, OSAL_MEM_DATA, m) != 0
            || put(m, 0, dword_bytes(sh.usc_words())) != 0
        {
            uploaded = false;
            break;
        }
    }
    if !uploaded {
        return fail(v, "out of shader memory", &[0], -util::ENOMEM);
    }
    Some(slot)
}

/// Ensure the program's USC + uniform GPU buffers exist and its USC is uploaded.
fn ensure_prog_gpu(pg: &mut ProgGpu, rvs: &ShcShader, rfs: &ShcShader) -> i32 {
    if pg.valid {
        return 0;
    }
    let rc = upload_prog_gpu(pg, rvs, rfs);
    if rc != 0 {
        /* all or nothing: a partial upload would be allocated again by the
         * next attempt and the first blocks leaked */
        for m in [&mut pg.vs_usc, &mut pg.fs_usc, &mut pg.uniforms] {
            if !m.impl_.is_null() {
                osal::gpu_free(m);
            }
            *m = OsalGpuMem::ZERO;
        }
    }
    rc
}

fn upload_prog_gpu(pg: &mut ProgGpu, rvs: &ShcShader, rfs: &ShcShader) -> i32 {
    if rvs.usc_dw != 0 && !rvs.usc.is_null() {
        let bytes = rvs.usc_dw as usize * 4;
        if osal::gpu_alloc(backend::heap_base(GLES_HEAP_SHADER), bytes, OSAL_MEM_DATA, &mut pg.vs_usc) != 0 {
            return -1;
        }
        if put(&mut pg.vs_usc, 0, dword_bytes(rvs.usc_words())) != 0 {
            return -1;
        }
    }
    if rfs.usc_dw != 0 && !rfs.usc.is_null() {
        let bytes = rfs.usc_dw as usize * 4;
        if osal::gpu_alloc(backend::heap_base(GLES_HEAP_SHADER), bytes, OSAL_MEM_DATA, &mut pg.fs_usc) != 0 {
            return -1;
        }
        if put(&mut pg.fs_usc, 0, dword_bytes(rfs.usc_words())) != 0 {
            return -1;
        }
    }
    /* Uniform blocks: the VS block then the FS block, 16-byte aligned
     * (reused every draw). */
    let bytes = MAX_UNIFORM_DWORDS * 4 * 2 + 16;
    if osal::gpu_alloc(backend::heap_base(GLES_HEAP_DATA), bytes, OSAL_MEM_DATA, &mut pg.uniforms) != 0 {
        return -1;
    }

    pg.valid = true;
    0
}

/* --------------------------------------------------------------------------- */

/// A GL error to record, with the name of the C function that raised it (the
/// error log names it). Helpers that run while parts of the context are
/// borrowed return it; the caller records it once the borrows have ended.
type DrawError = (GLenum, &'static str);

/// The bound framebuffer's colour buffer, or none (`target_gpu` 0, the
/// depth/stencil size) for a complete framebuffer with no colour attachment
/// where the backend draws without one (framebuffer::
/// fb_depth_stencil_only_size). Fails (GL_INVALID_FRAMEBUFFER_OPERATION)
/// when the bound framebuffer is not complete.
fn resolve_target(c: &mut GlContext, d: &mut GlesDraw) -> Result<(), DrawError> {
    /* a texture it renders into holds the pixel engine's channel order
     * from now on (texture::take_pe_order) */
    framebuffer::take_pe_order_targets(c);
    match framebuffer::fb_target_pitched(c) {
        Some((m, w, h, off, pitch)) if w != 0 && h != 0 => {
            d.target_gpu = m.gpu.wrapping_add(off as u64); /* the attached level/face/layer */
            d.target_w = w;
            d.target_h = h;
            /* the default surface may be a view into a wider buffer, and a
             * level of a PowerVR NPOT chain has its power-of-two pitch */
            d.target_pitch = if c.bound_framebuffer == 0 { c.default_pitch } else { pitch };
            Ok(())
        }
        _ => match framebuffer::fb_depth_stencil_only_size(c) {
            Some((w, h)) => {
                (d.target_gpu, d.target_w, d.target_h, d.target_pitch) = (0, w, h, 0);
                Ok(())
            }
            None => Err((GL_INVALID_FRAMEBUFFER_OPERATION, "resolve_target")),
        },
    }
}

/// Bring-up: a small fragment block of a large shader, in full. A loop whose
/// bound is a uniform (Quick 3D's environment prefilter runs
/// "i < ubuf2.sampleCount") turns a wrong dword into a kick that never ends.
fn dump_small_fs_block(fs_usc_dw: u32, p: &GlProgram, fs_dw: u16) {
    // SAFETY: single-threaded by contract; taken once here.
    let st = unsafe { STATE.get() };

    if !st.fsdump_started {
        st.fsdump_started = true;
        st.fsdump_left = FS_DUMP_BUDGET;
    }
    if st.fsdump_left == 0 || fs_dw > 8 || fs_usc_dw <= 900 {
        return;
    }
    st.fsdump_left -= 1;
    for (i, v) in p.fs_block().iter().take(fs_dw as usize).enumerate() {
        let u = v.to_bits();

        glog!("  fs uniform[{}] = 0x{:08x}  int {}  float {}", i, u, u as i32, v);
    }
}

/// Blocks `dump_small_fs_block` prints per run.
const FS_DUMP_BUDGET: u32 = 24;

/// The window surface may be XRGB8888 or RGB565 (scan-out); FBO textures are
/// RGBA8888 or one of the float formats.
fn target_format(c: &GlContext) -> u8 {
    framebuffer::fb_target_format(c)
}

/// Fill both halves of what the target's colour format is: the surface layout
/// the backend writes, and how many alpha bits the GL format behind it has.
fn set_target_format(c: &GlContext, d: &mut GlesDraw) {
    d.target_format = target_format(c);
    d.target_alpha_bits = framebuffer::fb_target_alpha_bits(c);
}

/// The draw's sample mask and alpha-to-coverage flag, from the context's
/// coverage state and the target's sample count (`d.samples`, set first):
/// Mesa's state tracker rule (st_atom_msaa.c st_update_sample_state, Mesa
/// 26.2.1): with more than one sample and GL_SAMPLE_COVERAGE on, the first
/// `(unsigned)(value * samples)` samples, inverted with the invert flag. A
/// single-sampled target writes every sample and ignores both (ES 2.0.25
/// 4.1.3: "No changes to the fragment alpha or coverage values are made at
/// this step if the value of SAMPLE_BUFFERS is not one").
fn set_sample_state(c: &GlContext, d: &mut GlesDraw) {
    (d.sample_mask, d.alpha_to_coverage) = (0xf, false);
    if d.samples <= 1 {
        return;
    }
    if c.sample_coverage {
        d.sample_mask = sample_coverage_mask(c, d.samples) & 0xf;
    }
    d.alpha_to_coverage = c.sample_alpha_to_coverage;
}

/// Integer surfaces already reported as refused (bit `format - GLES_SURF_INT_FIRST`):
/// one log line per format, not one per draw.
static REFUSED_INT_RT: Global<u32> = Global::new(0);

/// A colour target the back end cannot render into is refused by name before
/// the draw (or clear) reaches it: an integer surface it has not learned
/// ([`backend::INTEGER_RT_FORMATS`]). The framebuffer is complete and the
/// storage is there (ES 3.0 requires the formats); drawing garbage into it is
/// worse than saying no, so the draw is dropped with GL_INVALID_OPERATION.
/// `mask` is the clear's target mask (0: every target the draw writes).
fn refuse_undrawable_targets(d: &GlesDraw, mask: u8) -> Result<(), DrawError> {
    for (i, t) in d.targets[..d.num_targets as usize].iter().enumerate() {
        if t.gpu == 0 || (mask != 0 && mask & (1u8 << i.min(7)) == 0) || backend::rt_format_drawable(t.format) {
            continue;
        }
        // SAFETY: single-threaded by contract; no other reference is live.
        let reported = unsafe { REFUSED_INT_RT.get() };
        let bit = 1u32 << (t.format - GLES_SURF_INT_FIRST);
        if *reported & bit == 0 {
            *reported |= bit;
            glog!(
                "draw refused: this back end cannot render into a {} colour attachment (draw buffer {})",
                gles_surf_name(t.format),
                i
            );
        }
        return Err((GL_INVALID_OPERATION, "refuse_undrawable_targets"));
    }
    Ok(())
}

/// The draw's colour targets, built from the glDrawBuffers mapping (ES 3.0
/// 4.2.1): slot i is draw buffer i, which writes attachment i or nothing.
/// Each written slot carries its attachment's memory, size and format.
/// A slot with no target (a GL_NONE draw buffer, or no attachment) is kept
/// as `gpu` 0 and the walk goes on: glDrawBuffers(ATT0, NONE, ATT2) draws
/// into attachment 2 at slot 2. `num_targets` is the highest slot with a
/// target, plus one. A backend must skip a `gpu` 0 target: it takes the
/// fragment output of that slot and stores it nowhere.
///
/// Slot 0 is attachment 0, which is also the frame's target
/// (`target_gpu`, w, h, format). glDrawBuffers({GL_NONE, ATT1}) gives it
/// `gpu` 0 with the frame's size and format, and a backend that always
/// stores slot 0 refuses such a draw by name (review R-09: output 0 went to
/// attachment 0 whatever draw buffer 0 said). When no slot writes at all the
/// colour mask is zero instead ([`framebuffer::colour_writes_off`]) and slot
/// 0 keeps its memory.
fn resolve_targets(c: &GlContext, d: &mut GlesDraw) {
    d.num_targets = 1;
    d.targets[0] = GlesDrawTarget { gpu: d.target_gpu, w: d.target_w, h: d.target_h, format: d.target_format };
    if framebuffer::draw_buffer0_none(c) && !framebuffer::colour_writes_off(c) {
        d.targets[0].gpu = 0;
    }
    for i in 1..GLES_MAX_TARGETS as u8 {
        d.targets[i as usize] = GlesDrawTarget::ZERO;
        if let Some((gpu, w, h, format)) = framebuffer::fb_target_n(c, i) {
            d.targets[i as usize] = GlesDrawTarget { gpu, w, h, format };
            d.num_targets = i + 1;
        }
    }
}

/* ---- glClear, with the scissor box and the write masks ---------------------
 *
 * GL applies the same state to a clear that it applies to a draw: the colour
 * mask, the depth mask and the stencil write mask each say what the clear may
 * change, and the scissor box says where. The backend's background object can
 * do none of that -- it covers the whole target and writes every channel -- so
 * a clear that needs any of it goes through `GlesDraw::clear_quad`, a
 * full-screen quad the backend draws in sequence with the frame's other
 * objects.
 *
 * Three of the four are free: a buffer whose mask is entirely off is simply
 * not cleared, and the quad carries the scissor and the stencil write mask.
 * The fourth, a colour mask with some channels on and some off, travels in
 * `GlesDraw::clear_color_mask` and is the backend's to apply. The GC7000
 * writes it into the pixel engine's component mask, as it does for a draw
 * (measured on the i.MX8MP: gl_test 102). The
 * PowerVR has no per-channel write mask (Mesa's pvr lowers one into the
 * fragment shader too, docs/gpu-recipes/blending_recipe.md): it draws the
 * clear with the masked clear program for the mask (`pvr::backend`,
 * `CLEAR_MASKED`), and turns a partially masked background clear -- alone or
 * fused into a draw -- into that quad before the draw (behind
 * `pvr_debug_masked_clear`, likewise until the board has measured it). */
#[derive(Default)]
struct ClearPlan {
    /// The buffers this clear still changes.
    color: bool,
    /// The channels the colour clear may write (glColorMask at the glClear).
    color_mask: u8,
    depth: bool,
    stencil: bool,
    stencil_write_mask: u8,
    /// The background object cannot do it: draw a quad instead.
    quad: bool,
    scissor: bool,
    scissor_rect: [GLint; 4],
}

/// The colour channels glColorMask lets a clear write (bit 0 = R .. 3 = A).
fn clear_color_mask(c: &GlContext) -> u8 {
    if framebuffer::colour_writes_off(c) {
        return 0; /* no draw buffer: the colour buffer is not cleared */
    }
    let mut m = 0u8;
    for (i, v) in c.color_writemask.iter().enumerate() {
        if *v != 0.0 {
            m |= 1u8 << i;
        }
    }
    m
}

/// A partially masked colour clear has been reported once.
static PARTIAL_COLOR_MASK_WARNED: Global<bool> = Global::zeroed();

/// What the pending clear still does against a target of `w` x `h`, or `None`
/// when it does nothing at all.
fn plan_clear(
    pc: &PendingClear,
    (w, h, has_color): (u32, u32, bool),
    has_depth: bool,
    has_stencil: bool,
) -> Option<ClearPlan> {
    /* every one of these is the state at the glClear, not the state now:
     * see `PendingClear::state` */
    let cmask = pc.state.color_mask;
    let mut cp = ClearPlan { stencil_write_mask: pc.state.stencil_write_mask, ..Default::default() };

    /* A buffer the framebuffer does not have is not cleared: ES 2.0 4.2.3
     * clears "the buffers currently enabled for color, depth and stencil
     * writing", and a framebuffer object has each only where something is
     * attached. The target keeps a depth/stencil of its own regardless, so
     * without this a glClear(GL_DEPTH_BUFFER_BIT) on a colour-only FBO wrote
     * a buffer GL says is not there -- and emitted a kick to do it. */
    cp.color = pc.color && cmask != 0 && has_color;
    cp.color_mask = if cp.color { cmask } else { 0 };
    cp.depth = pc.depth && pc.state.depth_mask && has_depth;
    cp.stencil = pc.stencil && cp.stencil_write_mask != 0 && has_stencil;
    if cp.color && cmask != 0xf && !crate::backend::masked_clear_applied() {
        // SAFETY: single-threaded by contract; no other reference is live.
        let warned = unsafe { PARTIAL_COLOR_MASK_WARNED.get() };
        if !*warned {
            *warned = true;
            glog!(
                "glClear: a colour mask with only some channels on is not applied to the clear \
                 (no masked clear shader); every channel is cleared"
            );
        }
    }
    if !cp.color && !cp.depth && !cp.stencil {
        return None;
    }
    if let Some(box_) = pc.state.scissor {
        let (x0, y0) = (box_[0], box_[1]);
        if box_[2] <= 0 || box_[3] <= 0 {
            return None; /* an empty box clears nothing */
        }
        let (x1, y1) = (x0.saturating_add(box_[2]), y0.saturating_add(box_[3]));
        /* a box that holds the whole target is the same as no scissor, and
         * the background object is the cheaper way to do it */
        if x0 > 0 || y0 > 0 || x1 < w as GLint || y1 < h as GLint {
            cp.quad = true;
            cp.scissor = true;
            cp.scissor_rect = [x0, y0, box_[2], box_[3]];
        }
    }
    if cp.stencil && cp.stencil_write_mask != 0xff {
        cp.quad = true; /* only some stencil bits may change */
    }
    Some(cp)
}

/// Fill the clear fields of a draw from the plan (and clear the pending one).
fn apply_clear_plan(d: &mut GlesDraw, cp: &ClearPlan, pc: &mut PendingClear) {
    d.do_clear = cp.color;
    d.clear_color_mask = cp.color_mask;
    d.clear_target_mask = if cp.color { pc.state.target_mask } else { 0 };
    d.clear_rgba = if cp.color { pc.rgba } else { [0.0; 4] };
    /* A clear writes a value the target's format can hold. Only alpha is
     * rounded here: the colour channels of a narrow format (RGB565, RGBA4)
     * differ from the full-precision value by less than a step, which is
     * inside every tolerance, but alpha is 1 bit in RGB5_A1 and absent in
     * GL_RGB / GL_RGB565, where GL says the buffer reads 1 -- a whole-image
     * difference, not a rounding one. */
    /* (an integer target's clear is raw integer bits, not a colour; the alpha
     * bits are attachment 0's, so a clear of another draw buffer keeps its own) */
    if !gles_surf_is_integer(d.target_format) && (pc.state.target_mask == 0 || pc.state.target_mask & 1 != 0) {
        d.clear_rgba[3] = framebuffer::quantise_unorm(d.clear_rgba[3], d.target_alpha_bits);
    }
    d.do_clear_depth = cp.depth;
    d.clear_depth = if cp.depth { pc.depth_value } else { 1.0 };
    d.do_clear_stencil = cp.stencil;
    d.clear_stencil = if cp.stencil { pc.stencil_value } else { 0 };
    d.clear_stencil_write_mask = if cp.stencil { cp.stencil_write_mask } else { 0 };
    pc.color = false;
    pc.depth = false;
    pc.stencil = false;
}

/// Move the pending depth/stencil clears (glClear bits) into the draw.
fn take_pending_ds_clears(d: &mut GlesDraw, pc: &mut PendingClear, (has_depth, has_stencil): (bool, bool)) {
    d.do_clear_depth = pc.depth && has_depth;
    d.clear_depth = if d.do_clear_depth { pc.depth_value } else { 1.0 };
    pc.depth = false;
    d.do_clear_stencil = pc.stencil && has_stencil;
    d.clear_stencil = if d.do_clear_stencil { pc.stencil_value } else { 0 };
    pc.stencil = false;
}

/// Which depth/stencil buffer the target keeps across kicks: each window
/// surface owns one (identified by its buffer's page number, bit 31 set so it
/// never collides with an FBO id: an application double-buffering through two
/// window surfaces must not share one depth/stencil between them); an FBO has
/// one when a depth or stencil renderbuffer is attached (named by those
/// renderbuffers, so FBOs sharing them share it).
fn resolve_ds_storage(c: &GlContext, d: &mut GlesDraw) {
    d.ds_persist = false;
    d.ds_id = 0;
    (d.depth_id, d.depth_gen, d.stencil_id, d.stencil_gen, d.ds_w, d.ds_h) = (0, 0, 0, 0, 0, 0);
    if c.bound_framebuffer == 0 {
        d.ds_persist = true;
        d.ds_id = 0x8000_0000 | (d.target_gpu >> 12) as u32;
        /* the window surface's own D24S8: one object for both */
        (d.depth_id, d.stencil_id) = (d.ds_id, d.ds_id);
        (d.ds_w, d.ds_h) = (d.target_w, d.target_h);
        return;
    }
    if let Some(fb) = c.framebuffer_ref(c.bound_framebuffer) {
        let depth = ds_identity(c, &fb.depth);
        let stencil = ds_identity(c, &fb.stencil);
        (d.depth_id, d.depth_gen) = (depth.0, depth.1);
        (d.stencil_id, d.stencil_gen) = (stencil.0, stencil.1);
        let size = if depth.0 != 0 { (depth.2, depth.3) } else { (stencil.2, stencil.3) };
        (d.ds_w, d.ds_h) = size;
    }
    let (has_depth, has_stencil) = framebuffer::fb_target_depth_stencil(c);
    if let Some(fb) = c.framebuffer_ref(c.bound_framebuffer) {
        if has_depth || has_stencil {
            d.ds_persist = true;
            d.ds_id = ds_pair_id(&fb.depth, &fb.stencil);
        }
        /* a depth texture's depth reaches its texels only when it is next
         * read (texture::resolve_depth), from this stamp */
        if has_depth {
            framebuffer::stamp_depth_target(c, &fb.depth);
        }
    }
}

/// `GlesDraw::ds_id` of a framebuffer object: the depth attachment's half in
/// bits 30..16, the stencil's in bits 14..0, each the object's name with
/// bit 14 set for a texture. Names are slot indices (below MAX_TEXTURES and
/// MAX_RENDERBUFFERS), so a half never reaches its type bit, and bit 31 stays
/// the window surface's. Without the type bit a depth-stencil texture and a
/// D24S8 renderbuffer of the same name number, each on both points of its
/// own framebuffer, had one id, and the PowerVR kept both in one packed
/// buffer (DS_PACKED is keyed by this id).
fn ds_pair_id(depth: &super::context::GlAttachment, stencil: &super::context::GlAttachment) -> u32 {
    const _: () = assert!(super::context::MAX_TEXTURES <= 0x4000 && super::context::MAX_RENDERBUFFERS <= 0x4000);
    let half = |a: &super::context::GlAttachment| {
        (a.name & 0x3fff) | if a.type_ == GL_TEXTURE && a.name != 0 { 0x4000 } else { 0 }
    };
    (half(depth) << 16) | half(stencil)
}

/// (id, gen, w, h) of the object at a depth or stencil attachment point
/// (`GlesDraw::depth_id`): its serial, bit 30 for a texture; 0 for none.
fn ds_identity(c: &GlContext, a: &super::context::GlAttachment) -> (u32, u32, u32, u32) {
    match a.type_ {
        GL_RENDERBUFFER => c.renderbuffer_ref(a.name).map_or((0, 0, 0, 0), |rb| {
            (super::context::ds_object_id(rb.serial, false), rb.storage_gen, rb.width, rb.height)
        }),
        GL_TEXTURE => c.texture_ref(a.name).map_or((0, 0, 0, 0), |t| {
            let (w, h) = ((t.width >> a.level).max(1), (t.height >> a.level).max(1));
            (super::context::ds_object_id(t.serial, true), t.storage_gen, w, h)
        }),
        _ => (0, 0, 0, 0),
    }
}

/// Emit a clear-only kick for the pending clear, to the framebuffer bound
/// now. `with_ds`: also for a depth/stencil-only clear (which otherwise waits
/// for the next draw to that target). `quiet`: a target that cannot be
/// resolved drops the clear without raising GL_INVALID_FRAMEBUFFER_OPERATION.
fn emit_pending_clear(c: &mut GlContext, d: &mut GlesDraw, reported: &mut u32, with_ds: bool, quiet: bool) {
    let _stamps = DrawStampScope::open();
    let pc = pending_clear_of(c);
    if !pc.color && !(with_ds && (pc.depth || pc.stencil)) {
        return;
    }
    if osal::ops().is_none() {
        pc.color = false;
        return;
    }

    *d = GlesDraw::ZERO;
    if let Err((e, func)) = resolve_target(c, d) {
        if !quiet {
            c.set_error(e, func);
        }
        pc.color = false;
        if with_ds {
            pc.depth = false;
            pc.stencil = false;
        }
        return;
    }
    set_target_format(c, d);
    d.samples = if c.bound_framebuffer == 0 { c.default_samples } else { 1 };
    set_sample_state(c, d);
    resolve_targets(c, d); /* a clear opens the frame: it decides its attachments */
    if pc.color {
        if let Err((e, func)) = refuse_undrawable_targets(d, pc.state.target_mask) {
            if !quiet {
                c.set_error(e, func);
            }
            pc.color = false;
            if !(with_ds && (pc.depth || pc.stencil)) {
                return;
            }
        }
    }
    /* The write masks and the scissor box apply to a clear as they do to a
     * draw: a buffer whose mask is entirely off is not cleared, an empty
     * scissor box clears nothing, and anything narrower than the target goes
     * through the quad rather than the background object. */
    let (has_depth, has_stencil) = framebuffer::fb_target_depth_stencil(c);
    let Some(cp) = plan_clear(pc, (d.target_w, d.target_h, d.target_gpu != 0), has_depth, has_stencil) else {
        pc.color = false;
        if with_ds {
            pc.depth = false;
            pc.stencil = false;
        }
        return;
    };
    let color = cp.color;
    apply_clear_plan(d, &cp, pc);
    d.clear_quad = cp.quad;
    d.scissor = cp.scissor;
    d.scissor_rect = cp.scissor_rect;
    d.stencil_write_mask = cp.stencil_write_mask;
    d.stencil_back_write_mask = cp.stencil_write_mask;
    resolve_ds_storage(c, d);
    /* A depth- or stencil-only clear must leave the colour buffer alone, so
     * the mask follows the colour clear rather than being opened up. */
    d.color_write = color;
    d.color_mask = if color { 0xf } else { 0x0 };
    d.flip_y = framebuffer::fb_flip_y(c);
    d.prim = GLES_PRIM_TRIANGLES;
    d.index_count = 0; /* clear-only kick, no geometry */
    d.viewport = c.viewport;
    /* The depth range matters here as much as the viewport: a backend that
     * clears with a quad puts it through the same viewport transform as
     * geometry, and a zeroed range is not "the default", it is a range of no
     * width. Left unset, the Vivante backend derived PA_VIEWPORT_SCALE_Z =
     * far - near = 0, which collapses every z to the offset, so a clear with
     * no draw behind it wrote depth 0 whatever value it was given. */
    d.depth_range = c.depth_range;

    submit_draw(c, reported, d, "the pending clear", None);
}

/// A glClear applies to the framebuffer bound when it was called. A clear
/// still pending when the application has bound another framebuffer since
/// is emitted to its own target now (a clear-only kick, depth/stencil-only
/// clears included), so that the next draw neither fuses it into the wrong
/// target nor loses it. The binding is switched for the emission and put
/// back. A framebuffer deleted in between takes its clear with it (silently:
/// there is no object left to report the error against; a target that is
/// merely incomplete still raises GL_INVALID_FRAMEBUFFER_OPERATION, as it
/// did when the clear was fused into the next draw).
/// (Within `c`: each context has its own pending clear.)
fn settle_pending_clear(c: &mut GlContext, d: &mut GlesDraw, reported: &mut u32) {
    let pc = pending_clear_of(c);
    if !(pc.color || pc.depth || pc.stencil) || pc.fb == c.bound_framebuffer {
        return;
    }
    let now = c.bound_framebuffer;
    let quiet = pc.fb != 0 && c.framebuffer_ref(pc.fb).is_none();
    c.bound_framebuffer = pc.fb;
    emit_pending_clear(c, d, reported, true, quiet);
    c.bound_framebuffer = now;
    if quiet {
        /* the framebuffer is gone: so is its clear */
        pc.color = false;
        pc.depth = false;
        pc.stencil = false;
    }
    pc.fb = now;
}

/// The masks and scissor box a `glClear` issued now would be subject to.
fn clear_state(c: &GlContext) -> ClearState {
    ClearState {
        color_mask: clear_color_mask(c),
        depth_mask: c.depth_mask,
        stencil_write_mask: c.stencil_writemask as u8,
        scissor: c.scissor_test.then_some(c.scissor),
        target_mask: clear_target_mask(c),
    }
}

/// The alignment a u16 index buffer's address needs to be read in place.
/// The PowerVR's VDM takes it 4-byte aligned (VDMCTRL_INDEX_LIST1
/// index_base_addrlsb, driver/docs/gpu-recipes/multidraw_stencil_recipe.md;
/// pvr_vdm_emit_index_list refuses a misaligned one instead of masking it),
/// so an element-buffer offset of 2 must not reach it unstaged: the old
/// masked form silently read two bytes earlier, pairing every index's wrong
/// halves (dEQP-GLES2 draw.draw_elements.indices.buffer.index_short, its
/// "index offset 2" iteration). Such a draw stages the indices instead.
/// The other GPUs keep reading in place at any 2-byte offset.
#[cfg(feature = "gpu-powervr")]
const INDEX_ADDR_ALIGN: u64 = 4;
#[cfg(not(feature = "gpu-powervr"))]
const INDEX_ADDR_ALIGN: u64 = 1;

/// The colour targets a `glClear` issued now touches (`ClearState::
/// target_mask`): the one draw buffer of a glClearBuffer; for glClear, the
/// draw buffers that are not GL_NONE when glDrawBuffers left any out (0,
/// every target, otherwise). All of them GL_NONE is bit 7, a target no
/// framebuffer has: the colour clear touches nothing.
fn clear_target_mask(c: &GlContext) -> u8 {
    // SAFETY: single-threaded by contract; no other reference is live.
    let explicit = unsafe { *CLEAR_TARGET_MASK.get() };
    if explicit != 0 || c.bound_framebuffer == 0 {
        return explicit;
    }
    let Some(fb) = c.framebuffer_ref(c.bound_framebuffer) else { return 0 };
    let n = (fb.num_draw_buf as usize).min(fb.draw_buf.len());
    if n == 0 || fb.draw_buf[..n].iter().all(|&b| b >= 0) {
        return 0;
    }
    match fb.draw_buf[..n].iter().enumerate().filter(|(_, &b)| b >= 0).fold(0u8, |m, (i, _)| m | 1 << i.min(7)) {
        0 => 0x80,
        m => m,
    }
}

/// The colour targets the next `glClear` touches: 0 all, else one bit per
/// draw buffer. glClearBuffer sets it around its own glClear.
static CLEAR_TARGET_MASK: Global<u8> = Global::new(0);

/// glClearBuffer(GL_COLOR, `drawbuffer`): run `f` (its glClear) as a clear of
/// that draw buffer alone. The mask is part of the clear's state, so a
/// pending clear of other targets is emitted first rather than merged.
pub fn with_clear_target(drawbuffer: u32, f: impl FnOnce()) {
    // SAFETY: single-threaded by contract; no other reference is live.
    let m = unsafe { CLEAR_TARGET_MASK.get() };
    *m = 1u8 << drawbuffer.min(7);
    f();
    *m = 0;
}

/// A clear is carried out under the state in force when `glClear` was called
/// (`PendingClear::state`). So a `glClear` issued under *different* state is a
/// second clear, not more bits for the pending one: this emits the pending
/// clear first, with its own masks and box, and the caller then records the
/// new one under the new state.
///
/// Without this the single `PendingClear` slot silently dropped a clear --
/// `glClear(blue); glEnable(SCISSOR_TEST); glScissor(box); glClear(red)` is
/// two clears, a full blue one and a red box inside it, and the second used
/// to overwrite the first so the blue never happened.
fn settle_clear_state(c: &mut GlContext, d: &mut GlesDraw, reported: &mut u32) {
    {
        let pc = pending_clear_of(c);
        if !(pc.color || pc.depth || pc.stencil) || pc.state == clear_state(c) {
            return;
        }
    }
    emit_pending_clear(c, d, reported, true, false);
}

/// A pending clear the frame's background object cannot carry out -- one that
/// is scissored, or that must leave some stencil bits alone -- is drawn as its
/// own object BEFORE the draw that follows it, which is where GL put it. It
/// joins the frame rather than ending it, so a scissored clear in the middle
/// of a frame no longer forces a flush either.
fn settle_quad_clear(c: &mut GlContext, d: &mut GlesDraw, reported: &mut u32) {
    let needs_quad = {
        let (has_depth, has_stencil) = framebuffer::fb_target_depth_stencil(c);
        let pc = pending_clear_of(c);
        if !(pc.color || pc.depth || pc.stencil) {
            return;
        }
        /* the colour target's size, or a depth/stencil-only one's */
        let size = match framebuffer::fb_target(c) {
            Some((_, w, h, _)) if w != 0 && h != 0 => Some((w, h, true)),
            _ => framebuffer::fb_depth_stencil_only_size(c).map(|(w, h)| (w, h, false)),
        };
        match size {
            Some(size) => plan_clear(pc, size, has_depth, has_stencil).is_some_and(|cp| cp.quad),
            None => false, /* the target is the draw's problem to report */
        }
    };
    if needs_quad {
        emit_pending_clear(c, d, reported, true, false);
    }
}

/// glClear's view of the pending clear: settles a clear pending for another
/// framebuffer ([`settle_pending_clear`]) and records the framebuffer the
/// clear being added applies to. glClear sets the flags on the result.
pub fn pending_clear_for(c: &mut GlContext) -> &'static mut PendingClear {
    // SAFETY: single-threaded by contract; glClear, the only caller, holds
    // no other reference to STATE.
    let st = unsafe { STATE.get() };
    settle_pending_clear(c, &mut st.clear_draw, &mut st.draw_fail_reported);
    settle_clear_state(c, &mut st.clear_draw, &mut st.draw_fail_reported);
    let pc = pending_clear_of(c);
    if !(pc.color || pc.depth || pc.stencil) {
        pc.fb = c.bound_framebuffer;
        pc.state = clear_state(c);
    }
    pc
}

/// Emit a clear-only kick for a glClear with no following draw.
pub fn flush_pending_clear(c: &mut GlContext) {
    // SAFETY: single-threaded by contract; the draw entry points never call
    // this, so no other reference to STATE is live.
    let st = unsafe { STATE.get() };
    settle_pending_clear(c, &mut st.clear_draw, &mut st.draw_fail_reported);
    /* with_ds: a clear the application never drew behind still has to be
     * carried out here. `glClear(GL_DEPTH_BUFFER_BIT); glFinish();` is a
     * complete request, and gating this on the colour clear alone left it
     * pending, to be applied to whatever draw came next however much later. */
    emit_pending_clear(c, &mut st.clear_draw, &mut st.draw_fail_reported, true, false);
}

#[no_mangle]
pub unsafe extern "C" fn gl_flush_pending_clear() {
    if let Some(c) = context::current() {
        flush_pending_clear(c);
    }
}

/// `c.buffer(h)` over the table alone, so that the rest of the context stays
/// borrowable (build_common holds the program while it touches buffers).
fn live_buffer(buffers: &mut [GlBuffer; MAX_BUFFERS], h: GLuint) -> Option<&mut GlBuffer> {
    if h == 0 || h as usize >= MAX_BUFFERS {
        return None;
    }
    let b = &mut buffers[h as usize];
    b.used.then_some(b)
}

/// `c.texture(h)` over the table alone; see [`live_buffer`].
/// `h` is a slot: a name, or a default object's slot past MAX_TEXTURES.
fn live_texture(textures: &mut [GlTexture; TEXTURE_SLOTS], h: usize) -> Option<&mut GlTexture> {
    if h == 0 || h >= TEXTURE_SLOTS {
        return None;
    }
    let t = &mut textures[h];
    t.used.then_some(t)
}

/* ---- texture wrap and completeness -----------------------------------------
 *
 * GL ES 2.0 section 3.7.10: a texture that is not complete samples (0,0,0,1)
 * everywhere, whatever its texels hold. The rules are
 *   - a minification filter that uses mipmaps needs the whole chain;
 *   - a cube map needs all six faces, specified at one size;
 *   - and, in ES 2.0 without GL_OES_texture_npot, a texture whose width or
 *     height is not a power of two needs GL_CLAMP_TO_EDGE on both axes and a
 *     minification filter that does not use mipmaps.
 *
 * The third is ES 2.0's alone -- ES 3.0 supports NPOT textures with any wrap
 * mode and with mipmaps, and so does ES 2.0 with GL_OES_texture_npot, which
 * every build that reports ES 2.0 claims since 2026-10-07 -- so it applies
 * only where texture::NPOT_RESTRICTED holds, which no build does any more.
 * Where the backend keeps one level of a NPOT image (the PowerVR's stacked
 * NPOT cube maps, texture.rs npot_mip_chains) such a texture samples its
 * base level under a mipmap filter.
 *
 * This replaces demoting the filter and sampling level 0, which showed an
 * application's mistake as a picture rather than as black. */
fn draw_is_pot(v: u32) -> bool {
    v != 0 && v & (v - 1) == 0
}

fn min_filter_uses_mipmaps(f: GLenum) -> bool {
    matches!(
        f,
        GL_NEAREST_MIPMAP_NEAREST | GL_LINEAR_MIPMAP_NEAREST | GL_NEAREST_MIPMAP_LINEAR | GL_LINEAR_MIPMAP_LINEAR
    )
}

/// Why this texture is not complete, or `None` when it is.
///
/// The reason is carried rather than discarded because an incomplete texture
/// fails *silently*: GL ES 2.0 3.7.10 says every sample of it reads (0,0,0,1),
/// so the application sees black pixels, no GL error, and nothing pointing at
/// the texture. Saying which rule was broken, once, turns that into a one-line
/// answer. (It is what the i.MX8MP's case 20 needed, and it will be what the
/// next one needs.)
/// The levels a mipmap filter needs, from level 0 on. ES 3.0 3.8.13: the
/// base level up to q = min(p, GL_TEXTURE_MAX_LEVEL), p the last level of
/// the base image's chain; for an immutable texture no further than its last
/// level (3.8.4, "levels - 1" clamps the maximum level). ES 2.0 3.7.10 has no
/// GL_TEXTURE_MAX_LEVEL: the whole chain. The base level is level 0 here:
/// the draw does not apply GL_TEXTURE_BASE_LEVEL (`GlTexture::base_level`).
///
/// dEQP-GLES3 texture.specification samples every texture it makes with
/// NEAREST_MIPMAP_NEAREST and GL_TEXTURE_MAX_LEVEL at its last level; with
/// the whole chain required, every one-level texture (teximage2d_pbo,
/// texsubimage2d_pbo, the *_unpack_params and *_align groups) and every
/// short glTexStorage2D chain was incomplete and read (0,0,0,1).
fn mip_levels_needed(t: &GlTexture) -> u32 {
    let p = u32::from(super::context::gl_mip_levels(t.width, t.height).min(15));
    if crate::REPORT_ES2 {
        return p;
    }
    let mut q = p.min(u32::try_from(t.max_level_set).unwrap_or(0).saturating_add(1));
    if t.immutable && t.immutable_levels > 0 {
        q = q.min(u32::from(t.immutable_levels));
    }
    q.max(1)
}

fn incomplete_reason(t: &GlTexture) -> Option<&'static str> {
    let mips = min_filter_uses_mipmaps(t.min_filter);
    let needed = mip_levels_needed(t);

    if t.target == GL_TEXTURE_CUBE_MAP && t.faces_set != 0x3f {
        return Some("cube map without all six faces");
    }
    if t.level_defined & 1 == 0 || t.level_bad & 1 != 0 {
        return Some("level 0 has no image, or one with a zero side");
    }
    /* ES 3.0 3.8.13: an integer (or 32-bit float) texture is complete only
     * with nearest filtering */
    if let Some(e) = super::es3_formats::by_index(t.es3_fmt) {
        if e.incomplete_with(t.min_filter, t.mag_filter) {
            return Some("an integer or 32-bit float texture filtered by anything but NEAREST (ES 3.0 3.8.13)");
        }
    }
    if mips && u32::from(t.levels) < u32::from(t.chain_levels).min(needed) {
        return Some("a mipmap min filter, but fewer levels uploaded than the chain needs");
    }
    /* ES 2.0 3.7.10 / ES 3.0 3.8.13: with a mipmap filter every level of
     * the chain must have an image, of the chain's size and level 0's
     * format. `levels` is the highest level plus one and cannot see a hole
     * (dEQP texture.completeness.2d.missing_level_1) or a level of another
     * format (format_mismatch_*); the per-level record can. */
    if mips {
        /* GL's chain, not the store's: where the backend keeps one level of
         * a NPOT image (a PowerVR NPOT cube), GL still wants its whole chain, which
         * glGenerateMipmap, glTexStorage2D or the application's own levels
         * record as present without holding them */
        let chain = ((1u32 << needed) - 1) as u16;
        if t.level_defined & chain != chain {
            return Some("a mipmap min filter, but a level of the chain has no image");
        }
        if t.level_bad & chain != 0 {
            return Some("a mipmap min filter, but a level of the chain has another size or format");
        }
        /* a cube map's level is complete only when all six faces hold it */
        let levels = (needed as usize).min(t.level_faces.len());
        if t.target == GL_TEXTURE_CUBE_MAP && t.level_faces[1..levels.max(1)].iter().any(|&f| f != 0x3f) {
            return Some("a mipmap min filter, but a cube map face lacks a level of the chain, or has one of another size or format");
        }
    }
    if super::texture::NPOT_RESTRICTED
        && (!draw_is_pot(t.width) || !draw_is_pot(t.height))
        && (mips || t.wrap_s != GL_CLAMP_TO_EDGE || t.wrap_t != GL_CLAMP_TO_EDGE)
    {
        return Some("ES 2.0 non-power-of-two: needs CLAMP_TO_EDGE on both axes and no mipmap filter");
    }
    None
}

/// Reported once per process: enough to name the fault, not enough to drown a
/// conformance run, where incomplete textures are tested on purpose.
static INCOMPLETE_TEXTURE_WARNED: Global<bool> = Global::zeroed();

/// GL's wrap enum as the backend contract's [`GlesWrap`].
fn map_wrap(wrap: GLenum) -> u8 {
    match wrap {
        GL_CLAMP_TO_EDGE => GLES_WRAP_CLAMP_TO_EDGE,
        GL_MIRRORED_REPEAT => GLES_WRAP_MIRRORED_REPEAT,
        _ => GLES_WRAP_REPEAT,
    }
}

/// The GLSL sampler types that read a unit's cube map / 2D array binding.
const GL_SAMPLER_CUBE_T: u16 = 0x8B60;
const GL_SAMPLER_CUBE_SHADOW_T: u16 = 0x8DC5;
const GL_SAMPLER_2D_ARRAY_T: u16 = 0x8DC1;
const GL_SAMPLER_2D_ARRAY_SHADOW_T: u16 = 0x8DC4;
const GL_SAMPLER_3D_T: u16 = 0x8B5F;

/// The float sampler type an `isampler*` / `usampler*` reads the same
/// binding as (ES 3.0 2.11.9: the sampler's dimension picks the texture
/// target, its return type does not); every other type as it is. The
/// matches below on the float types take an integer sampler through this.
const fn binding_type(gl_type: u16) -> u16 {
    match gl_type as u32 {
        GL_INT_SAMPLER_2D | GL_UNSIGNED_INT_SAMPLER_2D => GL_SAMPLER_2D as u16,
        GL_INT_SAMPLER_3D | GL_UNSIGNED_INT_SAMPLER_3D => GL_SAMPLER_3D_T,
        GL_INT_SAMPLER_CUBE | GL_UNSIGNED_INT_SAMPLER_CUBE => GL_SAMPLER_CUBE_T,
        GL_INT_SAMPLER_2D_ARRAY | GL_UNSIGNED_INT_SAMPLER_2D_ARRAY => GL_SAMPLER_2D_ARRAY_T,
        _ => gl_type,
    }
}

/// The emulated 2D arrays carry their layer count in a hidden uniform
/// (pvr_layers_<sampler>) the translator added to the fragment block: the
/// shader divides the stacked texture's height by it. It has to be in the
/// block BEFORE the block is uploaded, so it is filled here and not where the
/// textures are bound (which happens after the upload: the count then reached
/// the GPU one draw late, and the first draw divided by zero).
///
/// Both stages: Qt Quick 3D's morph targets are a sampler2DArray read in the
/// VERTEX shader (texelFetch indexed by gl_VertexID), so a fragment-only fill
/// left the vertex block's layer count at zero and the shader divided the
/// stacked texture's height by it -- the morphed mesh came apart.
fn fill_sampler_layer_uniforms(
    p: &mut GlProgram,
    textures: &[GlTexture; TEXTURE_SLOTS],
    bound_2d_array: &[GLuint],
    bound_3d: &[GLuint],
) {
    let (vs_ptr, vs_cap) = (p.vs_uniforms, p.vs_uniform_cap as usize);
    let (fs_ptr, fs_cap) = (p.fs_uniforms, p.fs_uniform_cap as usize);
    let p: &GlProgram = p;

    for (sh, units, ptr, cap) in
        [(&p.rfs, &p.fs_sampler_unit, fs_ptr, fs_cap), (&p.rvs, &p.vs_sampler_unit, vs_ptr, vs_cap)]
    {
        /* The blocks are C heap allocations the program only points at, so
         * writing one does not alias the reflection read here. */
        let block: &mut [f32] = if ptr.is_null() {
            &mut []
        } else {
            // SAFETY: the stage's block is `cap` dwords the program owns
            // (glLinkProgram; null before a link), and the two stages'
            // blocks are distinct allocations.
            unsafe { core::slice::from_raw_parts_mut(ptr, cap) }
        };
        fill_stage_layer_uniforms(sh, units, block, textures, bound_2d_array, bound_3d);
    }
}

/// One stage of [`fill_sampler_layer_uniforms`]: `units` is the stage's
/// sampler-unit table and `block` its uniform block.
fn fill_stage_layer_uniforms(
    sh: &ShcShader,
    units: &[u8; CAP_SAMPLERS],
    block: &mut [f32],
    textures: &[GlTexture; TEXTURE_SLOTS],
    bound_2d_array: &[GLuint],
    bound_3d: &[GLuint],
) {
    let table = sh.uniform_table();

    for su in table {
        if su.dword_off != SHC_UNIFORM_NO_BLOCK || su.binding as usize >= CAP_SAMPLERS {
            continue;
        }
        let bound = match binding_type(su.gl_type) {
            GL_SAMPLER_3D_T => bound_3d, /* sampler3D: the slice count, same hidden uniform */
            GL_SAMPLER_2D_ARRAY_T | GL_SAMPLER_2D_ARRAY_SHADOW_T => bound_2d_array, /* (i)sampler2DArray */
            _ => continue,
        };
        let h = bound[units[su.binding as usize] as usize];
        let t = (h != 0 && (h as usize) < MAX_TEXTURES && textures[h as usize].used).then(|| &textures[h as usize]);
        /* snprintf(lname, SHC_UNIFORM_NAME_MAX + 16, "pvr_layers_%s", name):
         * the bytes as they are, truncated to the buffer */
        const PREFIX: &[u8] = b"pvr_layers_";
        let mut lname = [0u8; SHC_UNIFORM_NAME_MAX + 16];
        let name = util::name_bytes(&su.name);
        let len = (PREFIX.len() + name.len()).min(lname.len() - 1);
        lname[..PREFIX.len()].copy_from_slice(PREFIX);
        lname[PREFIX.len()..len].copy_from_slice(&name[..len - PREFIX.len()]);
        for k in table {
            if k.dword_off != SHC_UNIFORM_NO_BLOCK && util::name_bytes(&k.name) == &lname[..len] {
                let layers = match t {
                    Some(t) if t.layers != 0 => t.layers as u32,
                    _ => 1,
                };
                if let Some(slot) = block.get_mut(k.dword_off as usize) {
                    *slot = layers as f32;
                }
                break;
            }
        }
    }
}

/// Section 7.5's `gl_DepthRange`: the online compiler gives a shader that
/// reads it three rows after the application's uniforms,
/// `gl_DepthRange.near`, `.far` and `.diff` ([`SHC_DEPTH_RANGE_ROWS`]),
/// and they are filled here from glDepthRangef before the blocks are
/// uploaded, in both stages. A shader without the rows (every offline table
/// entry) is untouched.
pub(super) fn fill_depth_range_uniforms(p: &mut GlProgram, range: [GLclampf; 2]) {
    let values = [range[0], range[1], range[1] - range[0]];
    let (vs_ptr, vs_cap) = (p.vs_uniforms, p.vs_uniform_cap as usize);
    let (fs_ptr, fs_cap) = (p.fs_uniforms, p.fs_uniform_cap as usize);
    let p: &GlProgram = p;
    for (sh, ptr, cap) in [(&p.rfs, fs_ptr, fs_cap), (&p.rvs, vs_ptr, vs_cap)] {
        if ptr.is_null() {
            continue;
        }
        // SAFETY: as in `fill_sampler_layer_uniforms`: the stage's block is
        // `cap` dwords the program owns, distinct from the other stage's.
        let block = unsafe { core::slice::from_raw_parts_mut(ptr, cap) };
        fill_stage_depth_range(sh, block, values);
    }
}

/// One stage of [`fill_depth_range_uniforms`].
fn fill_stage_depth_range(sh: &ShcShader, block: &mut [f32], values: [f32; 3]) {
    for u in sh.uniform_table() {
        if u.dword_off == SHC_UNIFORM_NO_BLOCK {
            continue;
        }
        let name = util::name_bytes(&u.name);
        let Some(k) = SHC_DEPTH_RANGE_ROWS.iter().position(|m| m.as_bytes() == name) else {
            continue;
        };
        if let Some(slot) = block.get_mut(u.dword_off as usize) {
            *slot = values[k];
        }
    }
}

/* ---- the cube-map atlas dump (bring-up) ----------------------------------
 *
 * A light probe that renders flat or saturated magenta is either flat in
 * memory, prefiltered into fewer levels than the sampler is allowed past, or
 * stacked in the wrong order, and the per-draw binding line above cannot tell
 * those apart: one mean per face per level can. Two things it had to learn
 * (upstream 7d1e16c), both the hard way:
 *
 *   - Read after the GPU, not before. The first version read the storage at
 *     the first draw that sampled it and reported the -Z face as empty. It
 *     was still being rendered, and a finish alone does not cover a kick that
 *     is recorded but not yet submitted: the flush has to come first.
 *
 *   - Look more than once. Each atlas is dumped up to five times, spaced by
 *     KICKS rather than by samples of that texture: three dumps inside one
 *     kick say nothing about whether a level is still being rendered, and a
 *     texture the scene rarely samples would never get a second look.
 */
const CUBE_SEEN_MAX: usize = 4;

/// One cube map the dump has met. The C keys on the texture's CPU mapping and
/// keeps the `struct gl_texture *`; here the storage is the identity and the
/// texture is named by its context and table index, so a deleted or replaced
/// texture is noticed instead of followed.
#[derive(Clone, Copy)]
struct CubeSeen {
    cpu: *mut c_void,
    ti: usize,
    ctx: u32,
    dumps: u32,
    last_kick: u32,
}

struct CubeDumpState {
    seen: [CubeSeen; CUBE_SEEN_MAX],
    n: usize,
    /// Swaps dumped so far ([`debug_dump_cubes`]).
    frames: u32,
}

static CUBES: Global<CubeDumpState> = Global::zeroed();

/// The dump state (all-zero is "nothing seen yet").
fn cubes() -> &'static mut CubeDumpState {
    // SAFETY: single-threaded by contract; no other reference is live.
    unsafe { CUBES.get() }
}

/// The mean of one face at one level: every eighth texel of the face's rows,
/// R, G and B as floats.
fn cube_face_mean(texels: &[u8], off: usize, lw: u32, lh: u32, pitch: u32) -> f32 {
    let (sy, sx) = (lh.div_ceil(8).max(1), lw.div_ceil(8).max(1));
    let (mut sum, mut n) = (0.0f32, 0u32);

    for y in (0..lh).step_by(sy as usize) {
        for x in (0..lw).step_by(sx as usize) {
            let px = off + (y as usize * pitch as usize + x as usize) * 8;

            if px + 6 > texels.len() {
                return 0.0;
            }
            for c in 0..3 {
                sum += half_to_float(u16::from_le_bytes([texels[px + c * 2], texels[px + c * 2 + 1]]));
            }
            n += 3;
        }
    }
    if n != 0 {
        sum / n as f32
    } else {
        0.0
    }
}

/// One cube map's faces, level by level. `unit` is the sampler binding it was
/// sampled through (0 from the swap).
fn dump_cube_atlas(c: &mut GlContext, ti: usize, unit: u8, when: &str) {
    let t = &c.objs_ref().textures[ti];

    if t.layers < 2 {
        return;
    }
    /* One line for every cube sample, with no budget: the question "does the
     * skybox draw ever bind the light probe at all" cannot be answered from a
     * dump that stops after the first few. */
    glog!("  cube-bind: 0x{:x} unit {} kick {} ({})", t.mem.gpu, unit, backend::kick_id(), when);
    if t.texfmt != GLES_TEXFMT_RGBA16F || t.mem.cpu.is_null() {
        glog!(
            "  cube({}) @0x{:x} {}x{} x{}: fmt {}, cpu {:p} - not dumped",
            when,
            t.mem.gpu,
            t.width,
            t.height,
            t.layers,
            t.texfmt,
            t.mem.cpu
        );
        return;
    }
    let (cpu, ctx) = (t.mem.cpu, c.index as u32);
    let st = cubes();
    let i = match st.seen[..st.n].iter().position(|s| s.cpu == cpu) {
        Some(i) => i,
        None => {
            if st.n >= CUBE_SEEN_MAX {
                return;
            }
            st.seen[st.n] = CubeSeen { cpu, ti, ctx, dumps: 0, last_kick: 0 };
            st.n += 1;
            st.n - 1
        }
    };
    let kick = backend::kick_id();
    if st.seen[i].dumps >= 5 {
        return;
    }
    if st.seen[i].dumps != 0 && kick.wrapping_sub(st.seen[i].last_kick) < 4 {
        return;
    }
    st.seen[i].last_kick = kick;
    st.seen[i].dumps += 1;

    /* The faces are rendered by kicks this draw does not wait for: reading
     * the storage while the last of them is still in flight reports it as
     * empty, which is a property of the measurement and not of the probe.
     * Drain first, and invalidate whatever view of it the CPU holds. */
    flush_pending_clear(c);
    if backend::flush() != 0 || backend::finish() != 0 {
        glog!("  cube({}): the GPU work did not complete; nothing to read", when);
        return;
    }
    let t = &c.objs_ref().textures[ti];
    // SAFETY: the whole CPU mapping of the texture's live allocation.
    unsafe { osal::cache_invd(t.mem.cpu, t.mem.size) };
    glog!(
        "  cube({}) @0x{:x} {}x{} x{}, chain {}, written {}, max_level {}",
        when,
        t.mem.gpu,
        t.width,
        t.height,
        t.layers,
        t.chain_levels,
        t.levels_written,
        t.max_level
    );
    // SAFETY: the texture owns its live allocation and the GPU is idle
    // (drained just above); nothing else refers to the bytes here.
    let Some(texels) = (unsafe { t.mem.bytes_mut() }) else {
        return;
    };
    for lv in 0..t.chain_levels as u32 {
        let (lw, lh) = (t.width >> lv, t.height >> lv);

        if lw == 0 || lh == 0 {
            break;
        }
        let pitch = t.level_pitch(lw);
        let mut line = FmtBuf::<176>::new();

        for f in 0..t.layers as u32 {
            if line.as_bytes().len() >= 176 - 24 {
                break;
            }
            let off = t.level_off[lv as usize] as usize + (f * lh * pitch * 8) as usize;
            let _ = write!(line, " {:.4}", cube_face_mean(texels, off, lw, lh, pitch));
        }
        glog!(
            "  cube({}) level {} ({}x{} @0x{:x}):{}",
            when,
            lv,
            lw,
            lh,
            t.level_off[lv as usize],
            Bytes(line.as_bytes())
        );
    }
}

/// `gl_debug_dump_cubes()`: called from the EGL swap, after the frame's kicks
/// have completed -- the only place a read of GPU-written storage means what
/// it appears to mean.
pub fn debug_dump_cubes() {
    if !log_enabled() || cubes().frames >= 3 {
        return;
    }
    let Some(c) = context::current() else { return };
    cubes().frames += 1;
    for i in 0..cubes().n {
        let s = cubes().seen[i];
        /* the texture may be gone, or its slot reused for another one */
        if s.ctx != c.index as u32 || c.objs().textures[s.ti].mem.cpu != s.cpu {
            continue;
        }
        dump_cube_atlas(c, s.ti, 0, "swap");
    }
}

/// Whole-texture alpha scans left ([`dump_bound_texture`]): the walk is not
/// cheap, so only the first few glyph atlases are measured.
static ALPHA_SCANS: AtomicI32 = AtomicI32::new(4);

/// Debug (`gl_debug_attrib_dump`): a few texels of what the draw is about to
/// sample. A texture that renders flat is either flat in memory or sampled
/// wrongly, and nothing else in the log tells the two apart. `tw` x `rows` is
/// the texture as the sampler sees it (faces/layers stacked).
///
/// Unlike the C, every read is bounded by the allocation: the sample points
/// of a block-compressed texture are computed with the linear pitch, which
/// the store does not follow.
fn dump_bound_texture(t: &GlTexture, binding: u8, tw: u32, rows: u32) {
    if !attrib_dump_enabled() || t.mem.cpu.is_null() || !log_enabled() {
        return;
    }
    let bpt = gles_texfmt_bytes(t.texfmt) as usize;
    let pitch = tex_pitch(tw) as usize;
    let mut line = FmtBuf::<160>::new();

    for (px, py) in [(1u32, 1u32), (tw / 4, rows / 4), (tw / 2, rows / 2), (tw * 3 / 4, rows * 3 / 4)] {
        let x = if px < tw { px } else { 0 };
        let y = if py < rows { py } else { 0 };
        let need = if t.texfmt == GLES_TEXFMT_RGB565 || bpt == 0 { 2 } else { 4 };
        let off = (y as usize)
            .checked_mul(pitch)
            .and_then(|r| r.checked_add(x as usize))
            .and_then(|i| i.checked_mul(bpt.max(1)));
        let Some(off) = off.filter(|o| o.checked_add(need).is_some_and(|e| e <= t.mem.size)) else {
            continue;
        };
        let mut b = [0u8; 4];
        // SAFETY: off + need <= mem.size of a live CPU mapping; bytes need
        // no alignment.
        unsafe { core::ptr::copy_nonoverlapping((t.mem.cpu as *const u8).add(off), b.as_mut_ptr(), need) };
        let _ = match t.texfmt {
            GLES_TEXFMT_RGBA8 => {
                write!(line, " ({x},{y})={:02x}{:02x}{:02x}{:02x}", b[0], b[1], b[2], b[3])
            }
            GLES_TEXFMT_RGBA16F | GLES_TEXFMT_R16F => write!(
                line,
                " ({x},{y})={:04x}{:04x}",
                b[0] as u32 | ((b[1] as u32) << 8),
                b[2] as u32 | ((b[3] as u32) << 8)
            ),
            _ => write!(line, " ({x},{y})={:02x}{:02x}", b[0], b[1]),
        };
    }
    glog!("  tex unit {} ({}x{} fmt {}):{}", binding, tw, rows, t.texfmt, Bytes(line.as_bytes()));
    /* Where the texture's alpha actually is. A glyph atlas that samples flat
     * is either empty (the upload never arrived) or holds its glyphs
     * somewhere other than where the text's texture coordinates look; four
     * texels cannot tell those apart, a bounding box can. */
    if t.texfmt != GLES_TEXFMT_RGBA8 || ALPHA_SCANS.load(Ordering::Relaxed) <= 0 || tw < 1024 || rows >= 512 {
        return; /* not a glyph atlas's shape */
    }
    /* the last row's last texel must be inside the mapping */
    let last = (rows as usize)
        .checked_sub(1)
        .and_then(|r| r.checked_mul(pitch.checked_mul(4)?))
        .and_then(|r| r.checked_add((tw as usize).checked_mul(4)?));
    if last.is_none_or(|e| e > t.mem.size) {
        return;
    }
    ALPHA_SCANS.fetch_sub(1, Ordering::Relaxed);
    let (mut x0, mut y0, mut x1, mut y1, mut n) = (tw, rows, 0u32, 0u32, 0u32);
    for y in 0..rows {
        // SAFETY: the last texel of the last row is inside the mapping
        // (checked above); alpha is one byte, so no alignment is needed.
        let row = unsafe {
            core::slice::from_raw_parts((t.mem.cpu as *const u8).add(y as usize * pitch * 4), tw as usize * 4)
        };

        for x in 0..tw {
            if row[x as usize * 4 + 3] != 0 {
                n += 1;
                x0 = x0.min(x);
                x1 = x1.max(x);
                y0 = y0.min(y);
                y1 = y1.max(y);
            }
        }
    }
    glog!(
        "  tex unit {} alpha: {} of {} texels, box {},{}..{},{}",
        binding,
        n,
        tw.saturating_mul(rows),
        x0,
        y0,
        x1,
        y1
    );
}

/// Fill the shared fixed-function + target + clear + shader/uniform/texture parts
/// of a GlesDraw from the current context. On a GL error returns it for the
/// caller to record. Geometry (prim/index) is filled by the callers. `pgs` is
/// the context's row of the per-program cache, `staging` the per-attribute
/// staging; `program` is a live, linked program (current_drawable_program()).
/// [`super::texture::resolve_depth`] for every 2D texture `program`'s
/// samplers read, before the draw is built: the depth a kick rendered into a
/// depth texture reaches its texels only then.
fn resolve_sampled_depth(c: &mut GlContext, program: GLuint) {
    let mut slots = [usize::MAX; 2 * CAP_SAMPLERS];
    let mut n = 0;
    let p = &context::objects(c.group).programs[obj_index(program)];
    for (sh, units) in [(&p.rfs, &p.fs_sampler_unit), (&p.rvs, &p.vs_sampler_unit)] {
        for (binding, gl_type) in sh.sampler_bindings() {
            if binding as usize >= CAP_SAMPLERS
                || matches!(
                    binding_type(gl_type),
                    GL_SAMPLER_CUBE_T
                        | GL_SAMPLER_CUBE_SHADOW_T
                        | GL_SAMPLER_2D_ARRAY_T
                        | GL_SAMPLER_2D_ARRAY_SHADOW_T
                        | GL_SAMPLER_3D_T
                )
            {
                continue; /* what build_common binds from the other targets */
            }
            let bound = c.bound_texture_2d[units[binding as usize] as usize];
            let slot = if bound == 0 { super::texture::default_slot(GL_TEXTURE_2D) } else { Some(bound as usize) };
            if let Some(s) = slot.filter(|_| n < slots.len()) {
                slots[n] = s;
                n += 1;
            }
        }
    }
    for &s in &slots[..n] {
        super::texture::resolve_depth(c, s);
    }
}

fn build_common(
    c: &mut GlContext,
    pgs: &mut [ProgGpu; MAX_PROGRAMS],
    staging: AttribStaging<'_>,
    d: &mut GlesDraw,
    program: GLuint,
    vtx_first: u32,
    vtx_count: u32,
) -> Result<(), DrawError> {
    const FUNC: &str = "build_common";

    *d = GlesDraw::ZERO;
    /* each section below is a step of draw_steps.rs */
    let mut clock = draw_steps::Timer::start();

    if obj_index(program) < MAX_PROGRAMS {
        resolve_sampled_depth(c, program);
    }
    resolve_target(c, d)?;
    set_target_format(c, d);
    d.samples = if c.bound_framebuffer == 0 { c.default_samples } else { 1 };
    set_sample_state(c, d);
    resolve_targets(c, d);
    refuse_undrawable_targets(d, 0)?;
    d.instance_count = draw_instances();
    d.instance_id = DRAW_INSTANCE_ID.load(Ordering::Relaxed);
    /* the active occlusion query counts this draw (the pending clear, the
     * only other thing a draw can carry, is never counted) */
    d.occlusion_query = super::es3::occlusion_query(c);

    /* shaders + their uploaded USC */
    let p = &context::objects(c.group).programs[obj_index(program)];
    let pg = &mut pgs[obj_index(program)];
    if ensure_prog_gpu(pg, &p.rvs, &p.rfs) != 0 {
        return Err((GL_OUT_OF_MEMORY, FUNC));
    }
    d.vs = &p.rvs;
    d.fs = &p.rfs;
    d.vs_usc_gpu = pg.vs_usc.gpu;
    d.fs_usc_gpu = pg.fs_usc.gpu;
    d.vs_output_size_dwords = p.rvs.vs_output_size as u32;

    /* The sampler-state variant this draw's bound samplers ask for (see
     * `draw_lod_key`): the program's two stages as compiled for the key,
     * with the same interface -- varyings, uniforms, samplers, attributes
     * -- so only the code the draw points at changes. Blend variants below
     * are of its fragment shader. */
    let lod_key = draw_lod_key(c, pg, p, d);
    let lod = if lod_key == [0, 0] { None } else { lod_variant_for(pg, &p.rvs, &p.rfs, lod_key) };
    /* copies for the blend variant's compile: `pg` is borrowed mutably there */
    let lod_shaders = lod.map(|i| (pg.lod_var[i].vs, pg.lod_var[i].fs));
    let fs_lod_key = lod.map_or(0, |i| pg.lod_var[i].key[1]);
    if let Some(i) = lod {
        let v = &pg.lod_var[i];
        d.vs = &v.vs;
        d.fs = &v.fs;
        if v.key[0] != 0 {
            d.vs_usc_gpu = v.vs_usc.gpu;
        }
        if fs_rebuilt(v.key) {
            d.fs_usc_gpu = v.fs_usc.gpu;
        }
    }
    let (base_vs, base_fs): (&ShcShader, &ShcShader) = match &lod_shaders {
        Some((vs, fs)) => (vs, fs),
        None => (&p.rvs, &p.rfs),
    };

    /* blending: swap in the FS variant compiled for this blend state, and
     * pass the state on as well for a backend that blends in hardware */
    /* blending is ignored for an integer buffer (ES 3.0 4.1.8) */
    d.blend = c.blend && !framebuffer::fb_target_is_integer(c);
    d.blend_src_rgb = map_blend_factor(c.blend_src) as u8;
    d.blend_dst_rgb = map_blend_factor(c.blend_dst) as u8;
    d.blend_src_alpha = map_blend_factor(c.blend_src_a) as u8;
    d.blend_dst_alpha = map_blend_factor(c.blend_dst_a) as u8;
    d.blend_eq_rgb = map_blend_eq(c.blend_eq) as u8;
    d.blend_eq_alpha = map_blend_eq(c.blend_eq_a) as u8;
    d.blend_color = c.blend_color; /* GLclampf is f32 */
    let mut stencil_only_variant = false; /* see below; folded into d.color_write */
    let mut fs: &ShcShader = base_fs;
    {
        /* The V3D blends, masks colour and faces in fixed function (the TLB,
         * CFG_BITS), and its fragment shader is part of the linked program:
         * there is no variant of it to ask for, so its key is always 0 and the
         * program's own FS is the draw's. The keys below are the PowerVR's
         * (and the GC7000's winding). */
        let v3d = cfg!(feature = "gpu-v3d");
        /* The GC7000 blends and masks in its pixel engine as well: its table
         * has no blend variants, and its only key is the winding twin below.
         * (Its draws used to ask for a blend variant, miss it, and draw the
         * plain shader -- right there, and a refusal now.) */
        let fixed_function_blend = v3d || cfg!(feature = "gpu-vivante");
        let mut key = if fixed_function_blend { 0 } else { current_blend_key(c) };
        /* A colour mask with every channel off is normally an ISP tag-write
         * disable (no fragment shader runs, GlesDraw::color_write = false). A
         * shader with `discard` runs as a punch-through pass whose depth and
         * stencil update wait for the USC's visibility feedback, so with the
         * tags disabled the stencil op never happens (femtovg zeroes its
         * stencil exactly this way, gl_test case 38). Such draws run the
         * ZERO/ONE blend variant instead: the shader executes, its output
         * writes the destination colour back unchanged. */
        let stencil_only_discard = !fixed_function_blend && disabled_channels(c) == 0xf && p.rfs.uses_discard != 0;

        if stencil_only_discard {
            key = shc_blend_key(SHC_BF_ZERO, SHC_BF_ONE, SHC_BF_ZERO, SHC_BF_ONE, SHC_BLEND_ADD, SHC_BLEND_ADD);
        }
        /* gl_FrontFacing where the sense is compiled in rather than draw state
         * (the GC7000: see `YakoglBackend::front_facing_shader_variant`). Two
         * things decide which binary a draw wants, and they compose:
         * glFrontFace, and whether the target is y-flipped -- a flip reverses
         * the winding the rasteriser sees, which is the same asymmetry the cull
         * direction needs. So the variant is taken when exactly one of the two
         * is set, and only for a shader that reads gl_FrontFacing at all; every
         * other shader keeps key 0 and its single binary. */
        if backend::front_facing_shader_variant()
            && p.rfs.reads_front_facing != 0
            && ((c.front_face == GL_CW) ^ framebuffer::fb_flip_y(c))
        {
            key = SHC_VIV_KEY_FRONT_FLIPPED;
        }
        /* A variant the draw needs and cannot have refuses the draw: drawn
         * with the plain shader it would be a picture of another blend state
         * (the PowerVR's subtract variants never built, and fragment_ops
         * drew them as replaces, am62p-board ba0966d7). ES has no error for
         * it: counted (yakogl_draws_refused) and logged; the pending clear
         * below is left for the next draw. */
        match fs_variant_for(pg, base_vs, base_fs, key, fs_lod_key) {
            Ok(Some(slot)) => {
                let v = &pg.fs_var[slot];

                fs = &v.sh;
                d.fs = &v.sh;
                d.fs_usc_gpu = v.usc.gpu;
                if stencil_only_discard {
                    stencil_only_variant = true;
                }
            }
            Ok(None) => {}
            Err(why) => {
                backend::note_draw_refused(why, key);
                return Err((GL_NO_ERROR, FUNC));
            }
        }
    }

    /* Fuse a pending glClear into this kick, with the write masks applied: a
     * buffer whose mask is entirely off is not cleared, and an empty scissor
     * box clears nothing. A clear that needs the quad was drawn as its own
     * object before this draw (settle_quad_clear), so `quad` is false here;
     * if it were not, fusing it whole is what this always did. */
    let pc = pending_clear_of(c);
    if pc.color || pc.depth || pc.stencil {
        let (has_depth, has_stencil) = framebuffer::fb_target_depth_stencil(c);
        match plan_clear(pc, (d.target_w, d.target_h, d.target_gpu != 0), has_depth, has_stencil) {
            Some(cp) => apply_clear_plan(d, &cp, pc),
            None => {
                pc.color = false;
                pc.depth = false;
                pc.stencil = false;
            }
        }
    } else {
        take_pending_ds_clears(d, pc, framebuffer::fb_target_depth_stencil(c));
    }
    resolve_ds_storage(c, d);

    clock.lap(Step::GlProgram);

    /* uniforms: upload each stage's block (VS at 0, FS after it, 16-byte
     * aligned for the DMA) and point the stage at its block. The FS block
     * is the variant's: a CONSTANT_* blend variant reads glBlendColor from
     * 4 dwords after the app uniforms. */
    let (fs_uniform_dwords, fs_blend_consts_off, fs_usc_dw) = (fs.uniform_dwords, fs.blend_consts_off, fs.usc_dw);
    {
        /* one reference to the group, split by field: the program is
         * written while the texture table is read */
        let o = context::objects(c.group);
        fill_sampler_layer_uniforms(
            &mut o.programs[obj_index(program)],
            &o.textures,
            &c.bound_texture_2d_array,
            &c.bound_texture_3d,
        );
        fill_depth_range_uniforms(&mut o.programs[obj_index(program)], c.depth_range);
    }
    super::program::fill_uniform_blocks(c, program);
    /* glBlendColor lives in the block a blend variant reads, right after the
     * application's own uniforms; put it in the host-side array too, so the
     * backend's snapshot carries it (the array keeps BLEND_CONSTS_DWORDS
     * past the block for exactly this). */
    if fs_uniform_dwords != 0 && fs_blend_consts_off != SHC_NO_BLEND_CONSTS {
        let off = fs_blend_consts_off as usize;
        let bc = c.blend_color;
        let blk = c.objs().programs[obj_index(program)].fs_block_with_blend_mut();

        if off + 4 <= blk.len() {
            blk[off..off + 4].copy_from_slice(&bc);
        }
    }
    let p = &c.objs_ref().programs[obj_index(program)];
    {
        let mut vs_dw = p.rvs.uniform_dwords;
        let mut fs_dw = fs_uniform_dwords;
        let mut fs_app_dw = p.rfs.uniform_dwords;
        let fs_off = (vs_dw as usize * 4 + 15) & !15usize;

        if vs_dw as usize > MAX_UNIFORM_DWORDS {
            vs_dw = MAX_UNIFORM_DWORDS as u16;
        }
        if fs_dw as usize > MAX_UNIFORM_DWORDS {
            fs_dw = MAX_UNIFORM_DWORDS as u16;
        }
        if fs_app_dw > fs_dw {
            fs_app_dw = fs_dw;
        }
        if (vs_dw != 0 || fs_dw != 0) && pg.uniforms.cpu.is_null() {
            /* no CPU view: the kick reads the block itself, so a block a
             * pending kick reads is not rewritten (staging_reserve) */
            let bytes = pg.uniforms.size;
            let uniforms_at = StampAt::Uniforms { group: c.group, prog: obj_index(program) as u16 };
            if staging_reserve(&mut pg.uniforms, &mut pg.uniforms_kick, uniforms_at, bytes) != 0 {
                return Err((GL_OUT_OF_MEMORY, FUNC));
            }
        }
        /* a failing upload leaves the previous draw's values: the draw is refused */
        let mut rc = 0;
        if vs_dw != 0 {
            /* the block holds min(uniform_dwords, MAX_UNIFORM_DWORDS) = vs_dw dwords */
            let blk = p.vs_block();
            rc |= put(&mut pg.uniforms, 0, dword_bytes(&blk[..(vs_dw as usize).min(blk.len())]));
            d.vs_uniform_gpu = pg.uniforms.gpu;
            d.vs_uniform_dwords = vs_dw;
            /* The backend snapshots the block into the frame's own arena
             * (else draws that share a program in one kick would all read
             * the last one). Hand it the program's host-side array, not the
             * GPU buffer: that buffer belongs to the program and what it
             * holds between the copy above and the backend's read is the
             * program's business, while this array is the values the
             * application set (upstream 98f7331). */
            d.vs_uniform_cpu = p.vs_uniforms as *const c_void;
        }
        if fs_dw != 0 {
            if fs_app_dw != 0 {
                let blk = p.fs_block();
                rc |= put(&mut pg.uniforms, fs_off, dword_bytes(&blk[..(fs_app_dw as usize).min(blk.len())]));
            }
            if fs_blend_consts_off != SHC_NO_BLEND_CONSTS && fs_blend_consts_off as u32 + 4 <= fs_dw as u32 {
                rc |= put(&mut pg.uniforms, fs_off + fs_blend_consts_off as usize * 4, dword_bytes(&c.blend_color));
            }
            d.fs_uniform_gpu = pg.uniforms.gpu + fs_off as u64;
            d.fs_uniform_dwords = fs_dw;
            dump_small_fs_block(fs_usc_dw, p, fs_dw);
            /* as the vertex stage above. The variant's block is the shader's
             * uniforms plus the blend constants, which is what the array
             * holds; a variant that somehow asks for more than that keeps
             * the GPU buffer's view, which is always large enough. */
            d.fs_uniform_cpu = if fs_dw <= p.fs_uniform_alloc {
                p.fs_uniforms as *const c_void
            } else if pg.uniforms.cpu.is_null() {
                core::ptr::null()
            } else {
                // SAFETY: fs_off is inside the uniforms allocation
                // (2 * MAX_UNIFORM_DWORDS * 4 + 16 bytes).
                unsafe { (pg.uniforms.cpu as *const u8).add(fs_off) as *const c_void }
            };
        }
        if rc != 0 {
            return Err((GL_OUT_OF_MEMORY, FUNC));
        }
    }

    clock.lap(Step::GlUniforms);

    /* attributes: each VS input, resolved through the VS attrib map */
    let mut na = 0usize;
    let mut staged_apps = 0u32; /* attribute indices staged into `scratch` by this draw */
    let (inputs, num_inputs) = vertex_inputs(&p.rvs);
    for sa in &inputs[..num_inputs] {
        if na >= MAX_DRAW_ATTRIBS {
            break;
        }
        if sa.location >= 16 {
            continue;
        }
        let app = p.attrib_app_index[sa.location as usize] as usize; /* glBindAttribLocation remap */
        if app >= MAX_VERTEX_ATTRIBS {
            continue;
        }
        let ga = c.attribs[app];
        let comps = ga.size as u8;
        let slot = na;
        let pa = &mut d.attribs[na];
        na += 1;
        /* an array with a divisor: element i / divisor for instance i (a
         * plain draw is one instance, so every vertex reads element 0);
         * draw_instanced lets a divisor through here for more than one
         * instance only up to the backend's max_instance_divisor */
        let inst = ga.enabled && ga.divisor != 0;
        let a_first = if inst { 0 } else { vtx_first };
        let a_count = if inst { draw_instances().div_ceil(ga.divisor) } else { vtx_count };

        let src_fmt = map_attrib_format(ga.type_);

        pa.vi_reg = sa.vi_reg;
        pa.components = comps;
        pa.format = GLES_ATTRIBF_FLOAT32;
        pa.normalized = 0;
        pa.cpu = core::ptr::null();
        pa.cpu_bytes = 0;
        pa.per_instance = inst as u8;
        pa.divisor = if inst { ga.divisor.min(u32::from(u16::MAX)) as u16 } else { 0 };

        /* Where this input is staged: the attribute index's scratch, unless
         * another input of this draw already staged there (two inputs bound
         * to one index read it with their own widths). */
        let (m, kick, at) = if staged_apps & (1u32 << app) != 0 {
            (&mut staging.dup[slot], &mut staging.dup_kick[slot], StampAt::DupStaging(slot as u8))
        } else {
            (&mut staging.scratch[app], &mut staging.kick[app], StampAt::AttribStaging(app as u8))
        };

        if !ga.enabled {
            /* no array: every vertex reads the glVertexAttrib* value, a
             * zero-stride fetch of the input's width from scratch */
            if staging_reserve(m, kick, at, 16) != 0 || put(m, 0, dword_bytes(&c.current_attrib[app])) != 0 {
                return Err((GL_OUT_OF_MEMORY, FUNC));
            }
            staged_apps |= 1u32 << app;
            pa.components = sa.components;
            pa.stride = 0;
            pa.offset = 0;
            pa.buffer_gpu = m.gpu;
            pa.cpu = m.cpu; /* per-draw copy: the staging is shared */
            pa.cpu_bytes = if m.cpu.is_null() { 0 } else { 16 };
            let extent = u64::from(pa.components) * 4;
            if !fetch_fits(pa, a_count, extent, 16) {
                return Err(refuse_fetch(app, pa, a_count, extent, 16));
            }
            continue;
        }
        let mut vb = None;
        if ga.buffer != 0 {
            match live_buffer(&mut context::objects(c.group).buffers, ga.buffer) {
                Some(b) if !b.mem.impl_.is_null() => vb = Some(b),
                _ => return Err((GL_INVALID_OPERATION, FUNC)),
            }
        } else if attrib_elem_bytes(&ga) != 0 && ga.offset < CLIENT_POINTER_FLOOR {
            /* ES 2.0.25 2.9: an array whose buffer binding is zero is read
             * from client memory at its pointer. A pointer this small is an
             * offset into a buffer that is not bound (or was deleted): the
             * read would fault, so the draw is refused by name instead. */
            if CLIENT_ARRAY_LOGS.load(Ordering::Relaxed) > 0 && log_enabled() {
                CLIENT_ARRAY_LOGS.fetch_sub(1, Ordering::Relaxed);
                glog!(
                    "{}: attribute {} is a client array at {:#x} (no ARRAY_BUFFER binding): refused",
                    FUNC,
                    app,
                    ga.offset
                );
            }
            return Err((GL_INVALID_OPERATION, FUNC));
        }
        /* The bytes vertices a_first .. a_first + a_count - 1 span from the
         * array's offset: the last element starts at (last * stride) and is
         * `elem` bytes long. u64 cannot overflow (u32 * < 2^31 + ...). */
        let array_end = |elem: u64| -> Option<u64> {
            let last = (a_first as u64 + a_count as u64).checked_sub(1)?;
            (ga.offset as u64).checked_add(last * attrib_stride(&ga) as u64 + elem)
        };
        if let Some(vb) = vb.as_deref_mut().filter(|_| attrib_format_is_native(src_fmt) && comps >= sa.components) {
            /* a format the backend's vertex fetch reads, in a VBO, at least
             * as wide as the input: fetched in place (only the input's
             * components, so a wider array never spills into the next
             * input's registers) */
            pa.components = sa.components;
            pa.format = src_fmt;
            pa.normalized = (ga.normalized != 0) as u8;
            pa.stride = attrib_stride(&ga);
            /* The GPU fetches every vertex of the range: it must lie inside
             * the buffer (an indexed draw that is not staged passes vertex 0
             * only; its indices are not read, see draw_elements).
             *
             * One element is the input's components in the SOURCE format, not
             * in float32: upstream 850c039 kept the float width here when it
             * let the other formats through, and that over-estimates the last
             * element of a tightly packed byte or short array by up to twelve
             * bytes -- a legal draw refused with GL_INVALID_OPERATION, which
             * is exactly the case that commit set out to support. */
            let elem = sa.components as u64 * attrib_type_size(ga.type_) as u64;
            let offset = (ga.offset as u64).checked_add(a_first as u64 * pa.stride as u64);
            match (array_end(elem), offset.and_then(|o| u32::try_from(o).ok())) {
                (Some(end), Some(offset)) if a_count != 0 && end <= vb.size as u64 => pa.offset = offset,
                /* the array runs past the VBO: nothing downstream could tell
                 * a bad GPU address from a good one, so say which attribute */
                _ => {
                    if ATTRIB_RANGE_LOGS.load(Ordering::Relaxed) > 0 && log_enabled() {
                        ATTRIB_RANGE_LOGS.fetch_sub(1, Ordering::Relaxed);
                        glog!(
                            "yakogl: attribute {} fetches {} vertices of {} bytes at {} stride {} from a {}-byte buffer: out of range",
                            app,
                            a_count,
                            elem,
                            offset.unwrap_or(u64::MAX),
                            pa.stride,
                            vb.size
                        );
                    }
                    return Err((GL_INVALID_OPERATION, FUNC));
                }
            }
            pa.buffer_gpu = vb.mem.gpu;
            vb.last_use = draw_stamp(StampAt::Buffer { group: c.group, bi: ga.buffer as u16 });
            if attrib_dump_enabled() && src_fmt == GLES_ATTRIBF_FLOAT32 && !vb.mem.cpu.is_null() && log_enabled() {
                /* debug: the first vertices of an in-place stream */
                for v in 0..a_count.min(24) {
                    let mut f = [0.0f32; 4];

                    // SAFETY: debug aid; reads the vertices this draw is
                    // about to fetch from the VBO's mapping (range checked
                    // above for the whole draw).
                    unsafe {
                        let base = (vb.mem.cpu as *const u8).add(pa.offset as usize);
                        core::ptr::copy_nonoverlapping(
                            base.add(v as usize * pa.stride as usize),
                            f.as_mut_ptr() as *mut u8,
                            pa.components as usize * 4,
                        );
                    }
                    dump_in_place_vertex(sa.vi_reg, v, &f[..pa.components as usize]);
                }
            }
            let avail = (vb.size as u64).saturating_sub(u64::from(pa.offset));
            if !fetch_fits(pa, a_count, elem, avail) {
                return Err(refuse_fetch(app, pa, a_count, elem, avail));
            }
            continue;
        }

        /* client-side array, a packed format, or an array narrower than
         * the input: the vertex range is converted to packed float32 of
         * the input's width in a scratch buffer */
        pa.components = sa.components;
        if let Some(vb) = vb {
            match array_end(attrib_elem_bytes(&ga) as u64) {
                Some(end) if a_count != 0 && end <= vb.size as u64 => {}
                _ => return Err((GL_INVALID_OPERATION, FUNC)), /* the array runs past the VBO */
            }
            let src = if vb.mem.cpu.is_null() {
                core::ptr::null()
            } else {
                // SAFETY: offset .. end was just checked against the VBO's size.
                unsafe { (vb.mem.cpu as *const u8).add(ga.offset) }
            };
            let r = convert_attrib(
                m,
                kick,
                at,
                &ga,
                src,
                Some(&mut vb.mem),
                ga.offset,
                a_first,
                a_count,
                sa.components as u32,
            );
            if r != 0 {
                return Err((GL_OUT_OF_MEMORY, FUNC));
            }
        } else if convert_attrib(
            m,
            kick,
            at,
            &ga,
            ga.offset as *const u8,
            None,
            0,
            a_first,
            a_count,
            sa.components as u32,
        ) != 0
        {
            return Err((GL_OUT_OF_MEMORY, FUNC));
        }
        staged_apps |= 1u32 << app;
        pa.stride = sa.components as u32 * 4;
        pa.offset = 0;
        pa.buffer_gpu = m.gpu;
        /* The scratch is one buffer per attribute index, rewritten by the
         * next draw while this one is still waiting in the kick: hand the
         * backend the CPU bytes so it keeps a per-draw copy. */
        pa.cpu = m.cpu;
        pa.cpu_bytes = if m.cpu.is_null() {
            0
        } else {
            /* convert_attrib() allocated this many bytes: it fits a usize, and
             * a count beyond u32 bytes is not a draw the backend can take */
            match u32::try_from(a_count as u64 * sa.components.min(4) as u64 * 4) {
                Ok(n) => n,
                Err(_) => return Err((GL_OUT_OF_MEMORY, FUNC)),
            }
        };
        /* convert_attrib() packs rows of at most four floats */
        let (extent, avail) = (u64::from(pa.components) * 4, u64::from(a_count) * u64::from(sa.components.min(4)) * 4);
        if !fetch_fits(pa, a_count, extent, avail) {
            return Err(refuse_fetch(app, pa, a_count, extent, avail));
        }
    }
    d.num_attribs = na as u8;

    clock.lap(Step::GlAttribs);

    /* Textures: each sampler binding of a stage reads the texture bound to
     * the unit its glUniform1i named; GlesTextureBind::unit is the sampler
     * binding (the descriptor slot), not the GL unit. The vertex stage has
     * its own descriptors -- Qt Quick 3D reads bone matrices and morph
     * targets from a texture in the vertex shader. */
    /* Cube maps this draw samples, dumped once the loop below has let go of
     * the context (`dump_cube_atlas` drains the GPU, which needs it whole).
     * Upstream dumps each one inside the loop, so with two cube maps in one
     * draw the two dumps arrive after both binding lines rather than
     * interleaved. */
    let mut cube_bindings = [(0usize, 0u8); GLES_MAX_TEX_BINDS];
    let mut num_cube_bindings = 0usize;
    for stage in 0..2u8 {
        let (sh, units) = if stage == 1 { (&p.rfs, &p.fs_sampler_unit) } else { (&p.rvs, &p.vs_sampler_unit) };
        let dst_max = if stage == 1 { GLES_MAX_TEX_BINDS } else { crate::backend::GLES_VS_TEX_UNITS };

        /* One binding per sampler, and per ELEMENT of an array of samplers.
         * The link refused a stage with more than `dst_max` of them
         * (program.rs `check_sampler_counts`). */
        for (binding, gl_type) in sh.sampler_bindings() {
            let ndst = if stage == 1 { d.num_textures } else { d.num_vs_textures };

            if ndst as usize >= dst_max {
                break;
            }
            if binding as usize >= CAP_SAMPLERS {
                continue;
            }
            let unit = units[binding as usize] as usize;
            /* the sampler's GLSL type says which binding of the unit it reads
             * (a samplerCube the cube map, a sampler2DArray the array) */
            let (bound, target) = match binding_type(gl_type) {
                GL_SAMPLER_CUBE_T | GL_SAMPLER_CUBE_SHADOW_T => (c.bound_texture_cube[unit], GL_TEXTURE_CUBE_MAP),
                GL_SAMPLER_2D_ARRAY_T | GL_SAMPLER_2D_ARRAY_SHADOW_T => {
                    (c.bound_texture_2d_array[unit], GL_TEXTURE_2D_ARRAY)
                }
                GL_SAMPLER_3D_T => (c.bound_texture_3d[unit], GL_TEXTURE_3D),
                _ => (c.bound_texture_2d[unit], GL_TEXTURE_2D),
            };
            /* Binding 0 is the target's default texture object (ES 2.0 3.7.13),
             * which holds images like any other: dEQP-GLES2
             * texture.completeness.cube.extra_level specifies its six faces
             * with a 2D texture bound, so they land in the default cube map,
             * and expects it sampled. */
            let slot = if bound == 0 { super::texture::default_slot(target) } else { Some(bound as usize) };
            let sampler = super::es3::sampler_state(c.index, unit);
            /* the render-into stamp is kept by texture NAME (its slot) */
            let target_kick = slot.map_or(0, |s| framebuffer::texture_target_use(c, s));
            let Some((ti, t)) =
                slot.and_then(|s| live_texture(&mut context::objects(c.group).textures, s).map(|t| (s, t)))
            else {
                continue;
            };
            if t.mem.impl_.is_null() {
                continue; /* no texture: the backend binds a black 1x1 */
            }
            /* A backend whose texture unit reads the stack itself is given one
             * layer and the count; the others get the whole stack as one 2D
             * image -- faces and array layers stacked, a 3D texture's slices
             * in a grid. */
            let layered = backend::native_layered_textures();
            let nlayers = if t.layers != 0 { t.layers as u32 } else { 1 };
            let cols = if t.grid_cols != 0 && !layered { t.grid_cols as u32 } else { 1 };
            /* saturating: the texture's own limits keep these products small,
             * and a nonsense one must not wrap into a small byte count */
            let rows = if layered {
                t.height
            } else if t.grid_cols != 0 {
                t.height.saturating_mul(nlayers.div_ceil(cols))
            } else {
                t.height.saturating_mul(nlayers)
            };
            let tw = t.width.saturating_mul(cols);

            let tb = if stage == 1 {
                d.num_textures += 1;
                &mut d.textures[d.num_textures as usize - 1]
            } else {
                d.num_vs_textures += 1;
                &mut d.vs_textures[d.num_vs_textures as usize - 1]
            };
            tb.unit = binding;
            tb.format = t.texfmt;
            tb.type_ = match binding_type(gl_type) {
                GL_SAMPLER_CUBE_T | GL_SAMPLER_CUBE_SHADOW_T => GLES_TEXTYPE_CUBE,
                GL_SAMPLER_2D_ARRAY_T | GL_SAMPLER_2D_ARRAY_SHADOW_T => GLES_TEXTYPE_2D_ARRAY,
                GL_SAMPLER_3D_T => GLES_TEXTYPE_3D,
                _ => GLES_TEXTYPE_2D,
            };
            tb.layers = nlayers as u16;
            /* the whole stack, not one layer: the rows above are what the
             * hardware is told, these are what the storage holds */
            tb.bytes = if t.block_bytes != 0 {
                /* level 0: every face's block grid, back to back */
                t.block_w.saturating_mul(t.block_h).saturating_mul(t.block_bytes as u32).saturating_mul(nlayers)
            } else {
                t.level_pitch(tw)
                    .saturating_mul(if layered { rows.saturating_mul(nlayers) } else { rows })
                    .saturating_mul(gles_texfmt_bytes(t.texfmt))
            };
            tb.base_gpu = t.mem.gpu;
            /* the stamp names these texels at this address (0: never cached) */
            tb.content_gen = t.content_gen;
            tb.target_kick = target_kick;
            tb.cpu = t.mem.cpu as *const core::ffi::c_void;
            t.last_use = draw_stamp(StampAt::Texture { group: c.group, ti: ti as u16 });
            tb.width = tw;
            tb.height = rows;
            tb.stride_texels = t.level_pitch(tw);
            tb.bgra = (t.bgra || t.pe_order) as u8;
            tb.no_alpha = texture_has_no_alpha(t) as u8;
            tb.depth_luminance = texture_depth_luminance(t) as u8;
            /* The PowerVR's native cube (gl::native_cube, pvr_native_cube):
             * the TPU samples a twiddled copy. Whether the copy is stale is
             * asked HERE, at every draw, not at bind: the texels' stamp moves
             * at every CPU write, and a kick that rendered into the cube
             * (framebuffer::texture_target_use) makes it stale too -- that
             * kick is waited for before the copy reads the texels. A render
             * into the cube in the frame being recorded (a feedback loop,
             * undefined in GL) keeps the stacked image. */
            /* The program decides, and only the program: one the online
             * compiler built with the native cube, or a table row
             * pco compiled from a real samplerCube (glsl2usc since step C;
             * emit.rs sets the flag), samples the TPU's own cube and gets
             * the copy; a stacked-cube program (pvr_native_cube off) must
             * not. The switch no longer asks for it on its own: with it on
             * by default that would hand a copy to a stacked program. The
             * PowerVR's alone: the GC7000 and the V3D sample cubes their own
             * way. */
            #[cfg(feature = "gpu-powervr")]
            if tb.type_ == GLES_TEXTYPE_CUBE && sh.native_cube != 0 {
                /* the kick that last rendered into it: GlesTextureBind's
                 * target_kick, filled above, which the GC7000's tiled copy
                 * reads too (one staleness source for both copies) */
                let rendered = tb.target_kick;
                let open = rendered != 0 && rendered == backend::kick_id();
                if !open && (rendered == 0 || !backend::kick_pending(rendered) || backend::finish() == 0) {
                    /* one level sampled (no mipmap filter, or a one-level
                     * chain): a one-level copy, which the backend describes
                     * as one level (the same rule, pvr/backend.rs) */
                    let (_, _, mip_filter, mip_levels) = sampling_fields(t);
                    let single = !(mip_filter != 0 && mip_levels > 1);
                    if let Some((gpu, bytes)) = super::native_cube::copy(t, rendered, single) {
                        tb.type_ = GLES_TEXTYPE_CUBE_TWIDDLED_PROBE;
                        tb.base_gpu = gpu;
                        tb.content_gen = 0; /* a copy kept here: not the texture's stamp */
                        tb.bytes = bytes;
                        tb.width = t.width;
                        tb.height = t.height;
                        tb.stride_texels = t.width;
                    }
                }
            }
            /* A sampler object bound to the unit replaces the texture's own
             * filters and wraps (ES 3.0 3.8.2), completeness included; the
             * texture's are put back below. Its LOD range is not applied;
             * its compare mode and function are (`compare_op`). */
            let own = (t.min_filter, t.mag_filter, t.wrap_s, t.wrap_t);
            if let Some(s) = sampler {
                (t.min_filter, t.mag_filter, t.wrap_s, t.wrap_t) = s;
            }
            (tb.linear_filter, tb.min_linear, tb.mip_filter, tb.mip_levels) = sampling_fields(t);
            let (mode, func) = super::es3::sampler_compare(c.index, unit).unwrap_or((t.compare_mode, t.compare_func));
            tb.compare_op = compare_op(mode, func, t);
            tb.wrap_s = map_wrap(t.wrap_s);
            tb.wrap_t = map_wrap(t.wrap_t);
            let incomplete = incomplete_reason(t);
            (t.min_filter, t.mag_filter, t.wrap_s, t.wrap_t) = own;
            if let Some(why) = incomplete {
                tb.incomplete = 1;
                // SAFETY: single-threaded by contract; no other reference is live.
                let warned = unsafe { INCOMPLETE_TEXTURE_WARNED.get() };
                if !*warned {
                    *warned = true;
                    glog!(
                        "a texture on unit {} is incomplete and samples (0,0,0,1): {} \
                         (target 0x{:x}, {}x{}, levels {} of {}, min filter 0x{:x}, wrap 0x{:x}/0x{:x})",
                        unit,
                        why,
                        t.target,
                        t.width,
                        t.height,
                        t.levels,
                        t.chain_levels,
                        t.min_filter,
                        t.wrap_s,
                        t.wrap_t
                    );
                }
            }
            dump_bound_texture(t, binding, tw, rows);
            if t.layers >= 2 && num_cube_bindings < cube_bindings.len() {
                cube_bindings[num_cube_bindings] = (slot.unwrap_or(0), binding);
                num_cube_bindings += 1;
            }
        }
    }
    /* Not gated on the per-draw dump: that one is switched off after the
     * second frame, and a Quick 3D scene samples its prefiltered probe long
     * after. Three lines-per-level dumps of at most four cube maps is
     * nothing next to a frame's worth of draw dumps. */
    if log_enabled() {
        for &(ti, unit) in &cube_bindings[..num_cube_bindings] {
            dump_cube_atlas(c, ti, unit, "draw");
        }
    }

    clock.lap(Step::GlTextures);

    /* fixed-function render state */
    d.depth_range = c.depth_range;
    d.viewport = c.viewport;
    d.flip_y = framebuffer::fb_flip_y(c);

    /* ES 2.0 4.1.4 / 4.1.6: with no depth buffer the depth test always
     * passes and nothing is written; with no stencil buffer the stencil test
     * always passes and nothing is written. The target keeps a depth/stencil
     * buffer of its own whether GL says one exists or not, so a framebuffer
     * object with no such attachment has to be told here, or GL_DEPTH_TEST
     * would test against a buffer the application never asked for. */
    let (has_depth, has_stencil) = framebuffer::fb_target_depth_stencil(c);

    d.depth_test = c.depth_test && has_depth;
    d.polygon_offset = c.polygon_offset_fill;
    d.polygon_offset_factor = c.polygon_offset_factor;
    d.polygon_offset_units = c.polygon_offset_units;
    d.depth_func = map_cmp(c.depth_func);
    d.depth_write = c.depth_mask && has_depth;

    d.cull = map_cull(c);
    d.line_width = c.line_width;

    d.scissor = c.scissor_test;
    d.scissor_rect = c.scissor;

    d.stencil = c.stencil_test && has_stencil;
    d.stencil_func = map_cmp(c.stencil_func);
    d.stencil_ref = c.stencil_ref as u8;
    d.stencil_cmp_mask = c.stencil_mask as u8;
    d.stencil_write_mask = if has_stencil { c.stencil_writemask as u8 } else { 0 };
    d.stencil_sfail = map_stencil_op(c.stencil_sfail);
    d.stencil_dpfail = map_stencil_op(c.stencil_dpfail);
    d.stencil_dppass = map_stencil_op(c.stencil_dppass);
    d.front_cw = c.front_face == GL_CW;
    d.stencil_back_func = map_cmp(c.stencil_back_func);
    d.stencil_back_ref = c.stencil_back_ref as u8;
    d.stencil_back_cmp_mask = c.stencil_back_mask as u8;
    d.stencil_back_write_mask = if has_stencil { c.stencil_back_writemask as u8 } else { 0 };
    d.stencil_back_sfail = map_stencil_op(c.stencil_back_sfail);
    d.stencil_back_dpfail = map_stencil_op(c.stencil_back_dpfail);
    d.stencil_back_dppass = map_stencil_op(c.stencil_back_dppass);

    /* colour write mask: all-off is honoured by the ISP (depth/stencil-only
     * draws) unless the shader discards, when the ZERO/ONE variant above keeps
     * the fragment shader running; per-channel masks are variants too */
    d.color_write = disabled_channels(c) != 0xf || stencil_only_variant;
    /* and the mask itself, for a backend that masks in hardware */
    d.color_mask = (!disabled_channels(c) & 0xf) as u8;
    /* no colour target (a depth/stencil-only framebuffer): nothing of colour
     * is written, whatever glColorMask says */
    if d.target_gpu == 0 {
        (d.color_write, d.color_mask) = (false, 0);
    }

    clock.lap(Step::GlState);
    Ok(())
}

/// Record a GL error with or without a current context (without one,
/// gl_set_error() only logs).
fn raise(c: Option<&mut GlContext>, e: GLenum, func: &str) {
    match c {
        Some(c) => c.set_error(e, func),
        None => context::set_error_no_ctx(e, func),
    }
}

/// Shared validation: a linked, current program is required to draw, and the
/// bound framebuffer has to be complete.
///
/// The order matters. GL_INVALID_FRAMEBUFFER_OPERATION outranks every other
/// error a draw can raise (ES 2.0 4.4.5), so a draw with no program into an
/// incomplete framebuffer reports the framebuffer, not the program -- which
/// is what dEQP's negative_api.vertex_array.draw_* cases check. The argument
/// errors (mode, count, index type) come earlier still, in the callers.
fn current_drawable_program(c: Option<&mut GlContext>) -> Option<(&mut GlContext, GLuint)> {
    const FUNC: &str = "current_drawable_program";

    let Some(c) = c else {
        context::set_error_no_ctx(GL_INVALID_OPERATION, FUNC);
        return None;
    };
    if !framebuffer::fb_target_drawable(c) {
        c.set_error(GL_INVALID_FRAMEBUFFER_OPERATION, FUNC);
        return None;
    }
    if c.cur_program == 0 || !c.program_ref(c.cur_program).is_some_and(|p| p.linked) {
        c.set_error(GL_INVALID_OPERATION, FUNC);
        return None;
    }
    let handle = c.cur_program;
    Some((c, handle))
}

/// glDrawArrays (traced here: the instanced draws enter it per instance, as
/// the C calls the entry point).
fn draw_arrays(c: Option<&mut GlContext>, st: &mut DrawState, mode: GLenum, first: GLint, count: GLsizei) {
    const FUNC: &str = "glDrawArrays";
    gl_trace!(glDrawArrays);
    let _stamps = DrawStampScope::open();

    if count < 0 {
        return raise(c, GL_INVALID_VALUE, FUNC);
    }
    if first < 0 {
        return raise(c, GL_INVALID_VALUE, FUNC);
    }

    let Some(prim) = map_prim(mode) else {
        return raise(c, GL_INVALID_ENUM, FUNC);
    };

    let Some((c, handle)) = current_drawable_program(c) else {
        return;
    };
    if super::buffer::draw_reads_a_mapped_buffer(c, false) {
        return c.set_error(GL_INVALID_OPERATION, FUNC);
    }
    /* ES 3.0 transform feedback and rasterizer discard: the argument and
     * capture rules first, then a discarded draw stops here (no back end
     * sees it). Neither is reachable under report-es2 / with default state. */
    if !crate::REPORT_ES2
        && (!tf::pre_draw(c, mode, false, first, count, draw_instances(), FUNC) || c.rasterizer_discard)
    {
        return;
    }
    if count == 0 {
        return;
    }
    if osal::ops().is_none() {
        return;
    }

    /* `first` is folded into the attribute offsets (or the client-array
     * upload) by build_common: the backend fetches vertices sequentially. */
    settle_pending_clear(c, &mut st.clear_draw, &mut st.draw_fail_reported);
    settle_quad_clear(c, &mut st.clear_draw, &mut st.draw_fail_reported);
    let d = &mut st.arrays_draw;
    if culls_everything(c, prim) {
        if let Err((e, func)) = resolve_target(c, d) {
            c.set_error(e, func);
        }
        return;
    }
    let pgs = &mut st.pg[c.group as usize];
    let staging = AttribStaging {
        scratch: &mut st.attrib_scratch,
        kick: &mut st.attrib_kick,
        dup: &mut st.dup_scratch,
        dup_kick: &mut st.dup_kick,
    };
    if let Err((e, func)) = build_common(c, pgs, staging, d, handle, first as u32, count as u32) {
        /* GL_NO_ERROR: refused and counted (backend::draws_refused), no GL error */
        if e != GL_NO_ERROR {
            c.set_error(e, func);
        }
        return;
    }

    d.prim = prim;
    d.index_count = count as u32;
    d.index_buf_gpu = 0; /* non-indexed sequential */
    d.index_size = 0;
    d.index_cpu = core::ptr::null();
    d.index_bytes = 0;

    let mut kicks =
        StagingKicks { index: &mut st.index_kick, attrib: &mut st.attrib_kick, dup: &mut st.dup_kick, pg: &mut st.pg };
    submit_draw(c, &mut st.draw_fail_reported, d, FUNC, Some(&mut kicks));
}

#[no_mangle]
pub unsafe extern "C" fn glDrawArrays(mode: GLenum, first: GLint, count: GLsizei) {
    draw_arrays(context::current(), STATE.get(), mode, first, count);
}

/// A VBO copy on the C heap (an index buffer without a CPU mapping), freed
/// on every way out of glDrawElements.
struct HeapCopy(*mut u8);

/// ES 3.0 2.8.1, primitive restart with a fixed index: `get(k)` is index k
/// of `n`, and `restart` (the type's largest value) ends the primitive being
/// assembled. The pieces are written to `out` as the independent list
/// primitive `mode` is made of -- points, lines or triangles -- in GL's
/// vertex order, so the winding and the provoking (last) vertex of every
/// primitive are the ones `mode` would have given: an odd triangle of a
/// strip is (k+1, k, k+2), a fan's is (0, k, k+1), and a loop is closed by
/// (last, first). An incomplete primitive at a restart is dropped, as GL
/// drops it. `out` needs `3 * n` entries; the count written and the list
/// mode come back. `None` for a mode with no list form.
fn restart_to_lists(
    n: usize,
    get: impl Fn(usize) -> u32,
    restart: u32,
    mode: GLenum,
    out: &mut [u32],
) -> Option<(usize, GLenum)> {
    let list = match mode {
        GL_POINTS => GL_POINTS,
        GL_LINES | GL_LINE_STRIP | GL_LINE_LOOP => GL_LINES,
        GL_TRIANGLES | GL_TRIANGLE_STRIP | GL_TRIANGLE_FAN => GL_TRIANGLES,
        _ => return None,
    };
    let mut w = 0usize;
    let mut put = |v: u32| {
        if let Some(slot) = out.get_mut(w) {
            *slot = v;
            w += 1;
        }
    };
    let mut start = 0usize;
    for end in 0..=n {
        if end < n && get(end) != restart {
            continue;
        }
        let len = end - start;
        let s = |i: usize| get(start + i);
        match mode {
            GL_POINTS => (0..len).for_each(|i| put(s(i))),
            GL_LINES => (0..len / 2).for_each(|i| {
                put(s(2 * i));
                put(s(2 * i + 1));
            }),
            GL_LINE_STRIP | GL_LINE_LOOP => {
                for i in 0..len.saturating_sub(1) {
                    put(s(i));
                    put(s(i + 1));
                }
                if mode == GL_LINE_LOOP && len >= 2 {
                    put(s(len - 1));
                    put(s(0));
                }
            }
            GL_TRIANGLES => (0..len / 3).for_each(|i| {
                put(s(3 * i));
                put(s(3 * i + 1));
                put(s(3 * i + 2));
            }),
            GL_TRIANGLE_STRIP => {
                for i in 0..len.saturating_sub(2) {
                    let (a, b) = if i % 2 == 0 { (i, i + 1) } else { (i + 1, i) };
                    put(s(a));
                    put(s(b));
                    put(s(i + 2));
                }
            }
            _ => {
                /* GL_TRIANGLE_FAN */
                for i in 1..len.saturating_sub(1) {
                    put(s(0));
                    put(s(i));
                    put(s(i + 1));
                }
            }
        }
        start = end + 1;
    }
    Some((w, list))
}

impl Drop for HeapCopy {
    fn drop(&mut self) {
        // SAFETY: null or a live malloc block owned by self.
        unsafe { util::free(self.0 as *mut c_void) };
    }
}

fn draw_elements(
    c: Option<&mut GlContext>,
    st: &mut DrawState,
    mode: GLenum,
    count: GLsizei,
    type_: GLenum,
    indices: *const c_void,
) {
    const FUNC: &str = "glDrawElements";
    gl_trace!(glDrawElements);
    let _stamps = DrawStampScope::open();

    if count < 0 {
        return raise(c, GL_INVALID_VALUE, FUNC);
    }
    if type_ != GL_UNSIGNED_SHORT && type_ != GL_UNSIGNED_BYTE && type_ != GL_UNSIGNED_INT {
        return raise(c, GL_INVALID_ENUM, FUNC);
    }

    let Some(mut prim) = map_prim(mode) else {
        return raise(c, GL_INVALID_ENUM, FUNC);
    };

    let Some((c, handle)) = current_drawable_program(c) else {
        return;
    };
    if super::buffer::draw_reads_a_mapped_buffer(c, true) {
        return c.set_error(GL_INVALID_OPERATION, FUNC);
    }
    /* ES 3.0 transform feedback (an indexed draw while a capture is on is an
     * error) and rasterizer discard: see draw_arrays */
    if !crate::REPORT_ES2 && (!tf::pre_draw(c, mode, true, 0, count, draw_instances(), FUNC) || c.rasterizer_discard) {
        return;
    }
    if count == 0 {
        return;
    }
    let Some(ops) = osal::ops() else {
        return;
    };
    let (mut mode, mut count) = (mode, count);

    /* The backend consumes u16 indices in GPU memory. u16 in a VBO is used in
     * place; anything else (u8/u32, client-side arrays) is widened/copied into
     * the reusable u16 scratch. The largest index is found along the way so
     * client-side vertex arrays can be uploaded (0 .. max). */
    let mut index_gpu = 0u64;
    let mut max_index = 0u32;
    let mut src: *const u8 = core::ptr::null(); /* index bytes readable by the CPU */
    let mut tmp = HeapCopy(core::ptr::null_mut()); /* VBO copy when it has no CPU mapping */
    let mut esz: usize = match type_ {
        GL_UNSIGNED_BYTE => 1,
        GL_UNSIGNED_SHORT => 2,
        _ => 4,
    };
    /* count <= i32::MAX: at most 2^33 bytes, a usize on the 64-bit targets */
    let Some(mut src_bytes) = (count as usize).checked_mul(esz) else {
        return c.set_error(GL_OUT_OF_MEMORY, FUNC);
    };
    /* With primitive restart on, the indices are read on the CPU wherever
     * they are, so the restart index can be found (below). */
    let mut need_range = c.primitive_restart;

    /* The vertex range (0..max_index) must be known whenever an attribute
     * is converted at draw time -- client-side arrays, packed formats
     * (normalised bytes), or an array narrower than its input (a vec2
     * position feeding a vec4) -- because the conversion produces exactly
     * that many vertices. Only an all-float, in-place VBO draw can use the
     * index buffer without reading it (build_common applies the same test). */
    {
        let p = &c.objs_ref().programs[obj_index(handle)];

        for sa in &p.rvs.attribs[..p.rvs.num_attribs as usize] {
            let app = if sa.location < 16 { p.attrib_app_index[sa.location as usize] as usize } else { 0xff };

            if app >= MAX_VERTEX_ATTRIBS || !c.attribs[app].enabled {
                continue;
            }
            let ga = &c.attribs[app];
            if ga.buffer == 0 || ga.type_ != GL_FLOAT || (ga.size as u32) < sa.components as u32 {
                need_range = true;
            }
        }
    }

    if c.bound_element_buffer != 0 {
        /* `indices` is an offset into the buffer; a "negative" one (a
         * pointer value near the top of the address space) must not wrap
         * around the range check */
        let off = indices as usize;
        let ib = match live_buffer(&mut context::objects(c.group).buffers, c.bound_element_buffer) {
            Some(ib)
                if !ib.mem.impl_.is_null()
                    && off.checked_add(src_bytes).is_some_and(|end| end as u64 <= ib.size.max(0) as u64) =>
            {
                ib
            }
            _ => return c.set_error(GL_INVALID_OPERATION, FUNC),
        };

        if type_ == GL_UNSIGNED_SHORT && !need_range && (ib.mem.gpu + off as u64).is_multiple_of(INDEX_ADDR_ALIGN) {
            index_gpu = ib.mem.gpu + off as u64;
            ib.last_use = draw_stamp(StampAt::Buffer { group: c.group, bi: c.bound_element_buffer as u16 });
        } else if !ib.mem.cpu.is_null() {
            // SAFETY: off + src_bytes was checked against the buffer's size.
            unsafe {
                let at = (ib.mem.cpu as *mut u8).add(off);
                invd_cache(at as *mut c_void, src_bytes);
                src = at;
            }
        } else if let Some(read) = ops.gpu_read {
            // SAFETY: malloc has no preconditions.
            tmp.0 = unsafe { util::malloc(src_bytes) } as *mut u8;
            // SAFETY: `tmp` has room for src_bytes; `ib.mem` is a live allocation.
            if tmp.0.is_null() || unsafe { read(&mut ib.mem, off, tmp.0 as *mut c_void, src_bytes) } != 0 {
                return c.set_error(GL_OUT_OF_MEMORY, FUNC);
            }
            src = tmp.0;
        } else {
            return c.set_error(GL_INVALID_OPERATION, FUNC);
        }
    } else {
        /* No element buffer: `indices` is a client pointer. A small value
         * is a byte offset meant for a buffer that is not bound (seen from
         * Qt after a failed frame): report it instead of reading page 0. */
        if indices.is_null() || (indices as usize) < CLIENT_POINTER_FLOOR {
            if log_enabled() && st.index_reported < 3 {
                st.index_reported += 1;
                /* %p prints a null pointer as "(nil)" */
                if indices.is_null() {
                    glog!(
                        "glDrawElements: indices (nil) with no ELEMENT_ARRAY_BUFFER bound (vao {})",
                        c.bound_vertex_array
                    );
                } else {
                    glog!(
                        "glDrawElements: indices {:p} with no ELEMENT_ARRAY_BUFFER bound (vao {})",
                        indices,
                        c.bound_vertex_array
                    );
                }
                buffer::debug_dump_bind_hist();
            }
            return c.set_error(GL_INVALID_OPERATION, FUNC);
        }
        src = indices as *const u8;
    }

    /* ES 3.0 2.8.1: with GL_PRIMITIVE_RESTART_FIXED_INDEX on, the type's
     * largest index ends the primitive. The index list is rewritten as the
     * independent list primitive the mode is made of, without the restart
     * index, and drawn as that. Until 2026-09-24 the index was drawn as a
     * vertex: 0xffff made the client-array conversion read 65,536
     * vertices past the application's arrays (a segfault, dEQP-GLES3
     * primitive_restart.*.unsigned_short), and 0xffffffff was refused as
     * over 16 bits. */
    let mut restarted = HeapCopy(core::ptr::null_mut());
    if c.primitive_restart && !src.is_null() {
        let restart = match esz {
            1 => 0xff,
            2 => 0xffff,
            _ => u32::MAX,
        };
        // SAFETY: `src` holds `count` indices of `esz` bytes (as below).
        let sq = unsafe { core::slice::from_raw_parts(src, src_bytes) };
        let get = |k: usize| -> u32 {
            let s = &sq[k * esz..k * esz + esz];
            match esz {
                1 => u32::from(s[0]),
                2 => u32::from(u16::from_ne_bytes([s[0], s[1]])),
                _ => u32::from_ne_bytes([s[0], s[1], s[2], s[3]]),
            }
        };
        let n = count as usize;
        if (0..n).any(|k| get(k) == restart) {
            let Some(cap) = n.checked_mul(3) else {
                return c.set_error(GL_OUT_OF_MEMORY, FUNC);
            };
            // SAFETY: malloc has no preconditions.
            restarted.0 = unsafe { util::malloc(cap * 4) } as *mut u8;
            if restarted.0.is_null() {
                return c.set_error(GL_OUT_OF_MEMORY, FUNC);
            }
            // SAFETY: a fresh block of `cap` u32s (malloc aligns for any type).
            let out = unsafe { core::slice::from_raw_parts_mut(restarted.0 as *mut u32, cap) };
            let Some((w, list)) = restart_to_lists(n, get, restart, mode, out) else {
                return c.set_error(GL_INVALID_ENUM, FUNC);
            };
            if w == 0 {
                /* nothing but restarts and incomplete primitives */
                return;
            }
            (mode, count, esz, src_bytes, src) = (list, w as GLsizei, 4, w * 4, restarted.0 as *const u8);
            let Some(p) = map_prim(mode) else { unreachable!("a list mode") };
            prim = p;
        }
    }

    let staged = !src.is_null();
    if staged {
        /* widen into the scratch (u16 copy), tracking the range */
        let need = src_bytes / esz * 2;

        if staging_reserve(&mut st.index_scratch, &mut st.index_kick, StampAt::IndexStaging, need) != 0 {
            return c.set_error(GL_OUT_OF_MEMORY, FUNC);
        }
        let m = &mut st.index_scratch;
        let write = ops.gpu_write;
        if m.cpu.is_null() && write.is_none() {
            return c.set_error(GL_OUT_OF_MEMORY, FUNC);
        }
        // SAFETY: `src` is the index array (`count` indices of `esz` bytes:
        // a range-checked VBO mapping, its heap copy, or the application's
        // array, trusted as the C code did); bytes need no alignment.
        let sq = unsafe { core::slice::from_raw_parts(src, src_bytes) };
        /* without a CPU view the widened indices go out through gpu_write
         * in chunks of `chunk` */
        let mut chunk = [0u8; 512];
        let mut filled = 0usize;
        let mut written = 0usize;
        for (k, s) in sq.chunks_exact(esz).enumerate() {
            let v = match esz {
                1 => s[0] as u32,
                2 => u16::from_ne_bytes([s[0], s[1]]) as u32,
                _ => {
                    let v = u32::from_ne_bytes([s[0], s[1], s[2], s[3]]);
                    if v > 0xffff {
                        /* the vertex-fetch index path is 16-bit */
                        return c.set_error(GL_INVALID_OPERATION, FUNC);
                    }
                    v
                }
            };
            let q = (v as u16).to_ne_bytes();
            if !m.cpu.is_null() {
                // SAFETY: the scratch holds `need` = 2 * count bytes and k < count.
                unsafe { core::ptr::copy_nonoverlapping(q.as_ptr(), (m.cpu as *mut u8).add(k * 2), 2) };
            } else {
                chunk[filled..filled + 2].copy_from_slice(&q);
                filled += 2;
                if filled == chunk.len() || k * 2 + 2 == need {
                    let Some(write) = write else { unreachable!() };
                    // SAFETY: `m` is a live allocation of `need` bytes, written in order.
                    if unsafe { write(m, written, chunk.as_ptr() as *const c_void, filled) } != 0 {
                        return c.set_error(GL_OUT_OF_MEMORY, FUNC);
                    }
                    written += filled;
                    filled = 0;
                }
            }
            if v > max_index {
                max_index = v;
            }
        }
        if !m.cpu.is_null() {
            // SAFETY: the `need` bytes just written are inside the mapping.
            unsafe { flush_cache(m.cpu, need) };
        }
        index_gpu = m.gpu;
    }
    drop(tmp);
    drop(restarted);

    /* indexed draws address vertices 0..max_index absolutely */
    settle_pending_clear(c, &mut st.clear_draw, &mut st.draw_fail_reported);
    settle_quad_clear(c, &mut st.clear_draw, &mut st.draw_fail_reported);
    let d = &mut st.elements_draw;
    if culls_everything(c, prim) {
        if let Err((e, func)) = resolve_target(c, d) {
            c.set_error(e, func);
        }
        return;
    }
    let pgs = &mut st.pg[c.group as usize];
    let staging = AttribStaging {
        scratch: &mut st.attrib_scratch,
        kick: &mut st.attrib_kick,
        dup: &mut st.dup_scratch,
        dup_kick: &mut st.dup_kick,
    };
    if let Err((e, func)) = build_common(c, pgs, staging, d, handle, 0, max_index + 1) {
        /* GL_NO_ERROR: refused and counted (backend::draws_refused), no GL error */
        if e != GL_NO_ERROR {
            c.set_error(e, func);
        }
        return;
    }

    d.prim = prim;
    d.index_count = count as u32;
    d.index_buf_gpu = index_gpu;
    d.index_size = 2; /* u8 and u32 arrays were widened above */
    /* staged (widened/copied) indices: one scratch for every draw, so the
     * backend snapshots them per draw */
    let staged = staged && !st.index_scratch.cpu.is_null();
    d.index_cpu = if staged { st.index_scratch.cpu } else { core::ptr::null() };
    d.index_bytes = if staged { (count as u32).wrapping_mul(2) } else { 0 };
    if attrib_dump_enabled() && staged && log_enabled() {
        // SAFETY: the scratch holds the count * 2 bytes written above.
        let ix = unsafe { core::slice::from_raw_parts(st.index_scratch.cpu as *const u8, count as usize * 2) };
        let mut line = FmtBuf::<128>::new();

        for q in ix.as_chunks::<2>().0.iter().take(40) {
            let _ = write!(line, " {}", u16::from_ne_bytes([q[0], q[1]]));
            if line.as_bytes().len() > 100 {
                glog!("  indices:{}", Bytes(line.as_bytes()));
                line = FmtBuf::new();
            }
        }
        if !line.as_bytes().is_empty() {
            glog!("  indices:{} (count {}, max {}, mode 0x{:x})", Bytes(line.as_bytes()), count, max_index, mode);
        }
    }

    let mut kicks =
        StagingKicks { index: &mut st.index_kick, attrib: &mut st.attrib_kick, dup: &mut st.dup_kick, pg: &mut st.pg };
    submit_draw(c, &mut st.draw_fail_reported, d, FUNC, Some(&mut kicks));
}

#[no_mangle]
pub unsafe extern "C" fn glDrawElements(mode: GLenum, count: GLsizei, type_: GLenum, indices: *const c_void) {
    draw_elements(context::current(), STATE.get(), mode, count, type_, indices);
}

/* --------------------------------------------------------------------------- */
/* OpenGL ES 3.0 instanced draws.
 *
 * On the GPU when every per-instance array has divisor 1 (Qt Quick 3D's
 * instancing tables): one draw whose index list carries the instance count,
 * the divisor-1 arrays fetched by the instance number (PDS IR1), and
 * gl_InstanceID from the same register. Other divisors fall back to one draw
 * per instance with each per-instance attribute fed as that instance's
 * constant (the disabled-array path of build_common). */
#[derive(Clone, Copy)]
struct InstanceAttribSave {
    enabled: bool,
    /// Not "current": the driver's kernel-module harness builds the C view of
    /// these sources with the Linux headers, where `current` is a macro
    /// (upstream 7bb2e11).
    value: [GLfloat; 4],
}

/// draw_instanced()'s statics. Valid when all-zero.
struct InstancedState {
    save: [InstanceAttribSave; MAX_VERTEX_ATTRIBS],
}

static INSTANCED: Global<InstancedState> = Global::zeroed();

/// The value attribute array `ga` (divisor > 0) supplies to `instance`.
fn instance_attrib_value(c: &mut GlContext, ga: &GlVertexAttrib, instance: GLuint) -> Option<[GLfloat; 4]> {
    attrib_element(c, ga, u64::from(instance / ga.divisor)).map(|v| v.map(f32::from_bits))
}

/// The four dwords a shader input reads from element `element` of the
/// enabled attribute array `ga`, converted as the draw converts them
/// ([`attrib_row`]: components the array lacks read as (0, 0, 0, 1)). `None`
/// when the element is outside its buffer, or the array is an offset with
/// no buffer bound. Instanced draws read their divisor arrays through it,
/// and the transform-feedback capture every array (`tf_capture.rs`).
pub(super) fn attrib_element(c: &mut GlContext, ga: &GlVertexAttrib, element: u64) -> Option<[u32; 4]> {
    let comps = (ga.size as u32).min(4);
    let tsz = attrib_type_size(ga.type_);
    let elem = attrib_elem_bytes(ga) as usize;
    /* u64: (u32 element) * stride (< 2^31) cannot overflow */
    let rel = element.checked_mul(u64::from(attrib_stride(ga)))?;
    let mut raw = [0u8; 16];

    if elem == 0 {
        /* enabled, but no pointer was ever set (size 0): nothing to read,
         * the element reads as (0, 0, 0, 1) (as the C did) */
        return Some(attrib_row(&[], ga, 0, tsz, 4));
    }
    let src: *const u8 = if ga.buffer == 0 {
        if ga.offset < CLIENT_POINTER_FLOOR {
            return None; /* an offset with no buffer bound, not an address */
        }
        /* a client array: trusted, as the C code did (the address is formed
         * without assuming it stays in bounds) */
        (ga.offset as *const u8).wrapping_add(usize::try_from(rel).ok()?)
    } else {
        let ops = osal::ops()?;
        let vb = c.buffer(ga.buffer)?;
        let off = usize::try_from((ga.offset as u64).checked_add(rel)?).ok()?;

        if vb.mem.impl_.is_null() || off.checked_add(elem)? > vb.size.max(0) as usize {
            return None;
        }
        if !vb.mem.cpu.is_null() {
            // SAFETY: off + elem was just checked against the buffer's size.
            unsafe {
                let at = (vb.mem.cpu as *mut u8).add(off);
                invd_cache(at as *mut c_void, elem);
                at
            }
        } else {
            let read = ops.gpu_read?;
            // SAFETY: `vb.mem` is a live allocation; `raw` has room for the at
            // most 16 bytes of one element.
            if unsafe { read(&mut vb.mem, off, raw.as_mut_ptr() as *mut c_void, elem) } != 0 {
                return None;
            }
            raw.as_ptr()
        }
    };
    // SAFETY: `src` is the element inside the VBO mapping (range checked),
    // the local copy of it, or the application's array (trusted as in C).
    let vsrc = unsafe { core::slice::from_raw_parts(src, elem) };
    Some(attrib_row(vsrc, ga, comps, tsz, 4))
}

fn draw_instanced(
    c: Option<&mut GlContext>,
    mode: GLenum,
    first: GLint,
    count: GLsizei,
    type_: GLenum,
    indices: *const c_void,
    instancecount: GLsizei,
) {
    const FUNC: &str = "draw_instanced";
    let mut per_instance = 0u32; /* bit set of the divisor > 0 arrays in use */

    let Some(c) = c else {
        return;
    };
    // SAFETY: single-threaded by contract; taken once per entry point (this
    // is only called from the instanced entry points).
    let (st, inst_st) = unsafe { (STATE.get(), INSTANCED.get()) };
    if instancecount < 0 {
        return c.set_error(GL_INVALID_VALUE, FUNC);
    }
    if instancecount == 0 {
        return;
    }
    let mut gpu = true;
    let max_divisor = backend::max_instance_divisor();
    for (i, a) in c.attribs.iter().enumerate() {
        if a.enabled && a.divisor != 0 {
            per_instance |= 1u32 << i;
            if a.divisor > max_divisor {
                gpu = false;
            }
        }
    }
    /* Drawn instance by instance below, each instance captures as its own
     * draw: a capture that cannot hold them all is refused before the first
     * one writes anything (ES 3.0 2.15.2: nothing is captured or drawn). */
    if !crate::REPORT_ES2 && !gpu && type_ == 0 && !tf::fits(c, mode, count, instancecount as u32, FUNC) {
        return;
    }
    let draw_once = |c: &mut GlContext, st: &mut DrawState| {
        if type_ != 0 {
            draw_elements(Some(c), st, mode, count, type_, indices);
        } else {
            draw_arrays(Some(c), st, mode, first, count);
        }
    };
    if gpu || instancecount == 1 {
        DRAW_INSTANCES.store(instancecount, Ordering::Relaxed);
        draw_once(c, st);
        DRAW_INSTANCES.store(1, Ordering::Relaxed);
        return;
    }
    let save = &mut inst_st.save;
    for i in (0..MAX_VERTEX_ATTRIBS).filter(|i| per_instance & (1u32 << i) != 0) {
        save[i].enabled = c.attribs[i].enabled;
        save[i].value = c.current_attrib[i];
        c.attribs[i].enabled = false;
    }
    'instances: for inst in 0..instancecount {
        for i in (0..MAX_VERTEX_ATTRIBS).filter(|i| per_instance & (1u32 << i) != 0) {
            let ga = c.attribs[i];
            match instance_attrib_value(c, &ga, inst as GLuint) {
                Some(value) => c.current_attrib[i] = value,
                None => {
                    c.set_error(GL_INVALID_OPERATION, FUNC);
                    break 'instances;
                }
            }
        }
        /* gl_InstanceID for this instance (GlesDraw::instance_id) */
        DRAW_INSTANCE_ID.store(inst as u32, Ordering::Relaxed);
        draw_once(c, st);
    }
    DRAW_INSTANCE_ID.store(0, Ordering::Relaxed);
    for i in (0..MAX_VERTEX_ATTRIBS).filter(|i| per_instance & (1u32 << i) != 0) {
        c.attribs[i].enabled = save[i].enabled;
        c.current_attrib[i] = save[i].value;
    }
}

#[no_mangle]
pub unsafe extern "C" fn glDrawArraysInstanced(mode: GLenum, first: GLint, count: GLsizei, instancecount: GLsizei) {
    gl_trace!(glDrawArraysInstanced);
    draw_instanced(context::current(), mode, first, count, 0, core::ptr::null(), instancecount);
}

#[no_mangle]
pub unsafe extern "C" fn glDrawElementsInstanced(
    mode: GLenum,
    count: GLsizei,
    type_: GLenum,
    indices: *const c_void,
    instancecount: GLsizei,
) {
    gl_trace!(glDrawElementsInstanced);
    let c = context::current();
    if type_ != GL_UNSIGNED_SHORT && type_ != GL_UNSIGNED_BYTE && type_ != GL_UNSIGNED_INT {
        return raise(c, GL_INVALID_ENUM, "glDrawElementsInstanced");
    }
    draw_instanced(c, mode, 0, count, type_, indices, instancecount);
}

#[no_mangle]
pub unsafe extern "C" fn glDrawRangeElements(
    mode: GLenum,
    start: GLuint,
    end: GLuint,
    count: GLsizei,
    type_: GLenum,
    indices: *const c_void,
) {
    gl_trace!(glDrawRangeElements);
    let c = context::current();
    if end < start {
        return raise(c, GL_INVALID_VALUE, "glDrawRangeElements");
    }
    draw_elements(c, STATE.get(), mode, count, type_, indices); /* the range is a hint */
}

/// The draw path keeps grow-only staging (index / per-attribute scratch) in
/// a file static that outlives a TestGl, while the mock OSAL frees every
/// allocation with the guard: forget the descriptors on both sides of a
/// test so that no test stages into the previous test's freed memory
/// (testutil::TestGl calls this when it is created and dropped).
#[cfg(test)]
pub(crate) fn reset_for_tests(also_programs: bool) {
    // SAFETY: tests holding a TestGl are serialised; no GL call is running.
    unsafe {
        let st = STATE.get();
        st.index_scratch = OsalGpuMem::ZERO;
        st.attrib_scratch = [OsalGpuMem::ZERO; MAX_VERTEX_ATTRIBS];
        st.dup_scratch = [OsalGpuMem::ZERO; MAX_DRAW_ATTRIBS];
        st.index_kick = 0;
        st.attrib_kick = [0; MAX_VERTEX_ATTRIBS];
        st.dup_kick = [0; MAX_DRAW_ATTRIBS];
        st.draw_fail_reported = 0;
        st.index_reported = 0;
        if also_programs {
            // every field of ProgGpu is valid when all-zero
            core::ptr::write_bytes(&mut st.pg as *mut [[ProgGpu; MAX_PROGRAMS]; MAX_CONTEXTS], 0, 1);
        }
        core::ptr::write_bytes(INSTANCED.as_ptr(), 0, 1); /* valid when all-zero */
        DRAW_INSTANCES.store(1, Ordering::Relaxed);
        VARIANT_OOM_LOGGED.store(false, Ordering::Relaxed);
        VARIANT_NO_ROOM_LOGGED.store(false, Ordering::Relaxed);
        ATTRIB_RANGE_LOGS.store(8, Ordering::Relaxed);
        CLIENT_ARRAY_LOGS.store(3, Ordering::Relaxed);
        core::ptr::write_bytes(CUBES.as_ptr(), 0, 1); /* valid when all-zero: nothing seen */
        *PENDING.get() = [PendingClear::NONE; MAX_CONTEXTS];
        gl_debug_attrib_dump = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gl::{buffer::*, framebuffer::*, program::*, state::*, texture::*, vertex::*};
    use crate::testutil::{CacheOp, MemEvent, RecordedDraw, TestGl, FILL, HEAP_DATA, HEAP_SHADER};
    use std::vec::Vec;

    /* ---- fixtures ---------------------------------------------------------- */

    use super::reset_for_tests as reset_draw_statics;

    struct Gl(TestGl);

    impl Gl {
        fn new(w: u32, h: u32) -> Gl {
            let gl = TestGl::new(w, h);
            reset_draw_statics(true);
            Gl(gl)
        }
    }

    impl core::ops::Deref for Gl {
        type Target = TestGl;
        fn deref(&self) -> &TestGl {
            &self.0
        }
    }

    impl Drop for Gl {
        fn drop(&mut self) {
            reset_draw_statics(false); /* runs before the TestGl is dropped */
        }
    }

    /// An `isampler*` / `usampler*` reads the binding of its dimension:
    /// the cube map, the 2D array, the 3D texture or the 2D texture of the
    /// unit, as the float sampler of that dimension does (ES 3.0 2.11.9).
    #[test]
    fn an_integer_sampler_reads_the_binding_of_its_dimension() {
        for (t, want) in [
            (GL_INT_SAMPLER_2D, GL_SAMPLER_2D as u16),
            (GL_UNSIGNED_INT_SAMPLER_2D, GL_SAMPLER_2D as u16),
            (GL_INT_SAMPLER_3D, GL_SAMPLER_3D_T),
            (GL_UNSIGNED_INT_SAMPLER_3D, GL_SAMPLER_3D_T),
            (GL_INT_SAMPLER_CUBE, GL_SAMPLER_CUBE_T),
            (GL_UNSIGNED_INT_SAMPLER_CUBE, GL_SAMPLER_CUBE_T),
            (GL_INT_SAMPLER_2D_ARRAY, GL_SAMPLER_2D_ARRAY_T),
            (GL_UNSIGNED_INT_SAMPLER_2D_ARRAY, GL_SAMPLER_2D_ARRAY_T),
        ] {
            assert_eq!(binding_type(t as u16), want, "0x{t:x}");
        }
        /* the float ones are their own */
        for t in [GL_SAMPLER_2D as u16, GL_SAMPLER_CUBE_T, GL_SAMPLER_2D_ARRAY_SHADOW_T, GL_SAMPLER_3D_T] {
            assert_eq!(binding_type(t), t);
        }
    }

    /* ---- what the draw carries for a backend with fixed-function state ------ */

    #[test]
    #[cfg(not(feature = "gpu-powervr"))] /* a PowerVR build refuses a draw whose variant it lacks */
    fn the_draw_carries_the_blend_state_and_the_colour_mask() {
        /* The PowerVR blends in the fragment shader variant and reads none of
         * this; a GPU that blends in hardware (the Vivante PE) needs the
         * factors, the equations, the blend colour and the mask. */
        let gl = Gl::new(16, 16);
        basic(&gl);
        // SAFETY: plain GL calls.
        unsafe {
            glEnable(GL_BLEND);
            glBlendFuncSeparate(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA, GL_ONE, GL_ZERO);
            glBlendEquationSeparate(GL_FUNC_SUBTRACT, GL_FUNC_ADD);
            glBlendColor(0.25, 0.5, 0.75, 1.0);
            glColorMask(1, 0, 1, 0);
        }
        let d = draw_tri(&gl).draw;
        assert!(d.blend);
        assert_eq!(
            (d.blend_src_rgb as u32, d.blend_dst_rgb as u32),
            (map_blend_factor(GL_SRC_ALPHA), map_blend_factor(GL_ONE_MINUS_SRC_ALPHA))
        );
        assert_eq!(
            (d.blend_src_alpha as u32, d.blend_dst_alpha as u32),
            (map_blend_factor(GL_ONE), map_blend_factor(GL_ZERO))
        );
        assert_eq!(d.blend_eq_rgb as u32, SHC_BLEND_SUBTRACT);
        assert_eq!(d.blend_eq_alpha as u32, SHC_BLEND_ADD);
        assert_eq!(d.blend_color, [0.25, 0.5, 0.75, 1.0]);
        assert_eq!(d.color_mask, 0b0101, "bit 0 R, 1 G, 2 B, 3 A");
    }

    /// Each section of the GL core's draw is counted under its step
    /// (draw_steps.rs) and the build is reported to the OSAL once: with a
    /// clock that moves 1000 ns a read, each section's one lap is 1000 ns,
    /// and the mock backend times nothing.
    #[test]
    fn a_draw_reports_each_gl_core_section_under_its_step() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        gl.set_clock(Some((1_000_000, 1000)));
        gl.draw_stats_taken();
        let _ = draw_tri(&gl);
        let got = gl.draw_stats_taken();
        assert_eq!(got.len(), 1, "one report for one draw: {got:?}");
        for s in [Step::GlProgram, Step::GlUniforms, Step::GlAttribs, Step::GlTextures, Step::GlState] {
            assert_eq!(got[0].ns[s as usize], 1000, "{s:?}: {got:?}");
        }
        assert_eq!(got[0].ns.iter().sum::<u64>(), 5000, "the mock backend times nothing: {got:?}");
        assert_eq!(got[0].untimed, 0);
        gl.set_clock(None);
    }

    #[test]
    fn a_clear_only_kick_writes_every_channel() {
        let gl = Gl::new(16, 16);
        // SAFETY: plain GL calls.
        unsafe {
            glClearColor(1.0, 0.0, 0.0, 1.0);
            glClear(GL_COLOR_BUFFER_BIT);
            glFlush();
        }
        let d = one(&gl).draw;
        assert!(d.do_clear && d.color_write);
        assert_eq!(d.color_mask, 0xf);
    }

    #[test]
    fn the_draw_says_how_wide_an_index_is() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        assert_eq!(draw_tri(&gl).draw.index_size, 0, "a sequential draw has no index buffer");
        let idx: [u16; 3] = [0, 1, 2];
        buffer_with(GL_ELEMENT_ARRAY_BUFFER, &[0, 0, 1, 0, 2, 0]);
        // SAFETY: the bound index buffer holds three u16 indices.
        unsafe { glDrawElements(GL_TRIANGLES, 3, GL_UNSIGNED_SHORT, core::ptr::null()) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(one(&gl).draw.index_size, 2);
        /* a u8 array is widened to u16 by the GL core, so it stays 2 */
        // SAFETY: `idx` outlives the draw; three u8 indices.
        unsafe {
            glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, 0);
            let bytes: [u8; 3] = [0, 1, 2];
            glDrawElements(GL_TRIANGLES, 3, GL_UNSIGNED_BYTE, bytes.as_ptr() as *const c_void);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(one(&gl).draw.index_size, 2);
        let _ = idx;
    }

    #[test]
    fn a_short_array_reaches_a_backend_that_fetches_it_untouched() {
        /* The GL core converts an array to float32 only when the backend
         * cannot read its source format (native_attrib_formats). */
        let gl = Gl::new(16, 16);
        let prog = gl.program("passthrough.vert", "uniform_color.frag");
        /* three vertices of four normalised shorts */
        let verts: [i16; 12] = [-32767, -32767, 0, 32767, 32767, -32767, 0, 32767, 0, 32767, 0, 32767];
        let bytes: Vec<u8> = verts.iter().flat_map(|v| v.to_le_bytes()).collect();
        let vbo = buffer_with(GL_ARRAY_BUFFER, &bytes);
        // SAFETY: plain GL calls; the VBO holds the three vertices.
        unsafe {
            glUseProgram(prog);
            glVertexAttribPointer(0, 4, GL_SHORT, 1, 0, core::ptr::null());
            glEnableVertexAttribArray(0);
        }
        /* the mock takes float32 only: the array is converted into staging */
        let a = draw_tri(&gl).draw.attribs[0];
        assert_eq!(a.format, GLES_ATTRIBF_FLOAT32);
        assert_eq!((a.normalized, a.stride), (0, 16));
        assert_ne!(a.cpu, core::ptr::null(), "converted into the core's staging");

        gl.backend_like_gc7000();
        /* The buffer holds exactly three tightly packed elements of eight
         * bytes. Upstream's range check sizes the last element in float32
         * whatever the source format, so it refuses this draw; here the
         * element is the source format's (see build_common). */
        let a = draw_tri(&gl).draw.attribs[0];
        assert_eq!(a.format, GLES_ATTRIBF_SHORT, "read where it lies");
        assert_eq!((a.normalized, a.stride), (1, 8), "four shorts, tightly packed");
        assert_eq!(a.cpu, core::ptr::null(), "nothing was staged");
        assert_eq!(a.buffer_gpu, gl.ctx().objs().buffers[vbo as usize].mem.gpu);
    }

    #[test]
    fn a_none_draw_buffer_on_the_window_writes_and_clears_no_colour() {
        /* ES 3.0 4.2.1: "Specifying NONE as the draw buffer for a fragment
         * color will inhibit that fragment color from being written." The
         * window's glDrawBuffers({GL_NONE}) was accepted and ignored (review
         * R-22). Depth is still cleared and written. */
        let gl = Gl::new(16, 16);
        basic(&gl);
        // SAFETY: plain GL calls; one readable enum.
        unsafe {
            glDrawBuffers(1, [GL_NONE].as_ptr());
            glClearColor(1.0, 0.0, 0.0, 1.0);
            glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
        }
        let d = draw_tri(&gl).draw;
        assert!(!d.color_write && d.color_mask == 0, "no colour written");
        assert!(!d.do_clear, "the colour buffer is not cleared");
        assert!(d.do_clear_depth, "the depth buffer is");
        // SAFETY: one readable enum.
        unsafe { glDrawBuffers(1, [GL_BACK].as_ptr()) };
        let d = draw_tri(&gl).draw;
        assert!(d.color_write && d.color_mask == 0xf, "GL_BACK writes again");
    }

    /// A clear is a command of the context that issued it (review R-04).
    /// The pending clear was one process-wide record, and the C API's
    /// gl_context_set_current did not flush it: context B's next draw took
    /// context A's clear (A's colour in B's target) and A lost it. Now each
    /// context has its own, and a switch through the C API flushes the
    /// outgoing context's clear as eglMakeCurrent does; destroying the
    /// current context carries its clear out; destroying another context
    /// leaves the current one's pending clear alone.
    #[test]
    fn a_pending_clear_stays_with_the_context_that_issued_it() {
        let gl = Gl::new(8, 8);
        let a: *mut GlContext = gl.ctx();
        let a_surface = gl.ctx().default_surface.gpu;
        let clear = |rgba: [f32; 4]| {
            // SAFETY: plain GL calls.
            unsafe {
                glClearColor(rgba[0], rgba[1], rgba[2], rgba[3]);
                crate::gl::state::glClear(GL_COLOR_BUFFER_BIT);
            }
        };
        let kick = |gl: &Gl| {
            let d = one(gl).draw;
            assert!(d.do_clear, "a clear-only kick");
            (d.clear_rgba, d.target_gpu)
        };
        clear([1.0, 0.0, 0.0, 1.0]);
        assert!(gl.draws().is_empty(), "A's clear is pending");

        /* gl_context_create makes B current: A's clear goes into A */
        // SAFETY: plain C API call; the pool has a free slot.
        let b = unsafe { context::gl_context_create(8, 8) };
        assert!(!b.is_null());
        // SAFETY: b is live.
        let b_surface = unsafe { (*b).default_surface.gpu };
        assert_ne!(a_surface, b_surface);
        assert_eq!(kick(&gl), ([1.0, 0.0, 0.0, 1.0], a_surface), "A's clear, into A's surface");
        basic(&gl); /* B's own program */
        let d = draw_tri(&gl).draw;
        assert!(!d.do_clear && d.target_gpu == b_surface, "B's draw takes no clear of A's");

        /* B clears blue; the C API switch back to A carries it out in B */
        clear([0.0, 0.0, 1.0, 1.0]);
        // SAFETY: a is live.
        unsafe { context::gl_context_set_current(a) };
        assert_eq!(kick(&gl), ([0.0, 0.0, 1.0, 1.0], b_surface), "B's clear, into B's surface");
        basic(&gl);
        let d = draw_tri(&gl).draw;
        assert!(!d.do_clear && d.target_gpu == a_surface, "A's draw takes no clear of B's");

        /* destroying B leaves A's pending clear alone */
        clear([0.0, 1.0, 0.0, 1.0]);
        // SAFETY: b is live and not current.
        unsafe { context::gl_context_destroy(b) };
        assert!(gl.draws().is_empty() && pending_clear().color, "A's clear still pending");
        // SAFETY: plain GL call.
        unsafe { crate::gl::state::glFlush() };
        assert_eq!(kick(&gl), ([0.0, 1.0, 0.0, 1.0], a_surface), "and not lost");

        /* destroying the current context carries its clear out first */
        // SAFETY: plain C API call; B's slot is free again.
        let b = unsafe { context::gl_context_create(8, 8) };
        // SAFETY: b is live.
        let b_surface = unsafe { (*b).default_surface.gpu };
        clear([1.0, 1.0, 0.0, 1.0]);
        let finishes = gl.finishes();
        // SAFETY: b is live and current.
        unsafe { context::gl_context_destroy(b) };
        assert_eq!(kick(&gl), ([1.0, 1.0, 0.0, 1.0], b_surface), "B's last clear is executed");
        assert!(gl.finishes() > finishes, "and finished before its surface is freed");
        // SAFETY: a is live.
        unsafe { context::gl_context_set_current(a) };
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn deleting_a_bound_attribute_buffer_resets_the_attribute_binding() {
        /* ES 2.0.25 2.9: "If a buffer object is deleted while it is bound,
         * all bindings to that object in the current context ... are reset
         * to zero." The attribute kept the deleted slot (review R-24): the
         * binding query answered the dead name, and once glGenBuffers reused
         * the slot the attribute read an unrelated buffer without an error. */
        let gl = Gl::new(16, 16);
        let prog = gl.program("passthrough.vert", "uniform_color.frag");
        let vbo = buffer_with(GL_ARRAY_BUFFER, dword_bytes(&TRI));
        // SAFETY: plain GL calls; the VBO holds the three vertices.
        unsafe {
            glUseProgram(prog);
            glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, core::ptr::null());
            glEnableVertexAttribArray(0);
        }
        draw_tri(&gl);
        let binding = || {
            let mut v: GLint = -1;
            // SAFETY: one GLint out.
            unsafe { crate::gl::vertex::glGetVertexAttribiv(0, GL_VERTEX_ATTRIB_ARRAY_BUFFER_BINDING, &mut v) };
            v
        };
        assert_eq!(binding(), vbo as GLint);
        // SAFETY: one valid name.
        unsafe { glDeleteBuffers(1, &vbo) };
        assert_eq!(binding(), 0, "the attribute's buffer binding is reset to zero");
        assert_eq!(gl.ctx().bound_array_buffer, 0);

        /* A zero binding sources the array from client memory at its pointer
         * (2.9), and the pointer is the old offset 0: refused by name, not
         * read at address 0. */
        let refused = |gl: &Gl| {
            // SAFETY: the draw is refused before any array is read.
            unsafe { glDrawArrays(GL_TRIANGLES, 0, 3) };
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
            assert!(gl.draws().is_empty(), "nothing reached the backend");
        };
        refused(&gl);
        assert!(gl.logs().iter().any(|l| l.contains("attribute 0 is a client array at 0x0")), "{:?}", gl.logs());

        /* the slot comes back to glGenBuffers: the attribute does not follow it */
        let other = buffer_with(GL_ARRAY_BUFFER, &[0u8; 64]);
        assert_eq!(other, vbo, "the name is reused (else this step proves nothing)");
        refused(&gl);
        assert_eq!(binding(), 0);

        /* a real client pointer draws */
        // SAFETY: TRI is a static that outlives the draw.
        unsafe {
            glBindBuffer(GL_ARRAY_BUFFER, 0);
            glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, TRI.as_ptr() as *const c_void);
        }
        assert!(!draw_tri(&gl).draw.attribs[0].cpu.is_null(), "staged from the client array");
    }

    /// Turns the attribute/index dump on for a scope (also when it panics).
    struct AttribDump;

    impl AttribDump {
        fn on() -> AttribDump {
            // SAFETY: tests are serialised by the TestGl the caller holds.
            unsafe { gl_debug_set_attrib_dump(1) };
            AttribDump
        }
    }

    impl Drop for AttribDump {
        fn drop(&mut self) {
            // SAFETY: as above.
            unsafe { gl_debug_set_attrib_dump(0) };
        }
    }

    static TRI: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 1.0, -1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0];

    /// Does the linked vertex stage carry code of its own for the GL core to
    /// upload? Not on the V3D: it compiles a program, not a stage, so its VS
    /// record is reflection only (`usc_dw` 0, nothing uploaded, `vs_usc_gpu`
    /// 0) and its three QPU programs travel in the FS record
    /// (src/shaderc/online/v3d_abi.rs, `stage_shader` / `program_shader`).
    const VS_HAS_OWN_CODE: bool = !cfg!(feature = "gpu-v3d");

    /// passthrough.vert (a_pos @0 -> vi0, a_color @1 -> vi4) + uniform_color.frag,
    /// current, with a_pos fed from the client array TRI and a_color disabled.
    fn basic(gl: &Gl) -> GLuint {
        let prog = gl.program("passthrough.vert", "uniform_color.frag");
        // SAFETY: plain GL calls; TRI is a static.
        unsafe {
            glUseProgram(prog);
            glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, TRI.as_ptr() as *const c_void);
            glEnableVertexAttribArray(0);
        }
        prog
    }

    fn one(gl: &Gl) -> RecordedDraw {
        let mut v = gl.draws();
        assert_eq!(v.len(), 1, "exactly one draw expected");
        v.pop().unwrap()
    }

    /// A triangle the draw path refuses: no GL error, nothing reaches the
    /// backend. Returns how many refusals it counted. (The GPUs that blend
    /// in fixed function ask for no variant, so nothing there refuses.)
    #[cfg(feature = "gpu-powervr")]
    fn refused_tri(gl: &Gl) -> u32 {
        let before = crate::backend::draws_refused();
        // SAFETY: the arrays the fixtures set up are statics.
        unsafe { glDrawArrays(GL_TRIANGLES, 0, 3) };
        assert_eq!(gl.error(), GL_NO_ERROR, "a refusal is no GL error");
        assert!(gl.draws().is_empty(), "nothing reached the backend");
        crate::backend::draws_refused().wrapping_sub(before)
    }

    fn draw_tri(gl: &Gl) -> RecordedDraw {
        // SAFETY: the arrays the fixtures set up are statics.
        unsafe { glDrawArrays(GL_TRIANGLES, 0, 3) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        one(gl)
    }

    /// A buffer object holding `bytes`, left bound to `target`.
    fn buffer_with(target: GLenum, bytes: &[u8]) -> GLuint {
        let mut b: GLuint = 0;
        // SAFETY: valid out pointer; `bytes` covers the size passed.
        unsafe {
            glGenBuffers(1, &mut b);
            glBindBuffer(target, b);
            glBufferData(target, bytes.len() as GLsizeiptr, bytes.as_ptr() as *const c_void, GL_STATIC_DRAW);
        }
        b
    }

    /// A w x h RGBA8 TEXTURE_2D of zero texels, left bound to the active unit.
    fn texture_rgba(w: i32, h: i32) -> GLuint {
        let mut t: GLuint = 0;
        let texels = std::vec![0u8; (w * h * 4) as usize];
        // SAFETY: valid out pointer; `texels` holds the image (rows of 4n bytes need no alignment).
        unsafe {
            glGenTextures(1, &mut t);
            glBindTexture(GL_TEXTURE_2D, t);
            glTexImage2D(
                GL_TEXTURE_2D,
                0,
                GL_RGBA as GLint,
                w,
                h,
                0,
                GL_RGBA,
                GL_UNSIGNED_BYTE,
                texels.as_ptr() as *const c_void,
            );
        }
        t
    }

    /// Detach a texture from the storage it borrows before the context goes:
    /// gl_context_destroy would free it (a known bug, see
    /// egl::tests::destroying_the_context_leaves_the_images_storage_alone).
    fn forget_external(gl: &Gl, t: GLuint) {
        let t = &mut gl.ctx().objs().textures[t as usize];
        assert!(t.external);
        t.mem = OsalGpuMem::ZERO;
    }

    /// A framebuffer with an 8x8 RGBA8 colour texture, left bound. Returns (fbo, texture).
    fn fbo_8x8() -> (GLuint, GLuint) {
        let tex = texture_rgba(8, 8);
        let mut fbo: GLuint = 0;
        // SAFETY: valid out pointer, plain GL calls.
        unsafe {
            glGenFramebuffers(1, &mut fbo);
            glBindFramebuffer(GL_FRAMEBUFFER, fbo);
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
            assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
        }
        (fbo, tex)
    }

    /* ---- the channel order of a texture the pixel engine rendered into ----
     *
     * On a backend whose RGBA8888 target is B,G,R,A (the GC7000), a texture
     * a draw rendered into holds B,G,R,A texels and one the CPU uploaded
     * holds R,G,B,A. The GL core keeps which (`GlTexture::pe_order`,
     * texture::take_pe_order) and the binding's `bgra` carries it. The
     * GC7000 backend used to guess it from a ring of the last 8 render-target
     * addresses, wrong three ways, all set by the order of the draws (dEQP
     * fbo.render on the i.MX8MP, order-dependent at da80e725). */

    /// Render one triangle into `tex` through a framebuffer of its own,
    /// sampling `src` (never `tex` itself: no feedback loop).
    fn render_into(tex: GLuint, src: GLuint) {
        let mut fbo: GLuint = 0;
        // SAFETY: valid out pointer, plain GL calls; TRI is a static.
        unsafe {
            glGenFramebuffers(1, &mut fbo);
            glBindFramebuffer(GL_FRAMEBUFFER, fbo);
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
            assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
            glBindTexture(GL_TEXTURE_2D, src);
            glDrawArrays(GL_TRIANGLES, 0, 3);
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteFramebuffers(1, &fbo);
        }
    }

    /// Sample `tex` in a draw to the default framebuffer: its binding.
    fn sampled(gl: &Gl, tex: GLuint) -> GlesTextureBind {
        // SAFETY: plain GL calls; TRI is a static.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glBindTexture(GL_TEXTURE_2D, tex);
            glDrawArrays(GL_TRIANGLES, 0, 3);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        gl.draws().pop().expect("the draw").draw.textures[0]
    }

    /// Hazard 1: a texture rendered into is B,G,R,A for as long as nothing
    /// re-specifies it, however many other targets are rendered after it.
    #[test]
    fn a_rendered_texture_stays_in_the_pixel_engines_order_after_other_targets() {
        let gl = Gl::new(16, 16);
        gl.backend_like_gc7000();
        textured(&gl);
        let src = texture_rgba(8, 8);
        let t = texture_rgba(8, 8);
        render_into(t, src);
        for _ in 0..9 {
            render_into(texture_rgba(8, 8), src);
        }
        assert_eq!(sampled(&gl, t).bgra, 1, "B,G,R,A: the pixel engine wrote it");
        assert_eq!(sampled(&gl, src).bgra, 0, "R,G,B,A: only the CPU wrote it");
    }

    /// Hazard 3: glTexSubImage2D into a texture the pixel engine wrote puts
    /// its texels in the same B,G,R,A order, so the whole texture has one
    /// order (dEQP fbo.render.texsubimage.{after,between}_render_tex2d_*).
    #[test]
    fn a_sub_image_upload_into_a_rendered_texture_takes_its_order() {
        let gl = Gl::new(16, 16);
        gl.backend_like_gc7000();
        textured(&gl);
        let src = texture_rgba(8, 8);
        let t = texture_rgba(8, 8);
        render_into(t, src);
        let texel = [0x11u8, 0x22, 0x33, 0x44];
        // SAFETY: a 1x1 RGBA image; plain GL calls.
        unsafe {
            glBindTexture(GL_TEXTURE_2D, t);
            glTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, texel.as_ptr() as *const c_void);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        let tex = &gl.ctx().objs().textures[t as usize];
        // SAFETY: level 0's first texel lies inside the live CPU mapping.
        let stored =
            unsafe { core::slice::from_raw_parts((tex.mem.cpu as *const u8).add(tex.level_off[0] as usize), 4) };
        assert_eq!(stored, &[0x33, 0x22, 0x11, 0x44], "stored B,G,R,A like the rest");
        assert_eq!(sampled(&gl, t).bgra, 1);
    }

    /// The first render converts every texel the CPU wrote, not only the
    /// ones the draw covers: the pixel engine leaves the rest as they were,
    /// so a flag alone would leave the texture half in each order.
    #[test]
    fn the_first_render_into_a_texture_converts_all_its_texels() {
        let gl = Gl::new(16, 16);
        gl.backend_like_gc7000();
        textured(&gl);
        let src = texture_rgba(8, 8);
        let texels: Vec<u8> = (0..8 * 8).flat_map(|i| [i as u8, 0x80, 0xf0 - i as u8, 0x55]).collect();
        let t = texture_rgba(8, 8);
        // SAFETY: 8x8 RGBA texels; plain GL calls.
        unsafe {
            glBindTexture(GL_TEXTURE_2D, t);
            glTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 8, 8, GL_RGBA, GL_UNSIGNED_BYTE, texels.as_ptr() as *const c_void);
        }
        render_into(t, src);
        let tex = &gl.ctx().objs().textures[t as usize];
        assert!(tex.pe_order);
        // SAFETY: level 0 (8x8, pitch 8) lies inside the live CPU mapping.
        let stored = unsafe {
            core::slice::from_raw_parts((tex.mem.cpu as *const u8).add(tex.level_off[0] as usize), 8 * 8 * 4)
        };
        for i in 0..64usize {
            assert_eq!(&stored[i * 4..i * 4 + 4], &[0xf0 - i as u8, 0x80, i as u8, 0x55], "texel {i}: B,G,R,A");
        }
    }

    /// Not on a backend whose RGBA8888 target is R,G,B,A (the PowerVR): the
    /// texels stay as the CPU wrote them and nothing is flagged.
    #[test]
    fn a_rendered_texture_keeps_the_cpus_order_where_the_target_is_rgba() {
        let gl = Gl::new(16, 16);
        textured(&gl);
        let src = texture_rgba(8, 8);
        let t = texture_rgba(8, 8);
        render_into(t, src);
        assert!(!gl.ctx().objs().textures[t as usize].pe_order);
        assert_eq!(sampled(&gl, t).bgra, 0);
    }

    /// glReadPixels of a texture target gives back the texels as GL holds
    /// them whichever order they are stored in: R,G,B,A as the CPU wrote them
    /// before any render (it used to swap them, taking every RGBA8888 target
    /// on this backend for B,G,R,A), and the same after the first render
    /// converted the storage.
    #[test]
    fn reading_a_texture_target_back_is_right_before_and_after_the_first_render() {
        let gl = Gl::new(16, 16);
        gl.backend_like_gc7000();
        textured(&gl);
        let texels: Vec<u8> = (0..8 * 8).flat_map(|i| [i as u8, 0x80, 0xf0 - i as u8, 0x55]).collect();
        let t = texture_rgba(8, 8);
        // SAFETY: 8x8 RGBA texels; plain GL calls.
        unsafe {
            glBindTexture(GL_TEXTURE_2D, t);
            glTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 8, 8, GL_RGBA, GL_UNSIGNED_BYTE, texels.as_ptr() as *const c_void);
        }
        let read = || {
            let mut fbo: GLuint = 0;
            let mut out = std::vec![0u8; 8 * 8 * 4];
            // SAFETY: plain GL calls; `out` holds 8x8 RGBA.
            unsafe {
                glGenFramebuffers(1, &mut fbo);
                glBindFramebuffer(GL_FRAMEBUFFER, fbo);
                glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, t, 0);
                glReadPixels(0, 0, 8, 8, GL_RGBA, GL_UNSIGNED_BYTE, out.as_mut_ptr() as *mut c_void);
                glBindFramebuffer(GL_FRAMEBUFFER, 0);
                glDeleteFramebuffers(1, &fbo);
            }
            out
        };
        assert_eq!(read(), texels, "never rendered: as uploaded");
        let src = texture_rgba(8, 8);
        let mut fbo: GLuint = 0;
        // SAFETY: plain GL calls; a triangle into t that the mock never rasterises.
        unsafe {
            glGenFramebuffers(1, &mut fbo);
            glBindFramebuffer(GL_FRAMEBUFFER, fbo);
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, t, 0);
            glBindTexture(GL_TEXTURE_2D, src);
            glDrawArrays(GL_TRIANGLES, 0, 3);
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
        }
        assert!(gl.ctx().objs().textures[t as usize].pe_order);
        assert_eq!(read(), texels, "rendered (converted): the same texels");
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    /// Storage the CPU never wrote (glTexImage2D with no data: a render
    /// target) is not converted at the first render -- its texels are
    /// undefined, and a pass over a 1080p target costs 10-100 ms -- only
    /// flagged (frontend4's review of ee1d5894).
    #[test]
    fn a_render_target_the_cpu_never_wrote_is_flagged_without_a_pass() {
        let gl = Gl::new(16, 16);
        gl.backend_like_gc7000();
        textured(&gl);
        let src = texture_rgba(8, 8);
        let mut t: GLuint = 0;
        // SAFETY: valid out pointer; null pixels only allocate.
        unsafe {
            glGenTextures(1, &mut t);
            glBindTexture(GL_TEXTURE_2D, t);
            glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA as GLint, 8, 8, 0, GL_RGBA, GL_UNSIGNED_BYTE, core::ptr::null());
        }
        let (cpu, off) = {
            let tex = &gl.ctx().objs().textures[t as usize];
            assert!(!tex.cpu_wrote, "allocated and cleared, nothing written");
            (tex.mem.cpu as *mut u8, tex.level_off[0] as usize)
        };
        /* what an undefined texel might hold: a pass would swap it */
        // SAFETY: level 0's first texel inside the live CPU mapping.
        unsafe {
            core::ptr::copy_nonoverlapping([1u8, 2, 3, 4].as_ptr(), cpu.add(off), 4);
            crate::osal::cache_flush(cpu.add(off) as *mut c_void, 4);
        }
        render_into(t, src);
        let tex = &gl.ctx().objs().textures[t as usize];
        assert!(tex.pe_order);
        // SAFETY: as above.
        assert_eq!(unsafe { core::slice::from_raw_parts(cpu.add(off), 4) }, &[1, 2, 3, 4], "no pass over it");
    }

    /// A level kept through a re-layout (levels uploaded before level 0,
    /// relayout_keeping_levels) keeps the texture's channel order with its
    /// bytes (frontend4's must-fix on ee1d5894): texels in the pixel engine's
    /// order, then a level 0 of a chain shape that keeps level 1 -- level 1's
    /// texels are still B,G,R,A and the texture says so. The state is set up
    /// by hand: a render into level 1 while level 0 is undefined is an
    /// incomplete framebuffer here, but an image's storage can arrive so.
    #[test]
    fn a_level_kept_through_a_relayout_keeps_the_channel_order() {
        let gl = Gl::new(16, 16);
        gl.backend_like_gc7000();
        textured(&gl);
        /* level 2 first, 1x2: the chain it implies has a 4x8 level 0; then
         * a 2x8 level 0, whose level 2 is 1x2 too (clamped to one texel
         * wide), so the re-layout keeps it */
        let l2: [u8; 8] = [0x10, 0x40, 0xc0, 0x77, 0x11, 0x41, 0xc1, 0x78];
        let mut t: GLuint = 0;
        // SAFETY: plain GL calls; `l2` is a 1x2 RGBA image.
        unsafe {
            glGenTextures(1, &mut t);
            glBindTexture(GL_TEXTURE_2D, t);
            glTexImage2D(
                GL_TEXTURE_2D,
                2,
                GL_RGBA as GLint,
                1,
                2,
                0,
                GL_RGBA,
                GL_UNSIGNED_BYTE,
                l2.as_ptr() as *const c_void,
            );
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        {
            /* as texture::take_pe_order leaves it: every texel B,G,R,A */
            let tex = &mut gl.ctx().objs().textures[t as usize];
            // SAFETY: the texture's whole CPU mapping.
            unsafe {
                let b = core::slice::from_raw_parts_mut(tex.mem.cpu as *mut u8, tex.mem.size & !3);
                for px in b.as_chunks_mut::<4>().0 {
                    px.swap(0, 2);
                }
                crate::osal::cache_flush(tex.mem.cpu, tex.mem.size);
            }
            tex.pe_order = true;
        }
        let l0 = std::vec![0u8; 2 * 8 * 4];
        // SAFETY: a 2x8 RGBA image.
        unsafe {
            glTexImage2D(
                GL_TEXTURE_2D,
                0,
                GL_RGBA as GLint,
                2,
                8,
                0,
                GL_RGBA,
                GL_UNSIGNED_BYTE,
                l0.as_ptr() as *const c_void,
            );
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        let tex = &gl.ctx().objs().textures[t as usize];
        assert!(tex.level_defined & 0b100 != 0, "level 2 was kept");
        assert!(tex.pe_order, "the kept level's bytes are still B,G,R,A");
        let pitch = tex.level_pitch(1) as usize * 4;
        let at = tex.level_off[2] as usize;
        // SAFETY: level 2's two rows inside the live CPU mapping.
        let lv = unsafe { core::slice::from_raw_parts((tex.mem.cpu as *const u8).add(at), pitch + 4) };
        assert_eq!(&lv[..4], &[0xc0, 0x40, 0x10, 0x77], "level 2 row 0");
        assert_eq!(&lv[pitch..pitch + 4], &[0xc1, 0x41, 0x11, 0x78], "level 2 row 1");
        assert_eq!(sampled(&gl, t).bgra, 1, "and the binding says so");
    }

    /// Hazard 2, the GL core's half (it holds today; the backend's half is
    /// vivante backend_tests' a_cpu_upload_at_a_rendered_address_...): a
    /// texture the CPU uploaded is R,G,B,A even when its storage is a block a
    /// rendered texture freed. Whether the block is reused is the heap's
    /// business; the test says which it saw.
    #[test]
    fn a_new_upload_is_in_the_cpus_order_whatever_storage_it_reuses() {
        let gl = Gl::new(16, 16);
        gl.backend_like_gc7000();
        textured(&gl);
        let src = texture_rgba(8, 8);
        let t = texture_rgba(8, 8);
        render_into(t, src);
        let old = gl.ctx().objs().textures[t as usize].mem.gpu;
        // SAFETY: a live name.
        unsafe { glDeleteTextures(1, &t) };
        let u = texture_rgba(8, 8);
        let b = sampled(&gl, u);
        std::println!("reused the rendered texture's storage: {}", b.base_gpu == old);
        assert_eq!(b.bgra, 0, "R,G,B,A: the CPU wrote it");
    }

    fn renderbuffer_8x8(internalformat: GLenum) -> GLuint {
        let mut rb: GLuint = 0;
        // SAFETY: valid out pointer, plain GL calls.
        unsafe {
            glGenRenderbuffers(1, &mut rb);
            glBindRenderbuffer(GL_RENDERBUFFER, rb);
            glRenderbufferStorage(GL_RENDERBUFFER, internalformat, 8, 8);
        }
        rb
    }

    fn f32s(b: &[u8]) -> Vec<f32> {
        b.as_chunks::<4>().0.iter().map(|c| f32::from_ne_bytes(*c)).collect()
    }

    fn u16s(b: &[u8]) -> Vec<u16> {
        b.as_chunks::<2>().0.iter().map(|c| u16::from_ne_bytes(*c)).collect()
    }

    fn bits(v: &[f32]) -> Vec<u32> {
        v.iter().map(|f| f.to_bits()).collect()
    }

    /// Sizes of the allocations made in the shader heap.
    fn shader_allocs(ev: &[MemEvent]) -> Vec<usize> {
        ev.iter()
            .filter_map(|e| match e {
                MemEvent::Alloc { heap: HEAP_SHADER, size, .. } => Some(*size),
                _ => None,
            })
            .collect()
    }

    fn frees(ev: &[MemEvent]) -> Vec<u64> {
        ev.iter()
            .filter_map(|e| match e {
                MemEvent::Free { gpu } => Some(*gpu),
                _ => None,
            })
            .collect()
    }

    fn attrib(type_: GLenum, size: GLint, normalized: bool, integer: bool) -> GlVertexAttrib {
        GlVertexAttrib {
            enabled: true,
            buffer: 0,
            size,
            type_,
            normalized: normalized as GLboolean,
            stride: 0,
            offset: 0,
            divisor: 0,
            integer,
        }
    }

    /// attrib_component() of a float (non-integer) array, as the float it encodes.
    fn comp(src: &[u8], type_: GLenum, normalized: bool) -> f32 {
        f32::from_bits(attrib_component(src, type_, normalized as GLboolean, false))
    }

    /* ---- enum mapping ------------------------------------------------------ */

    #[test]
    fn map_prim_covers_the_seven_es_modes() {
        assert_eq!(map_prim(GL_POINTS), Some(GLES_PRIM_POINTS));
        assert_eq!(map_prim(GL_LINES), Some(GLES_PRIM_LINES));
        assert_eq!(map_prim(GL_LINE_LOOP), Some(GLES_PRIM_LINE_LOOP));
        assert_eq!(map_prim(GL_LINE_STRIP), Some(GLES_PRIM_LINE_STRIP));
        assert_eq!(map_prim(GL_TRIANGLES), Some(GLES_PRIM_TRIANGLES));
        assert_eq!(map_prim(GL_TRIANGLE_STRIP), Some(GLES_PRIM_TRI_STRIP));
        assert_eq!(map_prim(GL_TRIANGLE_FAN), Some(GLES_PRIM_TRI_FAN));
    }

    /// ES 3.0 2.8.1's fixed-index restart, mode by mode: the pieces as the
    /// independent list the mode is made of, winding and provoking vertex
    /// kept, an incomplete primitive at a restart dropped.
    #[test]
    fn restart_to_lists_splits_every_mode() {
        const R: u32 = 0xffff;
        let run = |ix: &[u32], mode: GLenum| {
            let mut out = std::vec![0u32; ix.len() * 3];
            let (w, list) = restart_to_lists(ix.len(), |k| ix[k], R, mode, &mut out).expect("a mode");
            out.truncate(w);
            (out, list)
        };
        assert_eq!(run(&[0, 1, R, 2], GL_POINTS), (std::vec![0, 1, 2], GL_POINTS));
        assert_eq!(run(&[0, 1, 2, R, 3, 4], GL_LINES), (std::vec![0, 1, 3, 4], GL_LINES), "2 is a lone vertex");
        assert_eq!(run(&[0, 1, 2, R, 3, 4], GL_LINE_STRIP), (std::vec![0, 1, 1, 2, 3, 4], GL_LINES));
        assert_eq!(run(&[0, 1, 2, R, 3, 4], GL_LINE_LOOP), (std::vec![0, 1, 1, 2, 2, 0, 3, 4, 4, 3], GL_LINES));
        assert_eq!(run(&[0, 1, 2, 3, R, 4, 5, 6], GL_TRIANGLES), (std::vec![0, 1, 2, 4, 5, 6], GL_TRIANGLES));
        assert_eq!(
            run(&[0, 1, 2, 3, R, 4, 5, 6], GL_TRIANGLE_STRIP),
            (std::vec![0, 1, 2, 2, 1, 3, 4, 5, 6], GL_TRIANGLES),
            "the odd triangle is (k+1, k, k+2)"
        );
        assert_eq!(
            run(&[0, 1, 2, 3, R, 4, 5, 6], GL_TRIANGLE_FAN),
            (std::vec![0, 1, 2, 0, 2, 3, 4, 5, 6], GL_TRIANGLES)
        );
        assert_eq!(run(&[R, R, 0, R], GL_TRIANGLE_STRIP), (std::vec![], GL_TRIANGLES));
        let mut out = [0u32; 3];
        assert_eq!(restart_to_lists(1, |_| 0, R, GL_FLOAT, &mut out), None);
    }

    /// glDrawElements with GL_PRIMITIVE_RESTART_FIXED_INDEX over a client
    /// array of three vertices: the restart index ends the strip and is no
    /// vertex. Until 2026-09-24 the draw took 0xffff for a vertex and
    /// converted 65,536 of them from the three-vertex array (a segfault in
    /// dEQP-GLES3 primitive_restart.*.unsigned_short); 0xffffffff was
    /// refused with GL_INVALID_OPERATION.
    ///
    /// Revert check: take the restart block out of `draw_elements` and the
    /// staged indices and vertex count below are wrong (or the draw reads
    /// past TRI).
    #[test]
    fn primitive_restart_ends_the_primitive_and_is_no_vertex() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        // SAFETY: plain GL call.
        unsafe { crate::gl::state::glEnable(GL_PRIMITIVE_RESTART_FIXED_INDEX) };
        let idx: [u16; 7] = [0, 1, 2, 0xffff, 2, 1, 0];
        // SAFETY: `idx` outlives the call.
        unsafe { glDrawElements(GL_TRIANGLE_STRIP, 7, GL_UNSIGNED_SHORT, idx.as_ptr() as *const c_void) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        let d = one(&gl);
        assert_eq!(d.draw.prim, GLES_PRIM_TRIANGLES);
        assert_eq!(u16s(&d.index_data), [0, 1, 2, 2, 1, 0]);
        assert_eq!(f32s(&d.attrib_data[0]), TRI, "three vertices, not 65,536");

        let idx: [u32; 5] = [0, 1, 0xffff_ffff, 1, 2];
        // SAFETY: as above.
        unsafe { glDrawElements(GL_LINE_LOOP, 5, GL_UNSIGNED_INT, idx.as_ptr() as *const c_void) };
        assert_eq!(gl.error(), GL_NO_ERROR, "0xffffffff is the restart, not an index over 16 bits");
        let d = one(&gl);
        assert_eq!(d.draw.prim, GLES_PRIM_LINES);
        assert_eq!(u16s(&d.index_data), [0, 1, 1, 0, 1, 2, 2, 1]);

        /* in a buffer object too, where a u16 list is otherwise used in place */
        let idx: [u8; 5] = [0, 1, 2, 0xff, 0];
        buffer_with(GL_ELEMENT_ARRAY_BUFFER, &idx);
        // SAFETY: an offset into the bound buffer.
        unsafe { glDrawElements(GL_TRIANGLE_FAN, 5, GL_UNSIGNED_BYTE, core::ptr::null()) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(u16s(&one(&gl).index_data), [0, 1, 2]);

        /* off: 0xff is an index like any other */
        // SAFETY: plain GL call; `idx` outlives the draw.
        unsafe {
            crate::gl::state::glDisable(GL_PRIMITIVE_RESTART_FIXED_INDEX);
            glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, 0);
            glDrawElements(GL_TRIANGLES, 3, GL_UNSIGNED_BYTE, idx.as_ptr() as *const c_void);
        }
        assert_eq!(u16s(&one(&gl).index_data), [0, 1, 2]);
    }

    #[test]
    fn map_prim_rejects_modes_es_does_not_have() {
        assert_eq!(map_prim(0x0007), None); /* GL_QUADS of desktop GL */
        assert_eq!(map_prim(0x0009), None); /* GL_POLYGON */
        assert_eq!(map_prim(GL_FLOAT), None);
        assert_eq!(map_prim(u32::MAX), None);
    }

    #[test]
    fn map_cmp_maps_each_gl_function_by_name() {
        assert_eq!(map_cmp(GL_NEVER), GLES_CMP_NEVER);
        assert_eq!(map_cmp(GL_LESS), GLES_CMP_LESS);
        assert_eq!(map_cmp(GL_EQUAL), GLES_CMP_EQUAL);
        assert_eq!(map_cmp(GL_LEQUAL), GLES_CMP_LEQUAL);
        assert_eq!(map_cmp(GL_GREATER), GLES_CMP_GREATER);
        assert_eq!(map_cmp(GL_NOTEQUAL), GLES_CMP_NOTEQUAL);
        assert_eq!(map_cmp(GL_GEQUAL), GLES_CMP_GEQUAL);
        assert_eq!(map_cmp(GL_ALWAYS), GLES_CMP_ALWAYS);
    }

    #[test]
    fn map_cmp_out_of_range_is_always() {
        assert_eq!(map_cmp(0), GLES_CMP_ALWAYS);
        assert_eq!(map_cmp(0x01ff), GLES_CMP_ALWAYS);
        assert_eq!(map_cmp(0x0208), GLES_CMP_ALWAYS);
        assert_eq!(map_cmp(u32::MAX), GLES_CMP_ALWAYS);
    }

    #[test]
    fn map_stencil_op_maps_each_gl_op() {
        assert_eq!(map_stencil_op(GL_KEEP), GLES_SOP_KEEP);
        assert_eq!(map_stencil_op(GL_ZERO), GLES_SOP_ZERO);
        assert_eq!(map_stencil_op(GL_REPLACE), GLES_SOP_REPLACE);
        assert_eq!(map_stencil_op(GL_INCR), GLES_SOP_INCR_SAT, "GL_INCR clamps at the maximum");
        assert_eq!(map_stencil_op(GL_DECR), GLES_SOP_DECR_SAT, "GL_DECR clamps at zero");
        assert_eq!(map_stencil_op(GL_INVERT), GLES_SOP_INVERT);
        assert_eq!(map_stencil_op(GL_INCR_WRAP), GLES_SOP_INCR_WRAP);
        assert_eq!(map_stencil_op(GL_DECR_WRAP), GLES_SOP_DECR_WRAP);
    }

    #[test]
    fn map_stencil_op_unknown_is_keep() {
        assert_eq!(map_stencil_op(GL_ONE), GLES_SOP_KEEP);
        assert_eq!(map_stencil_op(u32::MAX), GLES_SOP_KEEP);
    }

    #[test]
    fn map_blend_factor_maps_each_gl_factor() {
        let table = [
            (GL_ZERO, SHC_BF_ZERO),
            (GL_ONE, SHC_BF_ONE),
            (GL_SRC_COLOR, SHC_BF_SRC_COLOR),
            (GL_ONE_MINUS_SRC_COLOR, SHC_BF_INV_SRC_COLOR),
            (GL_SRC_ALPHA, SHC_BF_SRC_ALPHA),
            (GL_ONE_MINUS_SRC_ALPHA, SHC_BF_INV_SRC_ALPHA),
            (GL_DST_ALPHA, SHC_BF_DST_ALPHA),
            (GL_ONE_MINUS_DST_ALPHA, SHC_BF_INV_DST_ALPHA),
            (GL_DST_COLOR, SHC_BF_DST_COLOR),
            (GL_ONE_MINUS_DST_COLOR, SHC_BF_INV_DST_COLOR),
            (GL_SRC_ALPHA_SATURATE, SHC_BF_SRC_ALPHA_SATURATE),
            (GL_CONSTANT_COLOR, SHC_BF_CONST_COLOR),
            (GL_ONE_MINUS_CONSTANT_COLOR, SHC_BF_INV_CONST_COLOR),
            (GL_CONSTANT_ALPHA, SHC_BF_CONST_ALPHA),
            (GL_ONE_MINUS_CONSTANT_ALPHA, SHC_BF_INV_CONST_ALPHA),
        ];
        for (gl, shc) in table {
            assert_eq!(map_blend_factor(gl), shc, "factor 0x{gl:x}");
        }
    }

    #[test]
    fn map_blend_factor_one_minus_is_the_invert_bit_of_the_plain_factor() {
        for (plain, inv) in [
            (GL_SRC_COLOR, GL_ONE_MINUS_SRC_COLOR),
            (GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA),
            (GL_DST_COLOR, GL_ONE_MINUS_DST_COLOR),
            (GL_DST_ALPHA, GL_ONE_MINUS_DST_ALPHA),
            (GL_CONSTANT_COLOR, GL_ONE_MINUS_CONSTANT_COLOR),
            (GL_CONSTANT_ALPHA, GL_ONE_MINUS_CONSTANT_ALPHA),
            (GL_ONE, GL_ZERO),
        ] {
            assert_eq!(map_blend_factor(inv), map_blend_factor(plain) | SHC_BF_INVERT);
        }
    }

    #[test]
    fn map_blend_factor_unknown_is_one() {
        assert_eq!(map_blend_factor(GL_FLOAT), SHC_BF_ONE);
        assert_eq!(map_blend_factor(u32::MAX), SHC_BF_ONE);
    }

    #[test]
    fn map_blend_eq_maps_the_es2_equations_and_defaults_to_add() {
        assert_eq!(map_blend_eq(GL_FUNC_ADD), SHC_BLEND_ADD);
        assert_eq!(map_blend_eq(GL_FUNC_SUBTRACT), SHC_BLEND_SUBTRACT);
        assert_eq!(map_blend_eq(GL_FUNC_REVERSE_SUBTRACT), SHC_BLEND_REVERSE_SUBTRACT);
        assert_eq!(map_blend_eq(GL_MIN), SHC_BLEND_MIN);
        assert_eq!(map_blend_eq(GL_MAX), SHC_BLEND_MAX);
        assert_eq!(map_blend_eq(0), SHC_BLEND_ADD);
    }

    /* ---- attribute conversion ---------------------------------------------- */

    #[test]
    fn attrib_type_size_is_the_gl_type_width() {
        assert_eq!(attrib_type_size(GL_BYTE), 1);
        assert_eq!(attrib_type_size(GL_UNSIGNED_BYTE), 1);
        assert_eq!(attrib_type_size(GL_SHORT), 2);
        assert_eq!(attrib_type_size(GL_UNSIGNED_SHORT), 2);
        assert_eq!(attrib_type_size(GL_INT), 4);
        assert_eq!(attrib_type_size(GL_UNSIGNED_INT), 4);
        assert_eq!(attrib_type_size(GL_FLOAT), 4);
        assert_eq!(attrib_type_size(GL_FIXED), 4);
    }

    #[test]
    fn attrib_component_float_passes_the_bits_through() {
        for f in [0.0f32, -0.0, 1.5, -123.25, f32::MAX, f32::MIN_POSITIVE, f32::INFINITY] {
            assert_eq!(attrib_component(&f.to_ne_bytes(), GL_FLOAT, 0, false), f.to_bits());
        }
        let nan = 0x7fc1_2345u32; /* a NaN payload must survive too */
        assert_eq!(attrib_component(&nan.to_ne_bytes(), GL_FLOAT, 0, false), nan);
    }

    #[test]
    fn attrib_component_unsigned_byte_unnormalized_is_the_value() {
        assert_eq!(comp(&[0], GL_UNSIGNED_BYTE, false), 0.0);
        assert_eq!(comp(&[200], GL_UNSIGNED_BYTE, false), 200.0);
        assert_eq!(comp(&[255], GL_UNSIGNED_BYTE, false), 255.0);
    }

    #[test]
    fn attrib_component_unsigned_byte_normalized_divides_by_255() {
        assert_eq!(comp(&[0], GL_UNSIGNED_BYTE, true), 0.0);
        assert_eq!(comp(&[255], GL_UNSIGNED_BYTE, true), 1.0);
        assert_eq!(comp(&[51], GL_UNSIGNED_BYTE, true), 51.0 / 255.0);
        assert_eq!(comp(&[128], GL_UNSIGNED_BYTE, true), 128.0 / 255.0);
    }

    #[test]
    fn attrib_component_byte_unnormalized_is_sign_extended() {
        assert_eq!(comp(&[0x7f], GL_BYTE, false), 127.0);
        assert_eq!(comp(&[0xff], GL_BYTE, false), -1.0);
        assert_eq!(comp(&[0x80], GL_BYTE, false), -128.0);
    }

    #[test]
    fn attrib_component_byte_normalized_follows_es3_and_clamps_the_minimum() {
        /* ES 3.0 2.1.6.1: f = max(c / (2^(b-1) - 1), -1) */
        assert_eq!(comp(&[127], GL_BYTE, true), 1.0);
        assert_eq!(comp(&[0], GL_BYTE, true), 0.0);
        assert_eq!(comp(&[64], GL_BYTE, true), 64.0 / 127.0);
        assert_eq!(comp(&[(-127i8) as u8], GL_BYTE, true), -1.0);
        assert_eq!(comp(&[(-128i8) as u8], GL_BYTE, true), -1.0);
        assert_eq!(comp(&[(-1i8) as u8], GL_BYTE, true), -1.0 / 127.0);
    }

    #[test]
    fn attrib_component_short_unnormalized_is_sign_extended() {
        assert_eq!(comp(&(-3i16).to_ne_bytes(), GL_SHORT, false), -3.0);
        assert_eq!(comp(&i16::MAX.to_ne_bytes(), GL_SHORT, false), 32767.0);
        assert_eq!(comp(&i16::MIN.to_ne_bytes(), GL_SHORT, false), -32768.0);
    }

    #[test]
    fn attrib_component_short_normalized_follows_es3_and_clamps_the_minimum() {
        assert_eq!(comp(&i16::MAX.to_ne_bytes(), GL_SHORT, true), 1.0);
        assert_eq!(comp(&(-32767i16).to_ne_bytes(), GL_SHORT, true), -1.0);
        assert_eq!(comp(&i16::MIN.to_ne_bytes(), GL_SHORT, true), -1.0);
        assert_eq!(comp(&16384i16.to_ne_bytes(), GL_SHORT, true), 16384.0 / 32767.0);
        assert_eq!(comp(&0i16.to_ne_bytes(), GL_SHORT, true), 0.0);
    }

    #[test]
    fn attrib_component_unsigned_short() {
        assert_eq!(comp(&65535u16.to_ne_bytes(), GL_UNSIGNED_SHORT, false), 65535.0);
        assert_eq!(comp(&65535u16.to_ne_bytes(), GL_UNSIGNED_SHORT, true), 1.0);
        assert_eq!(comp(&0u16.to_ne_bytes(), GL_UNSIGNED_SHORT, true), 0.0);
        assert_eq!(comp(&13107u16.to_ne_bytes(), GL_UNSIGNED_SHORT, true), 13107.0 / 65535.0);
    }

    #[test]
    fn attrib_component_fixed_is_16_16() {
        assert_eq!(comp(&0x0001_8000i32.to_ne_bytes(), GL_FIXED, false), 1.5);
        assert_eq!(comp(&(-0x0002_4000i32).to_ne_bytes(), GL_FIXED, false), -2.25);
        assert_eq!(comp(&1i32.to_ne_bytes(), GL_FIXED, false), 1.0 / 65536.0);
        /* normalisation does not apply to GL_FIXED */
        assert_eq!(comp(&0x0001_0000i32.to_ne_bytes(), GL_FIXED, true), 1.0);
    }

    #[test]
    fn attrib_component_int_of_a_float_attribute_is_converted_to_float() {
        /* ES 3.0 2.9: VertexAttribPointer data is "converted directly to
         * floating-point" (or normalised); only VertexAttribIPointer keeps integers. */
        assert_eq!(comp(&7i32.to_ne_bytes(), GL_INT, false), 7.0);
        assert_eq!(comp(&(-7i32).to_ne_bytes(), GL_INT, false), -7.0);
        assert_eq!(comp(&7u32.to_ne_bytes(), GL_UNSIGNED_INT, false), 7.0);
        assert_eq!(comp(&i32::MAX.to_ne_bytes(), GL_INT, true), 1.0);
        assert_eq!(comp(&u32::MAX.to_ne_bytes(), GL_UNSIGNED_INT, true), 1.0);
    }

    #[test]
    fn attrib_component_integer_attribute_is_widened_not_converted() {
        assert_eq!(attrib_component(&[0xfe], GL_BYTE, 0, true), (-2i32) as u32);
        assert_eq!(attrib_component(&[0xfe], GL_UNSIGNED_BYTE, 0, true), 254);
        assert_eq!(attrib_component(&(-300i16).to_ne_bytes(), GL_SHORT, 0, true), (-300i32) as u32);
        assert_eq!(attrib_component(&65000u16.to_ne_bytes(), GL_UNSIGNED_SHORT, 0, true), 65000);
        assert_eq!(attrib_component(&(-70000i32).to_ne_bytes(), GL_INT, 0, true), (-70000i32) as u32);
        assert_eq!(attrib_component(&0xdead_beefu32.to_ne_bytes(), GL_UNSIGNED_INT, 0, true), 0xdead_beef);
    }

    #[test]
    fn attrib_component_integer_attribute_ignores_normalized() {
        assert_eq!(attrib_component(&[255], GL_UNSIGNED_BYTE, 1, true), 255);
    }

    #[test]
    fn attrib_row_fills_missing_components_with_0_0_0_1() {
        let ga = attrib(GL_FLOAT, 2, false, false);
        let src: Vec<u8> = [3.0f32, -4.0].iter().flat_map(|f| f.to_ne_bytes()).collect();
        assert_eq!(attrib_row(&src, &ga, 2, 4, 4), [3.0f32, -4.0, 0.0, 1.0].map(f32::to_bits));
        /* a vec3 input: z defaults to 0, there is no w */
        assert_eq!(attrib_row(&src, &ga, 2, 4, 3), [3.0f32.to_bits(), (-4.0f32).to_bits(), 0, 0]);
    }

    #[test]
    fn attrib_row_single_component_array_into_vec4() {
        let ga = attrib(GL_UNSIGNED_BYTE, 1, true, false);
        assert_eq!(attrib_row(&[255], &ga, 1, 1, 4), [1.0f32, 0.0, 0.0, 1.0].map(f32::to_bits));
    }

    #[test]
    fn attrib_row_drops_components_the_input_does_not_read() {
        let ga = attrib(GL_UNSIGNED_BYTE, 4, false, false);
        assert_eq!(attrib_row(&[1, 2, 3, 4], &ga, 4, 1, 2), [1.0f32.to_bits(), 2.0f32.to_bits(), 0, 0]);
    }

    #[test]
    fn attrib_row_steps_by_the_type_size() {
        let ga = attrib(GL_SHORT, 3, false, false);
        let src: Vec<u8> = [10i16, -20, 30].iter().flat_map(|v| v.to_ne_bytes()).collect();
        assert_eq!(attrib_row(&src, &ga, 3, 2, 4), [10.0f32, -20.0, 30.0, 1.0].map(f32::to_bits));
    }

    #[test]
    fn dword_bytes_is_the_native_byte_image() {
        assert_eq!(dword_bytes(&[1.0f32]), &1.0f32.to_ne_bytes()[..]);
        assert_eq!(dword_bytes(&[0x0403_0201u32, 0x0807_0605]).len(), 8);
        assert_eq!(dword_bytes::<u32>(&[]).len(), 0);
    }

    /* ---- pending clear ------------------------------------------------------ */

    #[test]
    fn take_pending_ds_clears_moves_values_and_resets_the_flags() {
        let mut pc = PendingClear {
            color: true,
            rgba: [0.5; 4],
            depth: true,
            depth_value: 0.25,
            stencil: true,
            stencil_value: 7,
            fb: 0,
            state: ClearState::CLEAR_ALL,
        };
        let mut d = GlesDraw::ZERO;
        take_pending_ds_clears(&mut d, &mut pc, (true, true));
        assert!(d.do_clear_depth && d.do_clear_stencil);
        assert_eq!((d.clear_depth, d.clear_stencil), (0.25, 7));
        assert!(!pc.depth && !pc.stencil);
        assert!(pc.color, "the colour clear is not this helper's");
    }

    #[test]
    fn take_pending_ds_clears_without_a_clear_reports_the_gl_defaults() {
        let mut pc = PendingClear {
            color: false,
            rgba: [0.0; 4],
            depth: false,
            depth_value: 0.25,
            stencil: false,
            stencil_value: 7,
            fb: 0,
            state: ClearState::CLEAR_ALL,
        };
        let mut d = GlesDraw::ZERO;
        take_pending_ds_clears(&mut d, &mut pc, (true, true));
        assert!(!d.do_clear_depth && !d.do_clear_stencil);
        assert_eq!((d.clear_depth, d.clear_stencil), (1.0, 0));
    }

    #[test]
    fn clear_then_draw_fuses_the_clear_into_the_draw() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        // SAFETY: plain GL calls.
        unsafe {
            glClearColor(0.25, 0.5, 0.75, 1.0);
            glClear(GL_COLOR_BUFFER_BIT);
        }
        assert!(gl.draws().is_empty(), "glClear alone only records");
        let d = draw_tri(&gl);
        assert!(d.draw.do_clear);
        assert_eq!(d.draw.clear_rgba, [0.25, 0.5, 0.75, 1.0]);
        assert!(!d.draw.do_clear_depth && !d.draw.do_clear_stencil);
        assert_eq!(d.draw.index_count, 3);
    }

    #[test]
    fn second_draw_after_a_clear_does_not_clear_again() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        // SAFETY: plain GL calls.
        unsafe { glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT) };
        let first = draw_tri(&gl);
        assert!(first.draw.do_clear && first.draw.do_clear_depth && first.draw.do_clear_stencil);
        let second = draw_tri(&gl);
        assert!(!second.draw.do_clear && !second.draw.do_clear_depth && !second.draw.do_clear_stencil);
    }

    #[test]
    fn draw_without_a_clear_has_no_clear_and_default_clear_values() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        let d = draw_tri(&gl);
        assert!(!d.draw.do_clear && !d.draw.do_clear_depth && !d.draw.do_clear_stencil);
        assert_eq!(d.draw.clear_depth, 1.0);
        assert_eq!(d.draw.clear_stencil, 0);
    }

    #[test]
    fn clear_colour_is_captured_at_glclear_time() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        // SAFETY: plain GL calls.
        unsafe {
            glClearColor(1.0, 0.0, 0.0, 1.0);
            glClear(GL_COLOR_BUFFER_BIT);
            glClearColor(0.0, 1.0, 0.0, 1.0);
        }
        assert_eq!(draw_tri(&gl).draw.clear_rgba, [1.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn depth_and_stencil_clear_values_reach_the_draw() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        // SAFETY: plain GL calls.
        unsafe {
            glClearDepthf(0.25);
            glClearStencil(0x1a5); /* masked to the 8 stencil bits */
            glClear(GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT);
            glClearDepthf(0.75); /* after the clear: not this clear's value */
        }
        let d = draw_tri(&gl);
        assert!(!d.draw.do_clear, "no colour clear was asked for");
        assert!(d.draw.do_clear_depth && d.draw.do_clear_stencil);
        assert_eq!(d.draw.clear_depth, 0.25);
        assert_eq!(d.draw.clear_stencil, 0xa5);
    }

    #[test]
    fn depth_only_clear_leaves_stencil_alone() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        // SAFETY: plain GL calls.
        unsafe { glClear(GL_DEPTH_BUFFER_BIT) };
        let d = draw_tri(&gl);
        assert!(d.draw.do_clear_depth);
        assert!(!d.draw.do_clear_stencil && !d.draw.do_clear);
        assert_eq!(d.draw.clear_depth, 1.0);
    }

    #[test]
    fn flush_emits_a_clear_only_draw_for_a_clear_nothing_followed() {
        let gl = Gl::new(32, 16);
        // SAFETY: plain GL calls.
        unsafe {
            glViewport(1, 2, 3, 4);
            glClearColor(0.0, 0.0, 1.0, 0.5);
            glClearDepthf(0.5);
            glClearStencil(3);
            glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT);
            glFlush();
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(gl.flushes(), 1);
        let d = one(&gl).draw;
        assert!(d.do_clear);
        assert_eq!(d.clear_rgba, [0.0, 0.0, 1.0, 0.5]);
        assert!(d.do_clear_depth && d.do_clear_stencil);
        assert_eq!((d.clear_depth, d.clear_stencil), (0.5, 3));
        assert_eq!(d.index_count, 0, "clear-only kick: no geometry");
        assert_eq!(d.prim, GLES_PRIM_TRIANGLES);
        assert!(d.vs.is_null() && d.fs.is_null());
        assert_eq!(d.num_attribs, 0);
        assert!(d.color_write);
        assert!(d.flip_y);
        assert_eq!(d.viewport, [1, 2, 3, 4]);
        assert_eq!(d.target_gpu, gl.ctx().default_surface.gpu);
        assert_eq!((d.target_w, d.target_h), (32, 16));
        assert!(d.ds_persist);
    }

    #[test]
    fn flushed_clear_is_not_repeated_by_a_second_flush_or_the_next_draw() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        // SAFETY: plain GL calls.
        unsafe {
            glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
            glFlush();
        }
        assert_eq!(gl.draws().len(), 1);
        // SAFETY: plain GL calls.
        unsafe { glFlush() };
        assert!(gl.draws().is_empty(), "nothing pending any more");
        let d = draw_tri(&gl);
        assert!(!d.draw.do_clear && !d.draw.do_clear_depth);
    }

    #[test]
    fn flush_without_a_pending_clear_draws_nothing() {
        let gl = Gl::new(16, 16);
        // SAFETY: plain GL calls.
        unsafe { glFlush() };
        assert!(gl.draws().is_empty());
        assert_eq!(gl.flushes(), 1);
    }

    #[test]
    fn finish_emits_the_pending_clear_then_flushes_and_waits() {
        let gl = Gl::new(16, 16);
        // SAFETY: plain GL calls.
        unsafe {
            glClear(GL_COLOR_BUFFER_BIT);
            glFinish();
        }
        let d = one(&gl).draw;
        assert!(d.do_clear);
        assert_eq!(d.index_count, 0);
        assert_eq!((gl.flushes(), gl.finishes()), (1, 1));
    }

    #[test]
    fn a_depth_only_clear_with_nothing_behind_it_is_carried_out_by_the_flush() {
        /* upstream 3864915: `glClear(GL_DEPTH_BUFFER_BIT); glFinish();` is a
         * complete request. Gating the flush on the colour clear left it
         * pending, to be applied to whatever draw came next however much
         * later -- or never. */
        let gl = Gl::new(16, 16);
        basic(&gl);
        // SAFETY: plain GL calls.
        unsafe {
            glClearDepthf(0.5);
            glClear(GL_DEPTH_BUFFER_BIT);
            glFlush();
        }
        let d = one(&gl).draw;
        assert!(d.do_clear_depth);
        assert_eq!(d.clear_depth, 0.5);
        assert_eq!(d.index_count, 0, "a clear-only kick, no geometry");
        /* and it leaves the colour buffer alone */
        assert!(!d.do_clear);
        assert!(!d.color_write);
        assert_eq!(d.color_mask, 0x0);
        /* the next draw does not clear depth again */
        assert!(!draw_tri(&gl).draw.do_clear_depth);
    }

    #[test]
    fn a_clear_only_kick_carries_the_depth_range() {
        /* upstream ea4d2f1: a backend that clears with a quad puts it through
         * the viewport transform, and a zeroed range is a range of no width,
         * not the default. The Vivante backend derived scale_z = far - near =
         * 0 from it and wrote depth 0 whatever value the clear asked for. */
        let gl = Gl::new(16, 16);
        basic(&gl);
        // SAFETY: plain GL calls.
        unsafe {
            glDepthRangef(0.25, 0.75);
            glClearDepthf(0.5);
            glClear(GL_DEPTH_BUFFER_BIT);
            glFlush();
        }
        assert_eq!(one(&gl).draw.depth_range, [0.25, 0.75]);
    }

    #[test]
    fn c_entry_point_flushes_the_pending_clear() {
        let gl = Gl::new(16, 16);
        // SAFETY: plain GL calls.
        unsafe {
            glClear(GL_COLOR_BUFFER_BIT);
            gl_flush_pending_clear();
        }
        assert!(one(&gl).draw.do_clear);
        assert_eq!(gl.flushes(), 0, "the clear is recorded, not submitted");
    }

    #[test]
    fn flushing_a_clear_of_an_incomplete_framebuffer_is_an_error_and_drops_it() {
        let gl = Gl::new(16, 16);
        let mut fbo: GLuint = 0;
        // SAFETY: valid out pointer, plain GL calls.
        unsafe {
            glGenFramebuffers(1, &mut fbo);
            glBindFramebuffer(GL_FRAMEBUFFER, fbo); /* no attachments */
            glClear(GL_COLOR_BUFFER_BIT);
            glFlush();
        }
        assert_eq!(gl.error(), GL_INVALID_FRAMEBUFFER_OPERATION);
        assert!(gl.draws().is_empty());
        assert!(!pending_clear().color, "the clear is dropped, not retried for ever");
    }

    #[test]
    fn clear_only_draw_targets_the_bound_fbo() {
        let gl = Gl::new(16, 16);
        let (_fbo, tex) = fbo_8x8();
        // SAFETY: plain GL calls.
        unsafe {
            glClear(GL_COLOR_BUFFER_BIT);
            glFlush();
        }
        let d = one(&gl).draw;
        assert_eq!(d.target_gpu, gl.ctx().objs().textures[tex as usize].mem.gpu);
        assert_eq!((d.target_w, d.target_h), (8, 8));
        assert!(!d.flip_y);
        assert_eq!(d.target_format, GLES_SURF_RGBA8888);
        assert_eq!(d.samples, 1);
        assert!(!d.ds_persist);
    }

    #[test]
    fn a_scissored_stencil_only_clear_carries_the_whole_box() {
        /* dEQP's fbo.render.recreate_* and .shared_colorbuffer clear the
         * stencil inside a box and then draw with GL_EQUAL against it, so the
         * box has to reach the backend whole. A colour clear of the same box
         * is gl_test case 58; this is the stencil-only one, which has no
         * colour write to carry the rectangle. */
        let gl = Gl::new(16, 16);
        basic(&gl);
        let (_fbo, _tex) = fbo_8x8();
        let ds = renderbuffer_8x8(GL_DEPTH24_STENCIL8);
        // SAFETY: plain GL calls.
        unsafe {
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_STENCIL_ATTACHMENT, GL_RENDERBUFFER, ds);
            glEnable(GL_SCISSOR_TEST);
            glScissor(2, 3, 4, 5);
            glClearStencil(1);
            glClear(GL_STENCIL_BUFFER_BIT);
            glFlush();
        }
        let d = one(&gl).draw;
        assert!(d.clear_quad, "a scissored clear is a quad, not the background object");
        assert!(d.do_clear_stencil && !d.do_clear && !d.do_clear_depth);
        assert_eq!(d.clear_stencil, 1);
        assert!(d.scissor);
        assert_eq!(d.scissor_rect, [2, 3, 4, 5], "x and y, width and height");
        assert!(!d.color_write, "a stencil-only clear writes no colour");
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn a_framebuffer_without_depth_or_stencil_does_not_test_or_clear_them() {
        /* ES 2.0 4.1.4 / 4.1.6: with no stencil buffer the stencil test
         * always passes and nothing is written; with no depth buffer the
         * depth test always passes. The backend keeps a depth/stencil buffer
         * for the target regardless, so an FBO with neither attached must be
         * told, or GL_DEPTH_TEST would test against a buffer GL says is not
         * there -- which is what dEQP's fbo.render.stencil counts on when its
         * configuration has a stencil buffer and no depth one. */
        let gl = Gl::new(16, 16);
        basic(&gl);
        let (_fbo, _tex) = fbo_8x8(); /* colour only */
        // SAFETY: plain GL calls.
        unsafe {
            glEnable(GL_DEPTH_TEST);
            glEnable(GL_STENCIL_TEST);
            glStencilFunc(GL_EQUAL, 1, 0xff);
            glStencilOp(GL_KEEP, GL_KEEP, GL_INCR);
            glClear(GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT);
        }
        let d = draw_tri(&gl).draw;
        assert!(!d.depth_test, "no depth attachment: the depth test always passes");
        assert!(!d.depth_write);
        assert!(!d.stencil, "no stencil attachment: the stencil test always passes");
        assert_eq!(d.stencil_write_mask, 0);
        assert_eq!(d.stencil_back_write_mask, 0);
        assert!(!d.do_clear_depth, "a buffer that is not there is not cleared");
        assert!(!d.do_clear_stencil);

        /* attach both and the state comes back */
        let ds = renderbuffer_8x8(GL_DEPTH24_STENCIL8);
        // SAFETY: plain GL calls.
        unsafe {
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_RENDERBUFFER, ds);
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_STENCIL_ATTACHMENT, GL_RENDERBUFFER, ds);
        }
        let d = draw_tri(&gl).draw;
        assert!(d.depth_test && d.stencil);
        assert_eq!(d.stencil_write_mask, 0xff);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn a_clear_into_a_target_with_no_alpha_writes_one() {
        /* GL_RGB565 has no alpha and GL_RGB5_A1 has one bit, so the clear
         * colour the backend gets is what the format can hold -- not the
         * application's 0.3, which dEQP's reference would never produce. */
        for (format, alpha, want) in
            [(GL_RGB565, 0.3f32, 1.0f32), (GL_RGB5_A1, 0.3, 0.0), (GL_RGB5_A1, 0.7, 1.0), (GL_RGBA8, 0.3, 0.3)]
        {
            let gl = Gl::new(16, 16);
            let mut fbo: GLuint = 0;
            let rb = renderbuffer_8x8(format);
            // SAFETY: valid out pointer, plain GL calls.
            unsafe {
                glGenFramebuffers(1, &mut fbo);
                glBindFramebuffer(GL_FRAMEBUFFER, fbo);
                glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb);
                assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
                glClearColor(0.25, 0.5, 0.75, alpha);
                glClear(GL_COLOR_BUFFER_BIT);
                glFlush();
            }
            let d = one(&gl).draw;
            assert!(d.do_clear);
            assert_eq!(d.clear_rgba[3], want, "0x{format:x} with alpha {alpha}");
            assert_eq!(&d.clear_rgba[..3], &[0.25, 0.5, 0.75], "the colour channels are left alone");
        }
    }

    #[test]
    fn clear_applies_to_the_framebuffer_bound_at_glclear_time() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        let (fbo, _tex) = fbo_8x8();
        let surface = gl.ctx().default_surface.gpu;
        // SAFETY: plain GL calls.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glClearColor(1.0, 0.0, 0.0, 1.0);
            glClear(GL_COLOR_BUFFER_BIT); /* clears the window surface ... */
            glBindFramebuffer(GL_FRAMEBUFFER, fbo);
            glDrawArrays(GL_TRIANGLES, 0, 3); /* ... not the FBO this draw renders to */
        }
        let draws = gl.draws();
        let fbo_draw = draws.iter().find(|d| d.draw.index_count == 3).expect("the FBO draw");
        assert!(!fbo_draw.draw.do_clear, "the FBO was never cleared by the application");
        assert!(
            draws.iter().any(|d| d.draw.do_clear && d.draw.target_gpu == surface),
            "the clear must reach the window surface"
        );
    }

    /* ---- target -------------------------------------------------------------- */

    #[test]
    fn default_surface_target() {
        let gl = Gl::new(40, 24);
        basic(&gl);
        let d = draw_tri(&gl).draw;
        assert_eq!(d.target_gpu, gl.ctx().default_surface.gpu);
        assert_eq!((d.target_w, d.target_h), (40, 24));
        assert_eq!(d.target_format, GLES_SURF_RGBA8888);
        assert_eq!(d.samples, 1);
        assert!(d.flip_y, "the window surface is stored top-down");
    }

    /// glmark2's shadow scene (scene-shadow.cpp:100-118, DepthRenderTarget at
    /// twice the canvas) renders into a framebuffer whose only attachment is
    /// a GL_DEPTH_COMPONENT / GL_UNSIGNED_INT texture. Where the backend can
    /// draw with no colour buffer (`backend::DEPTH_ONLY_TARGETS`, the GC7000,
    /// as Mesa's etnaviv does), that framebuffer is complete and a draw
    /// reaches the backend with no colour target: `target_gpu` 0, the depth
    /// texture's size, no colour clear and no colour writes. Elsewhere it
    /// stays GL_FRAMEBUFFER_UNSUPPORTED and the draw is refused. The i.MX8MP
    /// glmark2 run of 2026-10-04 11:56 named shadow as its one failed scene
    /// (record 594b120e 47d70027 22781093 03200258, why 15: the status line).
    #[test]
    fn a_depth_texture_alone_is_a_target_where_the_backend_draws_without_colour() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        let (w, h) = (1600, 1200);
        let (mut tex, mut fbo): (GLuint, GLuint) = (0, 0);
        /* glmark2's own call, unsized on both surfaces (scene-shadow.cpp:105) */
        let depth_format = GL_DEPTH_COMPONENT;
        // SAFETY: valid out pointers, plain GL calls; null pixels only allocate.
        unsafe {
            glGenTextures(1, &mut tex);
            glBindTexture(GL_TEXTURE_2D, tex);
            glTexImage2D(
                GL_TEXTURE_2D,
                0,
                depth_format as GLint,
                w,
                h,
                0,
                GL_DEPTH_COMPONENT,
                GL_UNSIGNED_INT,
                core::ptr::null(),
            );
            glBindTexture(GL_TEXTURE_2D, 0);
            glGenFramebuffers(1, &mut fbo);
            glBindFramebuffer(GL_FRAMEBUFFER, fbo);
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_TEXTURE_2D, tex, 0);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        // SAFETY: plain GL call.
        let status = unsafe { glCheckFramebufferStatus(GL_FRAMEBUFFER) };
        if !crate::backend::DEPTH_ONLY_TARGETS {
            assert_eq!(status, GL_FRAMEBUFFER_UNSUPPORTED);
            // SAFETY: the arrays the fixtures set up are statics.
            unsafe { glDrawArrays(GL_TRIANGLES, 0, 3) };
            assert_eq!(gl.error(), GL_INVALID_FRAMEBUFFER_OPERATION);
            assert!(gl.draws().is_empty());
            return;
        }
        assert_eq!(status, GL_FRAMEBUFFER_COMPLETE);
        // SAFETY: plain GL calls.
        unsafe {
            glEnable(GL_DEPTH_TEST);
            glClearColor(1.0, 0.0, 0.0, 1.0);
            glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
        }
        let d = draw_tri(&gl).draw;
        assert_eq!(d.target_gpu, 0, "no colour target");
        assert_eq!((d.target_w, d.target_h), (w as u32, h as u32), "the depth texture's size");
        assert_eq!((d.ds_w, d.ds_h), (w as u32, h as u32));
        assert_ne!(d.depth_id, 0, "the depth texture is the depth buffer");
        assert!(!d.do_clear, "a colour clear with no colour buffer clears nothing");
        assert!(d.do_clear_depth);
        assert_eq!(d.color_mask, 0, "no colour is written");
        assert!(d.num_targets <= 1 && d.targets[0].gpu == 0);
        /* the framebuffer is complete: the draw's target lookup does not
         * call its missing colour buffer a fault (the Pi 4's round-3
         * shadow printed "framebuffer 1 colour attachment has no storage"
         * at every depth-pass draw it logged) */
        let logs = gl.logs();
        assert!(!logs.iter().any(|l| l.contains("has no storage")), "{logs:?}");
        /* a read of a framebuffer with no colour buffer is an error, not pixels */
        let mut px = [0u8; 4];
        // SAFETY: room for one RGBA pixel.
        unsafe { glReadPixels(0, 0, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr() as *mut c_void) };
        assert_ne!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn default_surface_depth_stencil_is_named_by_the_surface_page() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        let d = draw_tri(&gl).draw;
        assert!(d.ds_persist);
        /* bit 31 | page number of the colour buffer */
        assert_eq!(d.ds_id, 0x8000_0000 | (gl.ctx().default_surface.gpu >> 12) as u32);
        assert_ne!(d.ds_id & 0x8000_0000, 0);
    }

    #[test]
    fn default_surface_format_and_samples_come_from_the_context() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        gl.ctx().default_format = GLES_SURF_RGB565;
        gl.ctx().default_samples = 4;
        let d = draw_tri(&gl).draw;
        assert_eq!(d.target_format, GLES_SURF_RGB565);
        assert_eq!(d.samples, 4);
    }

    /// glSampleCoverage reaches the backend as a sample mask, Mesa's state
    /// tracker rule: on a 4x surface, 0.5 is the first two samples, 0.25
    /// inverted the last three, 0.0 none; coverage off, or a single-sampled
    /// target, every sample. Alpha-to-coverage is carried on a 4x surface
    /// only. Before, GlesDraw had no field for either, so no backend saw
    /// them (v3d-tex's note, 2026-09-26).
    #[test]
    fn sample_coverage_reaches_the_draw_as_a_sample_mask() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        gl.ctx().default_samples = 4;
        let mask = |on: bool, value: f32, invert: bool| {
            let c = gl.ctx();
            (c.sample_coverage, c.sample_coverage_value, c.sample_coverage_invert) = (on, value, invert);
            draw_tri(&gl).draw.sample_mask
        };
        assert_eq!(mask(false, 0.5, false), 0xf, "coverage off: every sample");
        assert_eq!(mask(true, 0.5, false), 0b0011);
        assert_eq!(mask(true, 0.25, true), 0b1110);
        assert_eq!(mask(true, 1.0, false), 0xf);
        /* an empty mask changes no sample: nothing reaches the backend */
        for (value, invert) in [(0.0, false), (1.0, true)] {
            let c = gl.ctx();
            (c.sample_coverage, c.sample_coverage_value, c.sample_coverage_invert) = (true, value, invert);
            // SAFETY: the arrays the fixtures set up are statics.
            unsafe { glDrawArrays(GL_TRIANGLES, 0, 3) };
            assert_eq!(gl.error(), GL_NO_ERROR);
            assert!(gl.draws().is_empty(), "coverage {value} invert {invert}");
        }
        gl.ctx().sample_coverage = false;
        gl.ctx().sample_alpha_to_coverage = true;
        assert!(draw_tri(&gl).draw.alpha_to_coverage);
        /* a single-sampled target ignores both */
        gl.ctx().default_samples = 1;
        let d = draw_tri(&gl).draw;
        assert_eq!((d.sample_mask, d.alpha_to_coverage), (0xf, false));
    }

    /// glSampleCoverage's invert inverts the coverage mask and nothing else
    /// (ES 2.0.25 4.1.3), at every value dEQP's multisample.constancy_both
    /// and constancy_both_inverted give it (row * 16 + col over 255, the
    /// inverted case 1 minus that) with alpha to coverage on: the inverted
    /// draw's mask is the other draw's complement within the four samples,
    /// and the two carry the same alpha-to-coverage flag and fragment shader
    /// (an empty mask draws nothing). Those two cases failed together on the
    /// AM62P at 32778b57 for another reason (a split 4x frame, gl_test 198):
    /// this pins that the inverted arm was not it.
    #[test]
    fn sample_coverage_invert_is_the_masks_complement_at_deqps_values() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        gl.ctx().default_samples = 4;
        gl.ctx().sample_alpha_to_coverage = true;
        let draw_with = |value: f32, invert: bool| -> Option<(u8, bool, *const ShcShader)> {
            let c = gl.ctx();
            (c.sample_coverage, c.sample_coverage_value, c.sample_coverage_invert) = (true, value, invert);
            // SAFETY: the arrays the fixtures set up are statics.
            unsafe { glDrawArrays(GL_TRIANGLES, 0, 3) };
            assert_eq!(gl.error(), GL_NO_ERROR);
            gl.draws().pop().map(|r| (r.draw.sample_mask, r.draw.alpha_to_coverage, r.draw.fs))
        };
        for k in 0..256 {
            for value in [k as f32 / 255.0, 1.0 - k as f32 / 255.0] {
                let plain = draw_with(value, false);
                let inverted = draw_with(value, true);
                let mask = |d: Option<(u8, bool, *const ShcShader)>| d.map_or(0, |d| d.0);
                assert_eq!(mask(inverted), !mask(plain) & 0xf, "value {value}");
                if let (Some(a), Some(b)) = (plain, inverted) {
                    assert_eq!((a.1, a.2), (b.1, b.2), "value {value}: the same alpha to coverage and shader");
                }
            }
        }
    }

    #[test]
    fn fbo_colour_texture_target() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        gl.ctx().default_format = GLES_SURF_XRGB8888; /* must not leak into the FBO */
        gl.ctx().default_samples = 4;
        let (_fbo, tex) = fbo_8x8();
        let d = draw_tri(&gl).draw;
        assert_eq!(d.target_gpu, gl.ctx().objs().textures[tex as usize].mem.gpu);
        assert_eq!((d.target_w, d.target_h), (8, 8));
        assert_eq!(d.target_format, GLES_SURF_RGBA8888);
        assert_eq!(d.samples, 1);
        assert!(!d.flip_y, "FBO textures are in texel order");
    }

    #[test]
    fn fbo_without_depth_or_stencil_keeps_no_ds_buffer() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        fbo_8x8();
        let d = draw_tri(&gl).draw;
        assert!(!d.ds_persist);
        assert_eq!(d.ds_id, 0);
    }

    #[test]
    fn fbo_depth_renderbuffer_names_the_ds_buffer() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        fbo_8x8();
        let rb = renderbuffer_8x8(GL_DEPTH_COMPONENT16);
        // SAFETY: plain GL calls.
        unsafe {
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_RENDERBUFFER, rb);
            assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
        }
        let d = draw_tri(&gl).draw;
        assert!(d.ds_persist);
        assert_eq!(d.ds_id, rb << 16);
        assert_eq!(d.ds_id & 0x8000_0000, 0, "never collides with a window surface's id");
    }

    #[test]
    fn fbo_depth_and_stencil_renderbuffers_both_name_the_ds_buffer() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        fbo_8x8();
        let depth = renderbuffer_8x8(GL_DEPTH_COMPONENT16);
        let stencil = renderbuffer_8x8(GL_STENCIL_INDEX8);
        // SAFETY: plain GL calls.
        unsafe {
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_RENDERBUFFER, depth);
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_STENCIL_ATTACHMENT, GL_RENDERBUFFER, stencil);
            assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
        }
        let d = draw_tri(&gl).draw;
        assert!(d.ds_persist);
        assert_eq!(d.ds_id, (depth << 16) | stencil);
    }

    #[test]
    fn fbos_sharing_a_depth_renderbuffer_share_the_ds_id() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        let rb = renderbuffer_8x8(GL_DEPTH_COMPONENT16);
        let mut ids = Vec::new();
        for _ in 0..2 {
            fbo_8x8();
            // SAFETY: plain GL calls.
            unsafe { glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_RENDERBUFFER, rb) };
            ids.push(draw_tri(&gl).draw.ds_id);
        }
        assert_eq!(ids[0], ids[1]);
    }

    /* ---- depth/stencil identity and the clear's targets on the draw record -------------------- */

    fn ds_fields(d: &GlesDraw) -> (u32, u32, u32, u32, u32, u32) {
        (d.depth_id, d.depth_gen, d.stencil_id, d.stencil_gen, d.ds_w, d.ds_h)
    }

    /// (a) two FBOs sharing a colour buffer, each with its own depth
    /// renderbuffer, draw with different depth ids (the i.MX8MP
    /// shared_colorbuffer family); (b) two FBOs sharing one depth
    /// renderbuffer draw with the same one.
    #[test]
    fn the_depth_id_names_the_attached_object() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        let (_, colour) = fbo_8x8();
        let (d1, d2) = (renderbuffer_8x8(GL_DEPTH_COMPONENT16), renderbuffer_8x8(GL_DEPTH_COMPONENT16));
        // SAFETY: plain GL calls.
        unsafe { glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_RENDERBUFFER, d1) };
        let a = draw_tri(&gl).draw;
        let (_, _) = fbo_8x8();
        // SAFETY: plain GL calls.
        unsafe {
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, colour, 0);
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_RENDERBUFFER, d2);
        }
        let b = draw_tri(&gl).draw;
        assert!(a.depth_id != 0 && b.depth_id != 0 && a.depth_id != b.depth_id, "(a) own depth buffers");
        assert_eq!((a.stencil_id, a.ds_w, a.ds_h), (0, 8, 8));
        assert_eq!(a.depth_id & 0xc000_0000, 0, "a renderbuffer: neither the surface bit nor the texture bit");
        // SAFETY: plain GL call.
        unsafe { glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_RENDERBUFFER, d1) };
        assert_eq!(ds_fields(&draw_tri(&gl).draw), ds_fields(&a), "(b) one shared depth buffer");
    }

    /// (c) a D24S8 on both points is one object: stencil_id == depth_id;
    /// (d) specifying its storage again changes the generation, not the id.
    #[test]
    fn one_packed_depth_stencil_object_and_its_generation() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        fbo_8x8();
        let ds = renderbuffer_8x8(GL_DEPTH24_STENCIL8);
        // SAFETY: plain GL calls.
        unsafe { glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_DEPTH_STENCIL_ATTACHMENT, GL_RENDERBUFFER, ds) };
        let a = draw_tri(&gl).draw;
        assert!(a.depth_id != 0 && a.depth_id == a.stencil_id && a.depth_gen == a.stencil_gen, "(c)");
        // SAFETY: plain GL calls; the renderbuffer is still bound.
        unsafe { glRenderbufferStorage(GL_RENDERBUFFER, GL_DEPTH24_STENCIL8, 8, 8) };
        let b = draw_tri(&gl).draw;
        assert_eq!(b.depth_id, a.depth_id, "(d) the same object");
        assert_ne!(b.depth_gen, a.depth_gen, "(d) new storage");
    }

    /// A depth-stencil TEXTURE (GL_OES_packed_depth_stencil) on both points
    /// is one object too: the backends' packed D24S8, keyed by the texture
    /// (the texture bit); another one on the stencil point is another object.
    #[test]
    fn one_depth_stencil_texture_on_both_points_is_one_object() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        fbo_8x8();
        let tex = |w: GLsizei| {
            let mut t: GLuint = 0;
            // SAFETY: room for one name; null pixels only allocate.
            unsafe {
                glGenTextures(1, &mut t);
                glBindTexture(GL_TEXTURE_2D, t);
                glTexImage2D(
                    GL_TEXTURE_2D,
                    0,
                    GL_DEPTH_STENCIL as GLint,
                    w,
                    w,
                    0,
                    GL_DEPTH_STENCIL,
                    GL_UNSIGNED_INT_24_8,
                    core::ptr::null(),
                );
            }
            t
        };
        let (ds, other) = (tex(8), tex(8));
        // SAFETY: plain GL calls.
        unsafe {
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_TEXTURE_2D, ds, 0);
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_STENCIL_ATTACHMENT, GL_TEXTURE_2D, ds, 0);
        }
        let a = draw_tri(&gl).draw;
        assert!(a.depth_id != 0 && a.depth_id == a.stencil_id && a.depth_gen == a.stencil_gen);
        assert_ne!(a.depth_id & 0x4000_0000, 0, "the texture bit");
        assert_eq!((a.ds_w, a.ds_h), (8, 8));
        // SAFETY: plain GL call.
        unsafe { glFramebufferTexture2D(GL_FRAMEBUFFER, GL_STENCIL_ATTACHMENT, GL_TEXTURE_2D, other, 0) };
        let b = draw_tri(&gl).draw;
        assert_eq!(b.depth_id, a.depth_id);
        assert!(b.stencil_id != 0 && b.stencil_id != b.depth_id, "two objects");
    }

    /// `ds_id` tells a texture from a renderbuffer of the same name number: a
    /// depth-stencil texture on both points of one framebuffer and a D24S8
    /// renderbuffer on both points of another are two packed buffers, not one
    /// (the PowerVR's DS_PACKED key is this id). Both ids keep bit 31 clear,
    /// the window surface's.
    #[test]
    fn a_depth_stencil_texture_and_a_renderbuffer_of_one_name_have_two_ds_ids() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        let (fb_tex, _) = fbo_8x8();
        let mut ds: GLuint = 0;
        // SAFETY: room for one name; null pixels only allocate.
        unsafe {
            glGenTextures(1, &mut ds);
            glBindTexture(GL_TEXTURE_2D, ds);
            glTexImage2D(
                GL_TEXTURE_2D,
                0,
                GL_DEPTH_STENCIL as GLint,
                8,
                8,
                0,
                GL_DEPTH_STENCIL,
                GL_UNSIGNED_INT_24_8,
                core::ptr::null(),
            );
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_TEXTURE_2D, ds, 0);
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_STENCIL_ATTACHMENT, GL_TEXTURE_2D, ds, 0);
        }
        let a = draw_tri(&gl).draw;
        /* a renderbuffer with the texture's name number */
        let mut rb: GLuint = 0;
        while rb < ds {
            // SAFETY: room for one name.
            unsafe { glGenRenderbuffers(1, &mut rb) };
        }
        assert_eq!(rb, ds, "the renderbuffer names reach the texture's number");
        let (fb_rb, _) = fbo_8x8();
        assert_ne!(fb_rb, fb_tex);
        // SAFETY: plain GL calls.
        unsafe {
            glBindRenderbuffer(GL_RENDERBUFFER, rb);
            glRenderbufferStorage(GL_RENDERBUFFER, GL_DEPTH24_STENCIL8, 8, 8);
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_RENDERBUFFER, rb);
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_STENCIL_ATTACHMENT, GL_RENDERBUFFER, rb);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        let b = draw_tri(&gl).draw;
        assert!(a.ds_persist && b.ds_persist);
        assert_ne!(a.ds_id, b.ds_id, "one name number, two objects");
        assert_eq!(b.ds_id, (rb << 16) | rb, "a renderbuffer pair keeps the plain names");
        assert_eq!(a.ds_id, ((ds | 0x4000) << 16) | ds | 0x4000, "a texture's halves carry bit 14");
        assert_eq!((a.ds_id | b.ds_id) & 0x8000_0000, 0);
    }

    /// (e) the default framebuffer: the surface's id for both, the surface's size.
    #[test]
    fn the_default_framebuffer_has_the_surface_depth_stencil() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        let d = draw_tri(&gl).draw;
        assert_eq!(d.depth_id, 0x8000_0000 | (d.target_gpu >> 12) as u32);
        assert_eq!((d.stencil_id, d.ds_w, d.ds_h), (d.depth_id, 16, 16));
    }

    /// (g) the orphan rule moves a deleted-but-attached depth renderbuffer to
    /// another slot when its name is bound again; the framebuffer's
    /// depth_id and gen stay those of the kept image.
    #[test]
    fn the_depth_id_survives_the_orphan_move() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        let (fbo, _) = fbo_8x8();
        let rb = renderbuffer_8x8(GL_DEPTH_COMPONENT16);
        // SAFETY: plain GL call.
        unsafe { glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_RENDERBUFFER, rb) };
        let before = draw_tri(&gl).draw;
        // SAFETY: plain GL calls, one valid name each.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteRenderbuffers(1, &rb);
            glBindRenderbuffer(GL_RENDERBUFFER, rb); /* a new object under the old name */
            glRenderbufferStorage(GL_RENDERBUFFER, GL_DEPTH_COMPONENT16, 8, 8);
            glBindFramebuffer(GL_FRAMEBUFFER, fbo);
        }
        let after = draw_tri(&gl).draw;
        assert_eq!((after.depth_id, after.depth_gen), (before.depth_id, before.depth_gen));
    }

    /// (f) glClearBuffer(GL_COLOR, n) clears draw buffer n alone (it was
    /// dropped for n > 0); glClear clears every target (mask 0).
    #[test]
    fn clear_buffer_names_its_one_target() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        fbo_8x8();
        let red = [1.0f32, 0.0, 0.0, 1.0];
        let n = (context::MAX_DRAW_BUFFERS as u32 - 1).min(2);
        // SAFETY: four readable floats; plain GL calls.
        unsafe {
            /* the last draw buffer this GPU has (2 on the PowerVR, 0 where
             * GL_MAX_DRAW_BUFFERS is 1) */
            crate::gl::es3::glClearBufferfv(GL_COLOR, n as GLint, red.as_ptr());
            crate::gl::state::glClearColor(0.0, 1.0, 0.0, 1.0);
            crate::gl::state::glClear(GL_COLOR_BUFFER_BIT);
            glDrawArrays(GL_TRIANGLES, 0, 3);
        }
        let masks: Vec<(bool, u8)> = gl.draws().iter().map(|d| (d.draw.do_clear, d.draw.clear_target_mask)).collect();
        assert_eq!(masks, [(true, 1 << n), (true, 0)], "the draw buffer's clear kept apart from the glClear");
    }

    /// glClear clears only the draw buffers that are not GL_NONE.
    #[test]
    fn clear_skips_the_draw_buffers_set_to_none() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        fbo_8x8();
        // SAFETY: one readable enum; plain GL calls.
        unsafe {
            glDrawBuffers(1, [GL_NONE].as_ptr());
            crate::gl::state::glClear(GL_COLOR_BUFFER_BIT);
            glDrawArrays(GL_TRIANGLES, 0, 3);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        let d = &gl.draws()[0].draw;
        /* no draw buffer: no colour is cleared or written, on every backend
         * (the PowerVR does not read clear_target_mask) */
        assert!(!d.do_clear && !d.color_write, "no draw buffer: the clear touches nothing");
        assert_eq!(d.targets[0].gpu, d.target_gpu, "slot 0 stays the frame's target under a zero mask");
    }

    #[test]
    fn incomplete_fbo_draw_is_invalid_framebuffer_operation() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        let mut fbo: GLuint = 0;
        // SAFETY: valid out pointer, plain GL calls.
        unsafe {
            glGenFramebuffers(1, &mut fbo);
            glBindFramebuffer(GL_FRAMEBUFFER, fbo);
            glDrawArrays(GL_TRIANGLES, 0, 3);
        }
        assert_eq!(gl.error(), GL_INVALID_FRAMEBUFFER_OPERATION);
        assert!(gl.draws().is_empty());
        // SAFETY: TRI covers the 3 indices' vertices.
        unsafe {
            let idx: [u16; 3] = [0, 1, 2];
            glDrawElements(GL_TRIANGLES, 3, GL_UNSIGNED_SHORT, idx.as_ptr() as *const c_void);
        }
        assert_eq!(gl.error(), GL_INVALID_FRAMEBUFFER_OPERATION);
        assert!(gl.draws().is_empty());
    }

    #[test]
    fn missing_default_surface_is_invalid_framebuffer_operation() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        gl.ctx().default_surface_valid = false;
        // SAFETY: plain GL call.
        unsafe { glDrawArrays(GL_TRIANGLES, 0, 3) };
        gl.ctx().default_surface_valid = true; /* the guard frees it */
        assert_eq!(gl.error(), GL_INVALID_FRAMEBUFFER_OPERATION);
        assert!(gl.draws().is_empty());
    }

    /* ---- fixed-function state ------------------------------------------------- */

    #[test]
    fn default_state_draw() {
        let gl = Gl::new(64, 48);
        basic(&gl);
        let d = draw_tri(&gl).draw;
        assert_eq!(d.viewport, [0, 0, 64, 48]);
        assert_eq!(d.depth_range, [0.0, 1.0]);
        assert!(!d.depth_test);
        assert_eq!(d.depth_func, GLES_CMP_LESS);
        assert!(d.depth_write);
        assert!(!d.polygon_offset);
        assert_eq!((d.polygon_offset_factor, d.polygon_offset_units), (0.0, 0.0));
        assert!(!d.blend);
        assert_eq!(d.cull, GLES_CULL_NONE);
        assert!(!d.front_cw);
        assert_eq!(d.line_width, 1.0);
        assert!(!d.scissor);
        assert!(!d.stencil);
        assert_eq!((d.stencil_func, d.stencil_back_func), (GLES_CMP_ALWAYS, GLES_CMP_ALWAYS));
        assert_eq!((d.stencil_ref, d.stencil_back_ref), (0, 0));
        assert_eq!((d.stencil_cmp_mask, d.stencil_back_cmp_mask), (0xff, 0xff));
        assert_eq!((d.stencil_write_mask, d.stencil_back_write_mask), (0xff, 0xff));
        assert_eq!(
            (d.stencil_sfail, d.stencil_dpfail, d.stencil_dppass),
            (GLES_SOP_KEEP, GLES_SOP_KEEP, GLES_SOP_KEEP)
        );
        assert!(d.color_write);
        assert_eq!(d.index_count2, 0);
    }

    #[test]
    fn viewport_scissor_depth_range_pass_through() {
        let gl = Gl::new(64, 64);
        basic(&gl);
        // SAFETY: plain GL calls.
        unsafe {
            glViewport(-5, 7, 30, 20);
            glScissor(1, 2, 3, 4);
            glEnable(GL_SCISSOR_TEST);
            glDepthRangef(0.25, 0.75);
        }
        let d = draw_tri(&gl).draw;
        assert_eq!(d.viewport, [-5, 7, 30, 20]);
        assert!(d.scissor);
        assert_eq!(d.scissor_rect, [1, 2, 3, 4]);
        assert_eq!(d.depth_range, [0.25, 0.75]);
    }

    #[test]
    fn scissor_rect_passes_through_while_the_test_is_disabled() {
        let gl = Gl::new(64, 64);
        basic(&gl);
        // SAFETY: plain GL call.
        unsafe { glScissor(1, 2, 3, 4) };
        let d = draw_tri(&gl).draw;
        assert!(!d.scissor);
        assert_eq!(d.scissor_rect, [1, 2, 3, 4]);
    }

    #[test]
    fn depth_state_passes_through() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        // SAFETY: plain GL calls.
        unsafe {
            glEnable(GL_DEPTH_TEST);
            glDepthFunc(GL_GEQUAL);
            glDepthMask(0);
        }
        let d = draw_tri(&gl).draw;
        assert!(d.depth_test);
        assert_eq!(d.depth_func, GLES_CMP_GEQUAL);
        assert!(!d.depth_write);
    }

    #[test]
    fn polygon_offset_and_line_width_pass_through() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        // SAFETY: plain GL calls.
        unsafe {
            glEnable(GL_POLYGON_OFFSET_FILL);
            glPolygonOffset(-1.5, 4.0);
            glLineWidth(3.0);
            glDrawArrays(GL_LINE_LOOP, 0, 3);
        }
        let d = one(&gl).draw;
        assert!(d.polygon_offset);
        assert_eq!((d.polygon_offset_factor, d.polygon_offset_units), (-1.5, 4.0));
        assert_eq!(d.line_width, 3.0);
        assert_eq!(d.prim, GLES_PRIM_LINE_LOOP);
    }

    #[test]
    fn cull_resolves_enable_mode_and_front_face_to_the_culled_winding() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        /* (cull mode, front face) -> winding of the culled faces */
        let table = [
            (GL_BACK, GL_CCW, GLES_CULL_CW), /* GL defaults: back faces are clockwise */
            (GL_BACK, GL_CW, GLES_CULL_CCW),
            (GL_FRONT, GL_CCW, GLES_CULL_CCW),
            (GL_FRONT, GL_CW, GLES_CULL_CW),
        ];
        for (mode, front, want) in table {
            // SAFETY: plain GL calls.
            unsafe {
                glEnable(GL_CULL_FACE);
                glCullFace(mode);
                glFrontFace(front);
            }
            let d = draw_tri(&gl).draw;
            assert_eq!(d.cull, want, "cull 0x{mode:x} front 0x{front:x}");
            assert_eq!(d.front_cw, front == GL_CW);
            // SAFETY: plain GL call.
            unsafe { glDisable(GL_CULL_FACE) };
            assert_eq!(draw_tri(&gl).draw.cull, GLES_CULL_NONE, "culling disabled");
        }
    }

    #[test]
    fn cull_is_in_gl_winding_terms_for_either_target_orientation() {
        /* flip_y tells the backend to mirror the winding; the resolver
         * reports the GL-space winding for a window and an FBO alike. */
        let gl = Gl::new(16, 16);
        basic(&gl);
        // SAFETY: plain GL calls.
        unsafe { glEnable(GL_CULL_FACE) };
        let window = draw_tri(&gl).draw;
        fbo_8x8();
        let fbo = draw_tri(&gl).draw;
        assert!(window.flip_y && !fbo.flip_y);
        assert_eq!(window.cull, GLES_CULL_CW);
        assert_eq!(fbo.cull, GLES_CULL_CW);
    }

    #[test]
    fn cull_front_and_back_drops_polygons_and_keeps_points_and_lines() {
        /* ES 3.0 3.6.1: FRONT_AND_BACK culls every polygon; points and lines
         * are never culled. (The ignored test this replaces expected the
         * triangles to be drawn unculled, which misreads the spec.) */
        let gl = Gl::new(16, 16);
        basic(&gl);
        // SAFETY: plain GL calls.
        unsafe {
            glEnable(GL_CULL_FACE);
            glCullFace(GL_FRONT_AND_BACK);
            glClear(GL_COLOR_BUFFER_BIT);
            for mode in [GL_TRIANGLES, GL_TRIANGLE_STRIP, GL_TRIANGLE_FAN] {
                glDrawArrays(mode, 0, 3);
                glDrawElements(mode, 3, GL_UNSIGNED_SHORT, [0u16, 1, 2].as_ptr() as *const c_void);
            }
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert!(gl.draws().is_empty());
        assert!(pending_clear().color, "the clear waits for a draw that is recorded");
        for mode in [GL_POINTS, GL_LINES, GL_LINE_STRIP, GL_LINE_LOOP] {
            // SAFETY: plain GL call.
            unsafe { glDrawArrays(mode, 0, 3) };
            assert_eq!(gl.error(), GL_NO_ERROR);
            assert_eq!(one(&gl).draw.cull, GLES_CULL_NONE);
        }
        /* an incomplete framebuffer is still an error for a culled draw */
        let mut fbo: GLuint = 0;
        // SAFETY: valid out pointer, plain GL calls.
        unsafe {
            glGenFramebuffers(1, &mut fbo);
            glBindFramebuffer(GL_FRAMEBUFFER, fbo);
            glDrawArrays(GL_TRIANGLES, 0, 3);
        }
        assert_eq!(gl.error(), GL_INVALID_FRAMEBUFFER_OPERATION);
    }

    #[test]
    fn stencil_front_and_back_state_are_resolved_separately() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        // SAFETY: plain GL calls.
        unsafe {
            glEnable(GL_STENCIL_TEST);
            glStencilFuncSeparate(GL_FRONT, GL_LESS, 3, 0x0f);
            glStencilFuncSeparate(GL_BACK, GL_NOTEQUAL, 200, 0xf0);
            glStencilOpSeparate(GL_FRONT, GL_ZERO, GL_INCR, GL_REPLACE);
            glStencilOpSeparate(GL_BACK, GL_INVERT, GL_DECR_WRAP, GL_INCR_WRAP);
            glStencilMaskSeparate(GL_FRONT, 0x3c);
            glStencilMaskSeparate(GL_BACK, 0xc3);
        }
        let d = draw_tri(&gl).draw;
        assert!(d.stencil);
        assert_eq!((d.stencil_func, d.stencil_ref, d.stencil_cmp_mask), (GLES_CMP_LESS, 3, 0x0f));
        assert_eq!((d.stencil_back_func, d.stencil_back_ref, d.stencil_back_cmp_mask), (GLES_CMP_NOTEQUAL, 200, 0xf0));
        assert_eq!(
            (d.stencil_sfail, d.stencil_dpfail, d.stencil_dppass),
            (GLES_SOP_ZERO, GLES_SOP_INCR_SAT, GLES_SOP_REPLACE)
        );
        assert_eq!(
            (d.stencil_back_sfail, d.stencil_back_dpfail, d.stencil_back_dppass),
            (GLES_SOP_INVERT, GLES_SOP_DECR_WRAP, GLES_SOP_INCR_WRAP)
        );
        assert_eq!((d.stencil_write_mask, d.stencil_back_write_mask), (0x3c, 0xc3));
    }

    #[test]
    fn stencil_masks_are_truncated_to_the_8_stencil_bits() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        // SAFETY: plain GL calls.
        unsafe {
            glStencilFunc(GL_EQUAL, 1, 0xffff_ff0f);
            glStencilMask(0xffff_fff0);
        }
        let d = draw_tri(&gl).draw;
        assert_eq!((d.stencil_cmp_mask, d.stencil_back_cmp_mask), (0x0f, 0x0f));
        assert_eq!((d.stencil_write_mask, d.stencil_back_write_mask), (0xf0, 0xf0));
        assert!(!d.stencil, "state is resolved even with the test disabled");
        assert_eq!(d.stencil_func, GLES_CMP_EQUAL);
    }

    /* ---- blend keys, colour mask, FS variants ------------------------------------ */

    #[test]
    fn blend_key_is_zero_without_blending_or_masking() {
        let gl = Gl::new(8, 8);
        assert_eq!(current_blend_key(gl.ctx()), 0);
        assert_eq!(disabled_channels(gl.ctx()), 0);
    }

    /// An integer colour target selects the variant that packs the integer
    /// output for its layout, blending or not (ES 3.0 4.1.7), and keeps the
    /// mask of its own channels; with every channel off it is the ISP's.
    #[test]
    fn an_integer_target_keys_its_layout_never_a_blend() {
        if crate::REPORT_ES2 {
            return;
        }
        let gl = Gl::new(8, 8);
        let mut rb: GLuint = 0;
        let mut fbo: GLuint = 0;
        // SAFETY: plain GL calls.
        unsafe {
            glGenRenderbuffers(1, &mut rb);
            glBindRenderbuffer(GL_RENDERBUFFER, rb);
            glRenderbufferStorage(GL_RENDERBUFFER, GL_RG16I, 8, 8);
            glGenFramebuffers(1, &mut fbo);
            glBindFramebuffer(GL_FRAMEBUFFER, fbo);
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb);
            glEnable(GL_BLEND);
            glBlendFunc(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        let rt = ShcIntRt { channels: 2, bits: 16, signed: true };
        assert_eq!(current_blend_key(gl.ctx()), shc_blend_key_int_rt(rt), "no blend in the key");
        assert_eq!(ShcIntRt::of_key(current_blend_key(gl.ctx())), Some(rt));
        // SAFETY: plain GL calls.
        unsafe { glColorMask(GL_TRUE_B, GL_FALSE_B, GL_TRUE_B, GL_TRUE_B) };
        assert_eq!(current_blend_key(gl.ctx()), shc_blend_key_int_rt(rt) | shc_blend_key_mask_off(0b10), "green off");
        // SAFETY: plain GL calls.
        unsafe { glColorMask(GL_TRUE_B, GL_TRUE_B, GL_FALSE_B, GL_FALSE_B) };
        assert_eq!(current_blend_key(gl.ctx()), shc_blend_key_int_rt(rt), "blue and alpha it does not have");
        // SAFETY: plain GL calls.
        unsafe { glColorMask(GL_FALSE_B, GL_FALSE_B, GL_FALSE_B, GL_FALSE_B) };
        assert_eq!(current_blend_key(gl.ctx()), shc_blend_key_int_rt(rt), "every channel off: the ISP's");
    }

    #[test]
    fn blend_key_of_one_zero_is_a_plain_replace() {
        let gl = Gl::new(8, 8);
        // SAFETY: plain GL calls.
        unsafe {
            glEnable(GL_BLEND);
            glBlendFunc(GL_ONE, GL_ZERO);
        }
        assert_eq!(current_blend_key(gl.ctx()), 0, "ONE/ZERO ADD needs no variant");
    }

    #[test]
    fn blend_key_packs_factors_in_5_bit_fields() {
        let gl = Gl::new(8, 8);
        // SAFETY: plain GL calls.
        unsafe {
            glEnable(GL_BLEND);
            glBlendFunc(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA);
        }
        /* [0:4] src 3, [5:9] dst 0x13, [10:14] src_a 3, [15:19] dst_a 0x13 */
        assert_eq!(current_blend_key(gl.ctx()), 0x3 | (0x13 << 5) | (0x3 << 10) | (0x13 << 15));
        assert_eq!(current_blend_key(gl.ctx()), 0x98e63);
    }

    #[test]
    fn blend_key_separates_rgb_and_alpha_factors_and_equations() {
        let gl = Gl::new(8, 8);
        // SAFETY: plain GL calls.
        unsafe {
            glEnable(GL_BLEND);
            glBlendFuncSeparate(GL_DST_COLOR, GL_ZERO, GL_ONE, GL_ONE_MINUS_DST_ALPHA);
            glBlendEquationSeparate(GL_FUNC_SUBTRACT, GL_FUNC_REVERSE_SUBTRACT);
        }
        /* src DST_COLOR 5, dst ZERO 0x11, src_a ONE 1, dst_a INV_DST_ALPHA 0x14,
         * [20:22] eq_rgb SUBTRACT 1, [23:25] eq_a REVERSE_SUBTRACT 2 */
        let want = 0x5 | (0x11 << 5) | (0x1 << 10) | (0x14 << 15) | (1 << 20) | (2 << 23);
        assert_eq!(current_blend_key(gl.ctx()), want);
    }

    #[test]
    fn blend_key_ignores_the_blend_state_while_blending_is_disabled() {
        let gl = Gl::new(8, 8);
        // SAFETY: plain GL calls.
        unsafe { glBlendFunc(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA) };
        assert_eq!(current_blend_key(gl.ctx()), 0);
    }

    #[test]
    fn colour_mask_bits_go_to_bits_26_to_29_of_the_replace_key() {
        let gl = Gl::new(8, 8);
        // SAFETY: plain GL calls.
        unsafe { glColorMask(1, 1, 1, 0) };
        assert_eq!(disabled_channels(gl.ctx()), 0b1000);
        /* REPLACE = ONE(1) | ZERO(0x11) << 5 | ONE << 10 | ZERO << 15 = 0x88621 */
        assert_eq!(current_blend_key(gl.ctx()), 0x88621 | (0b1000 << 26));
        // SAFETY: plain GL calls.
        unsafe { glColorMask(0, 1, 0, 1) };
        assert_eq!(disabled_channels(gl.ctx()), 0b0101);
        assert_eq!(current_blend_key(gl.ctx()), 0x88621 | (0b0101 << 26));
    }

    #[test]
    fn colour_mask_bits_combine_with_a_blend_state() {
        let gl = Gl::new(8, 8);
        // SAFETY: plain GL calls.
        unsafe {
            glEnable(GL_BLEND);
            glBlendFunc(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA);
            glColorMask(1, 1, 1, 0);
        }
        assert_eq!(current_blend_key(gl.ctx()), 0x98e63 | (0b1000 << 26));
    }

    #[test]
    fn fully_masked_colour_is_not_a_variant() {
        let gl = Gl::new(8, 8);
        // SAFETY: plain GL calls.
        unsafe { glColorMask(0, 0, 0, 0) };
        assert_eq!(disabled_channels(gl.ctx()), 0xf);
        assert_eq!(current_blend_key(gl.ctx()), 0, "all-off is the ISP's colour-write disable");
    }

    #[test]
    #[cfg(feature = "gpu-powervr")] /* the PowerVR table's own values */
    fn unblended_draw_uses_the_programs_own_fragment_shader() {
        let gl = Gl::new(16, 16);
        let prog = basic(&gl);
        let d = draw_tri(&gl).draw;
        let p = &gl.ctx().objs_ref().programs[obj_index(prog)];
        assert!(core::ptr::eq(d.vs, &p.rvs));
        assert!(core::ptr::eq(d.fs, &p.rfs));
        assert!(!d.blend);
        assert_eq!(d.vs_output_size_dwords, 8, "passthrough.vert: position + one vec4 varying");
    }

    #[test]
    fn only_a_partial_sample_mask_or_alpha_to_coverage_on_multisample_needs_the_kill_variant() {
        let mut d = GlesDraw::ZERO;
        let mut fs = ShcShader::ZERO;
        (d.samples, d.sample_mask) = (4, 0xf);
        assert!(!needs_sample_kill(&d, &fs), "every sample");
        d.sample_mask = 0x3;
        assert!(needs_sample_kill(&d, &fs), "glSampleCoverage(0.5)");
        fs.uses_discard = 1;
        assert!(!needs_sample_kill(&d, &fs), "a discarding shader has the epilogue");
        fs.uses_discard = 0;
        (d.sample_mask, d.alpha_to_coverage) = (0xf, true);
        assert!(needs_sample_kill(&d, &fs), "alpha to coverage");
        d.samples = 1;
        assert!(!needs_sample_kill(&d, &fs), "one sample: GL ignores both");
        (d.samples, d.sample_mask, d.alpha_to_coverage) = (2, 0x3, false);
        assert!(!needs_sample_kill(&d, &fs), "both of two samples");
    }

    #[test]
    #[cfg(feature = "gpu-powervr")] /* the PowerVR table's own values */
    fn blended_draw_selects_the_precompiled_fbfetch_variant() {
        let gl = Gl::new(16, 16);
        let prog = basic(&gl);
        let plain = draw_tri(&gl).draw;
        gl.mem_events();
        // SAFETY: plain GL calls.
        unsafe {
            glEnable(GL_BLEND);
            glBlendFunc(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA);
        }
        let d = draw_tri(&gl).draw;
        assert!(d.blend);
        // SAFETY: the variant lives in the draw path's static cache.
        let fs = unsafe { &*d.fs };
        assert_eq!(fs.blend_key, 0x98e63);
        assert_eq!(fs.uses_fbfetch, 1, "blending is lowered into the FS: it reads the destination");
        assert!(!core::ptr::eq(d.fs, &gl.ctx().objs_ref().programs[obj_index(prog)].rfs));
        assert_ne!(d.fs_usc_gpu, plain.fs_usc_gpu);
        assert_eq!(d.vs_usc_gpu, plain.vs_usc_gpu, "the vertex shader has no variants");
        /* its USC was uploaded to the shader heap */
        assert_eq!(shader_allocs(&gl.mem_events()), [fs.usc_dw as usize * 4]);
        assert!(d.fs_usc_gpu >= HEAP_SHADER);
        assert_eq!(gl.gpu_bytes(d.fs_usc_gpu, fs.usc_dw as usize * 4), dword_bytes(fs.usc_words()));
    }

    #[test]
    #[cfg(feature = "gpu-powervr")] /* the PowerVR table's own values */
    fn a_blend_variant_is_linked_to_the_vertex_shader_like_the_base_shader() {
        /* The variant is a fresh copy from the offline table and carries the
         * fragment shader's own varying packing; without the link it
         * iterates the wrong vertex outputs. */
        let gl = Gl::new(16, 16);
        let prog = textured(&gl);
        /* This pair happens to pack its varyings the same way in both
         * stages, so move the vertex shader's outputs: the link has to
         * follow them, an unlinked variant keeps the table's offsets. */
        let n = gl.ctx().objs().programs[obj_index(prog)].rfs.num_varyings as usize;
        assert!(n > 0);
        let table_off: Vec<u8> =
            (0..n).map(|i| gl.ctx().objs().programs[obj_index(prog)].rfs.varyings[i].vtxout_off).collect();
        {
            let p = &mut gl.ctx().objs().programs[obj_index(prog)];
            for k in 0..p.rvs.num_varyings as usize {
                p.rvs.varyings[k].vtxout_off += 3;
            }
            p.rvs.vs_output_size += 3; /* the outputs moved up, the block grew */
            for v in p.rfs.varyings.iter_mut().take(n) {
                v.vtxout_off += 3; /* as glLinkProgram would have left it */
            }
        }
        // SAFETY: plain GL calls.
        unsafe {
            glEnable(GL_BLEND);
            glBlendFunc(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA);
        }
        let d = draw_tri(&gl).draw;
        // SAFETY: the variant lives in the draw path's static cache.
        let var = unsafe { &*d.fs };
        assert_ne!(var.blend_key, 0, "a variant was selected");
        assert_eq!(var.num_varyings as usize, n);
        for (i, &off) in table_off.iter().enumerate() {
            assert_eq!(
                var.varyings[i].vtxout_off,
                off + 3,
                "varying {i} is iterated from the vertex shader's slot, not the table's"
            );
        }
    }

    #[test]
    #[cfg(feature = "gpu-powervr")] /* the PowerVR table's own values */
    fn blend_variant_is_uploaded_once_and_dropped_when_blending_stops() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        // SAFETY: plain GL calls.
        unsafe {
            glEnable(GL_BLEND);
            glBlendFunc(GL_ONE, GL_ONE);
        }
        let first = draw_tri(&gl).draw;
        gl.mem_events();
        let second = draw_tri(&gl).draw;
        assert!(gl.mem_events().is_empty(), "the variant is cached per (program, key)");
        assert_eq!(first.fs_usc_gpu, second.fs_usc_gpu);
        // SAFETY: plain GL call.
        unsafe { glDisable(GL_BLEND) };
        let plain = draw_tri(&gl).draw;
        assert_ne!(plain.fs_usc_gpu, first.fs_usc_gpu);
        // SAFETY: the shader is the program's own.
        assert_eq!(unsafe { (*plain.fs).blend_key }, 0);
    }

    /* ---- blend-variant slots: least recently used, and what a refusal means ---- */

    #[cfg(feature = "gpu-powervr")]
    /// The blend keys the fake back end was asked for, in order.
    static FAKE_CALLS: std::sync::Mutex<Vec<u32>> = std::sync::Mutex::new(Vec::new());
    #[cfg(feature = "gpu-powervr")]
    /// Calls left that the fake answers with VARIANT_NO_ROOM.
    static FAKE_NO_ROOM: AtomicU32 = AtomicU32::new(0);
    #[cfg(feature = "gpu-powervr")]
    /// A key the fake refuses for good (-ENOENT); 0 = none.
    static FAKE_REFUSED: AtomicU32 = AtomicU32::new(0);

    #[cfg(feature = "gpu-powervr")]
    /// A blend variant that is the program's own fragment shader marked with
    /// the key, or the refusal the statics above script.
    unsafe extern "C" fn fake_blend_variant(
        fs: *const ShcShader,
        key: u32,
        out: *mut ShcShader,
        log: *mut core::ffi::c_char,
        log_sz: usize,
    ) -> i32 {
        FAKE_CALLS.lock().unwrap().push(key);
        // SAFETY: the caller's log, valid for log_sz bytes.
        let mut w = unsafe { util::CStrWriter::new(log, log_sz) };
        if FAKE_NO_ROOM.load(Ordering::Relaxed) != 0 {
            FAKE_NO_ROOM.fetch_sub(1, Ordering::Relaxed);
            let _ = w.write_str("fake: keep full");
            return VARIANT_NO_ROOM;
        }
        if key == FAKE_REFUSED.load(Ordering::Relaxed) {
            let _ = w.write_str("fake: refused for good");
            return -util::ENOENT;
        }
        // SAFETY: both point at shc_shaders the caller holds.
        unsafe {
            let mut v = fs.read();
            v.blend_key = key;
            out.write(v);
        }
        0
    }

    /// Put the fake in as the shader back end's blend variants for one test
    /// (the back end in place compiles and links); put it back on drop.
    #[cfg(feature = "gpu-powervr")]
    struct FakeVariants(*const ShcBackend);
    #[cfg(feature = "gpu-powervr")]
    impl FakeVariants {
        fn install() -> Self {
            FAKE_CALLS.lock().unwrap().clear();
            FAKE_NO_ROOM.store(0, Ordering::Relaxed);
            FAKE_REFUSED.store(0, Ordering::Relaxed);
            // SAFETY: the tests are serialised; the back end in place is a static.
            unsafe {
                let prev = crate::shaderc::g_shaderc;
                let fake: &'static ShcBackend =
                    Box::leak(Box::new(ShcBackend { blend_variant: Some(fake_blend_variant), ..*prev }));
                crate::shaderc::g_shaderc = fake;
                FakeVariants(prev)
            }
        }
        fn calls(&self) -> Vec<u32> {
            FAKE_CALLS.lock().unwrap().clone()
        }
    }
    #[cfg(feature = "gpu-powervr")]
    impl Drop for FakeVariants {
        fn drop(&mut self) {
            // SAFETY: as in install.
            unsafe { crate::shaderc::g_shaderc = self.0 };
        }
    }

    #[cfg(feature = "gpu-powervr")]
    /// Five blend states with five different keys.
    const FIVE_BLENDS: [(GLenum, GLenum); 5] = [
        (GL_ONE, GL_ONE),
        (GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA),
        (GL_ONE, GL_ONE_MINUS_SRC_ALPHA),
        (GL_ZERO, GL_SRC_COLOR),
        (GL_DST_COLOR, GL_ZERO),
    ];

    #[cfg(feature = "gpu-powervr")]
    /// Draw with blend state `i` of [`FIVE_BLENDS`]; the draw, or None when
    /// it was refused.
    fn draw_blended(gl: &Gl, i: usize) -> Option<RecordedDraw> {
        // SAFETY: plain GL calls; the arrays basic() set up are statics.
        unsafe {
            glEnable(GL_BLEND);
            glBlendFunc(FIVE_BLENDS[i].0, FIVE_BLENDS[i].1);
            glDrawArrays(GL_TRIANGLES, 0, 3);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        gl.draws().pop()
    }

    #[test]
    #[cfg(feature = "gpu-powervr")] /* blend variants are this GPU's */
    fn a_fifth_blend_state_evicts_the_least_recently_used_variant() {
        let gl = Gl::new(16, 16);
        let fake = FakeVariants::install();
        basic(&gl);
        assert_eq!(PROG_MAX_FS_VARIANTS, 4, "the test fills the slots with four keys");
        for i in 0..4 {
            assert!(draw_blended(&gl, i).is_some());
        }
        let keys = fake.calls();
        assert_eq!(keys.len(), 4);
        assert!(keys.iter().all(|&k| k != 0) && keys.windows(2).all(|w| w[0] != w[1]), "{keys:x?}");
        assert!(draw_blended(&gl, 0).is_some(), "state 0 used again: state 1 is now the oldest");
        assert_eq!(fake.calls().len(), 4, "a cached variant is not asked for again");
        let d = draw_blended(&gl, 4).expect("the fifth state draws");
        // SAFETY: the variant lives in the draw path's cache.
        assert_eq!(unsafe { (*d.draw.fs).blend_key }, fake.calls()[4]);
        for i in [0, 2, 3] {
            assert!(draw_blended(&gl, i).is_some());
        }
        assert_eq!(fake.calls().len(), 5, "states 0, 2 and 3 kept their variants: {:x?}", fake.calls());
        assert!(draw_blended(&gl, 1).is_some());
        assert_eq!(fake.calls()[5], keys[1], "state 1 was the one evicted");
    }

    #[test]
    #[cfg(feature = "gpu-powervr")]
    fn a_variant_refused_for_room_is_asked_for_again_by_the_next_draw() {
        let gl = Gl::new(16, 16);
        let fake = FakeVariants::install();
        basic(&gl);
        FAKE_NO_ROOM.store(1, Ordering::Relaxed);
        gl.logs();
        assert!(draw_blended(&gl, 0).is_none(), "no room: the draw is refused");
        let logs = gl.logs();
        assert!(
            logs.iter().any(|l| l.contains("no room for the blend key") && l.contains("fake: keep full")),
            "{logs:?}"
        );
        let d = draw_blended(&gl, 0).expect("room again: the next draw asks again and draws");
        let calls = fake.calls();
        assert_eq!(calls.len(), 2, "{calls:x?}");
        assert_eq!(calls[0], calls[1]);
        // SAFETY: the variant lives in the draw path's cache.
        assert_eq!(unsafe { (*d.draw.fs).blend_key }, calls[0]);
    }

    #[test]
    #[cfg(feature = "gpu-powervr")]
    fn a_variant_refused_for_good_is_not_asked_for_again_and_says_why() {
        let gl = Gl::new(16, 16);
        let fake = FakeVariants::install();
        basic(&gl);
        assert!(draw_blended(&gl, 1).is_some());
        let key = fake.calls()[0];
        /* a second program, so its slot table starts empty */
        basic(&gl);
        FAKE_REFUSED.store(key, Ordering::Relaxed);
        gl.logs();
        assert!(draw_blended(&gl, 1).is_none());
        let logs = gl.logs();
        let want = format!("no fragment shader variant for blend key {key:#x} (fake: refused for good)");
        assert!(logs.iter().any(|l| l.contains(&want)), "{want} not in {logs:?}");
        assert!(draw_blended(&gl, 1).is_none());
        assert_eq!(fake.calls().len(), 2, "remembered: {:x?}", fake.calls());
    }

    /* ---- gl_FrontFacing as a shader variant ------------------------------------------------------- */

    /// basic(), with the fragment shader marked as reading gl_FrontFacing, and
    /// a backend that compiles the winding in. Returns the key the draw asked
    /// the shader backend for (0 = it asked for none), read out of the log the
    /// miss leaves: no offline table here holds a variant of basic().
    fn front_facing_key(flip_y: bool, front_face: GLenum) -> u32 {
        let gl = Gl::new(16, 16);
        gl.backend_like_gc7000();
        let prog = basic(&gl);
        gl.ctx().objs().programs[obj_index(prog)].rfs.reads_front_facing = 1;
        if !flip_y {
            /* a texture is stored bottom-up, so rendering into one does not flip */
            fbo_8x8();
        }
        // SAFETY: plain GL call.
        unsafe { glFrontFace(front_face) };
        gl.logs();
        // SAFETY: the arrays the fixtures set up are statics.
        unsafe { glDrawArrays(GL_TRIANGLES, 0, 3) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        gl.draws();
        let logs = gl.logs();
        let line = logs.iter().find(|l| l.contains("no fragment shader variant for blend key 0x"));
        match line {
            Some(l) => {
                let hex = l.split("blend key 0x").nth(1).unwrap();
                let digits: String = hex.chars().take_while(|c| c.is_ascii_hexdigit()).collect();
                u32::from_str_radix(&digits, 16).unwrap()
            }
            None => 0,
        }
    }

    #[test]
    fn a_compiled_in_winding_takes_a_variant_when_exactly_one_of_two_things_holds() {
        /* upstream 6c8044b: this GPU hands the fragment a raw 0.0/1.0 for
         * gl_FrontFacing and the comparison is compiled in, so there is one
         * binary per winding. glFrontFace picks one; a y-flipped target
         * reverses the winding the rasteriser sees and picks the other. Both
         * together cancel, which is the same asymmetry the cull direction
         * needed. */
        assert_eq!(front_facing_key(true, GL_CCW), SHC_VIV_KEY_FRONT_FLIPPED, "the flip alone");
        assert_eq!(front_facing_key(false, GL_CW), SHC_VIV_KEY_FRONT_FLIPPED, "glFrontFace alone");
        assert_eq!(front_facing_key(true, GL_CW), 0, "both: back to the base binary");
        assert_eq!(front_facing_key(false, GL_CCW), 0, "neither");
    }

    #[test]
    fn a_shader_that_never_reads_gl_frontfacing_keeps_its_one_binary() {
        let gl = Gl::new(16, 16);
        gl.backend_like_gc7000();
        basic(&gl); /* reads_front_facing stays 0 */
        gl.logs();
        draw_tri(&gl);
        assert!(gl.logs().iter().all(|l| !l.contains("no fragment shader variant")), "no variant was asked for");
    }

    #[test]
    fn a_backend_with_a_front_face_register_asks_for_no_variant() {
        /* the PowerVR writes the front-face op into a shared register at draw
         * time, so one binary serves both windings whatever the shader reads */
        let gl = Gl::new(16, 16);
        let prog = basic(&gl);
        gl.ctx().objs().programs[obj_index(prog)].rfs.reads_front_facing = 1;
        gl.logs();
        draw_tri(&gl);
        assert!(gl.logs().iter().all(|l| !l.contains("no fragment shader variant")));
    }

    #[test]
    #[cfg(feature = "gpu-powervr")] /* blends in the shader */
    fn blend_state_without_a_variant_refuses_the_draw_and_logs_once() {
        /* Drawn with the plain FS, the draw would be a picture of another
         * blend state (the PowerVR's subtract variants, 2026-09-24): refused,
         * counted, logged once per program and key. */
        let gl = Gl::new(16, 16);
        basic(&gl);
        // SAFETY: plain GL calls.
        unsafe {
            glEnable(GL_BLEND);
            glBlendFunc(GL_DST_COLOR, GL_ZERO); /* modulate: not in the offline table */
        }
        gl.logs();
        assert_eq!(refused_tri(&gl), 1);
        /* DST_COLOR 5 | ZERO 0x11 << 5 | 5 << 10 | 0x11 << 15 */
        let logs = gl.logs();
        assert!(
            logs.iter().any(|l| l.contains("no fragment shader variant for blend key 0x89625")),
            "logged: {logs:?}"
        );
        gl.mem_events();
        assert_eq!(refused_tri(&gl), 1, "every refused draw is counted");
        assert!(gl.logs().iter().all(|l| !l.contains("no fragment shader variant")), "the miss is remembered");
        assert!(gl.mem_events().is_empty());
        /* blending off: the plain shader is the right one again */
        // SAFETY: plain GL call.
        unsafe { glDisable(GL_BLEND) };
        draw_tri(&gl);
    }

    #[test]
    #[cfg(feature = "gpu-powervr")] /* blends in the shader */
    fn a_refused_draw_leaves_its_pending_clear_for_the_next() {
        /* The glClear fused into a draw must not vanish with the draw. */
        let gl = Gl::new(16, 16);
        basic(&gl);
        // SAFETY: plain GL calls.
        unsafe {
            glClearColor(0.0, 1.0, 0.0, 1.0);
            glClear(GL_COLOR_BUFFER_BIT);
            glEnable(GL_BLEND);
            glBlendFunc(GL_DST_COLOR, GL_ZERO);
        }
        assert_eq!(refused_tri(&gl), 1);
        // SAFETY: plain GL calls.
        unsafe {
            glDisable(GL_BLEND);
            glDrawArrays(GL_TRIANGLES, 0, 3);
        }
        let d = one(&gl).draw;
        assert!(d.do_clear, "the clear rides the next draw");
        assert_eq!(d.clear_rgba, [0.0, 1.0, 0.0, 1.0]);
    }

    #[test]
    #[cfg(feature = "gpu-powervr")] /* the PowerVR table's own values */
    fn partial_colour_mask_selects_a_mask_variant() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        // SAFETY: plain GL calls.
        unsafe { glColorMask(0, 1, 0, 1) };
        let d = draw_tri(&gl).draw;
        // SAFETY: the variant lives in the draw path's static cache.
        assert_eq!(unsafe { (*d.fs).blend_key }, 0x88621 | (0b0101 << 26));
        assert!(d.color_write);
        assert!(!d.blend);
    }

    #[test]
    fn fully_masked_colour_disables_colour_write_without_a_variant() {
        let gl = Gl::new(16, 16);
        let prog = basic(&gl);
        // SAFETY: plain GL calls.
        unsafe { glColorMask(0, 0, 0, 0) };
        let d = draw_tri(&gl).draw;
        assert!(!d.color_write);
        assert!(core::ptr::eq(d.fs, &gl.ctx().objs_ref().programs[obj_index(prog)].rfs));
    }

    #[test]
    #[cfg(feature = "gpu-powervr")] /* the PowerVR table's own values */
    fn fully_masked_discard_shader_runs_the_zero_one_variant_with_colour_write_on() {
        /* the stencil-only pass of a shader with `discard` (femtovg): the ISP
         * tag write must stay enabled or the stencil op never happens */
        let gl = Gl::new(16, 16);
        let prog = gl.program("main.vert", "fvgtest_discard.frag");
        assert_eq!(gl.ctx().objs().programs[obj_index(prog)].rfs.uses_discard, 1);
        // SAFETY: plain GL calls; both vec2 inputs read their constants.
        unsafe {
            glUseProgram(prog);
            glColorMask(0, 0, 0, 0);
            glDrawArrays(GL_TRIANGLES, 0, 3);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        let d = one(&gl).draw;
        assert!(d.color_write);
        /* ZERO 0x11 | ONE 1 << 5 | ZERO << 10 | ONE << 15 */
        // SAFETY: the variant lives in the draw path's static cache.
        assert_eq!(unsafe { (*d.fs).blend_key }, 0x11 | (1 << 5) | (0x11 << 10) | (1 << 15));
    }

    #[test]
    #[cfg(feature = "gpu-powervr")] /* the PowerVR table's own values */
    fn constant_colour_variant_reads_glblendcolor_after_the_app_uniforms() {
        let gl = Gl::new(16, 16);
        let prog = basic(&gl);
        // SAFETY: plain GL calls; valid NUL-terminated name.
        unsafe {
            glUniform4f(glGetUniformLocation(prog, c"u_color".as_ptr()), 1.0, 2.0, 3.0, 4.0);
            glEnable(GL_BLEND);
            glBlendFunc(GL_CONSTANT_COLOR, GL_ONE_MINUS_CONSTANT_COLOR);
            glBlendColor(0.125, 0.25, 0.5, 1.0);
        }
        let d = draw_tri(&gl);
        assert_eq!(d.draw.fs_uniform_dwords, 8, "u_color + the 4 blend constants");
        assert_eq!(d.fs_uniforms, bits(&[1.0, 2.0, 3.0, 4.0, 0.125, 0.25, 0.5, 1.0]));
    }

    #[test]
    #[cfg(feature = "gpu-powervr")] /* the PowerVR table's own values */
    fn fifth_blend_state_recycles_the_first_variant_slot() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        // SAFETY: plain GL calls.
        unsafe { glEnable(GL_BLEND) };
        let mut uscs = Vec::new();
        for (s, d) in [
            (GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA),
            (GL_ONE, GL_ONE_MINUS_SRC_ALPHA),
            (GL_ONE, GL_ONE),
            (GL_SRC_ALPHA, GL_ONE),
        ] {
            // SAFETY: plain GL call.
            unsafe { glBlendFunc(s, d) };
            uscs.push(draw_tri(&gl).draw.fs_usc_gpu);
        }
        assert!(frees(&gl.mem_events()).is_empty(), "four variants fit");
        // SAFETY: plain GL call.
        unsafe { glBlendFunc(GL_ZERO, GL_ONE) };
        let fifth = draw_tri(&gl).draw;
        assert_eq!(frees(&gl.mem_events()), [uscs[0]], "the table is full: slot 0 is recycled");
        // SAFETY: the variant lives in the draw path's static cache.
        assert_eq!(unsafe { (*fifth.fs).blend_key }, 0x11 | (1 << 5) | (0x11 << 10) | (1 << 15));
    }

    #[test]
    #[cfg(feature = "gpu-powervr")] /* blends in the shader */
    fn variant_upload_failure_refuses_the_draw() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        draw_tri(&gl);
        // SAFETY: plain GL calls.
        unsafe {
            glEnable(GL_BLEND);
            glBlendFunc(GL_ONE, GL_ONE);
        }
        gl.fail_alloc_in(1);
        assert_eq!(refused_tri(&gl), 1);
    }

    /* ---- per-program GPU cache ---------------------------------------------------- */

    #[test]
    #[cfg(feature = "gpu-powervr")] /* the PowerVR table's own values */
    fn usc_is_uploaded_to_the_shader_heap_on_the_first_draw() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        gl.mem_events();
        let d = draw_tri(&gl).draw;
        let ev = gl.mem_events();
        /* passthrough.vert is 8 USC dwords, uniform_color.frag 16 */
        assert_eq!(shader_allocs(&ev), [32, 64]);
        assert!(
            ev.contains(&MemEvent::Alloc {
                heap: HEAP_DATA,
                size: MAX_UNIFORM_DWORDS * 4 * 2 + 16,
                gpu: d.fs_uniform_gpu
            }),
            "one uniform buffer for both stages: {ev:?}"
        );
        assert!(ev.contains(&MemEvent::Alloc { heap: HEAP_SHADER, size: 32, gpu: d.vs_usc_gpu }));
        assert!(ev.contains(&MemEvent::Alloc { heap: HEAP_SHADER, size: 64, gpu: d.fs_usc_gpu }));
        // SAFETY: the shaders are the linked program's.
        let (vs, fs) = unsafe { (&*d.vs, &*d.fs) };
        assert_eq!(gl.gpu_bytes(d.vs_usc_gpu, 32), dword_bytes(vs.usc_words()));
        assert_eq!(gl.gpu_bytes(d.fs_usc_gpu, 64), dword_bytes(fs.usc_words()));
    }

    #[test]
    fn usc_is_uploaded_once_per_link() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        let first = draw_tri(&gl).draw;
        gl.mem_events();
        let second = draw_tri(&gl).draw;
        assert!(shader_allocs(&gl.mem_events()).is_empty());
        assert_eq!((first.vs_usc_gpu, first.fs_usc_gpu), (second.vs_usc_gpu, second.fs_usc_gpu));
        assert_eq!(first.fs_uniform_gpu, second.fs_uniform_gpu);
    }

    #[test]
    fn each_program_has_its_own_usc() {
        let gl = Gl::new(16, 16);
        let a = basic(&gl);
        let da = draw_tri(&gl).draw;
        let b = gl.program("passthrough.vert", "interp_color.frag");
        // SAFETY: plain GL call.
        unsafe { glUseProgram(b) };
        let db = draw_tri(&gl).draw;
        assert_ne!(da.fs_usc_gpu, db.fs_usc_gpu);
        if VS_HAS_OWN_CODE {
            assert_ne!(da.vs_usc_gpu, db.vs_usc_gpu);
        } else {
            assert_eq!((da.vs_usc_gpu, db.vs_usc_gpu), (0, 0), "a V3D VS has no code of its own");
        }
        // SAFETY: plain GL call.
        unsafe { glUseProgram(a) };
        gl.mem_events();
        assert_eq!(draw_tri(&gl).draw.fs_usc_gpu, da.fs_usc_gpu);
        assert!(gl.mem_events().iter().all(|e| !matches!(e, MemEvent::Alloc { heap: HEAP_SHADER, .. })));
    }

    #[test]
    #[cfg(feature = "gpu-powervr")] /* the PowerVR table's own values */
    fn relink_frees_the_cached_uploads_and_the_next_draw_uploads_again() {
        let gl = Gl::new(16, 16);
        let prog = basic(&gl);
        // SAFETY: plain GL calls.
        unsafe {
            glEnable(GL_BLEND);
            glBlendFunc(GL_ONE, GL_ONE);
        }
        let before = draw_tri(&gl).draw;
        // SAFETY: plain GL call.
        unsafe { glDisable(GL_BLEND) };
        let plain = draw_tri(&gl).draw;
        gl.mem_events();
        // SAFETY: plain GL call.
        unsafe { glLinkProgram(prog) };
        let mut freed = frees(&gl.mem_events());
        freed.sort_unstable();
        let mut want = std::vec![plain.vs_usc_gpu, plain.fs_usc_gpu, plain.fs_uniform_gpu, before.fs_usc_gpu];
        want.sort_unstable();
        assert_eq!(freed, want, "VS USC, FS USC, the uniform buffer and the blend variant");
        draw_tri(&gl);
        assert_eq!(shader_allocs(&gl.mem_events()), [32, 64]);
    }

    #[test]
    fn delete_program_releases_its_gpu_side() {
        let gl = Gl::new(16, 16);
        let prog = basic(&gl);
        let d = draw_tri(&gl).draw;
        gl.mem_events();
        // SAFETY: plain GL calls.
        unsafe {
            glDeleteProgram(prog);
            /* the current program is only flagged (GL ES 3.0 2.12.3); it goes when it stops being current */
            glUseProgram(0);
        }
        let freed = frees(&gl.mem_events());
        let mut owned = std::vec![d.fs_usc_gpu, d.fs_uniform_gpu];
        if VS_HAS_OWN_CODE {
            owned.push(d.vs_usc_gpu);
        } else {
            assert_eq!(d.vs_usc_gpu, 0, "a V3D VS has no code of its own");
        }
        for gpu in owned {
            assert!(freed.contains(&gpu), "0x{gpu:x} not freed: {freed:x?}");
        }
    }

    #[test]
    fn c_invalidate_and_release_entry_points_free_the_cache() {
        let gl = Gl::new(16, 16);
        let prog = basic(&gl);
        draw_tri(&gl);
        gl.mem_events();
        /* the VS code, the FS code and the uniform buffer (no VS code on the V3D) */
        let blocks = if VS_HAS_OWN_CODE { 3 } else { 2 };
        // SAFETY: plain calls on the current context.
        unsafe { gl_draw_invalidate_program(prog) };
        assert_eq!(frees(&gl.mem_events()).len(), blocks);
        draw_tri(&gl);
        gl.mem_events();
        // SAFETY: as above.
        unsafe {
            gl_draw_release_program(prog);
            assert_eq!(frees(&gl.mem_events()).len(), blocks);
            gl_draw_release_program(prog); /* nothing left: no double free */
            /* out of range: ignored. It has to be built as a NAME -- the raw
             * number MAX_PROGRAMS decodes to slot 256, which is in range. */
            gl_draw_invalidate_program(context::obj_name(MAX_PROGRAMS, context::OBJ_PROGRAM));
            gl_draw_invalidate_program(u32::MAX);
        }
        assert!(gl.mem_events().is_empty());
    }

    #[test]
    #[cfg(feature = "gpu-powervr")] /* the PowerVR table's own values */
    fn debug_usc_reports_the_uploads_of_a_drawn_program() {
        let gl = Gl::new(16, 16);
        let prog = basic(&gl);
        // SAFETY: plain calls on the current context.
        unsafe {
            assert!(gl_draw_debug_usc(prog, 0).is_null(), "nothing uploaded before the first draw");
            assert!(gl_draw_debug_usc(context::obj_name(MAX_PROGRAMS, context::OBJ_PROGRAM), 0).is_null());
        }
        let d = draw_tri(&gl).draw;
        // SAFETY: non-null results point into the draw path's static cache.
        unsafe {
            assert_eq!((*gl_draw_debug_usc(prog, 0)).gpu, d.vs_usc_gpu);
            assert_eq!((*gl_draw_debug_usc(prog, 1)).gpu, d.fs_usc_gpu);
            assert_eq!((*gl_draw_debug_usc(prog, 0)).size, 32);
        }
    }

    #[test]
    fn usc_allocation_failure_is_out_of_memory_and_a_later_draw_recovers() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        gl.fail_alloc_in(1);
        // SAFETY: plain GL call.
        unsafe { glDrawArrays(GL_TRIANGLES, 0, 3) };
        assert_eq!(gl.error(), GL_OUT_OF_MEMORY);
        assert!(gl.draws().is_empty());
        assert_eq!(draw_tri(&gl).draw.index_count, 3);
    }

    #[test]
    fn partial_usc_upload_failure_does_not_leak() {
        let gl = Gl::new(16, 16);
        let prog = basic(&gl);
        let baseline = gl.live_allocs();
        gl.fail_alloc_in(2); /* VS USC succeeds, FS USC fails */
        // SAFETY: plain GL calls.
        unsafe { glDrawArrays(GL_TRIANGLES, 0, 3) };
        assert_eq!(gl.error(), GL_OUT_OF_MEMORY);
        draw_tri(&gl);
        /* GL ES 3.0 2.12.3: a program deleted while current is destroyed when
         * it stops being current */
        // SAFETY: plain GL calls.
        unsafe {
            glDeleteProgram(prog);
            glUseProgram(0);
        }
        /* what is left: the two attribute scratch buffers of the draw */
        assert_eq!(gl.live_allocs(), baseline + 2);
    }

    /* ---- uniforms -------------------------------------------------------------------- */

    #[test]
    fn fs_uniforms_reach_the_draw_and_gpu_memory() {
        let gl = Gl::new(16, 16);
        let prog = basic(&gl);
        // SAFETY: plain GL calls; valid NUL-terminated name.
        unsafe { glUniform4f(glGetUniformLocation(prog, c"u_color".as_ptr()), 0.5, -1.0, 2.0, 8.0) };
        let d = draw_tri(&gl);
        assert_eq!(d.draw.fs_uniform_dwords, 4);
        assert_eq!(d.fs_uniforms, bits(&[0.5, -1.0, 2.0, 8.0]));
        assert_eq!(f32s(&gl.gpu_bytes(d.draw.fs_uniform_gpu, 16)), [0.5, -1.0, 2.0, 8.0]);
        /* passthrough.vert has no uniforms */
        assert_eq!(d.draw.vs_uniform_dwords, 0);
        assert_eq!(d.draw.vs_uniform_gpu, 0);
        assert!(d.draw.vs_uniform_cpu.is_null());
        assert!(d.vs_uniforms.is_empty());
    }

    #[test]
    fn vs_block_comes_first_and_the_fs_block_follows_16_byte_aligned() {
        let gl = Gl::new(16, 16);
        let prog = gl.program("mvp.vert", "uniform_color.frag");
        let mvp: [f32; 16] = core::array::from_fn(|i| i as f32 + 1.0);
        // SAFETY: plain GL calls; valid names; `mvp` has the 16 floats of one mat4.
        unsafe {
            glUseProgram(prog);
            glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, TRI.as_ptr() as *const c_void);
            glEnableVertexAttribArray(0);
            glUniformMatrix4fv(glGetUniformLocation(prog, c"u_mvp".as_ptr()), 1, 0, mvp.as_ptr());
            glUniform4f(glGetUniformLocation(prog, c"u_color".as_ptr()), 9.0, 8.0, 7.0, 6.0);
        }
        let d = draw_tri(&gl);
        assert_eq!(d.draw.vs_uniform_dwords, 16);
        assert_eq!(d.vs_uniforms, bits(&mvp));
        assert_eq!(d.draw.fs_uniform_dwords, 4);
        assert_eq!(d.fs_uniforms, bits(&[9.0, 8.0, 7.0, 6.0]));
        assert_eq!(d.draw.fs_uniform_gpu, d.draw.vs_uniform_gpu + 64, "16 dwords = 64 bytes, already aligned");
        assert_eq!(f32s(&gl.gpu_bytes(d.draw.vs_uniform_gpu, 64)), mvp);
        assert_eq!(f32s(&gl.gpu_bytes(d.draw.fs_uniform_gpu, 16)), [9.0, 8.0, 7.0, 6.0]);
    }

    #[test]
    #[cfg(feature = "gpu-powervr")] /* the PowerVR table's own values */
    fn odd_sized_vs_block_pads_the_fs_block_to_16_bytes() {
        let gl = Gl::new(16, 16);
        /* femtovg main.vert: uniform vec2 viewSize (2 dwords) */
        let prog = gl.program("main.vert", "fvgtest_discard.frag");
        // SAFETY: plain GL calls; valid name.
        unsafe {
            glUseProgram(prog);
            glUniform2f(glGetUniformLocation(prog, c"viewSize".as_ptr()), 640.0, 480.0);
            glDrawArrays(GL_TRIANGLES, 0, 3);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        let d = one(&gl);
        assert_eq!(d.vs_uniforms, bits(&[640.0, 480.0]));
        assert_eq!(d.draw.fs_uniform_gpu, d.draw.vs_uniform_gpu + 16);
        assert_eq!(d.draw.fs_uniform_dwords, 52);
    }

    #[test]
    fn uniforms_are_snapshotted_per_draw() {
        let gl = Gl::new(16, 16);
        let prog = basic(&gl);
        // SAFETY: plain GL calls; valid NUL-terminated name.
        unsafe {
            let loc = glGetUniformLocation(prog, c"u_color".as_ptr());
            glUniform4f(loc, 1.0, 0.0, 0.0, 1.0);
            glDrawArrays(GL_TRIANGLES, 0, 3);
            glUniform4f(loc, 0.0, 1.0, 0.0, 1.0);
            glDrawArrays(GL_TRIANGLES, 0, 3);
        }
        let draws = gl.draws();
        assert_eq!(draws[0].fs_uniforms, bits(&[1.0, 0.0, 0.0, 1.0]));
        assert_eq!(draws[1].fs_uniforms, bits(&[0.0, 1.0, 0.0, 1.0]));
        assert!(!draws[0].draw.fs_uniform_cpu.is_null(), "the backend gets the CPU block to copy");
    }

    /* ---- attributes ---------------------------------------------------------------------- */

    #[test]
    fn float_vbo_attribute_is_fetched_in_place() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        let vbo = buffer_with(GL_ARRAY_BUFFER, dword_bytes(&TRI));
        // SAFETY: plain GL call; the pointer is a buffer offset.
        unsafe { glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, core::ptr::null()) };
        let d = draw_tri(&gl);
        let a = d.draw.attribs[0];
        assert_eq!(a.vi_reg, 0);
        assert_eq!(a.components, 4);
        assert_eq!(a.format, 0);
        assert_eq!(a.stride, 16, "stride 0 means tightly packed");
        assert_eq!(a.offset, 0);
        assert_eq!(a.buffer_gpu, gl.ctx().objs().buffers[vbo as usize].mem.gpu);
        assert!(a.cpu.is_null());
        assert_eq!(a.cpu_bytes, 0);
        assert!(d.attrib_data[0].is_empty());
    }

    #[test]
    fn in_place_attribute_folds_first_offset_and_stride() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        buffer_with(GL_ARRAY_BUFFER, &[0u8; 256]);
        // SAFETY: plain GL calls; the pointer is a buffer offset.
        unsafe {
            glVertexAttribPointer(0, 4, GL_FLOAT, 0, 24, 8 as *const c_void);
            glDrawArrays(GL_TRIANGLES, 2, 3);
        }
        let a = one(&gl).draw.attribs[0];
        assert_eq!(a.stride, 24);
        assert_eq!(a.offset, 8 + 2 * 24, "vertex `first` is the first one fetched");
    }

    #[test]
    fn in_place_attribute_wider_than_the_input_fetches_only_the_inputs_components() {
        let gl = Gl::new(16, 16);
        let prog = gl.program("main.vert", "fvgtest_discard.frag"); /* vec2 vertex @0, vec2 tcoord @1 */
        buffer_with(GL_ARRAY_BUFFER, &[0u8; 64]);
        // SAFETY: plain GL calls; the pointer is a buffer offset.
        unsafe {
            glUseProgram(prog);
            glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, core::ptr::null());
            glEnableVertexAttribArray(0);
            glDrawArrays(GL_TRIANGLES, 0, 3);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        let d = one(&gl);
        assert_eq!(d.draw.attribs[0].components, 2);
        assert_eq!(d.draw.attribs[0].stride, 16, "the array's own stride");
        assert!(d.attrib_data[0].is_empty());
    }

    #[test]
    fn vbo_attribute_use_is_stamped_with_the_kick_id() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        let vbo = buffer_with(GL_ARRAY_BUFFER, dword_bytes(&TRI));
        // SAFETY: plain GL calls.
        unsafe {
            glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, core::ptr::null());
            glFlush(); /* nothing recorded: nothing submitted */
        }
        draw_tri(&gl);
        assert_eq!(gl.ctx().objs().buffers[vbo as usize].last_use, 1);
        // SAFETY: plain GL call.
        unsafe { glFlush() }; /* kick 1 submitted, 2 is recorded next */
        draw_tri(&gl);
        assert_eq!(gl.ctx().objs().buffers[vbo as usize].last_use, 2);
    }

    #[test]
    fn client_float_array_is_staged() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        let d = draw_tri(&gl);
        let a = d.draw.attribs[0];
        assert_eq!((a.vi_reg, a.components, a.format), (0, 4, 0));
        assert_eq!((a.stride, a.offset), (16, 0));
        assert_eq!(a.cpu_bytes, 3 * 16);
        assert!(a.buffer_gpu >= HEAP_DATA && a.buffer_gpu < HEAP_SHADER);
        assert_eq!(f32s(&d.attrib_data[0]), TRI);
        assert_eq!(f32s(&gl.gpu_bytes(a.buffer_gpu, 48)), TRI, "the staging is GPU memory too");
    }

    #[test]
    fn client_array_staging_starts_at_first() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        let verts: [f32; 20] = core::array::from_fn(|i| i as f32);
        // SAFETY: `verts` outlives the draw and holds vertices 0..4.
        unsafe {
            glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, verts.as_ptr() as *const c_void);
            glDrawArrays(GL_TRIANGLES, 2, 3);
        }
        let d = one(&gl);
        assert_eq!(d.draw.index_count, 3);
        assert_eq!(f32s(&d.attrib_data[0]), verts[8..20]);
        assert_eq!(d.draw.attribs[0].offset, 0);
    }

    #[test]
    fn client_array_honours_its_stride() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        /* 4 floats of position then 2 of padding per vertex */
        let verts: [f32; 18] = core::array::from_fn(|i| i as f32);
        // SAFETY: `verts` outlives the draw: 3 vertices of 24 bytes.
        unsafe {
            glVertexAttribPointer(0, 4, GL_FLOAT, 0, 24, verts.as_ptr() as *const c_void);
            glDrawArrays(GL_TRIANGLES, 0, 3);
        }
        let d = one(&gl);
        assert_eq!(f32s(&d.attrib_data[0]), [0.0, 1.0, 2.0, 3.0, 6.0, 7.0, 8.0, 9.0, 12.0, 13.0, 14.0, 15.0]);
        assert_eq!(d.draw.attribs[0].stride, 16, "the staged copy is tightly packed");
    }

    #[test]
    fn array_narrower_than_the_input_is_widened_with_0_0_0_1() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        let xy: [f32; 6] = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
        // SAFETY: `xy` outlives the draw: 3 vertices of 2 floats.
        unsafe {
            glVertexAttribPointer(0, 2, GL_FLOAT, 0, 0, xy.as_ptr() as *const c_void);
            glDrawArrays(GL_TRIANGLES, 0, 3);
        }
        let d = one(&gl);
        assert_eq!(d.draw.attribs[0].components, 4);
        assert_eq!(f32s(&d.attrib_data[0]), [1.0, 2.0, 0.0, 1.0, 3.0, 4.0, 0.0, 1.0, 5.0, 6.0, 0.0, 1.0]);
    }

    #[test]
    fn narrow_float_vbo_array_is_converted_not_fetched_in_place() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        let xy: [f32; 6] = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
        let vbo = buffer_with(GL_ARRAY_BUFFER, dword_bytes(&xy));
        // SAFETY: plain GL calls; the pointer is a buffer offset.
        unsafe { glVertexAttribPointer(0, 2, GL_FLOAT, 0, 0, core::ptr::null()) };
        let d = draw_tri(&gl);
        assert_ne!(d.draw.attribs[0].buffer_gpu, gl.ctx().objs().buffers[vbo as usize].mem.gpu);
        assert_eq!(f32s(&d.attrib_data[0]), [1.0, 2.0, 0.0, 1.0, 3.0, 4.0, 0.0, 1.0, 5.0, 6.0, 0.0, 1.0]);
    }

    /// No vertex element is fetched past the memory behind it. A record that
    /// claims more components than an element can hold -- etna_shc's
    /// 16-component mat4 before per-column expansion -- is refused with
    /// GL_INVALID_OPERATION on every sourcing path, and nothing reaches the
    /// backend.
    #[test]
    fn an_attribute_that_would_be_fetched_out_of_range_is_refused() {
        for path in ["client array", "vbo", "no array"] {
            let gl = Gl::new(16, 16);
            let prog = basic(&gl);
            gl.ctx().objs().programs[obj_index(prog)].rvs.attribs[0].components = 16;
            let vbo = buffer_with(GL_ARRAY_BUFFER, dword_bytes(&TRI));
            // SAFETY: plain GL calls; TRI is a static.
            unsafe {
                match path {
                    "vbo" => glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, core::ptr::null()),
                    "no array" => glDisableVertexAttribArray(0),
                    _ => {
                        glBindBuffer(GL_ARRAY_BUFFER, 0);
                        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, TRI.as_ptr() as *const c_void)
                    }
                }
                glDrawArrays(GL_TRIANGLES, 0, 3);
            }
            let _ = vbo;
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "{path}");
            assert!(gl.draws().is_empty(), "{path}: nothing may reach the backend");
        }
    }

    #[test]
    fn a_fetch_fits_exactly_to_its_last_byte() {
        let pa = |components: u8, stride: u32| GlesAttrib { components, stride, ..GlesAttrib::ZERO };
        assert!(fetch_fits(&pa(4, 16), 3, 16, 48), "three packed vec4s in 48 bytes");
        assert!(!fetch_fits(&pa(4, 16), 3, 16, 47), "one byte short");
        assert!(!fetch_fits(&pa(16, 64), 3, 64, 48), "gl_test 63's mat4: a 64-byte stride over 16-byte rows");
        assert!(!fetch_fits(&pa(0, 16), 3, 0, 48), "no components is not an element");
        assert!(fetch_fits(&pa(4, 0), 1000, 16, 16), "a zero stride reads one element");
    }

    /// A mat4 attribute is four vertex inputs, one per column, each four
    /// wide, into registers vi_reg .. vi_reg + 3, each staged from its own
    /// location's array: gl_test 63 (matattr) and the case whose etna_shc
    /// program left the GPU drawing garbage for later clients. The table
    /// (etna_shc) records it as 16 components, the in-process compiler as 4;
    /// whichever built the program, the elements must be these.
    #[cfg(feature = "gpu-vivante")] /* the PowerVR table has no matattr */
    #[test]
    fn a_matrix_attribute_is_one_input_per_column() {
        let gl = Gl::new(16, 16);
        let prog = gl.program("matattr4.vert", "matattr.frag");
        let tri: [f32; 12] = [0.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0];
        /* column k of vertex v holds 100k + 4v + (0..4) */
        let cols: [[f32; 12]; 4] =
            core::array::from_fn(|k| core::array::from_fn(|i| (100 * k + (i / 4) * 4 + i % 4) as f32));
        // SAFETY: plain GL calls; the arrays outlive the draw.
        unsafe {
            glUseProgram(prog);
            assert_eq!(glGetAttribLocation(prog, c"a_xf".as_ptr()), 1);
            glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, tri.as_ptr() as *const c_void);
            glEnableVertexAttribArray(0);
            for (k, col) in cols.iter().enumerate() {
                glVertexAttribPointer(1 + k as GLuint, 4, GL_FLOAT, 0, 0, col.as_ptr() as *const c_void);
                glEnableVertexAttribArray(1 + k as GLuint);
            }
            glDrawArrays(GL_TRIANGLES, 0, 3);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        let d = one(&gl);
        assert_eq!(d.draw.num_attribs, 5, "a_pos and four columns");
        for (k, col) in cols.iter().enumerate() {
            let a = d.draw.attribs[1 + k];
            assert_eq!((a.vi_reg, a.components, a.stride), (1 + k as u8, 4, 16), "column {k}");
            assert_eq!(a.cpu_bytes, 3 * 16, "column {k}: the copy covers what the stride walks");
            assert_eq!(f32s(&d.attrib_data[1 + k]), col, "column {k} is staged from its own location");
        }
    }

    /// dEQP-GLES2 default_vertex_attrib.mat{2,3,4}.vertex_attrib_* as dEQP
    /// drives it, through the in-process compiler: the arrays stay disabled,
    /// glVertexAttrib* loads column 1 (the base location + 1) and the shader
    /// reads `a_value[1]`. Every one of the 16 failed on the i.MX8MP Zephyr
    /// run (2026-09-24). The GL core's part: the draw stages one zero-stride
    /// constant per column, column 1 carrying the value and the others their
    /// untouched (0, 0, 0, 1), on consecutive input registers. HOST-ONLY:
    /// what the back end reads from those registers is not covered here.
    ///
    /// It was ignored while the GC7000 code generator dropped a matrix
    /// attribute read only through a column past the first (`select.rs`
    /// decided an input was used by the exact slot of a `LoadInput`, so
    /// `a_value` left the attribute table and glGetAttribLocation answered
    /// -1); f318bd77 made any column's read keep the whole matrix.
    #[cfg(all(feature = "shaderc-online", feature = "gpu-vivante"))]
    #[test]
    fn a_matrix_default_attribute_stages_the_set_column_and_the_defaults() {
        use std::ffi::CString;
        struct BackEnd(*const crate::shaderc::ShcBackend);
        impl Drop for BackEnd {
            fn drop(&mut self) {
                // SAFETY: restoring what was saved, under the suite's guard.
                unsafe { crate::shaderc::g_shaderc = self.0 };
            }
        }
        let gl = Gl::new(32, 32);
        // SAFETY: the guard serialises the suite; the restorer puts it back.
        let _be = BackEnd(unsafe { crate::shaderc::g_shaderc });
        // SAFETY: as above; a static table pointer.
        unsafe { crate::shaderc::g_shaderc = &crate::shaderc::online::SHC_ONLINE };
        let fs = CString::new("varying highp vec4 v_color;\nvoid main (void) { gl_FragColor = v_color; }\n").unwrap();
        let value = [0.1f32, 0.7, 1.0, 0.6]; /* dEQP's second test value */
        for (cols, ty, colour) in [
            (2usize, "mat2", "vec4(v.xy, 0.0, 1.0)"),
            (3, "mat3", "vec4(v.xyz, 1.0)"),
            (4, "mat4", "vec4((v.xy + v.zz) / 2.0, v.w, 1.0)"),
        ] {
            let vs_src = std::format!(
                "attribute highp vec4 a_position;\nattribute highp {ty} a_value;\nvarying highp vec4 v_color;\n\
                 void main (void)\n{{\n    gl_Position = a_position;\n    highp vec{cols} v = a_value[1];\n    \
                 v_color = {colour};\n}}\n"
            );
            let vs = CString::new(vs_src.clone()).unwrap();
            // SAFETY: plain GL calls on handles this test owns; TRI is a static.
            let (pos, val) = unsafe {
                let sh = |kind: GLenum, t: &CString| {
                    use crate::gl::shader::{glCompileShader, glCreateShader, glShaderSource};
                    let s = glCreateShader(kind);
                    let p: [*const core::ffi::c_char; 1] = [t.as_ptr()];
                    glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
                    glCompileShader(s);
                    s
                };
                let p = glCreateProgram();
                glAttachShader(p, sh(GL_VERTEX_SHADER, &vs));
                glAttachShader(p, sh(GL_FRAGMENT_SHADER, &fs));
                glLinkProgram(p);
                let mut ok: GLint = 0;
                glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
                assert_eq!(ok, 1, "{vs_src}");
                glUseProgram(p);
                let pos = glGetAttribLocation(p, c"a_position".as_ptr());
                let val = glGetAttribLocation(p, c"a_value".as_ptr());
                assert!(pos >= 0 && val >= 0, "{ty}: locations {pos} {val}");
                for i in 0..MAX_VERTEX_ATTRIBS as GLuint {
                    glDisableVertexAttribArray(i);
                    glVertexAttrib4f(i, 0.0, 0.0, 0.0, 1.0);
                }
                glVertexAttribPointer(pos as GLuint, 4, GL_FLOAT, 0, 0, TRI.as_ptr() as *const c_void);
                glEnableVertexAttribArray(pos as GLuint);
                glVertexAttrib4f(val as GLuint + 1, value[0], value[1], value[2], value[3]);
                (pos as GLuint, val as GLuint)
            };
            let d = draw_tri(&gl);
            let n = usize::from(d.draw.num_attribs);
            assert_eq!(n, 1 + cols, "{ty}: a_position and one input per column");
            /* the columns: every zero-stride constant of the draw, by register */
            let mut columns: Vec<(u8, Vec<f32>)> = (0..n)
                .filter(|&i| d.draw.attribs[i].stride == 0)
                .map(|i| (d.draw.attribs[i].vi_reg, f32s(&d.attrib_data[i])))
                .collect();
            columns.sort_by_key(|c| c.0);
            assert_eq!(columns.len(), cols, "{ty}: one constant per column (pos {pos}, value {val})");
            let base = columns[0].0;
            for (k, (reg, data)) in columns.iter().enumerate() {
                assert_eq!(*reg, base + k as u8, "{ty}: column {k} on the next register");
                let want: &[f32] = if k == 1 { &value[..cols] } else { &[0.0, 0.0, 0.0, 1.0][..cols] };
                assert_eq!(&data[..cols], want, "{ty}: column {k}");
            }
        }
    }

    /* ---- sampler-state variants (shaderc::LodKey) ---- */

    /// The in-process compiler as the back end for one test, put back after.
    #[cfg(all(feature = "shaderc-online", not(feature = "gpu-v3d")))]
    struct OnlineBackEnd(*const crate::shaderc::ShcBackend);

    #[cfg(all(feature = "shaderc-online", not(feature = "gpu-v3d")))]
    impl OnlineBackEnd {
        fn install() -> OnlineBackEnd {
            // SAFETY: the suite's guard serialises tests; restored on drop.
            let old = OnlineBackEnd(unsafe { crate::shaderc::g_shaderc });
            // SAFETY: as above; a static table pointer.
            unsafe { crate::shaderc::g_shaderc = &crate::shaderc::online::SHC_ONLINE };
            old
        }
    }

    #[cfg(all(feature = "shaderc-online", not(feature = "gpu-v3d")))]
    impl Drop for OnlineBackEnd {
        fn drop(&mut self) {
            // SAFETY: restoring what was saved, under the suite's guard.
            unsafe { crate::shaderc::g_shaderc = self.0 };
        }
    }

    /// A program of two sources through the GL, made current, its attribute
    /// `p` fed the triangle.
    #[cfg(all(feature = "shaderc-online", not(feature = "gpu-v3d")))]
    fn online_program(vs: &str, fs: &str) -> GLuint {
        use crate::gl::shader::{glCompileShader, glCreateShader, glShaderSource};
        use std::ffi::CString;
        let (vs, fs) = (CString::new(vs).unwrap(), CString::new(fs).unwrap());
        // SAFETY: plain GL calls on handles this test owns; TRI is a static.
        unsafe {
            let sh = |kind: GLenum, t: &CString| {
                let s = glCreateShader(kind);
                let p: [*const core::ffi::c_char; 1] = [t.as_ptr()];
                glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
                glCompileShader(s);
                s
            };
            let p = glCreateProgram();
            glAttachShader(p, sh(GL_VERTEX_SHADER, &vs));
            glAttachShader(p, sh(GL_FRAGMENT_SHADER, &fs));
            glLinkProgram(p);
            let mut ok: GLint = 0;
            glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
            assert_eq!(ok, 1);
            glUseProgram(p);
            let pos = glGetAttribLocation(p, c"p".as_ptr());
            assert!(pos >= 0);
            glVertexAttribPointer(pos as GLuint, 4, GL_FLOAT, 0, 0, TRI.as_ptr() as *const c_void);
            glEnableVertexAttribArray(pos as GLuint);
            p
        }
    }

    /// A 4x4 RGBA8 texture with its whole chain (three levels), bound to
    /// `unit`, with these filters.
    #[cfg(all(feature = "shaderc-online", not(feature = "gpu-v3d")))]
    fn mipmapped_texture(unit: GLuint, min: GLenum, mag: GLenum) -> GLuint {
        // SAFETY: plain GL calls.
        unsafe { glActiveTexture(GL_TEXTURE0 + unit) };
        let t = texture_rgba(4, 4);
        // SAFETY: plain GL calls on the texture just bound.
        unsafe {
            glGenerateMipmap(GL_TEXTURE_2D);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, min as GLint);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, mag as GLint);
            glActiveTexture(GL_TEXTURE0);
        }
        t
    }

    #[cfg(all(feature = "shaderc-online", not(feature = "gpu-v3d")))]
    fn set_filters(t: GLuint, unit: GLuint, min: GLenum, mag: GLenum) {
        // SAFETY: plain GL calls.
        unsafe {
            glActiveTexture(GL_TEXTURE0 + unit);
            glBindTexture(GL_TEXTURE_2D, t);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, min as GLint);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, mag as GLint);
            glActiveTexture(GL_TEXTURE0);
        }
    }

    /// The code a draw's stage runs, and the bytes at the USC address the
    /// draw hands the backend (they must be that code).
    #[cfg(all(feature = "shaderc-online", not(feature = "gpu-v3d")))]
    fn stage_code(gl: &Gl, d: &GlesDraw, fragment: bool) -> Vec<u32> {
        // SAFETY: the draw's shaders live in the program or its variant cache.
        let sh = unsafe { &*(if fragment { d.fs } else { d.vs }) };
        let gpu = if fragment { d.fs_usc_gpu } else { d.vs_usc_gpu };
        assert_eq!(gl.gpu_bytes(gpu, sh.usc_dw as usize * 4), dword_bytes(sh.usc_words()), "the uploaded code");
        sh.usc_words().to_vec()
    }

    /// The program's GPU cache row (the draw path's static state).
    #[cfg(all(feature = "shaderc-online", not(feature = "gpu-v3d")))]
    fn prog_gpu(gl: &Gl, prog: GLuint) -> &'static ProgGpu {
        // SAFETY: single-threaded under the suite's guard; read between draws.
        unsafe { &STATE.get().pg[gl.ctx().group as usize][obj_index(prog)] }
    }

    /// Switch (a), end to end through the GL: a fragment shader that samples
    /// a varying with exact gradients runs them while the bound texture has
    /// a mipmap filter, and runs the program `pvr_proj_lod = 0` compiles --
    /// dword for dword, uploaded -- while it has none; the key follows
    /// glTexParameter, a sampler object and glUniform1i re-pointing the
    /// sampler at another unit, with no hook, and a variant is compiled
    /// and uploaded once.
    #[cfg(all(feature = "shaderc-online", feature = "gpu-powervr"))]
    #[test]
    fn a_plain_texture_draws_the_fragment_shader_without_gradients() {
        const VS: &str = "attribute vec4 p; varying vec2 uv; void main(){ uv = p.xy; gl_Position = p; }";
        const FS: &str = "precision mediump float; uniform sampler2D s; varying vec2 uv; \
                          void main(){ gl_FragColor = texture2D(s, uv); }";
        let gl = Gl::new(16, 16);
        let _be = OnlineBackEnd::install();
        /* the reference: the fragment shader as pvr_proj_lod = 0 compiles it,
         * released before the program compiles (the cache keys on the text) */
        let off = {
            crate::shaderc::pvr_set_proj_lod(0);
            let src = std::format!("{FS}\0");
            let mut sh = ShcShader::ZERO;
            let compile = crate::shaderc::online::SHC_ONLINE.compile.unwrap();
            // SAFETY: a NUL-terminated source and a real output; no log.
            let rc = unsafe {
                compile(crate::shaderc::SHC_STAGE_FRAGMENT, src.as_ptr() as *const _, &mut sh, core::ptr::null_mut(), 0)
            };
            assert_eq!(rc, 0);
            let code = sh.usc_words().to_vec();
            shaderc::release_shader(&sh);
            crate::shaderc::pvr_set_proj_lod(crate::shaderc::PVR_PROJ_LOD_DEFAULT);
            code
        };
        let prog = online_program(VS, FS);
        let own = gl.ctx().objs().programs[obj_index(prog)].rfs.usc_words().to_vec();
        assert_ne!(own, off, "the program samples with gradients");
        let t0 = mipmapped_texture(0, GL_LINEAR_MIPMAP_LINEAR, GL_LINEAR);
        let t1 = mipmapped_texture(1, GL_LINEAR, GL_LINEAR);

        let d = draw_tri(&gl).draw;
        assert_eq!(stage_code(&gl, &d, true), own, "a mipmap filter: the gradients");
        assert_eq!(prog_gpu(&gl, prog).lod_sense, [0, 1], "binding 0 of the fragment stage, mip_free");

        set_filters(t0, 0, GL_LINEAR, GL_LINEAR);
        let d = draw_tri(&gl).draw;
        assert_eq!(stage_code(&gl, &d, true), off, "no mipmap filter: the switch-off program");
        let plain_usc = d.fs_usc_gpu;

        set_filters(t0, 0, GL_NEAREST_MIPMAP_NEAREST, GL_LINEAR);
        let d = draw_tri(&gl).draw;
        assert_eq!(stage_code(&gl, &d, true), own, "a mipmap filter again");

        /* a sampler object's filters replace the texture's (ES 3.0 3.8.2) */
        let mut smp: GLuint = 0;
        // SAFETY: plain GL calls.
        unsafe {
            crate::gl::es3::glGenSamplers(1, &mut smp);
            crate::gl::es3::glSamplerParameteri(smp, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
            crate::gl::es3::glBindSampler(0, smp);
        }
        gl.mem_events();
        let d = draw_tri(&gl).draw;
        assert_eq!(stage_code(&gl, &d, true), off, "the sampler object has no mipmap filter");
        assert_eq!(d.fs_usc_gpu, plain_usc, "the same key: the variant uploaded before");
        assert!(gl.mem_events().is_empty(), "nothing compiled or uploaded again");
        // SAFETY: plain GL call.
        unsafe { crate::gl::es3::glBindSampler(0, 0) };

        /* the sampler uniform re-pointed at unit 1, which holds a plain texture */
        // SAFETY: plain GL calls on the current program.
        unsafe { glUniform1i(glGetUniformLocation(prog, c"s".as_ptr()), 1) };
        let d = draw_tri(&gl).draw;
        assert_eq!(stage_code(&gl, &d, true), off, "unit 1: no mipmap filter");
        // SAFETY: as above.
        unsafe { glUniform1i(glGetUniformLocation(prog, c"s".as_ptr()), 0) };
        let d = draw_tri(&gl).draw;
        assert_eq!(stage_code(&gl, &d, true), own, "unit 0: the gradients");

        /* the switch off: the sensitivity is asked after the next link */
        crate::shaderc::shc_set_lod_variants(0, -1);
        // SAFETY: plain GL call.
        unsafe { glLinkProgram(prog) };
        set_filters(t0, 0, GL_LINEAR, GL_LINEAR);
        let d = draw_tri(&gl).draw;
        crate::shaderc::shc_set_lod_variants(1, -1);
        assert_eq!(stage_code(&gl, &d, true), own, "switch (a) off: the program's own");
        let _ = t1;
    }

    /// The cube key's filter rule (`cube_filters_see_seams`): any LINEAR tap
    /// (mag LINEAR, or min LINEAR / LINEAR_MIPMAP_*) is keyed, NEAREST with
    /// NEAREST or NEAREST_MIPMAP_NEAREST is not, and NEAREST_MIPMAP_LINEAR
    /// is keyed only by a LINEAR mag filter (its taps are NEAREST texels).
    #[test]
    fn the_cube_key_follows_the_filters() {
        for (min, mag, keyed) in [
            (GL_NEAREST, GL_NEAREST, false),
            (GL_NEAREST_MIPMAP_NEAREST, GL_NEAREST, false),
            (GL_NEAREST_MIPMAP_LINEAR, GL_NEAREST, false),
            (GL_NEAREST_MIPMAP_LINEAR, GL_LINEAR, true),
            (GL_NEAREST, GL_LINEAR, true),
            (GL_LINEAR, GL_NEAREST, true),
            (GL_LINEAR_MIPMAP_NEAREST, GL_NEAREST, true),
            (GL_LINEAR_MIPMAP_LINEAR, GL_NEAREST, true),
            (GL_LINEAR_MIPMAP_LINEAR, GL_LINEAR, true),
        ] {
            assert_eq!(cube_filters_see_seams(min, mag), keyed, "min {min:#x} mag {mag:#x}");
        }
    }

    /// `LodKey::ret32` from the draw's state: of the bindings the
    /// sensitivity names, those whose bound texture is stored R32F (a depth
    /// texture, `GL_DEPTH_COMPONENT`) take the bit, and an RGBA8 texture or a
    /// binding outside the sensitivity does not. GPU-independent (the V3D's
    /// program records ask for it), so checked in the online build these
    /// helpers run in. Revert check: without the texfmt test every texture
    /// takes the bit.
    #[cfg(all(feature = "shaderc-online", not(feature = "gpu-v3d"), not(feature = "report-es2")))]
    #[test]
    fn the_ret32_key_bit_follows_the_bound_textures_format() {
        const VS: &str = "attribute vec4 p; varying vec2 v; void main(){ v = p.xy; gl_Position = p; }";
        const FS: &str = "precision mediump float; varying vec2 v; uniform sampler2D s; \
                          void main(){ gl_FragColor = texture2D(s, v); }";
        let gl = Gl::new(16, 16);
        let _be = OnlineBackEnd::install();
        let prog = online_program(VS, FS);
        let sense = shaderc::LodKey { ret32: 1, ..shaderc::LodKey::NONE }.bits();
        let key = |gl: &Gl| -> shaderc::LodKey {
            let c = gl.ctx();
            let p = &c.objs_ref().programs[obj_index(prog)];
            shaderc::LodKey::from_bits(stage_lod_key(c, sense, &p.rfs, &p.fs_sampler_unit))
        };
        let _rgba = texture_rgba(4, 4);
        assert_eq!(key(&gl).ret32, 0, "an RGBA8 texture at binding 0");
        let mut depth = 0;
        // SAFETY: plain GL calls; a 4x4 depth texture with no data.
        unsafe {
            glGenTextures(1, &mut depth);
            glBindTexture(GL_TEXTURE_2D, depth);
            glTexImage2D(
                GL_TEXTURE_2D,
                0,
                GL_DEPTH_COMPONENT as GLint,
                4,
                4,
                0,
                GL_DEPTH_COMPONENT,
                GL_UNSIGNED_INT,
                core::ptr::null(),
            );
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(key(&gl).ret32, 1, "a depth texture (R32F) at binding 0");
        let c = gl.ctx();
        let p = &c.objs_ref().programs[obj_index(prog)];
        assert_eq!(stage_lod_key(c, 0, &p.rfs, &p.fs_sampler_unit), 0, "no sensitivity, no key");
    }

    /// Switch (b), end to end through the GL, in a build that reports ES
    /// 2.0 (in any other the program's own code runs, checked too): a vertex
    /// shader's explicit level is remapped exactly while the bound sampler
    /// has mag LINEAR and min NEAREST_MIPMAP_*, and the program's own code
    /// runs otherwise; the
    /// variant keeps the vertex inputs. The variants a program keeps are
    /// bounded: three keys cycled through two slots recompile from the keep
    /// region's cache, whose use does not grow.
    #[cfg(all(feature = "shaderc-online", not(feature = "gpu-v3d")))]
    #[test]
    fn a_nearest_mipmap_sampler_draws_the_remapped_vertex_shader() {
        const VS: &str = "attribute vec4 p; uniform highp float u_lod; uniform sampler2D s0; uniform sampler2D s1; \
                          varying vec4 v; void main(){ v = texture2DLod(s0, p.xy, u_lod) + \
                          texture2DLod(s1, p.yx, u_lod); gl_Position = p; }";
        const FS: &str = "precision mediump float; varying vec4 v; void main(){ gl_FragColor = v; }";
        let gl = Gl::new(16, 16);
        let _be = OnlineBackEnd::install();
        let prog = online_program(VS, FS);
        // SAFETY: plain GL calls on the current program.
        unsafe {
            glUniform1i(glGetUniformLocation(prog, c"s0".as_ptr()), 0);
            glUniform1i(glGetUniformLocation(prog, c"s1".as_ptr()), 1);
        }
        let own_vs = gl.ctx().objs().programs[obj_index(prog)].rvs.usc_words().to_vec();
        let own_fs = gl.ctx().objs().programs[obj_index(prog)].rfs.usc_words().to_vec();
        let t0 = mipmapped_texture(0, GL_LINEAR_MIPMAP_NEAREST, GL_LINEAR);
        let t1 = mipmapped_texture(1, GL_LINEAR_MIPMAP_NEAREST, GL_LINEAR);

        let base = draw_tri(&gl);
        assert_eq!(stage_code(&gl, &base.draw, false), own_vs, "no NEAREST_MIPMAP_* with LINEAR: the own");
        let sense = prog_gpu(&gl, prog).lod_sense;
        assert_eq!(sense[1], 0, "the fragment stage samples nothing");
        if !crate::REPORT_ES2 {
            /* an ES 3 report: no remap, whatever the filters */
            assert_eq!(sense[0], 0, "no sensitivity outside an ES 2.0 report");
            set_filters(t0, 0, GL_NEAREST_MIPMAP_NEAREST, GL_LINEAR);
            set_filters(t1, 1, GL_NEAREST_MIPMAP_LINEAR, GL_LINEAR);
            let d = draw_tri(&gl).draw;
            assert_eq!(stage_code(&gl, &d, false), own_vs, "ES 3 report: the own program");
            return;
        }
        assert_eq!(shaderc::LodKey::from_bits(sense[0]).mag_switch, 0b11, "both bindings of the vertex stage");

        let mut codes = Vec::new();
        for (m0, m1) in [
            (GL_NEAREST_MIPMAP_NEAREST, GL_LINEAR_MIPMAP_NEAREST),
            (GL_LINEAR_MIPMAP_NEAREST, GL_NEAREST_MIPMAP_LINEAR),
            (GL_NEAREST_MIPMAP_LINEAR, GL_NEAREST_MIPMAP_NEAREST),
        ] {
            set_filters(t0, 0, m0, GL_LINEAR);
            set_filters(t1, 1, m1, GL_LINEAR);
            let d = draw_tri(&gl);
            let code = stage_code(&gl, &d.draw, false);
            assert_ne!(code, own_vs, "{m0:#x}/{m1:#x}: remapped");
            assert_eq!(stage_code(&gl, &d.draw, true), own_fs, "the fragment stage is the program's own");
            assert_eq!(d.draw.num_attribs, base.draw.num_attribs, "the vertex inputs are the program's");
            codes.push(code);
        }
        assert!(codes[0] != codes[1] && codes[1] != codes[2] && codes[0] != codes[2], "three keys, three programs");

        /* mag NEAREST: c = 0 is GL's switch too, the own program */
        set_filters(t0, 0, GL_NEAREST_MIPMAP_NEAREST, GL_NEAREST);
        set_filters(t1, 1, GL_NEAREST_MIPMAP_LINEAR, GL_NEAREST);
        let d = draw_tri(&gl).draw;
        assert_eq!(stage_code(&gl, &d, false), own_vs, "mag NEAREST: the own program");

        /* bounded: the three keys in turn, many times */
        let keep = || crate::shaderc::online::store::keep_in_use();
        let mut after_warm_up = 0;
        for round in 0..20 {
            for (k, (m0, m1)) in [
                (GL_NEAREST_MIPMAP_NEAREST, GL_LINEAR_MIPMAP_NEAREST),
                (GL_LINEAR_MIPMAP_NEAREST, GL_NEAREST_MIPMAP_LINEAR),
                (GL_NEAREST_MIPMAP_LINEAR, GL_NEAREST_MIPMAP_NEAREST),
            ]
            .into_iter()
            .enumerate()
            {
                set_filters(t0, 0, m0, GL_LINEAR);
                set_filters(t1, 1, m1, GL_LINEAR);
                let d = draw_tri(&gl).draw;
                assert_eq!(stage_code(&gl, &d, false), codes[k], "round {round}, key {k}");
                let used = prog_gpu(&gl, prog).lod_var.iter().filter(|v| v.used).count();
                assert!(used <= PROG_MAX_LOD_VARIANTS);
            }
            if round == 1 {
                after_warm_up = keep();
            }
        }
        assert_eq!(keep(), after_warm_up, "the keep region does not grow");
        /* the program's end: its variants go with it */
        // SAFETY: plain GL calls.
        unsafe {
            glUseProgram(0);
            glDeleteProgram(prog);
        }
        assert!(prog_gpu(&gl, prog).lod_var.iter().all(|v| !v.used));
    }

    /// The two records of one matrix attribute -- etna_shc's rows x
    /// columns components and ours of one column -- give the same inputs,
    /// one per column, as tall as the matrix's rows.
    #[test]
    fn both_records_of_a_matrix_attribute_give_the_same_columns() {
        let rec = |components: u8, gl_type: u32| {
            let mut sh = ShcShader::ZERO;
            sh.num_attribs = 2;
            sh.attribs[0] = ShcAttrib { location: 0, components: 4, vi_reg: 0, gl_type: 0x8b52, ..ShcAttrib::ZERO };
            sh.attribs[1] =
                ShcAttrib { location: 1, components, vi_reg: 1, gl_type: gl_type as u16, ..ShcAttrib::ZERO };
            util::set_name(&mut sh.attribs[0].name, b"pos");
            util::set_name(&mut sh.attribs[1].name, b"xf");
            let (v, n) = vertex_inputs(&sh);
            v[..n].iter().map(|a| (a.location, a.components, a.vi_reg)).collect::<Vec<_>>()
        };
        if cfg!(not(feature = "gpu-powervr")) {
            let mat4 = [(0, 4, 0), (1, 4, 1), (2, 4, 2), (3, 4, 3), (4, 4, 4)];
            assert_eq!(rec(16, GL_FLOAT_MAT4), mat4, "etna_shc's record");
            assert_eq!(rec(4, GL_FLOAT_MAT4), mat4, "the in-process compiler's");
            let mat3 = [(0, 4, 0), (1, 3, 1), (2, 3, 2), (3, 3, 3)];
            assert_eq!(rec(9, GL_FLOAT_MAT3), mat3);
            assert_eq!(rec(3, GL_FLOAT_MAT3), mat3);
            /* ES 3.00's matCxR: C columns of R components (a mat4x3 is four
             * locations of three, a mat2x4 two of four), whichever way the
             * record counts the components. Before, a non-square matrix was
             * one input: its other columns were never fetched. */
            let mat4x3 = [(0, 4, 0), (1, 3, 1), (2, 3, 2), (3, 3, 3), (4, 3, 4)];
            assert_eq!(rec(12, GL_FLOAT_MAT4x3), mat4x3);
            assert_eq!(rec(3, GL_FLOAT_MAT4x3), mat4x3);
            assert_eq!(rec(4, GL_FLOAT_MAT2x4), [(0, 4, 0), (1, 4, 1), (2, 4, 2)]);
            assert_eq!(rec(2, GL_FLOAT_MAT3x2), [(0, 4, 0), (1, 2, 1), (2, 2, 2), (3, 2, 3)]);
        } else {
            assert_eq!(rec(16, GL_FLOAT_MAT4), [(0, 4, 0), (1, 16, 1)], "the PowerVR path is unchanged");
        }
        assert_eq!(rec(2, 0x8b50), [(0, 4, 0), (1, 2, 1)], "a vec2 is one input");
    }

    /// Draws 3 vertices whose a_color comes from a 4-component client array of
    /// `type_`; returns the staged floats of a_color.
    #[cfg(feature = "gpu-powervr")] /* only the PowerVR table has these shaders */
    fn staged_colour(gl: &Gl, type_: GLenum, normalized: bool, data: &[u8]) -> Vec<f32> {
        // SAFETY: `data` outlives the draw and holds 3 vertices.
        unsafe {
            glVertexAttribPointer(1, 4, type_, normalized as GLboolean, 0, data.as_ptr() as *const c_void);
            glEnableVertexAttribArray(1);
            glDrawArrays(GL_TRIANGLES, 0, 3);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        let d = one(gl);
        let a = d.draw.attribs[1];
        assert_eq!((a.vi_reg, a.components, a.format), (4, 4, 0));
        assert_eq!((a.stride, a.offset, a.cpu_bytes), (16, 0, 48));
        f32s(&d.attrib_data[1])
    }

    #[test]
    #[cfg(feature = "gpu-powervr")] /* the PowerVR table's own values */
    fn unsigned_byte_normalized_array_is_staged_as_unit_floats() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        let data: [u8; 12] = [255, 0, 51, 255, 0, 255, 102, 255, 1, 2, 3, 4];
        let want: Vec<f32> = data.iter().map(|b| *b as f32 / 255.0).collect();
        let got = staged_colour(&gl, GL_UNSIGNED_BYTE, true, &data);
        assert_eq!(got, want);
        assert_eq!(&got[..4], [1.0, 0.0, 0.2, 1.0]);
    }

    #[test]
    #[cfg(feature = "gpu-powervr")] /* the PowerVR table's own values */
    fn unsigned_byte_unnormalized_array_is_staged_as_its_values() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        let data: [u8; 12] = [255, 0, 51, 255, 0, 255, 102, 255, 1, 2, 3, 4];
        let want: Vec<f32> = data.iter().map(|b| *b as f32).collect();
        assert_eq!(staged_colour(&gl, GL_UNSIGNED_BYTE, false, &data), want);
    }

    #[test]
    #[cfg(feature = "gpu-powervr")] /* the PowerVR table's own values */
    fn byte_arrays_are_staged_signed() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        let data: [i8; 12] = [127, -128, -127, 0, 1, -1, 64, -64, 2, 3, 4, 5];
        let raw: Vec<u8> = data.iter().map(|v| *v as u8).collect();
        let plain: Vec<f32> = data.iter().map(|v| *v as f32).collect();
        assert_eq!(staged_colour(&gl, GL_BYTE, false, &raw), plain);
        let norm: Vec<f32> = data.iter().map(|v| (*v as f32 / 127.0).max(-1.0)).collect();
        let got = staged_colour(&gl, GL_BYTE, true, &raw);
        assert_eq!(got, norm);
        assert_eq!(&got[..4], [1.0, -1.0, -1.0, 0.0]);
    }

    #[test]
    #[cfg(feature = "gpu-powervr")] /* the PowerVR table's own values */
    fn short_arrays_are_staged_normalized_and_not() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        let data: [i16; 12] = [32767, -32768, -32767, 0, 300, -300, 16384, 1, 2, 3, 4, 5];
        let plain: Vec<f32> = data.iter().map(|v| *v as f32).collect();
        assert_eq!(staged_colour(&gl, GL_SHORT, false, dword_bytes(&data)), plain);
        let norm: Vec<f32> = data.iter().map(|v| (*v as f32 / 32767.0).max(-1.0)).collect();
        let got = staged_colour(&gl, GL_SHORT, true, dword_bytes(&data));
        assert_eq!(got, norm);
        assert_eq!(&got[..4], [1.0, -1.0, -1.0, 0.0]);
    }

    #[test]
    #[cfg(feature = "gpu-powervr")] /* the PowerVR table's own values */
    fn unsigned_short_and_fixed_arrays_are_staged_as_floats() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        let us: [u16; 12] = [65535, 0, 13107, 1, 2, 3, 4, 5, 6, 7, 8, 9];
        let norm: Vec<f32> = us.iter().map(|v| *v as f32 / 65535.0).collect();
        assert_eq!(staged_colour(&gl, GL_UNSIGNED_SHORT, true, dword_bytes(&us)), norm);
        let fixed: [i32; 12] = [0x1_8000, -0x2_4000, 0x1_0000, 0, 1, 2, 3, 4, 5, 6, 7, 8];
        let want: Vec<f32> = fixed.iter().map(|v| *v as f32 / 65536.0).collect();
        let got = staged_colour(&gl, GL_FIXED, false, dword_bytes(&fixed));
        assert_eq!(got, want);
        assert_eq!(&got[..3], [1.5, -2.25, 1.0]);
    }

    #[test]
    fn integer_attribute_array_is_staged_as_widened_integer_bits() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        let data: [i16; 12] = [-1, 2, -300, 400, 5, 6, 7, 8, 9, 10, 11, 12];
        // SAFETY: `data` outlives the draw and holds 3 vertices.
        unsafe {
            glVertexAttribIPointer(1, 4, GL_SHORT, 0, data.as_ptr() as *const c_void);
            glEnableVertexAttribArray(1);
        }
        let d = draw_tri(&gl);
        let got: Vec<u32> = d.attrib_data[1].as_chunks::<4>().0.iter().map(|c| u32::from_ne_bytes(*c)).collect();
        let want: Vec<u32> = data.iter().map(|v| *v as i32 as u32).collect();
        assert_eq!(got, want);
    }

    #[test]
    fn packed_vbo_array_is_converted_from_the_buffer_with_offset_and_stride() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        /* 2 bytes of lead-in, then per vertex: 4 colour bytes + 1 pad */
        let bytes: [u8; 17] = [9, 9, 255, 0, 0, 255, 7, 0, 255, 0, 255, 7, 0, 0, 255, 51, 7];
        buffer_with(GL_ARRAY_BUFFER, &bytes);
        // SAFETY: plain GL calls; the pointer is a buffer offset.
        unsafe {
            glVertexAttribPointer(1, 4, GL_UNSIGNED_BYTE, 1, 5, 2 as *const c_void);
            glEnableVertexAttribArray(1);
            glBindBuffer(GL_ARRAY_BUFFER, 0);
        }
        let d = draw_tri(&gl);
        assert_eq!(f32s(&d.attrib_data[1]), [1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.2]);
        assert_eq!(d.draw.attribs[1].offset, 0);
        assert_eq!(d.draw.attribs[1].stride, 16);
    }

    #[test]
    fn converted_vbo_array_running_past_the_buffer_is_invalid_operation() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        buffer_with(GL_ARRAY_BUFFER, &[0u8; 11]); /* one byte short of 3 RGBA8 vertices */
        // SAFETY: plain GL calls; the pointer is a buffer offset.
        unsafe {
            glVertexAttribPointer(1, 4, GL_UNSIGNED_BYTE, 1, 0, core::ptr::null());
            glEnableVertexAttribArray(1);
            glBindBuffer(GL_ARRAY_BUFFER, 0);
            glDrawArrays(GL_TRIANGLES, 0, 3);
        }
        assert_eq!(gl.error(), GL_INVALID_OPERATION);
        assert!(gl.draws().is_empty());
        // SAFETY: plain GL call; 2 vertices fit.
        unsafe { glDrawArrays(GL_LINES, 0, 2) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(gl.draws().len(), 1);
    }

    #[test]
    fn attribute_buffer_without_storage_is_invalid_operation() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        let mut b: GLuint = 0;
        // SAFETY: valid out pointer; plain GL calls.
        unsafe {
            glGenBuffers(1, &mut b);
            glBindBuffer(GL_ARRAY_BUFFER, b); /* never glBufferData'd */
            glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, core::ptr::null());
            glDrawArrays(GL_TRIANGLES, 0, 3);
        }
        assert_eq!(gl.error(), GL_INVALID_OPERATION);
        assert!(gl.draws().is_empty());
    }

    #[test]
    #[cfg(feature = "gpu-powervr")] /* the PowerVR table's own values */
    fn disabled_array_reads_the_current_attribute_as_a_zero_stride_constant() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        // SAFETY: plain GL call.
        unsafe { glVertexAttrib4f(1, 0.1, 0.2, 0.3, 0.4) };
        let d = draw_tri(&gl);
        assert_eq!(d.draw.num_attribs, 2);
        let a = d.draw.attribs[1];
        assert_eq!(a.vi_reg, 4);
        assert_eq!(a.components, 4);
        assert_eq!((a.stride, a.offset), (0, 0), "every vertex reads the same dwords");
        assert_eq!(a.cpu_bytes, 16);
        assert_eq!(f32s(&d.attrib_data[1]), [0.1, 0.2, 0.3, 0.4]);
        assert_eq!(f32s(&gl.gpu_bytes(a.buffer_gpu, 16)), [0.1, 0.2, 0.3, 0.4]);
    }

    #[test]
    fn untouched_current_attribute_is_0_0_0_1() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        assert_eq!(f32s(&draw_tri(&gl).attrib_data[1]), [0.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn constant_of_a_vec2_input_is_two_components_wide() {
        let gl = Gl::new(16, 16);
        let prog = gl.program("uv.vert", "texture.frag"); /* vec4 a_pos @0, vec2 a_uv @1 */
        // SAFETY: plain GL calls; TRI is a static.
        unsafe {
            glUseProgram(prog);
            glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, TRI.as_ptr() as *const c_void);
            glEnableVertexAttribArray(0);
            glVertexAttrib2f(1, 0.5, 0.25);
        }
        let d = draw_tri(&gl);
        assert_eq!(d.draw.attribs[1].components, 2);
        assert_eq!(d.draw.attribs[1].stride, 0);
        assert_eq!(&f32s(&d.attrib_data[1])[..2], [0.5, 0.25]);
    }

    #[test]
    #[cfg(feature = "gpu-powervr")] /* the PowerVR table's own values */
    fn attributes_follow_glbindattriblocation() {
        let gl = Gl::new(16, 16);
        let vs = gl.shader(GL_VERTEX_SHADER, "passthrough.vert");
        let fs = gl.shader(GL_FRAGMENT_SHADER, "uniform_color.frag");
        // SAFETY: plain GL calls; valid names; TRI is a static.
        unsafe {
            let prog = glCreateProgram();
            glAttachShader(prog, vs);
            glAttachShader(prog, fs);
            glBindAttribLocation(prog, 5, c"a_pos".as_ptr());
            glBindAttribLocation(prog, 3, c"a_color".as_ptr());
            glLinkProgram(prog);
            glUseProgram(prog);
            glVertexAttribPointer(5, 4, GL_FLOAT, 0, 0, TRI.as_ptr() as *const c_void);
            glEnableVertexAttribArray(5);
            glVertexAttrib4f(3, 4.0, 3.0, 2.0, 1.0);
            /* decoys on the GLSL locations: must not be read */
            glVertexAttrib4f(0, 9.0, 9.0, 9.0, 9.0);
            glVertexAttrib4f(1, 8.0, 8.0, 8.0, 8.0);
        }
        let d = draw_tri(&gl);
        assert_eq!(d.draw.num_attribs, 2);
        assert_eq!(d.draw.attribs[0].vi_reg, 0, "a_pos still lands in its VS register");
        assert_eq!(f32s(&d.attrib_data[0]), TRI);
        assert_eq!(d.draw.attribs[1].vi_reg, 4);
        assert_eq!(f32s(&d.attrib_data[1]), [4.0, 3.0, 2.0, 1.0]);
    }

    #[test]
    fn only_the_vertex_shaders_inputs_become_attributes() {
        let gl = Gl::new(16, 16);
        let prog = gl.program("mvp.vert", "uniform_color.frag"); /* a_pos only */
        // SAFETY: plain GL calls; TRI is a static.
        unsafe {
            glUseProgram(prog);
            for i in 0..4 {
                glVertexAttribPointer(i, 4, GL_FLOAT, 0, 0, TRI.as_ptr() as *const c_void);
                glEnableVertexAttribArray(i);
            }
        }
        assert_eq!(draw_tri(&gl).draw.num_attribs, 1);
    }

    #[test]
    fn attribute_staging_failure_is_out_of_memory() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        draw_tri(&gl); /* program uploads done; scratch exists for 3 vertices */
        let verts = [0.0f32; 64];
        gl.fail_alloc_in(1);
        // SAFETY: `verts` outlives the draw: 16 vertices need a larger scratch.
        unsafe {
            glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, verts.as_ptr() as *const c_void);
            glDrawArrays(GL_TRIANGLES, 0, 15);
        }
        assert_eq!(gl.error(), GL_OUT_OF_MEMORY);
        assert!(gl.draws().is_empty());
    }

    #[test]
    fn attribute_scratch_grows_by_replacing_the_block() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        let small = draw_tri(&gl).draw.attribs[0].buffer_gpu;
        gl.mem_events();
        draw_tri(&gl);
        assert!(gl.mem_events().is_empty(), "the scratch is reused while it is large enough");
        let verts = [0.0f32; 64];
        // SAFETY: `verts` outlives the draw: 16 vertices.
        unsafe {
            glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, verts.as_ptr() as *const c_void);
            glDrawArrays(GL_TRIANGLES, 0, 15);
        }
        let ev = gl.mem_events();
        assert_eq!(frees(&ev), [small]);
        assert!(ev.iter().any(|e| matches!(e, MemEvent::Alloc { heap: HEAP_DATA, size: 240, .. })), "{ev:?}");
    }

    #[test]
    fn attrib_dump_logs_the_vertices_of_an_in_place_stream() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        buffer_with(GL_ARRAY_BUFFER, dword_bytes(&TRI));
        // SAFETY: plain GL call; the pointer is a buffer offset.
        unsafe { glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, core::ptr::null()) };
        let _dump = AttribDump::on();
        gl.logs();
        draw_tri(&gl);
        let logs = gl.logs();
        assert!(logs.iter().any(|l| l.contains("attrib vi0 (in place) v0: -1 -1 0 1")), "{logs:?}");
        assert!(logs.iter().any(|l| l.contains("attrib vi0 (in place) v2: 0 1 0 1")), "{logs:?}");

        /* the C printed the components with %g, not with Rust's {} */
        let odd: [f32; 4] = [1.0e-7, 1234567.0, 0.1, -0.0];
        buffer_with(GL_ARRAY_BUFFER, dword_bytes(&odd));
        // SAFETY: plain GL calls; the pointer is a buffer offset.
        unsafe {
            glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, core::ptr::null());
            glDrawArrays(GL_POINTS, 0, 1);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        let logs = gl.logs();
        assert!(logs.iter().any(|l| l.contains("v0: 1e-07 1.23457e+06 0.1 -0")), "{logs:?}");
    }

    /* ---- indices ----------------------------------------------------------------------------- */

    /// basic() with a_pos in a float VBO: nothing needs the vertex range.
    fn basic_vbo(gl: &Gl) -> GLuint {
        let prog = basic(gl);
        buffer_with(GL_ARRAY_BUFFER, dword_bytes(&TRI));
        // SAFETY: plain GL calls; the pointer is a buffer offset.
        unsafe {
            glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, core::ptr::null());
            glBindBuffer(GL_ARRAY_BUFFER, 0);
        }
        prog
    }

    #[test]
    fn u16_element_buffer_is_used_in_place() {
        let gl = Gl::new(16, 16);
        basic_vbo(&gl);
        let idx: [u16; 6] = [0, 1, 2, 2, 1, 0];
        let ib = buffer_with(GL_ELEMENT_ARRAY_BUFFER, dword_bytes(&idx));
        // SAFETY: plain GL calls.
        unsafe {
            glFlush(); /* kick id 1 */
            glDrawElements(GL_TRIANGLES, 6, GL_UNSIGNED_SHORT, core::ptr::null());
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        let d = one(&gl);
        assert_eq!(d.draw.prim, GLES_PRIM_TRIANGLES);
        assert_eq!(d.draw.index_count, 6);
        assert_eq!(d.draw.index_buf_gpu, gl.ctx().objs().buffers[ib as usize].mem.gpu);
        assert!(d.draw.index_cpu.is_null());
        assert_eq!(d.draw.index_bytes, 0);
        assert!(d.index_data.is_empty());
        assert_eq!(gl.ctx().objs().buffers[ib as usize].last_use, 1, "the kick reads the buffer in place");
        assert_eq!(d.draw.attribs[0].offset, 0, "indexed draws address vertices absolutely");
    }

    #[test]
    fn u16_element_buffer_offset_is_added_to_the_gpu_address() {
        let gl = Gl::new(16, 16);
        basic_vbo(&gl);
        let idx: [u16; 6] = [0, 1, 2, 2, 1, 0];
        let ib = buffer_with(GL_ELEMENT_ARRAY_BUFFER, dword_bytes(&idx));
        // SAFETY: plain GL call; the pointer is a buffer offset.
        unsafe { glDrawElements(GL_TRIANGLE_STRIP, 3, GL_UNSIGNED_SHORT, 4 as *const c_void) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        let d = one(&gl).draw;
        assert_eq!(d.index_buf_gpu, gl.ctx().objs().buffers[ib as usize].mem.gpu + 4);
        assert_eq!(d.index_count, 3);
        assert_eq!(d.prim, GLES_PRIM_TRI_STRIP);
    }

    /// An element-buffer offset that leaves the u16 indices off the
    /// index address alignment ([`INDEX_ADDR_ALIGN`]): on the PowerVR they
    /// are staged, the three indices from byte 2 on, not read in place two
    /// bytes early (dEQP draw_elements.indices.buffer.index_short, offset 2).
    #[test]
    fn a_u16_element_buffer_off_the_address_alignment_is_staged() {
        let gl = Gl::new(16, 16);
        basic_vbo(&gl);
        let idx: [u16; 6] = [7, 0, 1, 2, 1, 0];
        let ib = buffer_with(GL_ELEMENT_ARRAY_BUFFER, dword_bytes(&idx));
        let base = gl.ctx().objs().buffers[ib as usize].mem.gpu;
        assert!(base.is_multiple_of(4), "the buffer itself is aligned: only the offset is off");
        // SAFETY: plain GL call; the pointer is a buffer offset.
        unsafe { glDrawElements(GL_TRIANGLES, 3, GL_UNSIGNED_SHORT, 2 as *const c_void) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        let d = one(&gl);
        if INDEX_ADDR_ALIGN == 1 {
            assert_eq!(d.draw.index_buf_gpu, base + 2, "in place on this GPU");
            return;
        }
        assert_ne!(d.draw.index_buf_gpu, base + 2, "not read in place from an unaligned address");
        assert!(!d.draw.index_cpu.is_null() && d.draw.index_bytes == 6, "staged");
        assert_eq!(u16s(&d.index_data), [0, 1, 2], "the indices from byte 2 on");
    }

    #[test]
    fn u8_element_buffer_is_widened_into_staging() {
        let gl = Gl::new(16, 16);
        basic_vbo(&gl);
        let ib = buffer_with(GL_ELEMENT_ARRAY_BUFFER, &[9, 9, 2, 1, 0, 255]);
        // SAFETY: plain GL call; the pointer is a buffer offset.
        unsafe { glDrawElements(GL_TRIANGLES, 3, GL_UNSIGNED_BYTE, core::ptr::without_provenance(2)) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        let d = one(&gl);
        assert_eq!(u16s(&d.index_data), [2, 1, 0]);
        assert_eq!(d.draw.index_bytes, 6);
        assert_ne!(d.draw.index_buf_gpu, gl.ctx().objs().buffers[ib as usize].mem.gpu + 2);
        assert_eq!(u16s(&gl.gpu_bytes(d.draw.index_buf_gpu, 6)), [2, 1, 0]);
    }

    #[test]
    fn u32_element_buffer_is_narrowed_into_staging() {
        let gl = Gl::new(16, 16);
        basic_vbo(&gl);
        let idx: [u32; 4] = [7, 2, 0, 1];
        buffer_with(GL_ELEMENT_ARRAY_BUFFER, dword_bytes(&idx));
        // SAFETY: plain GL call; the pointer is a buffer offset.
        unsafe { glDrawElements(GL_POINTS, 3, GL_UNSIGNED_INT, 4 as *const c_void) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        let d = one(&gl);
        assert_eq!(u16s(&d.index_data), [2, 0, 1]);
        assert_eq!(d.draw.index_count, 3);
        assert_eq!(d.draw.prim, GLES_PRIM_POINTS);
    }

    #[test]
    fn staged_indices_naming_vertices_past_the_vbo_are_refused() {
        let gl = Gl::new(16, 16);
        basic_vbo(&gl);
        let idx: [u32; 3] = [2, 0, 0xffff];
        buffer_with(GL_ELEMENT_ARRAY_BUFFER, dword_bytes(&idx));
        // SAFETY: plain GL call; the pointer is a buffer offset.
        unsafe { glDrawElements(GL_POINTS, 3, GL_UNSIGNED_INT, core::ptr::null()) };
        /* index 0xffff names a vertex the 3-vertex VBO does not have: the
         * staged range is known, so the GPU is not sent past the buffer */
        assert_eq!(gl.error(), GL_INVALID_OPERATION);
        assert!(gl.draws().is_empty());
    }

    #[test]
    fn u32_index_beyond_16_bits_is_invalid_operation() {
        let gl = Gl::new(16, 16);
        basic_vbo(&gl);
        let idx: [u32; 3] = [0, 0x1_0000, 1];
        buffer_with(GL_ELEMENT_ARRAY_BUFFER, dword_bytes(&idx));
        // SAFETY: plain GL call.
        unsafe { glDrawElements(GL_TRIANGLES, 3, GL_UNSIGNED_INT, core::ptr::null()) };
        assert_eq!(gl.error(), GL_INVALID_OPERATION, "the vertex-fetch index path is 16-bit");
        assert!(gl.draws().is_empty());
    }

    #[test]
    fn element_range_past_the_buffer_is_invalid_operation() {
        let gl = Gl::new(16, 16);
        basic_vbo(&gl);
        let idx: [u16; 3] = [0, 1, 2];
        buffer_with(GL_ELEMENT_ARRAY_BUFFER, dword_bytes(&idx));
        // SAFETY: plain GL calls; the pointer is a buffer offset.
        unsafe {
            glDrawElements(GL_TRIANGLES, 4, GL_UNSIGNED_SHORT, core::ptr::null());
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
            glDrawElements(GL_TRIANGLES, 3, GL_UNSIGNED_SHORT, 2 as *const c_void);
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
            glDrawElements(GL_TRIANGLES, 3, GL_UNSIGNED_SHORT, core::ptr::null());
            assert_eq!(gl.error(), GL_NO_ERROR);
        }
        assert_eq!(gl.draws().len(), 1);
    }

    #[test]
    fn element_buffer_without_storage_is_invalid_operation() {
        let gl = Gl::new(16, 16);
        basic_vbo(&gl);
        let mut ib: GLuint = 0;
        // SAFETY: valid out pointer; plain GL calls.
        unsafe {
            glGenBuffers(1, &mut ib);
            glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, ib);
            glDrawElements(GL_TRIANGLES, 3, GL_UNSIGNED_SHORT, core::ptr::null());
        }
        assert_eq!(gl.error(), GL_INVALID_OPERATION);
        assert!(gl.draws().is_empty());
    }

    #[test]
    fn client_memory_indices_are_staged() {
        let gl = Gl::new(16, 16);
        basic_vbo(&gl);
        let idx: [u16; 3] = [2, 0, 1];
        // SAFETY: `idx` outlives the draw.
        unsafe { glDrawElements(GL_TRIANGLE_FAN, 3, GL_UNSIGNED_SHORT, idx.as_ptr() as *const c_void) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        let d = one(&gl);
        assert_eq!(u16s(&d.index_data), [2, 0, 1]);
        assert_eq!(d.draw.prim, GLES_PRIM_TRI_FAN);
        assert!(d.draw.index_buf_gpu >= HEAP_DATA);
    }

    #[test]
    fn client_vertex_arrays_are_staged_up_to_the_largest_index() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        let verts: [f32; 24] = core::array::from_fn(|i| i as f32);
        let idx: [u8; 3] = [4, 0, 2];
        // SAFETY: `verts` (6 vertices) and `idx` outlive the draw.
        unsafe {
            glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, verts.as_ptr() as *const c_void);
            glDrawElements(GL_TRIANGLES, 3, GL_UNSIGNED_BYTE, idx.as_ptr() as *const c_void);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        let d = one(&gl);
        assert_eq!(f32s(&d.attrib_data[0]), verts[..20], "vertices 0..=4");
        assert_eq!(u16s(&d.index_data), [4, 0, 2]);
    }

    #[test]
    fn u16_element_buffer_is_staged_when_the_vertex_range_is_needed() {
        /* a client-side vertex array is converted for exactly 0..=max index,
         * so the indices must be read: they go out staged, not in place */
        let gl = Gl::new(16, 16);
        basic(&gl);
        let idx: [u16; 3] = [1, 2, 0];
        let ib = buffer_with(GL_ELEMENT_ARRAY_BUFFER, dword_bytes(&idx));
        // SAFETY: plain GL call.
        unsafe { glDrawElements(GL_TRIANGLES, 3, GL_UNSIGNED_SHORT, core::ptr::null()) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        let d = one(&gl);
        assert_eq!(u16s(&d.index_data), [1, 2, 0]);
        assert_ne!(d.draw.index_buf_gpu, gl.ctx().objs().buffers[ib as usize].mem.gpu);
        assert_eq!(f32s(&d.attrib_data[0]), TRI);
    }

    #[test]
    fn small_indices_pointer_without_an_element_buffer_is_reported() {
        let gl = Gl::new(16, 16);
        basic_vbo(&gl);
        gl.logs();
        for _ in 0..5 {
            // SAFETY: the pointer is never dereferenced (that is the point).
            unsafe { glDrawElements(GL_TRIANGLES, 3, GL_UNSIGNED_SHORT, 12 as *const c_void) };
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
        }
        // SAFETY: as above.
        unsafe { glDrawElements(GL_TRIANGLES, 3, GL_UNSIGNED_SHORT, core::ptr::null()) };
        assert_eq!(gl.error(), GL_INVALID_OPERATION);
        assert!(gl.draws().is_empty());
        let reports = gl.logs().iter().filter(|l| l.contains("with no ELEMENT_ARRAY_BUFFER bound")).count();
        assert_eq!(reports, 3, "only the first three are logged");
    }

    #[test]
    fn null_indices_report_prints_nil() {
        let gl = Gl::new(16, 16);
        basic_vbo(&gl);
        gl.logs();
        // SAFETY: the pointer is never dereferenced.
        unsafe { glDrawElements(GL_TRIANGLES, 3, GL_UNSIGNED_SHORT, core::ptr::null()) };
        assert_eq!(gl.error(), GL_INVALID_OPERATION);
        let logs = gl.logs();
        assert!(
            logs.iter().any(|l| l.contains("glDrawElements: indices (nil) with no ELEMENT_ARRAY_BUFFER")),
            "{logs:?}"
        );
    }

    #[test]
    fn draw_elements_rejects_other_index_types() {
        let gl = Gl::new(16, 16);
        basic_vbo(&gl);
        let idx: [u16; 3] = [0, 1, 2];
        for type_ in [GL_FLOAT, GL_SHORT, GL_BYTE, GL_INT, 0] {
            // SAFETY: `idx` outlives the call.
            unsafe { glDrawElements(GL_TRIANGLES, 3, type_, idx.as_ptr() as *const c_void) };
            assert_eq!(gl.error(), GL_INVALID_ENUM, "type 0x{type_:x}");
        }
        assert!(gl.draws().is_empty());
    }

    #[test]
    fn index_staging_failure_is_out_of_memory() {
        let gl = Gl::new(16, 16);
        basic_vbo(&gl);
        let idx: [u16; 3] = [0, 1, 2];
        gl.fail_alloc_in(1);
        // SAFETY: `idx` outlives the call.
        unsafe { glDrawElements(GL_TRIANGLES, 3, GL_UNSIGNED_SHORT, idx.as_ptr() as *const c_void) };
        assert_eq!(gl.error(), GL_OUT_OF_MEMORY);
        assert!(gl.draws().is_empty());
    }

    #[test]
    fn attrib_dump_logs_the_staged_indices() {
        let gl = Gl::new(16, 16);
        basic_vbo(&gl);
        let idx: [u16; 3] = [2, 0, 1];
        let _dump = AttribDump::on();
        gl.logs();
        // SAFETY: `idx` outlives the call.
        unsafe { glDrawElements(GL_TRIANGLES, 3, GL_UNSIGNED_SHORT, idx.as_ptr() as *const c_void) };
        let logs = gl.logs();
        assert!(logs.iter().any(|l| l.contains("indices: 2 0 1 (count 3, max 2, mode 0x4)")), "{logs:?}");
    }

    #[test]
    fn the_attrib_dump_shows_a_few_texels_of_every_bound_texture() {
        /* a texture that renders flat is either flat in memory or sampled
         * wrongly, and nothing else in the log tells the two apart */
        let gl = Gl::new(16, 16);
        textured(&gl);
        let px: Vec<u8> = (0..4 * 4 * 4).map(|i| i as u8).collect();
        // SAFETY: plain GL calls; `px` holds the 4x4 image.
        unsafe {
            let mut t = 0;
            glGenTextures(1, &mut t);
            glBindTexture(GL_TEXTURE_2D, t);
            glTexImage2D(
                GL_TEXTURE_2D,
                0,
                GL_RGBA as GLint,
                4,
                4,
                0,
                GL_RGBA,
                GL_UNSIGNED_BYTE,
                px.as_ptr() as *const c_void,
            );
        }
        let _dump = AttribDump::on();
        gl.logs();
        draw_tri(&gl);
        let logs = gl.logs();
        /* (1,1) is texel 5 of the row-major image: bytes 20..24 */
        assert!(logs.iter().any(|l| l.contains("tex unit 0 (4x4 fmt 0):") && l.contains("(1,1)=14151617")), "{logs:?}");
    }

    #[test]
    fn the_attrib_dump_measures_a_glyph_atlas_alpha_box() {
        /* a glyph atlas that samples flat is either empty or holds its
         * glyphs somewhere other than where the text looks */
        let gl = Gl::new(16, 16);
        textured(&gl);
        let (w, h) = (1024usize, 64usize);
        let mut px = std::vec![0u8; w * h * 4];
        for y in 10..12 {
            for x in 100..104 {
                px[(y * w + x) * 4 + 3] = 0xff; /* eight opaque texels */
            }
        }
        // SAFETY: plain GL calls; `px` holds the image.
        unsafe {
            let mut t = 0;
            glGenTextures(1, &mut t);
            glBindTexture(GL_TEXTURE_2D, t);
            glTexImage2D(
                GL_TEXTURE_2D,
                0,
                GL_RGBA as GLint,
                w as GLsizei,
                h as GLsizei,
                0,
                GL_RGBA,
                GL_UNSIGNED_BYTE,
                px.as_ptr() as *const c_void,
            );
        }
        let _dump = AttribDump::on();
        gl.logs();
        draw_tri(&gl);
        let logs = gl.logs();
        assert!(logs.iter().any(|l| l.contains("tex unit 0 alpha: 8 of 65536 texels, box 100,10..103,11")), "{logs:?}");
    }

    #[test]
    fn draw_range_elements_treats_the_range_as_a_hint() {
        let gl = Gl::new(16, 16);
        basic_vbo(&gl);
        let idx: [u16; 3] = [0, 1, 2];
        // SAFETY: `idx` outlives the calls.
        unsafe {
            glDrawRangeElements(GL_TRIANGLES, 5, 4, 3, GL_UNSIGNED_SHORT, idx.as_ptr() as *const c_void);
            assert_eq!(gl.error(), GL_INVALID_VALUE, "end < start");
            assert!(gl.draws().is_empty());
            glDrawRangeElements(GL_TRIANGLES, 0, 2, 3, GL_UNSIGNED_SHORT, idx.as_ptr() as *const c_void);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(u16s(&one(&gl).index_data), [0, 1, 2]);
    }

    /* ---- textures -------------------------------------------------------------------------------- */

    /// uv.vert + texture.frag, current, a_pos from TRI, a_uv constant.
    fn textured(gl: &Gl) -> GLuint {
        let prog = gl.program("uv.vert", "texture.frag");
        // SAFETY: plain GL calls; TRI is a static.
        unsafe {
            glUseProgram(prog);
            glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, TRI.as_ptr() as *const c_void);
            glEnableVertexAttribArray(0);
        }
        prog
    }

    /// The binding an upload passes to `texture_uploaded` carries the key a
    /// backend files a copy of the texels under, and a later draw of the same
    /// texture carries the same key: a copy built at the upload is found at
    /// the draw. A parameter change after the upload (here the min filter)
    /// changes the sampling fields, never the key, so the sampling fields of
    /// an upload are not part of the key.
    #[test]
    fn the_upload_hooks_key_is_the_draws() {
        let gl = Gl::new(16, 16);
        textured(&gl);
        let tex = texture_rgba(3, 8); /* pitch 4 texels: stride_texels != width */
        let up = gl.uploads();
        assert_eq!(up.len(), 1, "one RGBA / UNSIGNED_BYTE upload, one call");
        let ub = up[0].b;
        let key = |b: &GlesTextureBind| {
            (b.base_gpu, b.content_gen, b.width, b.height, b.stride_texels, b.bytes, b.format, b.bgra)
        };
        assert_ne!(ub.content_gen, 0, "0 is 'no stamp' and is never cached");
        assert_eq!(ub.content_gen, gl.ctx().objs().textures[tex as usize].content_gen);
        let tb = draw_tri(&gl).draw.textures[0];
        assert_eq!(key(&ub), key(&tb));
        assert_eq!((ub.mip_filter, ub.mip_levels, ub.min_linear), (tb.mip_filter, tb.mip_levels, tb.min_linear));
        // SAFETY: plain GL call on the bound texture.
        unsafe { glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR as GLint) };
        let tb = draw_tri(&gl).draw.textures[0];
        assert_eq!(key(&ub), key(&tb), "a parameter change is not a texel write");
        assert_ne!((ub.mip_filter, ub.min_linear), (tb.mip_filter, tb.min_linear));
        assert!(gl.uploads().is_empty());
    }

    #[test]
    fn bound_texture_is_wired_to_its_sampler_binding() {
        let gl = Gl::new(16, 16);
        textured(&gl);
        let tex = texture_rgba(4, 8);
        // SAFETY: plain GL call.
        unsafe { glFlush() }; /* kick id 1 */
        let d = draw_tri(&gl).draw;
        assert_eq!(d.num_textures, 1);
        let tb = d.textures[0];
        assert_eq!(tb.unit, 0);
        assert_eq!(tb.format, GLES_TEXFMT_RGBA8);
        assert_eq!(tb.base_gpu, gl.ctx().objs().textures[tex as usize].mem.gpu);
        assert_eq!((tb.width, tb.height), (4, 8));
        assert_eq!(tb.stride_texels, 4);
        assert_eq!(tb.bytes, 4 * 8 * 4);
        assert_eq!(tb.bgra, 0);
        assert_eq!(gl.ctx().objs().textures[tex as usize].last_use, 1);
        /* the texels' write stamp reaches the backend (a backend keeping a
         * copy of the texels keys on it) */
        assert_ne!(tb.content_gen, 0);
        assert_eq!(tb.content_gen, gl.ctx().objs().textures[tex as usize].content_gen);
    }

    #[test]
    fn default_texture_parameters() {
        /* GL defaults: MAG LINEAR, MIN NEAREST_MIPMAP_LINEAR, WRAP REPEAT;
         * with a single level the mipmap filter cannot be honoured */
        let gl = Gl::new(16, 16);
        textured(&gl);
        texture_rgba(4, 4);
        let tb = draw_tri(&gl).draw.textures[0];
        assert_eq!(tb.linear_filter, 1);
        assert_eq!(tb.min_linear, 0);
        assert_eq!(tb.wrap_s, GLES_WRAP_REPEAT);
        assert_eq!(tb.mip_levels, 1);
        assert_eq!(tb.mip_filter, 0, "no chain: sampled from level 0");
    }

    #[test]
    fn filters_and_wrap_are_resolved() {
        let gl = Gl::new(16, 16);
        textured(&gl);
        texture_rgba(4, 4);
        // SAFETY: plain GL calls.
        unsafe {
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR as GLint);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE as GLint);
        }
        let tb = draw_tri(&gl).draw.textures[0];
        assert_eq!((tb.linear_filter, tb.min_linear, tb.wrap_s, tb.mip_filter), (0, 1, GLES_WRAP_CLAMP_TO_EDGE, 0));
    }

    #[test]
    fn s_and_t_wrap_reach_the_backend_separately() {
        let gl = Gl::new(16, 16);
        textured(&gl);
        texture_rgba(4, 4);
        for (s, t, want) in [
            (GL_REPEAT, GL_REPEAT, (GLES_WRAP_REPEAT, GLES_WRAP_REPEAT)),
            (GL_CLAMP_TO_EDGE, GL_REPEAT, (GLES_WRAP_CLAMP_TO_EDGE, GLES_WRAP_REPEAT)),
            (GL_REPEAT, GL_CLAMP_TO_EDGE, (GLES_WRAP_REPEAT, GLES_WRAP_CLAMP_TO_EDGE)),
            (GL_CLAMP_TO_EDGE, GL_CLAMP_TO_EDGE, (GLES_WRAP_CLAMP_TO_EDGE, GLES_WRAP_CLAMP_TO_EDGE)),
            /* GL_MIRRORED_REPEAT used to be silently GL_REPEAT */
            (GL_MIRRORED_REPEAT, GL_REPEAT, (GLES_WRAP_MIRRORED_REPEAT, GLES_WRAP_REPEAT)),
            (GL_REPEAT, GL_MIRRORED_REPEAT, (GLES_WRAP_REPEAT, GLES_WRAP_MIRRORED_REPEAT)),
            (GL_MIRRORED_REPEAT, GL_CLAMP_TO_EDGE, (GLES_WRAP_MIRRORED_REPEAT, GLES_WRAP_CLAMP_TO_EDGE)),
        ] {
            // SAFETY: plain GL calls.
            unsafe {
                glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, s as GLint);
                glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, t as GLint);
            }
            let tb = draw_tri(&gl).draw.textures[0];
            assert_eq!((tb.wrap_s, tb.wrap_t), want, "wrap 0x{s:x}/0x{t:x}");
        }
    }

    /// An ETC1 chain (gl_test 126 etcmip's: 16x16, five levels) reaches the
    /// backend as one: its format, a mipmap filter and all five levels
    /// (the PowerVR's twiddled descriptor walks `mip_levels`).
    #[test]
    fn an_etc1_chain_reaches_the_backend_with_its_levels() {
        let gl = Gl::new(16, 16);
        textured(&gl);
        let blocks = [0u8; 16 * 8];
        // SAFETY: plain GL calls; `blocks` holds the largest level's 16 blocks.
        unsafe {
            let mut tex = 0;
            glGenTextures(1, &mut tex);
            glBindTexture(GL_TEXTURE_2D, tex);
            for l in 0..5 {
                let side = 16 >> l;
                let n = ((side + 3) / 4) * ((side + 3) / 4) * 8;
                glCompressedTexImage2D(
                    GL_TEXTURE_2D,
                    l,
                    GL_ETC1_RGB8_OES,
                    side,
                    side,
                    0,
                    n,
                    blocks.as_ptr() as *const c_void,
                );
            }
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST_MIPMAP_NEAREST as GLint);
            assert_eq!(gl.error(), GL_NO_ERROR);
        }
        let tb = draw_tri(&gl).draw.textures[0];
        assert_eq!((tb.format, tb.mip_filter, tb.mip_levels), (GLES_TEXFMT_ETC2_RGB, 1, 5));
    }

    #[test]
    fn the_sampler_stops_at_the_last_level_anyone_filled() {
        /* glTexStorage2D declares the whole chain; only the levels the
         * application uploads may be sampled (Qt prefilters part of a light
         * probe's chain and caps the rest with GL_TEXTURE_MAX_LEVEL). */
        let gl = Gl::new(16, 16);
        textured(&gl);
        let px = [0u8; 8 * 8 * 4];
        // SAFETY: plain GL calls; `px` outlives them.
        unsafe {
            let mut tex = 0;
            glGenTextures(1, &mut tex);
            glBindTexture(GL_TEXTURE_2D, tex);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR_MIPMAP_LINEAR as GLint);
            glTexStorage2D(GL_TEXTURE_2D, 4, GL_RGBA8, 8, 8);
            assert_eq!(gl.error(), GL_NO_ERROR);
        }
        assert_eq!(draw_tri(&gl).draw.textures[0].mip_levels, 1, "declared, nothing written");
        // SAFETY: as above; the level-1 upload is 4x4 RGBA8.
        unsafe {
            glTexSubImage2D(GL_TEXTURE_2D, 1, 0, 0, 4, 4, GL_RGBA, GL_UNSIGNED_BYTE, px.as_ptr() as *const c_void);
        }
        assert_eq!(draw_tri(&gl).draw.textures[0].mip_levels, 2, "levels 0 and 1 hold texels");
        // SAFETY: plain GL call.
        unsafe { glGenerateMipmap(GL_TEXTURE_2D) };
        assert_eq!(draw_tri(&gl).draw.textures[0].mip_levels, 4, "the whole chain");
        // SAFETY: plain GL call.
        unsafe { glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAX_LEVEL, 1) };
        assert_eq!(draw_tri(&gl).draw.textures[0].mip_levels, 2, "the application capped the sampler");
        // SAFETY: plain GL call.
        unsafe { glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAX_LEVEL, 0) };
        assert_eq!(draw_tri(&gl).draw.textures[0].mip_levels, 1);
    }

    #[test]
    fn rendering_into_a_level_marks_it_written() {
        /* Qt prefilters a light probe level by level through an FBO. */
        let gl = Gl::new(16, 16);
        textured(&gl);
        // SAFETY: plain GL calls.
        unsafe {
            let (mut tex, mut fb) = (0, 0);
            glGenTextures(1, &mut tex);
            glBindTexture(GL_TEXTURE_2D, tex);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR_MIPMAP_LINEAR as GLint);
            glTexStorage2D(GL_TEXTURE_2D, 4, GL_RGBA8, 8, 8);
            glGenFramebuffers(1, &mut fb);
            glBindFramebuffer(GL_FRAMEBUFFER, fb);
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 2);
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            assert_eq!(gl.error(), GL_NO_ERROR);
        }
        assert_eq!(draw_tri(&gl).draw.textures[0].mip_levels, 3, "levels 0..2 exist now");
    }

    #[test]
    fn mipmap_filters_need_and_use_the_chain() {
        let gl = Gl::new(16, 16);
        textured(&gl);
        texture_rgba(4, 4);
        // SAFETY: plain GL call.
        unsafe { glGenerateMipmap(GL_TEXTURE_2D) };
        /* (min filter) -> (min_linear, mip_filter) */
        let table = [
            (GL_NEAREST, 0, 0),
            (GL_LINEAR, 1, 0),
            (GL_NEAREST_MIPMAP_NEAREST, 0, 1),
            (GL_LINEAR_MIPMAP_NEAREST, 1, 1),
            (GL_NEAREST_MIPMAP_LINEAR, 0, 2),
            (GL_LINEAR_MIPMAP_LINEAR, 1, 2),
        ];
        for (filter, min_linear, mip_filter) in table {
            // SAFETY: plain GL call.
            unsafe { glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, filter as GLint) };
            let tb = draw_tri(&gl).draw.textures[0];
            assert_eq!(tb.mip_levels, 3, "4x4 -> 2x2 -> 1x1");
            assert_eq!((tb.min_linear, tb.mip_filter), (min_linear, mip_filter), "min filter 0x{filter:x}");
        }
    }

    /// A texture whose GL base format has no alpha says so on the bind
    /// (`no_alpha`), so a backend can read alpha 1 whatever the RGBA8
    /// storage's alpha byte holds; RGBA does not.
    #[test]
    fn a_texture_without_alpha_says_so_on_the_bind() {
        let gl = Gl::new(16, 16);
        textured(&gl);
        let rgb = [0u8; 4 * 4 * 3];
        // SAFETY: `rgb` holds the 4x4 RGB image; plain GL calls.
        unsafe {
            let mut t: GLuint = 0;
            glGenTextures(1, &mut t);
            glBindTexture(GL_TEXTURE_2D, t);
            glTexImage2D(
                GL_TEXTURE_2D,
                0,
                GL_RGB as GLint,
                4,
                4,
                0,
                GL_RGB,
                GL_UNSIGNED_BYTE,
                rgb.as_ptr() as *const c_void,
            );
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
        }
        assert_eq!(draw_tri(&gl).draw.textures[0].no_alpha, 1);
        texture_rgba(4, 4);
        assert_eq!(draw_tri(&gl).draw.textures[0].no_alpha, 0);
    }

    /// A depth texture is sampled as luminance, (d, d, d, 1), where
    /// GL_OES_depth_texture is claimed (`depth_luminance`), and as ES 3.0's
    /// red, (d, 0, 0, 1), elsewhere; a colour texture never is.
    #[test]
    fn a_depth_texture_samples_as_luminance_where_the_extension_is_claimed() {
        let gl = Gl::new(16, 16);
        textured(&gl);
        let want = u8::from(crate::gl::state::DEPTH_TEXTURE_EXTENSIONS);
        for (fmt, ty) in [(GL_DEPTH_COMPONENT, GL_UNSIGNED_SHORT), (GL_DEPTH_STENCIL, GL_UNSIGNED_INT_24_8)] {
            let px = [0u8; 4 * 4 * 4];
            // SAFETY: `px` covers a 4x4 image of up to 4 bytes a texel.
            unsafe {
                glTexImage2D(GL_TEXTURE_2D, 0, fmt as GLint, 4, 4, 0, fmt, ty, px.as_ptr() as *const c_void);
                glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
            }
            assert_eq!(gl.error(), GL_NO_ERROR);
            let tb = draw_tri(&gl).draw.textures[0];
            assert_eq!((tb.format, tb.depth_luminance), (GLES_TEXFMT_R32F, want), "0x{fmt:x}");
        }
        texture_rgba(4, 4);
        assert_eq!(draw_tri(&gl).draw.textures[0].depth_luminance, 0);
    }

    /// The depth texel (x, y) of level 0 of texture `name`.
    fn depth_texel(gl: &Gl, name: GLuint, x: usize, y: usize) -> f32 {
        let t = &gl.ctx().objs().textures[name as usize];
        let pitch = t.level_pitch(t.width) as usize;
        // SAFETY: the level's storage holds its rows of R32F texels.
        unsafe { (t.mem.cpu as *const f32).add(t.level_off[0] as usize / 4 + y * pitch + x).read() }
    }

    /// Depth a draw renders into a depth texture stays in the backend's
    /// buffer until the texture is next read: a draw that samples it first
    /// has the kick that rendered it waited for and the backend's depth
    /// written into the texels (`backend::depth_resolve`, once per render),
    /// whose stamp moves; a sub-image write resolves before it writes, so the
    /// rest of the level keeps the rendered depth; a backend that holds
    /// nothing leaves the texels, and is not asked again for that render.
    #[test]
    fn rendered_depth_reaches_the_texels_when_the_texture_is_next_read() {
        let gl = Gl::new(16, 16);
        textured(&gl);
        let mut depth: GLuint = 0;
        let zeros = [0u16; 64];
        // SAFETY: room for one name; `zeros` holds an 8x8 GL_UNSIGNED_SHORT image.
        unsafe {
            glGenTextures(1, &mut depth);
            glBindTexture(GL_TEXTURE_2D, depth);
            glTexImage2D(
                GL_TEXTURE_2D,
                0,
                GL_DEPTH_COMPONENT as GLint,
                8,
                8,
                0,
                GL_DEPTH_COMPONENT,
                GL_UNSIGNED_SHORT,
                zeros.as_ptr() as *const c_void,
            );
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
        }
        let colour = texture_rgba(8, 8); /* bound to unit 0 while rendering: no feedback loop */
        let mut fbo: GLuint = 0;
        // SAFETY: valid out pointer, plain GL calls.
        unsafe {
            glGenFramebuffers(1, &mut fbo);
            glBindFramebuffer(GL_FRAMEBUFFER, fbo);
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, colour, 0);
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_TEXTURE_2D, depth, 0);
            assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
            glEnable(GL_DEPTH_TEST);
        }
        let render = || {
            // SAFETY: plain GL calls.
            unsafe {
                glBindFramebuffer(GL_FRAMEBUFFER, fbo);
                glBindTexture(GL_TEXTURE_2D, colour);
            }
            draw_tri(&gl);
            // SAFETY: plain GL calls.
            unsafe {
                glBindFramebuffer(GL_FRAMEBUFFER, 0);
                glBindTexture(GL_TEXTURE_2D, depth);
            }
        };
        let id = context::ds_object_id(gl.ctx().objs().textures[depth as usize].serial, true);
        let pitch = gl.ctx().objs().textures[depth as usize].level_pitch(8);
        render();
        assert!(gl.depth_resolves().is_empty(), "rendering resolves nothing");
        /* (a) sampled: resolved once, after the render's kick completed */
        gl.set_resolve_depth(Some(0.5));
        let gen = gl.ctx().objs().textures[depth as usize].content_gen;
        let finishes = gl.finishes();
        let tb = draw_tri(&gl).draw.textures[0];
        let r = gl.depth_resolves();
        assert_eq!(r.len(), 1, "{r:?}");
        assert_eq!((r[0].id, r[0].w, r[0].h, r[0].pitch), (id, 8, 8, pitch));
        assert!(r[0].finishes > finishes, "the render was waited for first");
        assert_ne!(tb.content_gen, gen, "the texels' stamp moved");
        assert_eq!((depth_texel(&gl, depth, 0, 0), depth_texel(&gl, depth, 7, 7)), (0.5, 0.5));
        draw_tri(&gl);
        assert!(gl.depth_resolves().is_empty(), "no render since: nothing to resolve");
        if crate::REPORT_ES2 && !crate::gl::state::DEPTH_TEXTURE_EXTENSIONS {
            return; /* no depth sub-image on an ES 2.0 surface without the extensions (the V3D's) */
        }
        /* (b) a sub-image write: resolved before the write */
        render();
        gl.set_resolve_depth(Some(0.75));
        let one = [0xffffu16];
        // SAFETY: `one` holds a 1x1 GL_UNSIGNED_SHORT image.
        unsafe {
            glTexSubImage2D(
                GL_TEXTURE_2D,
                0,
                0,
                0,
                1,
                1,
                GL_DEPTH_COMPONENT,
                GL_UNSIGNED_SHORT,
                one.as_ptr() as *const c_void,
            )
        };
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(gl.depth_resolves().len(), 1);
        assert_eq!((depth_texel(&gl, depth, 0, 0), depth_texel(&gl, depth, 1, 0)), (1.0, 0.75));
        draw_tri(&gl);
        assert!(gl.depth_resolves().is_empty(), "the write did not ask again");
        /* (c) the backend holds nothing: the texels stay, asked once */
        render();
        gl.set_resolve_depth(None);
        draw_tri(&gl);
        draw_tri(&gl);
        assert_eq!(gl.depth_resolves().len(), 1);
        assert_eq!((depth_texel(&gl, depth, 0, 0), depth_texel(&gl, depth, 1, 0)), (1.0, 0.75));
    }

    /// A colour texture and a depth texture (8x8) attached to a new
    /// framebuffer object, left bound; the depth texture's name second.
    fn colour_depth_fbo() -> (GLuint, GLuint, GLuint) {
        let mut depth: GLuint = 0;
        let zeros = [0u16; 64];
        // SAFETY: room for one name; `zeros` holds an 8x8 GL_UNSIGNED_SHORT image.
        unsafe {
            glGenTextures(1, &mut depth);
            glBindTexture(GL_TEXTURE_2D, depth);
            glTexImage2D(
                GL_TEXTURE_2D,
                0,
                GL_DEPTH_COMPONENT as GLint,
                8,
                8,
                0,
                GL_DEPTH_COMPONENT,
                GL_UNSIGNED_SHORT,
                zeros.as_ptr() as *const c_void,
            );
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
        }
        let colour = texture_rgba(8, 8);
        let mut fbo: GLuint = 0;
        // SAFETY: valid out pointer, plain GL calls.
        unsafe {
            glGenFramebuffers(1, &mut fbo);
            glBindFramebuffer(GL_FRAMEBUFFER, fbo);
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, colour, 0);
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_TEXTURE_2D, depth, 0);
            assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
        }
        (fbo, colour, depth)
    }

    /// glmark2 shadow's frame on the PowerVR (pvr-depth-resolve): the frame
    /// open on the window when the render into a depth texture starts is
    /// submitted by that draw (the backend's frame split), and the render is
    /// the NEXT kick. The stamps the draw took while it was built named the
    /// kick being recorded then, the window's; they move to the kick the
    /// backend put the draw in (`restamp_draw`): the render-into stamps of
    /// both attachments and the sampled texture's last use. So the draw that
    /// samples the depth submits that kick and waits for it before the
    /// resolve. With kicks that complete at their submit (linux.c, and the
    /// usual case on the board) the window's kick is done by then, and a
    /// stamp left on it waited for nothing: the resolve ran while the render
    /// was still being recorded.
    #[test]
    fn a_render_that_ends_the_open_frame_is_stamped_with_the_kick_holding_it() {
        let gl = Gl::new(16, 16);
        gl.set_kick_lag(Some(0));
        textured(&gl);
        let (fbo, colour, depth) = colour_depth_fbo();
        // SAFETY: plain GL calls.
        unsafe {
            glEnable(GL_DEPTH_TEST);
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
        }
        draw_tri(&gl);
        let window = gl.kick_id();
        assert!(gl.kick_pending(window), "the window's frame is open");
        // SAFETY: plain GL calls.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, fbo);
            glBindTexture(GL_TEXTURE_2D, colour); /* sampled while rendering into it: only stamps here */
        }
        let render = draw_tri(&gl);
        assert_eq!(gl.submitted(), window, "the render's draw submitted the window's frame");
        assert!(!gl.kick_pending(window), "and it completed at once");
        assert_eq!(render.kick, window + 1, "the render is the next kick");
        for (what, ti) in [("depth", depth), ("colour", colour)] {
            assert_eq!(framebuffer::texture_target_use(gl.ctx(), ti as usize), render.kick, "{what} render-into stamp");
        }
        assert_eq!(gl.ctx().objs().textures[colour as usize].last_use, render.kick, "the sampled texture's last use");
        /* the ground: back to the window, sampling the depth */
        // SAFETY: plain GL calls.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glBindTexture(GL_TEXTURE_2D, depth);
        }
        gl.set_resolve_depth(Some(0.5));
        let flushes = gl.flushes();
        draw_tri(&gl);
        let r = gl.depth_resolves();
        assert_eq!(r.len(), 1, "{r:?}");
        assert!(r[0].flushes > flushes, "the render's kick was submitted before the resolve: {r:?}");
        assert_eq!(depth_texel(&gl, depth, 7, 7), 0.5);
    }

    /// The same stamps protect a CPU write: glTexSubImage2D into a texture
    /// a split-off render still has open, and glBufferSubData into the
    /// buffer that render reads, wait for (or step around) the render's kick
    /// rather than the completed one before it. The mock flags a write that
    /// lands under a kick still to run.
    #[test]
    fn a_cpu_write_after_a_render_that_ended_the_open_frame_waits_for_its_kick() {
        let gl = Gl::new(16, 16);
        gl.set_kick_lag(Some(0));
        let prog = gl.program("uv.vert", "texture.frag");
        let vbo = buffer_with(GL_ARRAY_BUFFER, dword_bytes(&TRI));
        // SAFETY: plain GL calls; the vertices come from `vbo`.
        unsafe {
            glUseProgram(prog);
            glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, core::ptr::null());
            glEnableVertexAttribArray(0);
        }
        let sampled = texture_rgba(4, 4);
        let (fbo, colour, _) = colour_depth_fbo();
        // SAFETY: plain GL calls.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glBindTexture(GL_TEXTURE_2D, sampled);
        }
        draw_tri(&gl);
        let window = gl.kick_id();
        // SAFETY: plain GL call.
        unsafe { glBindFramebuffer(GL_FRAMEBUFFER, fbo) };
        let render = draw_tri(&gl);
        assert_eq!((gl.submitted(), render.kick), (window, window + 1));
        assert_eq!(gl.ctx().objs().buffers[vbo as usize].last_use, render.kick, "the vertex buffer's last use");
        assert_eq!(gl.ctx().objs().textures[sampled as usize].last_use, render.kick);
        /* the render target, written by the CPU while its kick is open */
        let flushes = gl.flushes();
        let px = [0xffu8; 4];
        // SAFETY: `px` holds a 1x1 RGBA image; `TRI` covers the bytes passed.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glBindTexture(GL_TEXTURE_2D, colour);
            glTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, px.as_ptr() as *const c_void);
            glBufferSubData(GL_ARRAY_BUFFER, 0, 16, TRI.as_ptr() as *const c_void);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert!(gl.flushes() > flushes, "the render's kick was submitted before the write");
        assert!(!gl.kick_pending(render.kick));
        assert_eq!(gl.take_faults(), Vec::<String>::new());
    }

    #[test]
    fn odd_width_texture_reports_the_even_row_pitch() {
        let gl = Gl::new(16, 16);
        textured(&gl);
        texture_rgba(5, 3);
        let tb = draw_tri(&gl).draw.textures[0];
        assert_eq!((tb.width, tb.height), (5, 3));
        assert_eq!(tb.stride_texels, 6, "rows are padded to whole pixel pairs");
    }

    #[test]
    /* a BUG test until upstream fixed it (7e76640) */
    fn odd_width_texture_bytes_cover_the_padded_rows() {
        let gl = Gl::new(16, 16);
        textured(&gl);
        texture_rgba(5, 3);
        assert_eq!(draw_tri(&gl).draw.textures[0].bytes, 6 * 3 * 4);
    }

    #[test]
    fn external_xrgb_texture_is_flagged_bgra() {
        let gl = Gl::new(16, 16);
        textured(&gl);
        let mut t: GLuint = 0;
        let surface = gl.ctx().default_surface; /* any live allocation will do */
        gl.gpu_render(surface.gpu, &[0; 16 * 16 * 4]); /* the image holds pixels */
        // SAFETY: valid out pointer and descriptor; plain calls.
        unsafe {
            glGenTextures(1, &mut t);
            glBindTexture(GL_TEXTURE_2D, t);
            assert_eq!(gl_texture_attach_external(&surface, 16, 16, GLES_SURF_XRGB8888, core::ptr::null_mut()), 0);
        }
        let tb = draw_tri(&gl).draw.textures[0];
        assert_eq!(tb.bgra, 1);
        forget_external(&gl, t);
        assert_eq!(tb.format, GLES_TEXFMT_RGBA8);
        assert_eq!(tb.base_gpu, surface.gpu);
        assert_eq!(tb.bytes, 16 * 16 * 4);
    }

    #[test]
    fn external_rgb565_texture_is_two_bytes_per_texel() {
        let gl = Gl::new(16, 16);
        textured(&gl);
        let mut t: GLuint = 0;
        let surface = gl.ctx().default_surface;
        gl.gpu_render(surface.gpu, &[0; 16 * 8 * 2]); /* the image holds pixels */
        // SAFETY: valid out pointer and descriptor; plain calls.
        unsafe {
            glGenTextures(1, &mut t);
            glBindTexture(GL_TEXTURE_2D, t);
            assert_eq!(gl_texture_attach_external(&surface, 16, 8, GLES_SURF_RGB565, core::ptr::null_mut()), 0);
        }
        let tb = draw_tri(&gl).draw.textures[0];
        assert_eq!(tb.format, GLES_TEXFMT_RGB565);
        assert_eq!(tb.bytes, 16 * 8 * 2);
        assert_eq!(tb.bgra, 0);
        forget_external(&gl, t);
    }

    #[test]
    fn samplers_follow_their_gluniform1i_units() {
        let gl = Gl::new(16, 16);
        let prog = gl.program("uv.vert", "texture2.frag");
        // SAFETY: plain GL calls; valid names; TRI is a static.
        let (a, b) = unsafe {
            glUseProgram(prog);
            glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, TRI.as_ptr() as *const c_void);
            glEnableVertexAttribArray(0);
            glActiveTexture(GL_TEXTURE3);
            let a = texture_rgba(2, 2);
            glActiveTexture(GL_TEXTURE1);
            let b = texture_rgba(8, 8);
            glUniform1i(glGetUniformLocation(prog, c"u_tex0".as_ptr()), 3);
            glUniform1i(glGetUniformLocation(prog, c"u_tex1".as_ptr()), 1);
            (a, b)
        };
        let d = draw_tri(&gl).draw;
        assert_eq!(d.num_textures, 2);
        let by_binding = |n: u8| d.textures[..2].iter().find(|t| t.unit == n).expect("sampler binding");
        /* GlesTextureBind::unit is the sampler binding, not the GL unit */
        assert_eq!(by_binding(0).base_gpu, gl.ctx().objs().textures[a as usize].mem.gpu);
        assert_eq!(by_binding(0).width, 2);
        assert_eq!(by_binding(1).base_gpu, gl.ctx().objs().textures[b as usize].mem.gpu);
        assert_eq!(by_binding(1).width, 8);
    }

    /// An array of samplers is ONE row (the online compiler's
    /// `reflect::sampler_row`) whose element k reads binding `binding + k`,
    /// and every element is bound: texture2.frag's two samplers stand in as
    /// `u_tex[2]`. Binding only the row's own binding -- the loop before
    /// 2026-09-23 -- drew with element 1 unbound.
    #[test]
    fn every_element_of_a_sampler_array_is_bound() {
        let gl = Gl::new(16, 16);
        let prog = gl.program("uv.vert", "texture2.frag");
        {
            let p = &mut gl.ctx().objs().programs[obj_index(prog)];
            let mut table: Vec<ShcUniform> =
                p.rfs.uniform_table().iter().filter(|u| u.dword_off != SHC_UNIFORM_NO_BLOCK).copied().collect();
            let mut row = *p.rfs.uniform_table().iter().find(|u| u.dword_off == SHC_UNIFORM_NO_BLOCK).unwrap();
            util::set_name(&mut row.name, b"u_tex");
            (row.binding, row.array_len, row.dwords) = (0, 2, 2);
            table.push(row);
            let table: &'static [ShcUniform] = Vec::leak(table);
            p.rfs.uniforms = table.as_ptr();
            p.rfs.num_uniforms = table.len() as u8;
            (p.fs_sampler_unit[0], p.fs_sampler_unit[1]) = (3, 1);
        }
        // SAFETY: plain GL calls; valid names; TRI is a static.
        let (a, b) = unsafe {
            glUseProgram(prog);
            glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, TRI.as_ptr() as *const c_void);
            glEnableVertexAttribArray(0);
            glActiveTexture(GL_TEXTURE3);
            let a = texture_rgba(2, 2);
            glActiveTexture(GL_TEXTURE1);
            let b = texture_rgba(8, 8);
            (a, b)
        };
        let d = draw_tri(&gl).draw;
        assert_eq!(d.num_textures, 2, "both elements are bound");
        let by_binding = |n: u8| d.textures[..2].iter().find(|t| t.unit == n).expect("sampler binding");
        assert_eq!(by_binding(0).base_gpu, gl.ctx().objs().textures[a as usize].mem.gpu);
        assert_eq!(by_binding(1).base_gpu, gl.ctx().objs().textures[b as usize].mem.gpu);
    }

    /// The GC7000's sixteen vertex units (`GLES_VS_TEX_UNITS`) at the draw:
    /// a vertex stage with a sampler array of `n` (9..16) elements on units
    /// 0..n-1 carries all `n` bindings, each its own texture.
    #[cfg(feature = "gpu-vivante")]
    #[test]
    fn the_gc7000_vertex_stage_carries_up_to_sixteen_samplers() {
        let gl = Gl::new(16, 16);
        let prog = basic(&gl);
        let textures: Vec<GLuint> = (0..16u32)
            .map(|k| {
                // SAFETY: plain GL call.
                unsafe { glActiveTexture(GL_TEXTURE0 + k) };
                texture_rgba(k as i32 + 1, 1)
            })
            .collect();
        for n in 9..=16u8 {
            {
                let p = &mut gl.ctx().objs().programs[obj_index(prog)];
                let mut table: Vec<ShcUniform> =
                    p.rvs.uniform_table().iter().filter(|u| u.dword_off != SHC_UNIFORM_NO_BLOCK).copied().collect();
                let mut row = ShcUniform::ZERO;
                util::set_name(&mut row.name, b"u_s");
                (row.dword_off, row.gl_type, row.binding, row.array_len, row.dwords) =
                    (SHC_UNIFORM_NO_BLOCK, GL_SAMPLER_2D as u16, 0, n, n as u16);
                table.push(row);
                let table: &'static [ShcUniform] = Vec::leak(table);
                p.rvs.uniforms = table.as_ptr();
                p.rvs.num_uniforms = table.len() as u8;
                for k in 0..16u8 {
                    p.vs_sampler_unit[k as usize] = k;
                }
            }
            let d = draw_tri(&gl).draw;
            assert_eq!(d.num_vs_textures, n, "{n} vertex samplers");
            for k in 0..n {
                let t = d.vs_textures[..n as usize].iter().find(|t| t.unit == k).expect("sampler binding");
                assert_eq!(t.base_gpu, gl.ctx().objs().textures[textures[k as usize] as usize].mem.gpu, "binding {k}");
                assert_eq!(t.width, k as u32 + 1, "binding {k}");
            }
        }
    }

    #[test]
    fn sampler_without_a_usable_texture_is_left_to_the_backend() {
        let gl = Gl::new(16, 16);
        textured(&gl);
        assert_eq!(draw_tri(&gl).draw.num_textures, 0, "nothing bound");
        let mut t: GLuint = 0;
        // SAFETY: valid out pointer; plain GL calls.
        unsafe {
            glGenTextures(1, &mut t);
            glBindTexture(GL_TEXTURE_2D, t); /* no glTexImage2D: no storage */
        }
        assert_eq!(draw_tri(&gl).draw.num_textures, 0, "no storage");
    }

    #[test]
    fn program_without_samplers_binds_no_texture() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        texture_rgba(4, 4);
        assert_eq!(draw_tri(&gl).draw.num_textures, 0);
    }

    /* ---- instanced draws -------------------------------------------------------------------------- */

    static COLOURS: [f32; 12] = [1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0];

    fn per_instance_colour(divisor: GLuint) {
        // SAFETY: plain GL calls; COLOURS is a static.
        unsafe {
            glVertexAttribPointer(1, 4, GL_FLOAT, 0, 0, COLOURS.as_ptr() as *const c_void);
            glEnableVertexAttribArray(1);
            glVertexAttribDivisor(1, divisor);
        }
    }

    /// An enabled array with divisor 2 on an index the program does not read: an instanced draw then takes the
    /// per-instance CPU path (one draw per instance) for every per-instance array, divisor 1 included.
    fn force_per_draw_instances() {
        // SAFETY: plain GL calls; COLOURS is a static.
        unsafe {
            glBindBuffer(GL_ARRAY_BUFFER, 0);
            glVertexAttribPointer(7, 4, GL_FLOAT, 0, 0, COLOURS.as_ptr() as *const c_void);
            glEnableVertexAttribArray(7);
            glVertexAttribDivisor(7, 2);
        }
    }

    #[test]
    fn instanced_draw_is_issued_once_per_instance_with_the_instances_constant() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        force_per_draw_instances();
        per_instance_colour(1);
        // SAFETY: plain GL call.
        unsafe { glDrawArraysInstanced(GL_TRIANGLES, 0, 3, 3) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        let draws = gl.draws();
        assert_eq!(draws.len(), 3);
        for (i, d) in draws.iter().enumerate() {
            assert_eq!(d.draw.index_count, 3);
            assert_eq!(d.draw.attribs[1].stride, 0, "fed as a constant");
            assert_eq!(f32s(&d.attrib_data[1]), COLOURS[i * 4..i * 4 + 4], "instance {i}");
            assert_eq!(f32s(&d.attrib_data[0]), TRI, "per-vertex arrays are unaffected");
        }
    }

    #[test]
    fn instanced_draw_restores_the_array_and_the_current_attribute() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        force_per_draw_instances();
        per_instance_colour(1);
        // SAFETY: plain GL calls.
        unsafe {
            glVertexAttrib4f(1, 0.5, 0.5, 0.5, 0.5);
            glDrawArraysInstanced(GL_TRIANGLES, 0, 3, 2);
        }
        assert!(gl.ctx().attribs[1].enabled);
        assert_eq!(gl.ctx().current_attrib[1], [0.5, 0.5, 0.5, 0.5]);
    }

    #[test]
    fn divisor_advances_the_attribute_every_n_instances() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        per_instance_colour(2);
        // SAFETY: plain GL call.
        unsafe { glDrawArraysInstanced(GL_TRIANGLES, 0, 3, 5) };
        let got: Vec<Vec<f32>> = gl.draws().iter().map(|d| f32s(&d.attrib_data[1])).collect();
        let want: Vec<Vec<f32>> = [0, 0, 1, 1, 2].iter().map(|e| COLOURS[e * 4..e * 4 + 4].to_vec()).collect();
        assert_eq!(got, want);
    }

    /// P1: a backend that takes divisors (max_instance_divisor 0xffff, as the
    /// V3D) gets an instanced draw with a divisor-2 array as ONE draw: the
    /// instance count, the array per instance with its divisor, and the
    /// elements the instances read (5 instances at divisor 2: 3 elements).
    /// The PowerVR-shaped mock (divisor 1 only) still gets one draw per
    /// instance, as before. Revert check: `a.divisor > max_divisor` back to
    /// `!= 1` makes the first half five draws.
    #[test]
    fn a_divisor_the_backend_takes_is_one_draw_carrying_the_divisor() {
        let gl = Gl::new(16, 16);
        gl.backend_takes_divisors();
        basic(&gl);
        per_instance_colour(2);
        // SAFETY: plain GL call.
        unsafe { glDrawArraysInstanced(GL_TRIANGLES, 0, 3, 5) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        let draws = gl.draws();
        assert_eq!(draws.len(), 1, "one draw");
        let d = &draws[0];
        assert_eq!((d.draw.instance_count, d.draw.instance_id), (5, 0));
        let a = d.draw.attribs[1];
        assert_eq!((a.per_instance, a.divisor), (1, 2));
        assert_eq!(f32s(&d.attrib_data[1]), COLOURS[..12].to_vec(), "elements 0, 1, 2 of the colours");
        drop(draws);
        drop(gl); /* it holds the tests' lock: gone before the next Gl */
        /* the PowerVR-shaped mock: divisor 1 only, one draw per instance */
        let gl = Gl::new(16, 16);
        basic(&gl);
        per_instance_colour(2);
        // SAFETY: plain GL call.
        unsafe { glDrawArraysInstanced(GL_TRIANGLES, 0, 3, 5) };
        assert_eq!(gl.draws().len(), 5);
    }

    /// P2: drawn instance by instance, each draw carries its instance as
    /// GlesDraw::instance_id (gl_InstanceID's value), and a draw after the
    /// instanced one carries 0 again. Before the field every looped draw
    /// read gl_InstanceID as 0. Revert check: without the store in the loop
    /// every instance_id is 0.
    #[test]
    fn a_draw_per_instance_carries_its_instance_id() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        per_instance_colour(2);
        // SAFETY: plain GL calls.
        unsafe {
            glDrawArraysInstanced(GL_TRIANGLES, 0, 3, 5);
            glDrawArrays(GL_TRIANGLES, 0, 3);
        }
        let ids: Vec<u32> = gl.draws().iter().map(|d| d.draw.instance_id).collect();
        assert_eq!(ids, [0, 1, 2, 3, 4, 0]);
    }

    #[test]
    fn per_instance_attribute_is_read_from_a_vbo_with_conversion() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        force_per_draw_instances();
        buffer_with(GL_ARRAY_BUFFER, &[255, 0, 0, 255, 0, 51, 0, 255]);
        // SAFETY: plain GL calls; the pointer is a buffer offset.
        unsafe {
            glVertexAttribPointer(1, 4, GL_UNSIGNED_BYTE, 1, 0, core::ptr::null());
            glEnableVertexAttribArray(1);
            glVertexAttribDivisor(1, 1);
            glBindBuffer(GL_ARRAY_BUFFER, 0);
            glDrawArraysInstanced(GL_TRIANGLES, 0, 3, 2);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        let draws = gl.draws();
        assert_eq!(draws.len(), 2);
        assert_eq!(f32s(&draws[0].attrib_data[1]), [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(f32s(&draws[1].attrib_data[1]), [0.0, 0.2, 0.0, 1.0]);
    }

    #[test]
    fn per_instance_vbo_running_out_stops_with_invalid_operation() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        force_per_draw_instances();
        buffer_with(GL_ARRAY_BUFFER, &[255u8; 8]); /* two instances' worth */
        // SAFETY: plain GL calls; the pointer is a buffer offset.
        unsafe {
            glVertexAttribPointer(1, 4, GL_UNSIGNED_BYTE, 1, 0, core::ptr::null());
            glEnableVertexAttribArray(1);
            glVertexAttribDivisor(1, 1);
            glBindBuffer(GL_ARRAY_BUFFER, 0);
            glDrawArraysInstanced(GL_TRIANGLES, 0, 3, 4);
        }
        assert_eq!(gl.error(), GL_INVALID_OPERATION);
        assert_eq!(gl.draws().len(), 2, "the instances that had data were drawn");
        assert!(gl.ctx().attribs[1].enabled, "state is restored on the error path too");
    }

    #[test]
    fn instanced_draw_without_per_instance_attributes_is_one_gpu_instanced_draw() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        gl.logs();
        // SAFETY: plain GL calls.
        unsafe {
            glDrawArraysInstanced(GL_TRIANGLES, 0, 3, 7);
            glDrawArraysInstanced(GL_TRIANGLES, 0, 3, 7);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        let draws = gl.draws();
        assert_eq!(draws.len(), 2, "one draw per call");
        assert!(draws.iter().all(|d| d.draw.instance_count == 7), "the instance count reaches the backend");
        assert!(draws.iter().all(|d| d.draw.attribs[..2].iter().all(|a| a.per_instance == 0)));
        assert!(gl.logs().is_empty(), "nothing to warn about any more");
    }

    #[test]
    fn divisor_one_arrays_become_per_instance_streams_of_one_draw() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        per_instance_colour(1);
        // SAFETY: plain GL call.
        unsafe { glDrawArraysInstanced(GL_TRIANGLES, 0, 3, 3) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        let d = one(&gl);
        assert_eq!((d.draw.instance_count, d.draw.index_count), (3, 3));
        let (pos, col) = (d.draw.attribs[0], d.draw.attribs[1]);
        assert_eq!((pos.per_instance, col.per_instance), (0, 1));
        assert_eq!((col.stride, col.offset, col.cpu_bytes), (16, 0, 3 * 16), "one element per instance");
        assert_eq!(f32s(&d.attrib_data[1]), COLOURS, "every instance's element, converted from the client array");
        assert_eq!(f32s(&d.attrib_data[0]), TRI, "per-vertex arrays are unaffected");
        /* the next plain draw is one instance again */
        // SAFETY: plain GL call.
        unsafe { glDrawArrays(GL_TRIANGLES, 0, 3) };
        let d = one(&gl);
        assert_eq!(d.draw.instance_count, 1);
        assert_eq!(d.draw.attribs[1].per_instance, 1, "a divisor array stays per instance");
        assert_eq!(f32s(&d.attrib_data[1]), COLOURS[..4], "one instance: element 0");
    }

    #[test]
    fn divisor_one_instanced_elements_are_one_draw() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        per_instance_colour(1);
        let idx: [u16; 3] = [2, 1, 0];
        // SAFETY: `idx` outlives the call.
        unsafe { glDrawElementsInstanced(GL_TRIANGLES, 3, GL_UNSIGNED_SHORT, idx.as_ptr() as *const c_void, 2) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        let d = one(&gl);
        assert_eq!(d.draw.instance_count, 2);
        assert_eq!(u16s(&d.index_data), [2, 1, 0]);
        assert_eq!(f32s(&d.attrib_data[1]), COLOURS[..8]);
    }

    #[test]
    fn a_single_instance_is_a_plain_draw_even_with_other_divisors() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        per_instance_colour(3);
        // SAFETY: plain GL call.
        unsafe { glDrawArraysInstanced(GL_TRIANGLES, 1, 2, 1) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        let d = one(&gl);
        assert_eq!((d.draw.instance_count, d.draw.attribs[1].per_instance), (1, 1));
        assert_eq!(f32s(&d.attrib_data[1]), COLOURS[..4], "element 0, whatever the first vertex");
    }

    #[test]
    fn a_per_instance_float_vbo_is_fetched_in_place_from_its_first_element() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        let vb = buffer_with(GL_ARRAY_BUFFER, dword_bytes(&COLOURS));
        // SAFETY: plain GL calls; the pointer is a buffer offset.
        unsafe {
            glVertexAttribPointer(1, 4, GL_FLOAT, 0, 0, 4 as *const c_void);
            glEnableVertexAttribArray(1);
            glVertexAttribDivisor(1, 1);
            glBindBuffer(GL_ARRAY_BUFFER, 0);
            glDrawArraysInstanced(GL_TRIANGLES, 1, 2, 2);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        let d = one(&gl);
        let a = d.draw.attribs[1];
        assert_eq!((a.per_instance, a.stride, a.offset), (1, 16, 4), "not advanced by the first vertex");
        assert_eq!(a.buffer_gpu, gl.ctx().objs().buffers[vb as usize].mem.gpu);
        assert!(a.cpu.is_null() && d.attrib_data[1].is_empty(), "no staging for an in-place stream");
    }

    #[test]
    fn a_per_instance_vbo_too_short_for_the_instances_is_invalid_operation() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        buffer_with(GL_ARRAY_BUFFER, &[255u8; 8]); /* two instances' worth */
        // SAFETY: plain GL calls; the pointer is a buffer offset.
        unsafe {
            glVertexAttribPointer(1, 4, GL_UNSIGNED_BYTE, 1, 0, core::ptr::null());
            glEnableVertexAttribArray(1);
            glVertexAttribDivisor(1, 1);
            glBindBuffer(GL_ARRAY_BUFFER, 0);
            glDrawArraysInstanced(GL_TRIANGLES, 0, 3, 2);
            assert_eq!(gl.error(), GL_NO_ERROR);
            glDrawArraysInstanced(GL_TRIANGLES, 0, 3, 3);
        }
        assert_eq!(gl.error(), GL_INVALID_OPERATION);
        assert_eq!(gl.draws().len(), 1, "only the draw that fitted");
        /* the instance count does not leak into the next draw after the failure */
        // SAFETY: plain GL calls.
        unsafe {
            glDisableVertexAttribArray(1);
            glDrawArrays(GL_TRIANGLES, 0, 3);
        }
        assert_eq!(one(&gl).draw.instance_count, 1);
    }

    #[test]
    fn instance_count_zero_draws_nothing_and_negative_is_invalid_value() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        // SAFETY: plain GL calls.
        unsafe {
            glDrawArraysInstanced(GL_TRIANGLES, 0, 3, 0);
            assert_eq!(gl.error(), GL_NO_ERROR);
            glDrawArraysInstanced(GL_TRIANGLES, 0, 3, -1);
            assert_eq!(gl.error(), GL_INVALID_VALUE);
        }
        assert!(gl.draws().is_empty());
    }

    #[test]
    fn instanced_elements_draw_each_instance_with_the_same_indices() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        force_per_draw_instances();
        per_instance_colour(1);
        let idx: [u16; 3] = [2, 1, 0];
        // SAFETY: `idx` outlives the call.
        unsafe { glDrawElementsInstanced(GL_TRIANGLES, 3, GL_UNSIGNED_SHORT, idx.as_ptr() as *const c_void, 2) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        let draws = gl.draws();
        assert_eq!(draws.len(), 2);
        for (i, d) in draws.iter().enumerate() {
            assert_eq!(u16s(&d.index_data), [2, 1, 0]);
            assert_eq!(f32s(&d.attrib_data[1]), COLOURS[i * 4..i * 4 + 4]);
        }
    }

    #[test]
    fn instanced_elements_reject_a_bad_index_type() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        let idx: [u16; 3] = [2, 1, 0];
        // SAFETY: `idx` outlives the call.
        unsafe { glDrawElementsInstanced(GL_TRIANGLES, 3, GL_FLOAT, idx.as_ptr() as *const c_void, 2) };
        assert_eq!(gl.error(), GL_INVALID_ENUM);
        /* type 0 must not fall through to the non-indexed path */
        // SAFETY: as above.
        unsafe { glDrawElementsInstanced(GL_TRIANGLES, 3, 0, idx.as_ptr() as *const c_void, 2) };
        assert_eq!(gl.error(), GL_INVALID_ENUM);
        assert!(gl.draws().is_empty());
    }

    /* ---- error paths -------------------------------------------------------------------------------- */

    #[test]
    fn draw_without_a_program_is_invalid_operation() {
        let gl = Gl::new(16, 16);
        let idx: [u16; 3] = [0, 1, 2];
        // SAFETY: `idx` outlives the calls.
        unsafe {
            glDrawArrays(GL_TRIANGLES, 0, 3);
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
            glDrawElements(GL_TRIANGLES, 3, GL_UNSIGNED_SHORT, idx.as_ptr() as *const c_void);
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
        }
        assert!(gl.draws().is_empty());
    }

    #[test]
    fn draw_with_a_program_whose_relink_failed_is_invalid_operation() {
        let gl = Gl::new(16, 16);
        let vs = gl.shader(GL_VERTEX_SHADER, "passthrough.vert");
        let fs = gl.shader(GL_FRAGMENT_SHADER, "uniform_color.frag");
        // SAFETY: plain GL calls; TRI is a static.
        unsafe {
            let prog = glCreateProgram();
            glAttachShader(prog, vs);
            glAttachShader(prog, fs);
            glLinkProgram(prog);
            glUseProgram(prog);
            glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, TRI.as_ptr() as *const c_void);
            glEnableVertexAttribArray(0);
            glDetachShader(prog, fs);
            glLinkProgram(prog); /* fails: the current program is now unlinked */
            let mut ok: GLint = 1;
            glGetProgramiv(prog, GL_LINK_STATUS, &mut ok);
            assert_eq!(ok, 0);
            gl.error();
            glDrawArrays(GL_TRIANGLES, 0, 3);
        }
        assert_eq!(gl.error(), GL_INVALID_OPERATION);
        assert!(gl.draws().is_empty());
    }

    #[test]
    fn negative_count_or_first_is_invalid_value() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        let idx: [u16; 3] = [0, 1, 2];
        // SAFETY: `idx` outlives the calls.
        unsafe {
            glDrawArrays(GL_TRIANGLES, 0, -1);
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            glDrawArrays(GL_TRIANGLES, -1, 3);
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            glDrawElements(GL_TRIANGLES, -3, GL_UNSIGNED_SHORT, idx.as_ptr() as *const c_void);
            assert_eq!(gl.error(), GL_INVALID_VALUE);
        }
        assert!(gl.draws().is_empty());
    }

    #[test]
    fn bad_mode_is_invalid_enum() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        let idx: [u16; 3] = [0, 1, 2];
        // SAFETY: `idx` outlives the calls.
        unsafe {
            glDrawArrays(0x0007, 0, 3);
            assert_eq!(gl.error(), GL_INVALID_ENUM);
            glDrawElements(GL_TRIANGLES + 100, 3, GL_UNSIGNED_SHORT, idx.as_ptr() as *const c_void);
            assert_eq!(gl.error(), GL_INVALID_ENUM);
        }
        assert!(gl.draws().is_empty());
    }

    #[test]
    fn zero_count_draws_nothing_without_an_error() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        // SAFETY: plain GL calls.
        unsafe {
            glClear(GL_COLOR_BUFFER_BIT);
            glDrawArrays(GL_TRIANGLES, 0, 0);
            glDrawElements(GL_TRIANGLES, 0, GL_UNSIGNED_SHORT, core::ptr::null());
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert!(gl.draws().is_empty());
        assert!(draw_tri(&gl).draw.do_clear, "the pending clear waits for a real draw");
    }

    #[test]
    fn every_primitive_mode_reaches_the_backend() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        let table = [
            (GL_POINTS, GLES_PRIM_POINTS),
            (GL_LINES, GLES_PRIM_LINES),
            (GL_LINE_LOOP, GLES_PRIM_LINE_LOOP),
            (GL_LINE_STRIP, GLES_PRIM_LINE_STRIP),
            (GL_TRIANGLES, GLES_PRIM_TRIANGLES),
            (GL_TRIANGLE_STRIP, GLES_PRIM_TRI_STRIP),
            (GL_TRIANGLE_FAN, GLES_PRIM_TRI_FAN),
        ];
        for (mode, prim) in table {
            // SAFETY: plain GL call; TRI holds the 3 vertices.
            unsafe { glDrawArrays(mode, 0, 3) };
            let d = one(&gl).draw;
            assert_eq!(d.prim, prim);
            assert_eq!(d.index_count, 3);
            assert_eq!(d.index_buf_gpu, 0, "non-indexed: sequential");
            assert!(d.index_cpu.is_null());
        }
    }

    #[test]
    fn backend_draw_failure_is_out_of_memory_and_logged() {
        /* The backend reports a draw it could not record (arena overflow, a
         * count beyond its index path) with a negative result: the draw is
         * handed over once, the clear is consumed, and the loss is an error
         * with one log line per draw while the budget lasts. */
        let gl = Gl::new(16, 16);
        basic(&gl);
        gl.set_draw_result(-5);
        gl.logs();
        // SAFETY: plain GL calls.
        unsafe {
            glClear(GL_COLOR_BUFFER_BIT);
            glDrawArrays(GL_TRIANGLES, 0, 3);
        }
        assert_eq!(gl.error(), GL_OUT_OF_MEMORY);
        assert_eq!(gl.draws().len(), 1);
        assert!(!pending_clear().color);
        // SAFETY: plain GL calls; the index array outlives the call.
        unsafe {
            glDrawElements(GL_TRIANGLES, 3, GL_UNSIGNED_SHORT, [0u16, 1, 2].as_ptr() as *const c_void);
            assert_eq!(gl.error(), GL_OUT_OF_MEMORY);
            glClear(GL_COLOR_BUFFER_BIT);
            glFlush();
            assert_eq!(gl.error(), GL_OUT_OF_MEMORY, "a failed clear-only kick too");
        }
        let logs = gl.logs();
        assert_eq!(
            logs.iter().filter(|l| l.contains("was not recorded (-5); the frame is missing it")).count(),
            3,
            "{logs:?}"
        );
        gl.set_draw_result(0);
        gl.draws();
        draw_tri(&gl);
    }

    #[test]
    fn a_draw_the_backend_cannot_index_is_out_of_memory() {
        /* the real backend refuses more than 65535 vertices (-EINVAL):
         * the mock stands in with its result */
        let gl = Gl::new(16, 16);
        basic(&gl);
        gl.set_draw_result(-22);
        // SAFETY: plain GL call; TRI is only read for the vertices staged.
        unsafe { glDrawArraysInstanced(GL_TRIANGLES, 0, 3, 70000) };
        assert_eq!(gl.error(), GL_OUT_OF_MEMORY);
        gl.set_draw_result(0);
    }

    #[test]
    fn draw_without_a_current_context_is_ignored() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        let idx: [u16; 3] = [0, 1, 2];
        // SAFETY: the context is made current again before the guard needs it.
        unsafe {
            context::gl_context_set_current(core::ptr::null_mut());
            glDrawArrays(GL_TRIANGLES, 0, 3);
            glDrawArrays(GL_TRIANGLES, 0, -1);
            glDrawElements(GL_TRIANGLES, 3, GL_UNSIGNED_SHORT, idx.as_ptr() as *const c_void);
            glDrawArraysInstanced(GL_TRIANGLES, 0, 3, 2);
            gl_flush_pending_clear();
            gl_draw_invalidate_program(1);
            assert!(gl_draw_debug_usc(1, 0).is_null());
            context::gl_context_set_current(gl.ctx());
        }
        assert!(gl.draws().is_empty());
        assert_eq!(gl.error(), GL_NO_ERROR, "nothing was recorded in the (non-current) context");
    }

    /* ---- small helpers ------------------------------------------------------------------------------- */

    #[test]
    fn live_buffer_and_texture_reject_zero_unused_and_out_of_range_handles() {
        let gl = Gl::new(8, 8);
        let b = buffer_with(GL_ARRAY_BUFFER, &[0u8; 4]);
        let t = texture_rgba(1, 1);
        let c = gl.ctx();
        assert!(live_buffer(&mut c.objs().buffers, b).is_some());
        assert!(live_buffer(&mut c.objs().buffers, 0).is_none());
        assert!(live_buffer(&mut c.objs().buffers, b + 1).is_none(), "never generated");
        assert!(live_buffer(&mut c.objs().buffers, MAX_BUFFERS as GLuint).is_none());
        assert!(live_buffer(&mut c.objs().buffers, u32::MAX).is_none());
        assert!(live_texture(&mut c.objs().textures, t as usize).is_some());
        assert!(live_texture(&mut c.objs().textures, 0).is_none());
        assert!(live_texture(&mut c.objs().textures, t as usize + 1).is_none());
        assert!(live_texture(&mut c.objs().textures, MAX_TEXTURES).is_none(), "a default object never bound");
        assert!(live_texture(&mut c.objs().textures, TEXTURE_SLOTS).is_none());
    }

    #[test]
    fn scratch_reserve_reuses_a_large_enough_block_and_replaces_a_small_one() {
        let gl = Gl::new(8, 8);
        let mut m = OsalGpuMem::ZERO;
        assert_eq!(scratch_reserve(&mut m, 100), 0);
        let first = m.gpu;
        assert_eq!(m.size, 100);
        gl.mem_events();
        assert_eq!(scratch_reserve(&mut m, 100), 0);
        assert_eq!(scratch_reserve(&mut m, 1), 0);
        assert!(gl.mem_events().is_empty());
        assert_eq!(scratch_reserve(&mut m, 101), 0);
        let ev = gl.mem_events();
        assert_eq!(frees(&ev), [first]);
        assert_eq!(m.size, 101);
        gl.fail_alloc_in(1);
        assert_ne!(scratch_reserve(&mut m, 500), 0);
        assert!(m.impl_.is_null(), "a failed growth leaves no dangling descriptor");
        osal::gpu_free(&mut m);
    }

    #[test]
    fn put_copies_at_the_offset() {
        let gl = Gl::new(8, 8);
        let mut m = OsalGpuMem::ZERO;
        assert_eq!(scratch_reserve(&mut m, 8), 0);
        assert_eq!(put(&mut m, 0, &[1, 2, 3, 4, 5, 6, 7, 8]), 0);
        assert_eq!(put(&mut m, 4, &[9, 10]), 0);
        assert_eq!(gl.gpu_bytes(m.gpu, 8), [1, 2, 3, 4, 9, 10, 7, 8]);
        osal::gpu_free(&mut m);
    }

    #[test]
    fn put_goes_through_the_osal_when_the_block_has_no_cpu_mapping() {
        let gl = Gl::new(8, 8);
        let mut m = OsalGpuMem::ZERO;
        assert_eq!(scratch_reserve(&mut m, 8), 0);
        let mut unmapped = m;
        unmapped.cpu = core::ptr::null_mut();
        assert_eq!(put(&mut unmapped, 2, &[1, 2]), 0);
        assert_eq!(gl.gpu_bytes(m.gpu, 4), [FILL, FILL, 1, 2]);
        assert!(gl.cache_ops().is_empty(), "gpu_write needs no flush");
        /* a failing gpu_write: the error is passed on */
        gl.fail_gpu_write_in(1);
        assert_ne!(put(&mut unmapped, 0, &[3, 4]), 0);
        assert_eq!(gl.gpu_bytes(m.gpu, 2), [FILL, FILL]);
        osal::gpu_free(&mut m);
    }
    /* ---- cube / array samplers, float targets, id registers (upstream 7e76640) --------------------- */

    static QUAD2: [f32; 8] = [-1.0, -1.0, 1.0, -1.0, -1.0, 1.0, 1.0, 1.0];

    /// cube.vert (pos vec2 @0) + `fs`, current, fed from QUAD2.
    /// The table's cube.frag with its record's `native_cube` cleared: a
    /// program that samples the stacked image, as the online compiler's are
    /// with `pvr_native_cube` off. Since native cube step C the table's own
    /// row is native (the draw would bind the TPU copy), and these tests are
    /// about the stacked binding.
    fn stacked_cube_prog(gl: &Gl) -> GLuint {
        let p = es3_prog(gl, "cube.frag");
        gl.ctx().objs().programs[obj_index(p)].rfs.native_cube = 0;
        p
    }

    /// cube.vert + `fs`, current, fed from QUAD2. Both shaders are
    /// `#version 300 es`. A build that reports ES 2.0 and compiles online
    /// refuses them at line 1 ("GLSL ES 3.00 source; this build reports
    /// OpenGL ES 2.0", pinned by
    /// `shaderc::online::pp::tests::es300_is_preprocessed_unless_the_build_reports_es2`),
    /// so the tests that draw with them are ignored there, by name and
    /// reason. The offline table takes the text whatever the build reports,
    /// so a table build runs them.
    fn es3_prog(gl: &Gl, fs: &str) -> GLuint {
        let prog = gl.program("cube.vert", fs);
        // SAFETY: plain GL calls; QUAD2 is a static.
        unsafe {
            glUseProgram(prog);
            glVertexAttribPointer(0, 2, GL_FLOAT, 0, 0, QUAD2.as_ptr() as *const c_void);
            glEnableVertexAttribArray(0);
        }
        prog
    }

    fn bind_new_tex(target: GLenum) -> GLuint {
        let mut t: GLuint = 0;
        // SAFETY: valid out pointer, plain GL calls.
        unsafe {
            glGenTextures(1, &mut t);
            glBindTexture(target, t);
        }
        t
    }

    fn draw_strip(gl: &Gl) -> RecordedDraw {
        // SAFETY: the arrays the fixtures set up are statics.
        unsafe { glDrawArrays(GL_TRIANGLE_STRIP, 0, 4) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        one(gl)
    }

    /* ---- cache maintenance around a draw ------------------------------------------------------------ */

    /// Some flush covers the `len` bytes at device address `addr`.
    fn flushed(ops: &[CacheOp], addr: u64, len: usize) -> bool {
        ops.iter().any(|o| match *o {
            CacheOp::Flush { gpu, off, len: n } => {
                gpu + off as u64 <= addr && addr + len as u64 <= gpu + (off + n) as u64
            }
            CacheOp::Invd { .. } => false,
        })
    }

    #[test]
    fn everything_a_draw_uploads_is_flushed_before_the_backend_gets_it() {
        let gl = Gl::new(16, 16);
        let prog = basic(&gl);
        // SAFETY: plain GL calls.
        unsafe { glUniform4f(glGetUniformLocation(prog, c"u_color".as_ptr()), 0.5, 0.25, 1.0, 1.0) };
        gl.cache_ops();
        let d = draw_tri(&gl);
        let ops = gl.cache_ops();
        let dr = d.draw;
        // SAFETY: the program (and so its shaders) is alive.
        let (vs_dw, fs_dw) = unsafe { ((*dr.vs).usc_dw as usize, (*dr.fs).usc_dw as usize) };
        if VS_HAS_OWN_CODE {
            assert!(flushed(&ops, dr.vs_usc_gpu, vs_dw * 4), "the VS code: {ops:?}");
        } else {
            assert_eq!((dr.vs_usc_gpu, vs_dw), (0, 0), "a V3D VS has no code of its own");
        }
        assert!(flushed(&ops, dr.fs_usc_gpu, fs_dw * 4), "the FS code: {ops:?}");
        /* staged data: the backend copies it from the CPU side, and the core publishes it all the same */
        let (a0, a1) = (dr.attribs[0], dr.attribs[1]);
        assert!(flushed(&ops, a0.buffer_gpu, 48), "the converted client array: {ops:?}");
        assert!(flushed(&ops, a1.buffer_gpu, 16), "the disabled attribute's constant: {ops:?}");
        assert!(flushed(&ops, dr.fs_uniform_gpu, 16), "the FS uniforms: {ops:?}");
        assert!(!ops.iter().any(|o| matches!(o, CacheOp::Invd { .. })), "nothing is read back: {ops:?}");
    }

    #[test]
    fn staged_indices_are_flushed() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        gl.cache_ops();
        // SAFETY: the client index array outlives the call.
        unsafe { glDrawElements(GL_TRIANGLES, 3, GL_UNSIGNED_BYTE, [0u8, 1, 2].as_ptr() as *const c_void) };
        let d = one(&gl);
        let ops = gl.cache_ops();
        let staged = ops.iter().find_map(|o| match *o {
            CacheOp::Flush { gpu, off: 0, len: 6 } => Some(gpu),
            _ => None,
        });
        assert!(staged.is_some(), "{ops:?}");
        assert_eq!(u16s(&gl.gpu_bytes(staged.unwrap(), 6)), [0, 1, 2]);
        assert_eq!(u16s(&d.index_data), [0, 1, 2]);
    }

    #[test]
    fn a_widened_element_buffer_is_invalidated_before_it_is_read() {
        let gl = Gl::new(16, 16);
        basic_vbo(&gl);
        let ib = buffer_with(GL_ELEMENT_ARRAY_BUFFER, &[9, 2, 1, 0]);
        let gpu = gl.ctx().objs().buffers[ib as usize].mem.gpu;
        gl.cache_ops();
        // SAFETY: plain GL call; the pointer is a buffer offset.
        unsafe { glDrawElements(GL_TRIANGLES, 3, GL_UNSIGNED_BYTE, core::ptr::without_provenance(1)) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        let ops = gl.cache_ops();
        assert_eq!(ops[0], CacheOp::Invd { gpu, off: 1, len: 3 }, "{ops:?}");
        assert_eq!(u16s(&one(&gl).index_data), [2, 1, 0]);
    }

    #[test]
    fn an_element_buffer_used_in_place_is_not_read_back() {
        let gl = Gl::new(16, 16);
        basic_vbo(&gl);
        let ib = buffer_with(GL_ELEMENT_ARRAY_BUFFER, dword_bytes(&[0u16, 1, 2]));
        let gpu = gl.ctx().objs().buffers[ib as usize].mem.gpu;
        gl.cache_ops();
        // SAFETY: plain GL call.
        unsafe { glDrawElements(GL_TRIANGLES, 3, GL_UNSIGNED_SHORT, core::ptr::null()) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert!(!gl.cache_ops().iter().any(|o| matches!(*o, CacheOp::Invd { gpu: g, .. } if g == gpu)));
        assert_eq!(one(&gl).draw.index_buf_gpu, gpu);
    }

    #[test]
    fn a_converted_vbo_and_a_per_instance_vbo_are_invalidated_before_they_are_read() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        force_per_draw_instances();
        let vb = buffer_with(GL_ARRAY_BUFFER, &[255, 0, 0, 255, 0, 51, 0, 255]);
        let gpu = gl.ctx().objs().buffers[vb as usize].mem.gpu;
        gl.cache_ops();
        // SAFETY: plain GL calls; the pointer is a buffer offset.
        unsafe {
            glVertexAttribPointer(1, 4, GL_UNSIGNED_BYTE, 1, 0, core::ptr::null());
            glEnableVertexAttribArray(1);
            glVertexAttribDivisor(1, 1);
            glDrawArraysInstanced(GL_TRIANGLES, 0, 3, 2);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        let ops = gl.cache_ops();
        assert!(ops.contains(&CacheOp::Invd { gpu, off: 0, len: 4 }), "{ops:?}");
        assert!(ops.contains(&CacheOp::Invd { gpu, off: 4, len: 4 }), "{ops:?}");
        gl.draws();
        // SAFETY: plain GL calls.
        unsafe {
            glVertexAttribDivisor(1, 0);
            glDrawArrays(GL_TRIANGLES, 0, 2); /* two vertices of normalised bytes, converted */
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        /* (the per-vertex conversion reads the CPU view without an invalidate, unlike the
         * two paths above; the GPU never writes a buffer, so that view is current) */
        assert_eq!(f32s(&one(&gl).attrib_data[1]), [1.0, 0.0, 0.0, 1.0, 0.0, 0.2, 0.0, 1.0]);
    }

    #[test]
    fn a_fresh_uniform_block_reaches_the_gpu_for_every_draw() {
        let gl = Gl::new(16, 16);
        let prog = basic(&gl);
        // SAFETY: a NUL-terminated name.
        let loc = unsafe { glGetUniformLocation(prog, c"u_color".as_ptr()) };
        for v in [0.25f32, 0.75] {
            // SAFETY: plain GL call.
            unsafe { glUniform4f(loc, v, v, v, v) };
            let d = draw_tri(&gl);
            assert_eq!(f32s(&gl.gpu_bytes(d.draw.fs_uniform_gpu, 16)), [v; 4]);
        }
    }

    /* ---- no CPU view of GPU memory ----------------------------------------------------------------------- */

    #[test]
    fn an_element_buffer_without_a_cpu_view_is_read_through_gpu_read() {
        let gl = Gl::new(16, 16);
        basic_vbo(&gl);
        gl.set_cpu_mapping(false);
        buffer_with(GL_ELEMENT_ARRAY_BUFFER, &[2, 1, 0, 1, 0]);
        gl.set_cpu_mapping(true); /* the staging is mapped */
        // SAFETY: plain GL calls; the pointer is a buffer offset.
        unsafe { glDrawElements(GL_TRIANGLES, 4, GL_UNSIGNED_BYTE, core::ptr::null()) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(u16s(&one(&gl).index_data), [2, 1, 0, 1]);
        // SAFETY: as above.
        unsafe { glDrawElements(GL_TRIANGLES, 3, GL_UNSIGNED_BYTE, core::ptr::without_provenance(2)) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(u16s(&one(&gl).index_data), [0, 1, 0], "read from the offset");
        assert!(gl.cache_ops().iter().all(|o| !matches!(o, CacheOp::Invd { .. })));
    }

    #[test]
    fn a_failing_read_of_an_element_buffer_is_out_of_memory() {
        let gl = Gl::new(16, 16);
        basic_vbo(&gl);
        gl.set_cpu_mapping(false);
        buffer_with(GL_ELEMENT_ARRAY_BUFFER, &[2, 1, 0]);
        gl.set_cpu_mapping(true);
        gl.fail_gpu_read_in(1);
        // SAFETY: plain GL call.
        unsafe { glDrawElements(GL_TRIANGLES, 3, GL_UNSIGNED_BYTE, core::ptr::null()) };
        assert_eq!(gl.error(), GL_OUT_OF_MEMORY);
        assert!(gl.draws().is_empty());
    }

    #[test]
    fn staged_indices_need_no_cpu_view() {
        let gl = Gl::new(16, 16);
        gl.set_cpu_mapping(false);
        basic_vbo(&gl);
        buffer_with(GL_ELEMENT_ARRAY_BUFFER, &[2, 1, 0]);
        // SAFETY: plain GL call.
        unsafe { glDrawElements(GL_TRIANGLES, 3, GL_UNSIGNED_BYTE, core::ptr::null()) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        let d = one(&gl).draw;
        assert_eq!(u16s(&gl.gpu_bytes(d.index_buf_gpu, 6)), [2, 1, 0]);
    }

    #[test]
    fn a_vbo_without_a_cpu_view_is_converted_through_gpu_read() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        gl.set_cpu_mapping(false);
        buffer_with(GL_ARRAY_BUFFER, &[0, 0, 0, 0, 255, 0, 0, 255, 0, 51, 0, 255]);
        gl.set_cpu_mapping(true);
        // SAFETY: plain GL calls; the pointer is a buffer offset.
        unsafe {
            glVertexAttribPointer(1, 4, GL_UNSIGNED_BYTE, 1, 0, 4 as *const c_void);
            glEnableVertexAttribArray(1);
            glDrawArrays(GL_TRIANGLES, 0, 2);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(f32s(&one(&gl).attrib_data[1]), [1.0, 0.0, 0.0, 1.0, 0.0, 0.2, 0.0, 1.0]);
        /* a failing read: the draw is refused */
        gl.fail_gpu_read_in(2);
        // SAFETY: plain GL call.
        unsafe { glDrawArrays(GL_TRIANGLES, 0, 2) };
        assert_eq!(gl.error(), GL_OUT_OF_MEMORY);
        assert!(gl.draws().is_empty());
    }

    #[test]
    fn a_per_instance_vbo_without_a_cpu_view_is_read_through_gpu_read() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        force_per_draw_instances();
        gl.set_cpu_mapping(false);
        buffer_with(GL_ARRAY_BUFFER, &[255, 0, 0, 255, 0, 51, 0, 255]);
        gl.set_cpu_mapping(true);
        // SAFETY: plain GL calls; the pointer is a buffer offset.
        unsafe {
            glVertexAttribPointer(1, 4, GL_UNSIGNED_BYTE, 1, 0, core::ptr::null());
            glEnableVertexAttribArray(1);
            glVertexAttribDivisor(1, 1);
            glDrawArraysInstanced(GL_TRIANGLES, 0, 3, 2);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        let draws = gl.draws();
        assert_eq!(f32s(&draws[0].attrib_data[1]), [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(f32s(&draws[1].attrib_data[1]), [0.0, 0.2, 0.0, 1.0]);
        gl.fail_gpu_read_in(2);
        // SAFETY: plain GL call.
        unsafe { glDrawArraysInstanced(GL_TRIANGLES, 0, 3, 2) };
        assert_eq!(gl.error(), GL_INVALID_OPERATION);
        assert_eq!(gl.draws().len(), 1, "the first instance was drawn");
        assert!(gl.ctx().attribs[1].enabled, "state is restored");
    }

    #[test]
    fn staging_without_a_cpu_view_is_written_through_gpu_write() {
        let gl = Gl::new(16, 16);
        gl.set_cpu_mapping(false);
        let prog = basic(&gl);
        // SAFETY: plain GL calls.
        unsafe {
            glUniform4f(glGetUniformLocation(prog, c"u_color".as_ptr()), 0.5, 0.25, 1.0, 1.0);
            glVertexAttrib4f(1, 0.1, 0.2, 0.3, 0.4);
        }
        let d = draw_tri(&gl);
        let (a0, a1) = (d.draw.attribs[0], d.draw.attribs[1]);
        assert!(a0.cpu.is_null() && a0.cpu_bytes == 0, "no CPU copy for the backend");
        assert_eq!(f32s(&gl.gpu_bytes(a0.buffer_gpu, 48)), TRI);
        assert_eq!(f32s(&gl.gpu_bytes(a1.buffer_gpu, 16)), [0.1, 0.2, 0.3, 0.4]);
        /* the uniform snapshot comes from the program's own arrays, which
         * exist whether or not the GPU buffer has a CPU view (upstream
         * 98f7331) */
        assert!(!d.draw.fs_uniform_cpu.is_null(), "the fragment block comes from the program's array");
        assert!(d.draw.vs_uniform_cpu.is_null(), "this vertex shader has no uniforms");
        assert_eq!(f32s(&gl.gpu_bytes(d.draw.fs_uniform_gpu, 16)), [0.5, 0.25, 1.0, 1.0]);
        // SAFETY: the program is alive.
        let vs_dw = unsafe { (*d.draw.vs).usc_dw } as usize;
        // SAFETY: as above.
        let vs_code = unsafe { dword_bytes((*d.draw.vs).usc_words()).to_vec() };
        if VS_HAS_OWN_CODE {
            assert_eq!(gl.gpu_bytes(d.draw.vs_usc_gpu, vs_dw * 4), vs_code);
        } else {
            assert_eq!((d.draw.vs_usc_gpu, vs_dw), (0, 0), "a V3D VS has no code of its own");
        }
        /* the code this build does upload: the FS record's (all three programs on the V3D) */
        // SAFETY: as above.
        let (fs_dw, fs_code) =
            unsafe { ((*d.draw.fs).usc_dw as usize, dword_bytes((*d.draw.fs).usc_words()).to_vec()) };
        assert!(fs_dw > 0);
        assert_eq!(gl.gpu_bytes(d.draw.fs_usc_gpu, fs_dw * 4), fs_code, "written through gpu_write");
        assert!(gl.cache_ops().is_empty());
        // SAFETY: plain GL call.
        unsafe { glFinish() }; /* the kick read what the draw staged */
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn a_failing_write_of_a_converted_array_is_out_of_memory() {
        let gl = Gl::new(16, 16);
        gl.set_cpu_mapping(false);
        basic(&gl);
        draw_tri(&gl);
        /* the FS uniform block is written first, then the rows of a_pos */
        gl.fail_gpu_write_in(2);
        // SAFETY: plain GL call.
        unsafe { glDrawArrays(GL_TRIANGLES, 0, 3) };
        assert_eq!(gl.error(), GL_OUT_OF_MEMORY);
        assert!(gl.draws().is_empty());
    }

    #[test]
    fn a_failing_staging_write_is_out_of_memory() {
        let gl = Gl::new(16, 16);
        gl.set_cpu_mapping(false);
        basic(&gl);
        draw_tri(&gl);
        gl.set_gpu_io_fails(true);
        // SAFETY: plain GL call.
        unsafe { glDrawArrays(GL_TRIANGLES, 0, 3) };
        assert_eq!(gl.error(), GL_OUT_OF_MEMORY);
        assert!(gl.draws().is_empty());
    }

    #[test]
    fn staging_without_a_cpu_view_is_not_overwritten_by_the_next_draw_of_the_frame() {
        let gl = Gl::new(16, 16);
        gl.set_cpu_mapping(false);
        let prog = basic(&gl);
        // SAFETY: a NUL-terminated name.
        let loc = unsafe { glGetUniformLocation(prog, c"u_color".as_ptr()) };
        let other: [f32; 12] = [0.0; 12];
        // SAFETY: plain GL calls; the client arrays outlive the draws.
        unsafe {
            glUniform4f(loc, 1.0, 0.0, 0.0, 1.0);
            glDrawArrays(GL_TRIANGLES, 0, 3);
            glUniform4f(loc, 0.0, 1.0, 0.0, 1.0);
            glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, other.as_ptr() as *const c_void);
            glDrawArrays(GL_TRIANGLES, 0, 3);
            glFinish();
        }
        /* the mock reports "... changed between the draw and kick 1" */
        assert_eq!(gl.draws().len(), 2);
    }

    #[test]
    fn a_failing_uniform_upload_is_out_of_memory() {
        let gl = Gl::new(16, 16);
        gl.set_cpu_mapping(false);
        let prog = basic_vbo(&gl);
        // SAFETY: a NUL-terminated name.
        let loc = unsafe { glGetUniformLocation(prog, c"u_color".as_ptr()) };
        let vbo = gl.ctx().attribs[0].buffer;
        // SAFETY: plain GL calls; both VS inputs read the VBO in place.
        unsafe {
            glBindBuffer(GL_ARRAY_BUFFER, vbo);
            glVertexAttribPointer(1, 4, GL_FLOAT, 0, 0, core::ptr::null());
            glEnableVertexAttribArray(1);
            glUniform4f(loc, 1.0, 0.0, 0.0, 1.0);
            glDrawArrays(GL_TRIANGLES, 0, 3);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        gl.draws();
        gl.set_gpu_io_fails(true);
        // SAFETY: plain GL calls.
        unsafe {
            glUniform4f(loc, 0.0, 1.0, 0.0, 1.0);
            glDrawArrays(GL_TRIANGLES, 0, 3);
        }
        assert_eq!(gl.error(), GL_OUT_OF_MEMORY);
    }

    /* ---- application offsets and sizes (fix round 2) ------------------------------------------------ */

    #[test]
    fn a_negative_element_buffer_offset_is_invalid_operation() {
        let gl = Gl::new(16, 16);
        basic_vbo(&gl);
        buffer_with(GL_ELEMENT_ARRAY_BUFFER, dword_bytes(&[0u16, 1, 2]));
        for off in [usize::MAX - 1, usize::MAX, usize::MAX - 5] {
            for (t, n) in [(GL_UNSIGNED_SHORT, 3), (GL_UNSIGNED_BYTE, 3), (GL_UNSIGNED_INT, 1)] {
                // SAFETY: plain GL call; the pointer is a (hostile) buffer offset.
                unsafe { glDrawElements(GL_TRIANGLES, n, t, core::ptr::without_provenance(off)) };
                assert_eq!(gl.error(), GL_INVALID_OPERATION, "offset 0x{off:x} type 0x{t:x}");
            }
        }
        assert!(gl.draws().is_empty());
    }

    #[test]
    fn an_in_place_vbo_range_past_the_buffer_is_invalid_operation() {
        let gl = Gl::new(16, 16);
        basic_vbo(&gl); /* 3 float vertices */
        let vbo = gl.ctx().attribs[0].buffer;
        // SAFETY: plain GL calls; the pointers are (hostile) buffer offsets.
        unsafe {
            glDrawArrays(GL_TRIANGLES, 1, 3);
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "one vertex too many");
            glDrawArrays(GL_TRIANGLES, i32::MAX - 1, 2);
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "first far past the buffer");
            glBindBuffer(GL_ARRAY_BUFFER, vbo);
            for off in [usize::MAX - 3, 1usize << 40, 33] {
                glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, core::ptr::without_provenance(off));
                glDrawArrays(GL_TRIANGLES, 0, 1);
                assert_eq!(gl.error(), GL_INVALID_OPERATION, "offset 0x{off:x}");
            }
            glVertexAttribPointer(0, 4, GL_FLOAT, 0, i32::MAX, core::ptr::null());
            glDrawArrays(GL_TRIANGLES, 0, 3);
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "a huge stride");
        }
        assert!(gl.draws().is_empty());
        // SAFETY: plain GL calls.
        unsafe {
            glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, 4 as *const c_void);
            glDrawArrays(GL_TRIANGLES, 1, 1); /* bytes 20..36 of 48: fits */
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(one(&gl).draw.attribs[0].offset, 20);
    }

    #[test]
    fn a_converted_vbo_offset_that_would_wrap_is_invalid_operation() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        buffer_with(GL_ARRAY_BUFFER, &[0u8; 64]);
        for (off, stride) in [(usize::MAX - 2, 0), (usize::MAX / 2, 0), (8, i32::MAX), (usize::MAX - 30, 16)] {
            // SAFETY: plain GL calls; the pointer is a (hostile) buffer offset.
            unsafe {
                glVertexAttribPointer(1, 4, GL_UNSIGNED_BYTE, 1, stride, core::ptr::without_provenance(off));
                glEnableVertexAttribArray(1);
                glDrawArrays(GL_TRIANGLES, 0, 3);
            }
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "offset 0x{off:x} stride {stride}");
        }
        assert!(gl.draws().is_empty());
    }

    #[test]
    fn a_per_instance_offset_that_would_wrap_is_invalid_operation() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        force_per_draw_instances();
        buffer_with(GL_ARRAY_BUFFER, &[0u8; 16]);
        // SAFETY: plain GL calls; the pointer is a (hostile) buffer offset.
        unsafe {
            glVertexAttribPointer(1, 4, GL_UNSIGNED_BYTE, 1, 0, core::ptr::without_provenance(usize::MAX - 1));
            glEnableVertexAttribArray(1);
            glVertexAttribDivisor(1, 1);
            glDrawArraysInstanced(GL_TRIANGLES, 0, 3, 2);
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
            glVertexAttribPointer(1, 4, GL_UNSIGNED_BYTE, 1, i32::MAX, 8 as *const c_void);
            glDrawArraysInstanced(GL_TRIANGLES, 0, 3, 2);
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "the second instance is far past the buffer");
        }
        assert_eq!(gl.draws().len(), 1, "the first instance was drawn");
    }

    #[test]
    fn a_per_instance_array_without_a_pointer_reads_0_0_0_1() {
        /* hostile.instanced_attrib_without_pointer: enabled with a divisor,
         * never given glVertexAttribPointer (size 0, no buffer, address 0) */
        let gl = Gl::new(16, 16);
        basic(&gl);
        force_per_draw_instances();
        // SAFETY: plain GL calls.
        unsafe {
            glEnableVertexAttribArray(1);
            glVertexAttribDivisor(1, 1);
            glDrawArraysInstanced(GL_TRIANGLES, 0, 3, 2);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        let draws = gl.draws();
        assert_eq!(draws.len(), 2);
        for d in &draws {
            assert_eq!(f32s(&d.attrib_data[1]), [0.0, 0.0, 0.0, 1.0]);
        }
        /* the GPU instancing path: the same array as a per-instance stream */
        // SAFETY: plain GL calls.
        unsafe {
            glDisableVertexAttribArray(7);
            glDrawArraysInstanced(GL_TRIANGLES, 0, 3, 2);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(f32s(&one(&gl).attrib_data[1]), [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]);
        /* a client array at address 0 with a real size is refused, not read */
        // SAFETY: plain GL calls.
        unsafe {
            glVertexAttribPointer(1, 4, GL_FLOAT, 0, 0, core::ptr::null());
            glEnableVertexAttribArray(7);
            glDrawArraysInstanced(GL_TRIANGLES, 0, 3, 2);
        }
        assert_eq!(gl.error(), GL_INVALID_OPERATION);
        assert!(gl.draws().is_empty());
    }

    #[test]
    fn two_inputs_bound_to_one_attribute_index_get_their_own_staging() {
        /* Aliasing: two inputs on one attribute index, which the linker
         * accepts (ES 2.0 2.10.4) and dEQP's `bind_aliasing.cond_*` needs.
         * The table is built by hand here so the case is pinned whatever the
         * linker does. */
        let gl = Gl::new(16, 16);
        let prog = basic(&gl);
        {
            let p = &mut gl.ctx().objs().programs[obj_index(prog)];
            let (l0, l1) = (p.rvs.attribs[0].location as usize, p.rvs.attribs[1].location as usize);
            p.attrib_app_index[l0] = 5;
            p.attrib_app_index[l1] = 5;
            p.rvs.attribs[1].components = 2; /* the inputs read index 5 with different widths */
        }
        // SAFETY: plain GL calls; TRI is a static.
        unsafe {
            glVertexAttribPointer(5, 4, GL_FLOAT, 0, 0, TRI.as_ptr() as *const c_void);
            glEnableVertexAttribArray(5);
        }
        let d = draw_tri(&gl);
        assert_eq!(d.draw.num_attribs, 2);
        let (a0, a1) = (d.draw.attribs[0], d.draw.attribs[1]);
        assert_ne!(a0.buffer_gpu, a1.buffer_gpu, "one buffer each");
        let rows = |n: usize| -> Vec<f32> { TRI.chunks(4).flat_map(|v| v[..n].to_vec()).collect() };
        let w0 = a0.components as usize;
        let w1 = a1.components as usize;
        assert_eq!(f32s(&gl.gpu_bytes(a0.buffer_gpu, 12 * w0)), rows(w0));
        assert_eq!(f32s(&gl.gpu_bytes(a1.buffer_gpu, 12 * w1)), rows(w1));
        assert_eq!(f32s(&d.attrib_data[0]), rows(w0));
        assert_eq!(f32s(&d.attrib_data[1]), rows(w1));
    }

    #[test]
    #[cfg(feature = "gpu-powervr")] /* the PowerVR table's own values */
    fn a_blend_variant_that_failed_to_upload_is_retried() {
        let gl = Gl::new(16, 16);
        let prog = basic(&gl);
        draw_tri(&gl);
        // SAFETY: plain GL calls.
        unsafe {
            glEnable(GL_BLEND);
            glBlendFunc(GL_ONE, GL_ONE);
        }
        gl.logs();
        gl.fail_alloc_in(1);
        assert_eq!(refused_tri(&gl), 1, "this draw is refused");
        gl.fail_alloc_in(1);
        assert_eq!(refused_tri(&gl), 1);
        let logs = gl.logs();
        assert_eq!(logs.iter().filter(|l| l.contains("out of memory for the blend key")).count(), 1, "{logs:?}");
        let d = draw_tri(&gl).draw;
        assert!(
            !core::ptr::eq(d.fs, &gl.ctx().objs_ref().programs[obj_index(prog)].rfs),
            "memory is back: blended again"
        );
        // SAFETY: the variant lives in the draw path's static cache.
        assert_ne!(unsafe { (*d.fs).blend_key }, 0);
    }

    #[test]
    fn a_clear_pending_for_a_framebuffer_is_emitted_before_another_is_cleared_or_drawn() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        let (fbo, tex) = fbo_8x8();
        /* the clear below is a depth one, so the framebuffer needs a depth
         * buffer: without one GL clears nothing (ES 2.0 4.2.3) */
        let depth = renderbuffer_8x8(GL_DEPTH_COMPONENT16);
        // SAFETY: plain GL call.
        unsafe { glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_RENDERBUFFER, depth) };
        let surface = gl.ctx().default_surface.gpu;
        // SAFETY: plain GL calls.
        unsafe {
            glClear(GL_DEPTH_BUFFER_BIT); /* the FBO's: a depth-only clear waits for a draw */
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glClear(GL_COLOR_BUFFER_BIT); /* the window's */
        }
        let draws = gl.draws();
        assert_eq!(draws.len(), 1, "the FBO's clear went out on its own");
        assert_eq!(draws[0].draw.target_gpu, gl.ctx().objs().textures[tex as usize].mem.gpu);
        assert!(draws[0].draw.do_clear_depth && !draws[0].draw.do_clear);
        // SAFETY: plain GL calls.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, fbo);
            glBindFramebuffer(GL_FRAMEBUFFER, 0); /* back where the clear was made */
        }
        let d = draw_tri(&gl).draw;
        assert!(d.do_clear && !d.do_clear_depth && d.target_gpu == surface);
        /* a clear of a framebuffer deleted before anything used it goes with it */
        // SAFETY: plain GL calls.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, fbo);
            glClear(GL_COLOR_BUFFER_BIT);
            glDeleteFramebuffers(1, &fbo);
            glFlush();
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert!(gl.draws().is_empty());
        assert!(!pending_clear().color);
        /* a clear of an incomplete framebuffer is still reported when the
         * application moves on (as it was when the clear was fused into the
         * next draw) */
        let mut empty: GLuint = 0;
        // SAFETY: valid out pointer, plain GL calls.
        unsafe {
            glGenFramebuffers(1, &mut empty);
            glBindFramebuffer(GL_FRAMEBUFFER, empty); /* no attachments */
            glClear(GL_COLOR_BUFFER_BIT);
            assert_eq!(gl.error(), GL_NO_ERROR, "the clear is only recorded");
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glClear(GL_COLOR_BUFFER_BIT);
        }
        assert_eq!(gl.error(), GL_INVALID_FRAMEBUFFER_OPERATION);
        assert!(gl.draws().is_empty());
        assert!(pending_clear().color, "the window's clear is pending");
    }

    #[test]
    fn attrib_component_converts_the_es3_types() {
        assert_eq!(comp(&0x3c00u16.to_ne_bytes(), GL_HALF_FLOAT, false), 1.0);
        assert_eq!(comp(&0xc000u16.to_ne_bytes(), GL_HALF_FLOAT, true), -2.0, "never normalised");
        assert_eq!(comp(&0x3555u16.to_ne_bytes(), GL_HALF_FLOAT, false), 0.333_251_95);
        assert_eq!(comp(&0x0001u16.to_ne_bytes(), GL_HALF_FLOAT, false), 2.0f32.powi(-24), "subnormal");
        assert_eq!(comp(&0x03ffu16.to_ne_bytes(), GL_HALF_FLOAT, false), 1023.0 * 2.0f32.powi(-24));
        assert_eq!(comp(&0x7c00u16.to_ne_bytes(), GL_HALF_FLOAT, false), f32::INFINITY);
        assert!(comp(&0x7e00u16.to_ne_bytes(), GL_HALF_FLOAT, false).is_nan());
        assert_eq!(comp(&0x8000u16.to_ne_bytes(), GL_HALF_FLOAT, false).to_bits(), 0x8000_0000);
        assert_eq!(attrib_type_size(GL_HALF_FLOAT), 2);
    }

    #[test]
    fn packed_2_10_10_10_arrays_unpack_per_field() {
        let word = |x: u32, y: u32, z: u32, w: u32| (x | (y << 10) | (z << 20) | (w << 30)).to_ne_bytes();
        let row = |b: [u8; 4], t: GLenum, norm: bool| -> Vec<f32> {
            let mut ga = attrib(t, 4, norm, false);
            ga.size = 4;
            attrib_row(&b, &ga, 4, attrib_type_size(t), 4).iter().map(|v| f32::from_bits(*v)).collect()
        };
        assert_eq!(row(word(1023, 0, 511, 3), GL_UNSIGNED_INT_2_10_10_10_REV, false), [1023.0, 0.0, 511.0, 3.0]);
        assert_eq!(row(word(1023, 0, 1023, 3), GL_UNSIGNED_INT_2_10_10_10_REV, true), [1.0, 0.0, 1.0, 1.0]);
        /* signed: 0x200 is -512, 0x3ff is -1, w 2 is -2 */
        assert_eq!(row(word(0x200, 0x3ff, 511, 2), GL_INT_2_10_10_10_REV, false), [-512.0, -1.0, 511.0, -2.0]);
        assert_eq!(row(word(0x200, 0x3ff, 511, 1), GL_INT_2_10_10_10_REV, true), [-1.0, -1.0 / 511.0, 1.0, 1.0]);
        assert_eq!(row(word(0, 0, 0, 2), GL_INT_2_10_10_10_REV, true)[3], -1.0);
        let ga = attrib(GL_INT_2_10_10_10_REV, 4, false, false);
        assert_eq!((attrib_elem_bytes(&ga), attrib_stride(&ga)), (4, 4), "one word per element");
        /* an input narrower than 4 reads the first fields */
        let mut ga = attrib(GL_UNSIGNED_INT_2_10_10_10_REV, 4, false, false);
        ga.size = 4;
        assert_eq!(attrib_row(&word(5, 6, 7, 1), &ga, 4, 4, 2)[..2], [5.0f32.to_bits(), 6.0f32.to_bits()]);
    }

    #[cfg(not(feature = "report-es2"))] // ES 3.0 array types; report-es2 refuses them
    #[test]
    fn half_float_and_packed_arrays_reach_the_shader_as_floats() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        let half: [u16; 12] = [0x3c00, 0, 0, 0x3c00, 0, 0x3800, 0, 0x3c00, 0xbc00, 0, 0, 0x3c00];
        let packed: [u32; 3] = [1023 | (3 << 30), 1023 << 10 | (3 << 30), 1023 << 20];
        // SAFETY: plain GL calls; the arrays outlive the draws.
        unsafe {
            glVertexAttribPointer(1, 4, GL_HALF_FLOAT, 0, 0, half.as_ptr() as *const c_void);
            glEnableVertexAttribArray(1);
        }
        let d = draw_tri(&gl);
        assert_eq!(f32s(&d.attrib_data[1]), [1.0, 0.0, 0.0, 1.0, 0.0, 0.5, 0.0, 1.0, -1.0, 0.0, 0.0, 1.0]);
        // SAFETY: as above.
        unsafe { glVertexAttribPointer(1, 4, GL_UNSIGNED_INT_2_10_10_10_REV, 1, 0, packed.as_ptr() as *const c_void) };
        let d = draw_tri(&gl);
        assert_eq!(f32s(&d.attrib_data[1]), [1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0]);
    }

    #[cfg(not(feature = "report-es2"))] // GL_INT is an ES 3.0 array type; report-es2 refuses it
    #[test]
    fn int_arrays_of_glvertexattribpointer_reach_the_shader_as_floats() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        let ints: [i32; 12] = [1, -2, 3, 4, i32::MAX, 0, 0, 1, -5, 0, 0, 1];
        // SAFETY: plain GL calls; the array outlives the draw.
        unsafe {
            glVertexAttribPointer(1, 4, GL_INT, 0, 0, ints.as_ptr() as *const c_void);
            glEnableVertexAttribArray(1);
        }
        let d = draw_tri(&gl);
        assert_eq!(f32s(&d.attrib_data[1]), [1.0, -2.0, 3.0, 4.0, 2147483647.0, 0.0, 0.0, 1.0, -5.0, 0.0, 0.0, 1.0]);
    }

    /* ---- attribute locations at the edge of the table ---------------------------------------------- */

    /// basic(), with a_pos (VS input 0) moved to GLSL `location` and read from generic attribute 0.
    fn a_pos_at_location(gl: &Gl, location: u8) -> RecordedDraw {
        let prog = basic(gl);
        {
            let p = &mut gl.ctx().objs().programs[obj_index(prog)];
            let i = (0..p.rvs.num_attribs as usize).find(|&i| p.rvs.attribs[i].location == 0).unwrap();
            p.rvs.attribs[i].location = location;
            if location < 16 {
                p.attrib_app_index[location as usize] = 0;
            }
        }
        // SAFETY: TRI is a static.
        unsafe { glDrawArrays(GL_TRIANGLES, 0, 3) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        one(gl)
    }

    /// Native cube step C: the table's cube program is pco's native cube
    /// (glsl2usc hands pco a real samplerCube; emit.rs sets the flag), and
    /// the draw binds the TPU's twiddled copy for it by default, with no
    /// switch involved -- on the PowerVR only.
    #[test]
    #[cfg(feature = "gpu-powervr")]
    fn the_tables_cube_program_is_native_and_gets_the_copy() {
        let gl = Gl::new(16, 16);
        gl.allow_uninit_reads(true);
        let p = es3_prog(&gl, "cube.frag");
        assert_eq!(gl.ctx().objs().programs[obj_index(p)].rfs.native_cube, 1, "the table row says native");
        let cube = bind_new_tex(GL_TEXTURE_CUBE_MAP);
        // SAFETY: plain GL call.
        unsafe { glTexStorage2D(GL_TEXTURE_CUBE_MAP, 1, GL_RGBA8, 4, 4) };
        let _ = cube;
        let d = draw_strip(&gl).draw;
        assert_eq!(d.textures[0].type_, GLES_TEXTYPE_CUBE_TWIDDLED_PROBE, "the TPU's own cube");
    }

    #[test]
    #[cfg_attr(
        all(feature = "no-shader-table", feature = "report-es2"),
        ignore = "GLSL ES 3.00 test shader (cube.vert) in a build that reports ES 2.0"
    )]
    fn a_sampler_cube_reads_the_units_cube_map_as_one_tall_texture() {
        let gl = Gl::new(16, 16);
        /* glTexStorage leaves the texels undefined: the test checks the binding only */
        gl.allow_uninit_reads(true);
        stacked_cube_prog(&gl);
        /* a 2D texture on the same unit is not what the samplerCube reads */
        texture_rgba(8, 8);
        let cube = bind_new_tex(GL_TEXTURE_CUBE_MAP);
        // SAFETY: plain GL call.
        unsafe { glTexStorage2D(GL_TEXTURE_CUBE_MAP, 1, GL_RGBA8, 3, 3) };
        let d = draw_strip(&gl).draw;
        assert_eq!(d.num_textures, 1);
        let t = &d.textures[0];
        let tex = &gl.ctx().objs_ref().textures[cube as usize];
        assert_eq!((t.base_gpu, t.width, t.height, t.stride_texels), (tex.mem.gpu, 3, 18, 4));
        assert_eq!(t.bytes, 4 * 18 * 4, "the stacked rows at the pair-padded pitch");
        assert_eq!(t.format, GLES_TEXFMT_RGBA8);
    }

    #[test]
    #[cfg(feature = "gpu-powervr")] /* the PowerVR's native cube */
    fn a_native_cube_is_copied_again_after_a_render_into_it_without_a_rebind() {
        /* gl::native_cube: the copy's staleness is asked at every draw. A
         * cube bound once, rendered into through a framebuffer and sampled
         * by the next draw -- no rebind, no CPU write -- is copied again. */
        let gl = Gl::new(16, 16);
        gl.allow_uninit_reads(true);
        let cube_prog = es3_prog(&gl, "cube.frag");
        let cube = bind_new_tex(GL_TEXTURE_CUBE_MAP);
        // SAFETY: plain GL call.
        unsafe { glTexStorage2D(GL_TEXTURE_CUBE_MAP, 1, GL_RGBA8, 4, 4) };
        /* the program says it samples the TPU's own cube (what the online
         * compiler records with pvr_native_cube on) */
        gl.ctx().objs().programs[obj_index(cube_prog)].rfs.native_cube = 1;
        let copies = super::super::native_cube::copies;
        let n0 = copies();
        let d = draw_strip(&gl).draw;
        assert_eq!(d.textures[0].type_, GLES_TEXTYPE_CUBE_TWIDDLED_PROBE, "the TPU's own cube");
        assert_eq!(copies() - n0, 1, "copied once");
        draw_strip(&gl);
        assert_eq!(copies() - n0, 1, "the same texels: kept");
        /* render into its +X face; the cube stays bound on its unit */
        let mut fbo: GLuint = 0;
        // SAFETY: plain GL calls.
        unsafe {
            glGenFramebuffers(1, &mut fbo);
            glBindFramebuffer(GL_FRAMEBUFFER, fbo);
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_CUBE_MAP_POSITIVE_X, cube, 0);
            assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
        }
        basic(&gl);
        draw_tri(&gl);
        // SAFETY: plain GL calls.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glFlush();
            glUseProgram(cube_prog);
        }
        gl.draws();
        let d = draw_strip(&gl).draw;
        assert_eq!(d.textures[0].type_, GLES_TEXTYPE_CUBE_TWIDDLED_PROBE);
        assert_eq!(copies() - n0, 2, "the render made the copy stale: copied again");
        gl.ctx().objs().programs[obj_index(cube_prog)].rfs.native_cube = 0;
        let d = draw_strip(&gl).draw;
        assert_eq!(d.textures[0].type_, GLES_TEXTYPE_CUBE, "a stacked-cube program: the stacked image");
    }

    #[test]
    #[cfg_attr(
        all(feature = "no-shader-table", feature = "report-es2"),
        ignore = "GLSL ES 3.00 test shader (cube.vert) in a build that reports ES 2.0"
    )]
    fn a_backend_that_reads_layers_itself_is_given_one_face_and_the_count() {
        /* upstream 976c632: the GC7000's texture unit has a cube map type and
         * a slice stride, so the binding names one face and how many there
         * are; the tall stack above is what a unit without them samples. */
        let gl = Gl::new(16, 16);
        gl.allow_uninit_reads(true);
        gl.backend_like_gc7000();
        stacked_cube_prog(&gl);
        let cube = bind_new_tex(GL_TEXTURE_CUBE_MAP);
        // SAFETY: plain GL call.
        unsafe { glTexStorage2D(GL_TEXTURE_CUBE_MAP, 1, GL_RGBA8, 3, 3) };
        let d = draw_strip(&gl).draw;
        assert_eq!(d.num_textures, 1);
        let t = &d.textures[0];
        assert_eq!((t.type_, t.layers), (GLES_TEXTYPE_CUBE, 6));
        assert_eq!((t.width, t.height), (3, 3), "one face, not the stack");
        assert_eq!(t.stride_texels, 4);
        assert_eq!(t.bytes, 4 * 3 * 6 * 4, "the six faces are still in there");
        assert_eq!(gl.ctx().objs().textures[cube as usize].mem.gpu, t.base_gpu);
    }

    /// A sampler object bound to the unit replaces the texture's filters and
    /// wraps at the draw, completeness included, and the texture's own come
    /// back when it is unbound (ES 3.0 3.8.2). The draw ignored samplers
    /// until 2026-09-24.
    #[test]
    fn a_bound_sampler_object_overrides_the_textures_filters_and_wraps() {
        let gl = Gl::new(16, 16);
        textured(&gl);
        texture_rgba(4, 4); /* min filter NEAREST_MIPMAP_LINEAR, one level: incomplete */
        assert_eq!(draw_tri(&gl).draw.textures[0].incomplete, 1);
        let mut s: GLuint = 0;
        // SAFETY: plain GL calls; `s` is one name.
        unsafe {
            super::super::es3::glGenSamplers(1, &mut s);
            super::super::es3::glSamplerParameteri(s, GL_TEXTURE_MIN_FILTER, GL_LINEAR as GLint);
            super::super::es3::glSamplerParameteri(s, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
            super::super::es3::glSamplerParameteri(s, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE as GLint);
            super::super::es3::glBindSampler(0, s);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        let t = draw_tri(&gl).draw.textures[0];
        assert_eq!(t.incomplete, 0, "the sampler's LINEAR needs no chain");
        assert_eq!((t.min_linear, t.linear_filter, t.mip_filter), (1, 0, 0));
        assert_eq!((t.wrap_s, t.wrap_t), (GLES_WRAP_CLAMP_TO_EDGE, GLES_WRAP_REPEAT));
        // SAFETY: plain GL call.
        unsafe { super::super::es3::glBindSampler(0, 0) };
        let t = draw_tri(&gl).draw.textures[0];
        assert_eq!((t.incomplete, t.wrap_s), (1, GLES_WRAP_REPEAT), "the texture's own again");
    }

    /// The depth compare a draw hands the backend (`compare_op`): the
    /// texture's GL_TEXTURE_COMPARE_FUNC less GL_NEVER under
    /// GL_COMPARE_REF_TO_TEXTURE, 16 (no compare: the depth itself) under
    /// GL_NONE, and a bound sampler object's pair in place of the texture's
    /// (ES 3.0.6 3.8.2).
    /// Revert check: without the fill every draw carries 0 (NEVER).
    /// The draw needs the offline table's `uv.vert` (the known gap of a
    /// no-table build); what it checks does not depend on the GPU.
    #[cfg(not(feature = "no-shader-table"))]
    #[test]
    fn the_depth_compare_comes_from_the_texture_or_the_bound_sampler() {
        let gl = Gl::new(16, 16);
        textured(&gl);
        texture_rgba(4, 4);
        let op = |gl: &Gl| draw_tri(gl).draw.textures[0].compare_op;
        assert_eq!(op(&gl), 16, "GL_NONE by default: no compare");
        // SAFETY: plain GL calls on the bound texture.
        unsafe {
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_COMPARE_MODE, GL_COMPARE_REF_TO_TEXTURE as GLint);
        }
        assert_eq!(op(&gl), 3, "the default function, GL_LEQUAL");
        // SAFETY: as above.
        unsafe { glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_COMPARE_FUNC, GL_GREATER as GLint) };
        assert_eq!(op(&gl), 4);
        let mut s: GLuint = 0;
        // SAFETY: plain GL calls; `s` is one name.
        unsafe {
            super::super::es3::glGenSamplers(1, &mut s);
            super::super::es3::glSamplerParameteri(s, GL_TEXTURE_COMPARE_MODE, GL_COMPARE_REF_TO_TEXTURE as GLint);
            super::super::es3::glSamplerParameteri(s, GL_TEXTURE_COMPARE_FUNC, GL_NOTEQUAL as GLint);
            super::super::es3::glBindSampler(0, s);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(op(&gl), 5, "the sampler's GL_NOTEQUAL, not the texture's GL_GREATER");
        // SAFETY: plain GL call.
        unsafe { super::super::es3::glBindSampler(0, 0) };
        assert_eq!(op(&gl), 4, "the texture's own again");
        /* a fixed-point depth texture sets bit 3 (its reference is
         * clamped); a float one does not */
        for (ifmt, fmt, ty, want) in [
            (GL_DEPTH_COMPONENT16, GL_DEPTH_COMPONENT, GL_UNSIGNED_SHORT, 4 | 8),
            (GL_DEPTH24_STENCIL8, GL_DEPTH_STENCIL, GL_UNSIGNED_INT_24_8, 4 | 8),
            (GL_DEPTH_COMPONENT32F, GL_DEPTH_COMPONENT, GL_FLOAT, 4),
        ] {
            if crate::REPORT_ES2 && ifmt == GL_DEPTH_COMPONENT32F {
                continue; /* ES 3.0's: no float depth texture on the ES 2.0 surface */
            }
            let px = [0u8; 4 * 4 * 8];
            // SAFETY: `px` covers a 4x4 image of up to 8 bytes a texel.
            unsafe {
                glTexImage2D(GL_TEXTURE_2D, 0, ifmt as GLint, 4, 4, 0, fmt, ty, px.as_ptr() as *const c_void);
                glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
            }
            assert_eq!(gl.error(), GL_NO_ERROR, "0x{ifmt:x}");
            assert_eq!(op(&gl), want, "0x{ifmt:x}");
        }
    }

    /// dEQP-GLES2 texture.completeness.cube.extra_level binds its texture to
    /// GL_TEXTURE_2D and then specifies six cube faces: they land in the
    /// default cube map (binding 0), which the samplerCube then reads and
    /// must find complete. Binding 0 sampled nothing before 2026-09-24.
    #[test]
    #[cfg_attr(
        all(feature = "no-shader-table", feature = "report-es2"),
        ignore = "GLSL ES 3.00 test shader (cube.vert) in a build that reports ES 2.0"
    )]
    fn a_sampler_reads_the_default_texture_object_at_binding_0() {
        let gl = Gl::new(16, 16);
        stacked_cube_prog(&gl);
        bind_new_tex(GL_TEXTURE_2D);
        let px = [7u8; 2 * 2 * 4];
        for level in 0..2 {
            for f in 0..6 {
                let side = 2 >> level;
                // SAFETY: `px` holds the 2x2 RGBA image, more than the 1x1 reads.
                unsafe {
                    glTexImage2D(
                        GL_TEXTURE_CUBE_MAP_POSITIVE_X + f,
                        level,
                        GL_RGBA as GLint,
                        side,
                        side,
                        0,
                        GL_RGBA,
                        GL_UNSIGNED_BYTE,
                        px.as_ptr() as *const c_void,
                    )
                };
            }
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        let d = draw_strip(&gl).draw;
        assert_eq!(d.num_textures, 1, "the default cube map is bound");
        let t = d.textures[0];
        let default = &gl.ctx().objs().textures[crate::gl::texture::default_slot(GL_TEXTURE_CUBE_MAP).unwrap()];
        assert_eq!((t.base_gpu, t.type_, t.incomplete), (default.mem.gpu, GLES_TEXTYPE_CUBE, 0));
    }

    #[test]
    #[cfg_attr(
        all(feature = "no-shader-table", feature = "report-es2"),
        ignore = "GLSL ES 3.00 test shader (cube.vert) in a build that reports ES 2.0"
    )]
    fn an_etc1_cube_binding_names_level_0_of_all_six_faces() {
        /* the GC7000 descriptor steps from level 0 to level 1 by `bytes`, and
         * a compressed cube's level 0 is six block grids back to back */
        let gl = Gl::new(16, 16);
        gl.backend_like_gc7000();
        stacked_cube_prog(&gl);
        let cube = bind_new_tex(GL_TEXTURE_CUBE_MAP);
        let blocks = [7u8; 4 * 8];
        for f in 0..6 {
            // SAFETY: `blocks` holds the 8x8 face's four ETC1 blocks.
            unsafe {
                glCompressedTexImage2D(
                    GL_TEXTURE_CUBE_MAP_POSITIVE_X + f,
                    0,
                    GL_ETC1_RGB8_OES,
                    8,
                    8,
                    0,
                    32,
                    blocks.as_ptr() as *const c_void,
                )
            };
        }
        // SAFETY: plain GL call.
        unsafe { glTexParameteri(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_MIN_FILTER, GL_LINEAR as GLint) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        let t = draw_strip(&gl).draw.textures[0];
        assert_eq!((t.type_, t.layers, t.width, t.height), (GLES_TEXTYPE_CUBE, 6, 8, 8));
        assert_eq!((t.format, t.incomplete), (GLES_TEXFMT_ETC2_RGB, 0));
        assert_eq!(t.bytes, 6 * 4 * 8);
        assert_eq!(gl.ctx().objs().textures[cube as usize].mem.gpu, t.base_gpu);
    }

    #[test]
    #[cfg_attr(
        all(feature = "no-shader-table", feature = "report-es2"),
        ignore = "GLSL ES 3.00 test shader (cube.vert) in a build that reports ES 2.0"
    )]
    fn the_sampler_type_names_the_binding_even_for_a_plain_backend() {
        /* the type travels whatever the backend does with the layers: the
         * PowerVR table ignores it, the GC7000 table picks the descriptor. */
        let gl = Gl::new(16, 16);
        gl.allow_uninit_reads(true);
        stacked_cube_prog(&gl);
        bind_new_tex(GL_TEXTURE_CUBE_MAP);
        // SAFETY: plain GL call.
        unsafe { glTexStorage2D(GL_TEXTURE_CUBE_MAP, 1, GL_RGBA8, 3, 3) };
        let t = draw_strip(&gl).draw.textures[0];
        assert_eq!((t.type_, t.layers), (GLES_TEXTYPE_CUBE, 6));
        assert_eq!((t.width, t.height), (3, 18), "the stack, as before");
    }

    #[test]
    #[cfg_attr(
        all(feature = "no-shader-table", feature = "report-es2"),
        ignore = "GLSL ES 3.00 test shader (cube.vert) in a build that reports ES 2.0"
    )]
    fn a_sampled_cube_map_is_dumped_face_by_face_and_level_by_level() {
        /* upstream 7d1e16c: a light probe that renders flat is either flat in
         * memory, prefiltered into fewer levels than the sampler is allowed
         * past, or stacked in the wrong order, and the binding line cannot
         * tell those apart. */
        let gl = Gl::new(16, 16);
        es3_prog(&gl, "cube.frag");
        let cube = bind_new_tex(GL_TEXTURE_CUBE_MAP);
        // SAFETY: plain GL call.
        unsafe { glTexStorage2D(GL_TEXTURE_CUBE_MAP, 2, GL_RGBA16F, 2, 2) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        /* face f holds the half value f + 1 in R, G and B */
        for f in 0..6u32 {
            let texel: [u16; 4] = [float_to_half(f as f32 + 1.0); 4];
            let px: Vec<u8> = (0..4).flat_map(|_| texel.iter().flat_map(|h| h.to_le_bytes())).collect();
            // SAFETY: four RGBA16F texels.
            unsafe {
                glTexSubImage2D(
                    GL_TEXTURE_CUBE_MAP_POSITIVE_X + f,
                    0,
                    0,
                    0,
                    2,
                    2,
                    GL_RGBA,
                    GL_HALF_FLOAT,
                    px.as_ptr() as *const c_void,
                )
            };
        }
        // SAFETY: plain GL call.
        unsafe { glGenerateMipmap(GL_TEXTURE_CUBE_MAP) };
        gl.logs();
        draw_strip(&gl);
        let logs = gl.logs();
        let gpu = gl.ctx().objs().textures[cube as usize].mem.gpu;
        assert!(logs.iter().any(|l| l.contains(&std::format!("cube-bind: 0x{gpu:x} unit 0"))), "{logs:?}");
        /* one mean per face, per level: each face keeps its own value */
        let level0 = logs.iter().find(|l| l.contains("cube(draw) level 0")).expect("a level 0 line");
        assert!(level0.contains("(2x2 @0x0): 1.0000 2.0000 3.0000 4.0000 5.0000 6.0000"), "{level0}");
        let level1 = logs.iter().find(|l| l.contains("cube(draw) level 1")).expect("a level 1 line");
        assert!(level1.ends_with(" 1.0000 2.0000 3.0000 4.0000 5.0000 6.0000"), "{level1}");

        let mut measurements = 1; /* the level lines above are one look */

        /* the same atlas is not looked at again until four kicks later, and
         * the binding line is printed every time */
        draw_strip(&gl);
        let logs = gl.logs();
        assert_eq!(logs.iter().filter(|l| l.contains("cube-bind")).count(), 1);
        assert_eq!(logs.iter().filter(|l| l.contains("cube(draw) level 0")).count(), 0, "one look per four kicks");

        /* The swap looks again, from a point where the GPU is provably idle
         * (eglSwapBuffers calls this after the frame's kicks have
         * completed). The same four-kick spacing applies, so the atlas is
         * measured again only once the frame has moved on. */
        super::debug_dump_cubes();
        let logs = gl.logs();
        assert!(logs.iter().any(|l| l.contains("cube-bind") && l.contains("(swap)")), "{logs:?}");
        assert!(!logs.iter().any(|l| l.contains("cube(swap) level")), "too soon: {logs:?}");
        /* five looks at one atlas, and no more, however many draws and swaps
         * sample it (the swap adds a budget of three frames on top) */
        for _ in 0..40 {
            // SAFETY: plain GL call.
            unsafe { glFlush() };
            draw_strip(&gl);
            super::debug_dump_cubes();
            measurements += gl.logs().iter().filter(|l| l.contains("level 0 (2x2")).count();
        }
        assert_eq!(measurements, 5, "the budget of one atlas");
    }

    #[test]
    #[cfg_attr(
        all(feature = "no-shader-table", feature = "report-es2"),
        ignore = "GLSL ES 3.00 test shader (cube.vert) in a build that reports ES 2.0"
    )]
    fn a_sampler_cube_without_a_cube_map_binds_nothing() {
        let gl = Gl::new(16, 16);
        es3_prog(&gl, "cube.frag");
        texture_rgba(8, 8);
        assert_eq!(draw_strip(&gl).draw.num_textures, 0);
    }

    #[test]
    #[cfg(feature = "gpu-powervr")] /* the PowerVR table's own values */
    fn an_array_sampler_gets_its_layer_count_in_the_same_draw() {
        let gl = Gl::new(16, 16);
        /* glTexStorage leaves the texels undefined: the test checks the binding only */
        gl.allow_uninit_reads(true);
        es3_prog(&gl, "array.frag");
        /* no array bound: the hidden uniform reads 1 (no division by zero) */
        let d = draw_strip(&gl);
        assert_eq!(d.draw.num_textures, 0);
        assert_eq!(d.fs_uniforms[0], 1.0f32.to_bits(), "pvr_layers_layers");

        let arr = bind_new_tex(GL_TEXTURE_2D_ARRAY);
        // SAFETY: plain GL call.
        unsafe { glTexStorage3D(GL_TEXTURE_2D_ARRAY, 1, GL_RGBA8, 2, 4, 5) };
        let d = draw_strip(&gl);
        assert_eq!(d.fs_uniforms[0], 5.0f32.to_bits(), "the first draw that binds the array already carries it");
        let t = &d.draw.textures[0];
        assert_eq!(
            (t.base_gpu, t.width, t.height, t.bytes),
            (gl.ctx().objs().textures[arr as usize].mem.gpu, 2, 20, 2 * 20 * 4)
        );
        /* the array on another unit: the sampler's unit has none */
        let prog = gl.ctx().cur_program;
        // SAFETY: plain GL calls.
        unsafe {
            let loc = glGetUniformLocation(prog, c"layers".as_ptr());
            glUniform1i(loc, 2);
            assert_eq!(gl.error(), GL_NO_ERROR, "glUniform1i sets a sampler2DArray");
        }
        let d = draw_strip(&gl);
        assert_eq!((d.draw.num_textures, d.fs_uniforms[0]), (0, 1.0f32.to_bits()));
    }

    #[test]
    #[cfg(feature = "gpu-powervr")] /* the PowerVR table's own values */
    fn a_vertex_stage_array_sampler_gets_its_layer_count_too() {
        /* Qt Quick 3D's morph targets are a sampler2DArray read in the
         * VERTEX shader (texelFetch indexed by gl_VertexID): its block needs
         * the hidden layer count as much as the fragment stage's, or the
         * shader divides the stacked texture's height by zero. The table has
         * no such vertex shader, so array.frag's reflection stands in for
         * one (as vol_prog does below). */
        let gl = Gl::new(16, 16);
        gl.allow_uninit_reads(true); /* glTexStorage leaves the texels undefined */
        let prog = es3_prog(&gl, "array.frag");
        {
            let p = &mut gl.ctx().objs().programs[obj_index(prog)];
            let table: &'static [ShcUniform] = Vec::leak(p.rfs.uniform_table().to_vec());

            p.rvs.uniforms = table.as_ptr();
            p.rvs.num_uniforms = table.len() as u8;
            p.rvs.uniform_dwords = p.rvs.uniform_dwords.max(1);
            p.vs_sampler_unit = p.fs_sampler_unit;
            if p.vs_uniform_cap == 0 {
                // SAFETY: the program owns its blocks (C heap); the old one
                // is freed and replaced by one the same allocator made.
                unsafe {
                    util::free(p.vs_uniforms as *mut c_void);
                    p.vs_uniforms = util::calloc(4, 4) as *mut f32;
                }
                p.vs_uniform_cap = 4;
            }
        }
        bind_new_tex(GL_TEXTURE_2D_ARRAY);
        // SAFETY: plain GL call.
        unsafe { glTexStorage3D(GL_TEXTURE_2D_ARRAY, 1, GL_RGBA8, 2, 4, 5) };
        let d = draw_strip(&gl);
        assert_eq!(d.vs_uniforms[0], 5.0f32.to_bits(), "the vertex block carries pvr_layers_layers");
        assert_eq!(d.fs_uniforms[0], 5.0f32.to_bits(), "and so does the fragment block");
    }

    /// array.frag with its sampler reported as a sampler3D: what the translator makes of vol.frag (a sampler2D
    /// plus the hidden pvr_layers_ count), which upstream's committed table does not carry.
    #[cfg(feature = "gpu-powervr")] /* only the PowerVR table has these shaders */
    fn vol_prog(gl: &Gl) -> GLuint {
        let prog = es3_prog(gl, "array.frag");
        let p = &mut gl.ctx().objs().programs[obj_index(prog)];
        let mut table: Vec<ShcUniform> = p.rfs.uniform_table().to_vec();
        let s = table.iter_mut().find(|u| u.dword_off == SHC_UNIFORM_NO_BLOCK).expect("the sampler");
        assert_eq!(s.gl_type, GL_SAMPLER_2D_ARRAY_T);
        s.gl_type = GL_SAMPLER_3D_T;
        let table: &'static [ShcUniform] = Vec::leak(table);
        p.rfs.uniforms = table.as_ptr();
        prog
    }

    #[test]
    #[cfg(feature = "gpu-powervr")] /* the PowerVR table's own values */
    fn a_sampler3d_reads_the_3d_binding_as_its_grid() {
        let gl = Gl::new(16, 16);
        gl.allow_uninit_reads(true); /* glTexStorage leaves the texels undefined */
        vol_prog(&gl);
        let d = draw_strip(&gl);
        assert_eq!((d.draw.num_textures, d.fs_uniforms[0]), (0, 1.0f32.to_bits()), "nothing bound: one slice");
        /* a 2D array on the unit is not what a sampler3D reads */
        bind_new_tex(GL_TEXTURE_2D_ARRAY);
        // SAFETY: plain GL call.
        unsafe { glTexStorage3D(GL_TEXTURE_2D_ARRAY, 1, GL_RGBA8, 2, 2, 7) };
        let d = draw_strip(&gl);
        assert_eq!((d.draw.num_textures, d.fs_uniforms[0]), (0, 1.0f32.to_bits()));

        let vol = bind_new_tex(GL_TEXTURE_3D);
        // SAFETY: plain GL call.
        unsafe { glTexStorage3D(GL_TEXTURE_3D, 1, GL_RGBA8, 3, 2, 5) };
        let d = draw_strip(&gl);
        assert_eq!(d.fs_uniforms[0], 5.0f32.to_bits(), "the slice count, in the same draw");
        assert_eq!(d.draw.num_textures, 1);
        let t = &d.draw.textures[0];
        /* 3 columns x 2 rows of 3x2 slices, sampled as one 9x4 texture at the pair-padded pitch */
        assert_eq!(
            (t.base_gpu, t.width, t.height, t.stride_texels, t.bytes, t.mip_levels),
            (gl.ctx().objs().textures[vol as usize].mem.gpu, 9, 4, 10, 10 * 4 * 4, 1)
        );
    }

    #[test]
    #[cfg(feature = "gpu-powervr")] /* the PowerVR table's own values */
    fn a_3d_texture_is_not_sampled_by_a_2d_array_sampler() {
        let gl = Gl::new(16, 16);
        gl.allow_uninit_reads(true);
        es3_prog(&gl, "array.frag");
        bind_new_tex(GL_TEXTURE_3D);
        // SAFETY: plain GL call.
        unsafe { glTexStorage3D(GL_TEXTURE_3D, 1, GL_RGBA8, 2, 2, 4) };
        let d = draw_strip(&gl);
        assert_eq!((d.draw.num_textures, d.fs_uniforms[0]), (0, 1.0f32.to_bits()));
    }

    /// `gl_DepthRange`'s rows get glDepthRangef's near, far and far - near,
    /// wherever they sit, and nothing else in the block is written.
    ///
    /// Revert check: make `fill_stage_depth_range` return at once and the
    /// three dwords stay at their sentinel.
    #[test]
    fn the_depth_range_rows_are_filled_from_gl_depth_range() {
        let row = |name: &str, off: u16| ShcUniform {
            name: crate::util::cname(name),
            dword_off: off,
            dwords: 1,
            gl_type: 0x1406,
            ..ShcUniform::ZERO
        };
        let rows =
            [row("k", 0), row("gl_DepthRange.far", 2), row("gl_DepthRange.near", 1), row("gl_DepthRange.diff", 3)];
        let sh = ShcShader { uniforms: rows.as_ptr(), num_uniforms: rows.len() as u8, ..ShcShader::ZERO };
        let mut block = [-9.0f32; 5];
        fill_stage_depth_range(&sh, &mut block, [0.25, 0.75, 0.5]);
        assert_eq!(block, [-9.0, 0.25, 0.75, 0.5, -9.0]);
    }

    #[test]
    #[cfg(feature = "gpu-powervr")] /* the PowerVR table's own values */
    fn layer_uniform_name_is_matched_on_the_raw_bytes() {
        let gl = Gl::new(16, 16);
        let prog = es3_prog(&gl, "array.frag");
        let arr = bind_new_tex(GL_TEXTURE_2D_ARRAY);
        let p = &mut gl.ctx().objs().programs[obj_index(prog)];
        p.blocks_mut().1[0] = 0.0;
        let mut bound = [0; crate::gl::context::CAP_COMBINED_TEXTURE_UNITS];
        bound[0] = arr;
        let c = gl.ctx();
        c.objs().textures[arr as usize].layers = 7;
        let textures = unsafe { &*(&c.objs_ref().textures as *const [GlTexture; TEXTURE_SLOTS]) };
        fill_sampler_layer_uniforms(&mut c.objs().programs[obj_index(prog)], textures, &bound, &[0; 8]);
        assert_eq!(gl.ctx().objs().programs[obj_index(prog)].fs_block()[0], 7.0);
    }

    #[test]
    #[cfg(feature = "gpu-powervr")] /* the PowerVR table's own values */
    fn a_float_render_target_selects_the_variant_packed_for_its_class() {
        let gl = Gl::new(16, 16);
        let prog = es3_prog(&gl, "cube.frag");
        let plain = draw_strip(&gl).draw;
        assert!(core::ptr::eq(plain.fs, &gl.ctx().objs_ref().programs[obj_index(prog)].rfs));
        for (internal, cls) in [(GL_RGBA16F, 1u32), (GL_R16F, 2), (GL_R32F, 3)] {
            let tex = bind_new_tex(GL_TEXTURE_2D);
            let mut fbo: GLuint = 0;
            // SAFETY: plain GL calls.
            unsafe {
                glTexStorage2D(GL_TEXTURE_2D, 1, internal, 8, 8);
                glGenFramebuffers(1, &mut fbo);
                glBindFramebuffer(GL_FRAMEBUFFER, fbo);
                glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
            }
            assert_eq!(current_blend_key(gl.ctx()), SHC_BLEND_KEY_REPLACE | shc_blend_key_rt(cls));
            let d = draw_strip(&gl).draw;
            // SAFETY: the variant lives in the draw path's static cache.
            let fs = unsafe { &*d.fs };
            assert_eq!(fs.blend_key >> SHC_BLEND_KEY_RT_SHIFT, cls, "0x{internal:x}");
            assert_eq!(d.target_format, [0, GLES_SURF_RGBA16F, GLES_SURF_R16F, GLES_SURF_R32F][cls as usize]);
            assert_eq!(d.target_gpu, gl.ctx().objs().textures[tex as usize].mem.gpu);
        }
        /* back on the RGBA8 window: the base shader again */
        // SAFETY: plain GL call.
        unsafe { glBindFramebuffer(GL_FRAMEBUFFER, 0) };
        assert_eq!(current_blend_key(gl.ctx()), 0);
    }

    #[test]
    fn a_draw_into_an_array_layer_targets_the_layer() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        let arr = bind_new_tex(GL_TEXTURE_2D_ARRAY);
        let mut fbo: GLuint = 0;
        // SAFETY: plain GL calls.
        unsafe {
            glTexStorage3D(GL_TEXTURE_2D_ARRAY, 1, GL_RGBA8, 4, 4, 3);
            glGenFramebuffers(1, &mut fbo);
            glBindFramebuffer(GL_FRAMEBUFFER, fbo);
            glFramebufferTextureLayer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, arr, 0, 1);
        }
        let d = draw_tri(&gl).draw;
        assert_eq!(d.target_gpu, gl.ctx().objs().textures[arr as usize].mem.gpu + 4 * 4 * 4);
        assert_eq!((d.target_w, d.target_h, d.flip_y), (4, 4, false));
    }

    #[test]
    #[cfg(feature = "gpu-powervr")] /* the PowerVR table's own values */
    fn a_vertex_shader_reading_gl_vertexid_carries_its_register() {
        let gl = Gl::new(16, 16);
        let prog = gl.program("vid.vert", "vcolor.frag");
        // SAFETY: plain GL calls; TRI is a static.
        unsafe {
            glUseProgram(prog);
            glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, TRI.as_ptr() as *const c_void);
            glEnableVertexAttribArray(0);
        }
        let d = draw_tri(&gl).draw;
        // SAFETY: the program's resolved VS lives as long as the program.
        let vs = unsafe { &*d.vs };
        /* vi0..3 hold the position; the compiler reads the vertex id from vi4 */
        assert_eq!((vs.vs_vertex_id_reg1, vs.vs_instance_id_reg1), (5, 0));
        assert_eq!(d.attribs[0].vi_reg, 0);
    }

    #[test]
    fn the_last_attribute_location_is_fed() {
        let gl = Gl::new(16, 16);
        let d = a_pos_at_location(&gl, 15);
        assert_eq!(d.draw.num_attribs, 2);
        let i = (0..2).find(|&i| d.draw.attribs[i].vi_reg == 0).expect("a_pos is fed");
        assert_eq!(f32s(&d.attrib_data[i]), TRI);
    }

    #[test]
    fn a_location_beyond_the_table_is_not_fed() {
        let gl = Gl::new(16, 16);
        let d = a_pos_at_location(&gl, 16);
        assert_eq!(d.draw.num_attribs, 1, "only a_color");
        assert_ne!(d.draw.attribs[0].vi_reg, 0);
    }

    /* ---- the sync to upstream e6c449c ------------------------------------ */

    #[test]
    fn a_draw_into_three_colour_attachments_carries_them_all() {
        /* upstream e471a73: the fragment shader's outputs po0..poN go to the
         * attachments in order. */
        let gl = Gl::new(8, 8);
        let (_, t0) = fbo_8x8();
        let (t1, t2) = (texture_rgba(8, 8), texture_rgba(8, 8));
        // SAFETY: plain GL calls.
        unsafe {
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT1, GL_TEXTURE_2D, t1, 0);
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT2, GL_TEXTURE_2D, t2, 0);
        }
        basic(&gl);
        let d = draw_tri(&gl).draw;
        assert_eq!(d.num_targets, 3);
        assert_eq!(d.targets[0].gpu, d.target_gpu);
        let gpu = |t: GLuint| gl.ctx().objs().textures[t as usize].mem.gpu;
        assert_eq!((d.targets[0].gpu, d.targets[1].gpu, d.targets[2].gpu), (gpu(t0), gpu(t1), gpu(t2)));
        assert!(d.targets[..3].iter().all(|t| (t.w, t.h, t.format) == (8, 8, GLES_SURF_RGBA8888)));
    }

    /// glDrawBuffers(ATT0, NONE, ATT2): slot 2 draws into attachment 2 (ES
    /// 3.0 4.2.1), so the draw carries three slots, slot 1 with no target
    /// (gpu 0, which a backend skips). Before, the walk stopped at the NONE
    /// slot and attachment 2 reached no backend.
    #[test]
    fn a_none_draw_buffer_keeps_its_slot_and_the_next_one_is_carried() {
        let gl = Gl::new(8, 8);
        let (_, t0) = fbo_8x8();
        let (t1, t2) = (texture_rgba(8, 8), texture_rgba(8, 8));
        let bufs = [GL_COLOR_ATTACHMENT0, GL_NONE, GL_COLOR_ATTACHMENT2];
        // SAFETY: plain GL calls.
        unsafe {
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT1, GL_TEXTURE_2D, t1, 0);
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT2, GL_TEXTURE_2D, t2, 0);
            glDrawBuffers(3, bufs.as_ptr());
        }
        basic(&gl);
        let d = draw_tri(&gl).draw;
        let gpu = |t: GLuint| gl.ctx().objs().textures[t as usize].mem.gpu;
        assert_eq!(d.num_targets, 3, "the NONE slot is kept and slot 2 carried");
        assert_eq!((d.targets[0].gpu, d.targets[1].gpu, d.targets[2].gpu), (gpu(t0), 0, gpu(t2)));
    }

    /// glDrawBuffers({GL_NONE, GL_COLOR_ATTACHMENT1}) (ES 3.0 4.2.1, review
    /// R-09): slot 0 writes nothing and slot 1 writes attachment 1. Slot 0
    /// was always attachment 0, whatever draw buffer 0 said, so attachment 0
    /// got output 0. The frame's own target (`target_gpu`) is still
    /// attachment 0: a backend that always stores slot 0 refuses the draw.
    /// All draw buffers GL_NONE is a zero colour mask instead, slot 0 kept.
    #[test]
    fn a_none_draw_buffer_0_leaves_slot_0_without_a_target() {
        let gl = Gl::new(8, 8);
        let (_, t0) = fbo_8x8();
        let t1 = texture_rgba(8, 8);
        // SAFETY: plain GL calls; two readable enums.
        unsafe {
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT1, GL_TEXTURE_2D, t1, 0);
            glDrawBuffers(2, [GL_NONE, GL_COLOR_ATTACHMENT1].as_ptr());
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        basic(&gl);
        let d = draw_tri(&gl).draw;
        let gpu = |t: GLuint| gl.ctx().objs().textures[t as usize].mem.gpu;
        assert_eq!(d.num_targets, 2);
        assert_eq!((d.targets[0].gpu, d.targets[1].gpu), (0, gpu(t1)), "only attachment 1 is written");
        assert_eq!(d.target_gpu, gpu(t0), "the frame is still attachment 0's");
        assert!(d.color_write && d.color_mask == 0xf);

        /* every draw buffer GL_NONE: nothing written, through the mask */
        // SAFETY: two readable enums.
        unsafe { glDrawBuffers(2, [GL_NONE, GL_NONE].as_ptr()) };
        let d = draw_tri(&gl).draw;
        assert!(!d.color_write && d.color_mask == 0, "no colour written");
        assert_eq!((d.num_targets, d.targets[0].gpu), (1, gpu(t0)), "slot 0 kept under a zero mask");
    }

    /// Colour attachments of different sizes (review R-02). ES 2.0.25 4.4.5:
    /// "All attached images have the same width and height" or the
    /// framebuffer is GL_FRAMEBUFFER_INCOMPLETE_DIMENSIONS, so a report-es2
    /// draw is GL_INVALID_FRAMEBUFFER_OPERATION. ES 3.0 4.4.4.2 allows them
    /// and renders their intersection: the draw carries each target's own
    /// size, and each backend clips to it (PowerVR) or refuses by name (V3D,
    /// the GC7000 for a later target smaller than the first).
    #[test]
    fn colour_attachments_of_different_sizes_by_version() {
        let gl = Gl::new(8, 8);
        let (_, t0) = fbo_8x8();
        let t1 = texture_rgba(4, 4);
        // SAFETY: plain GL calls.
        unsafe { glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT1, GL_TEXTURE_2D, t1, 0) };
        basic(&gl);
        if crate::REPORT_ES2 {
            // SAFETY: the arrays the fixtures set up are statics.
            unsafe { glDrawArrays(GL_TRIANGLES, 0, 3) };
            assert_eq!(gl.error(), GL_INVALID_FRAMEBUFFER_OPERATION);
            assert!(gl.draws().is_empty());
            return;
        }
        let d = draw_tri(&gl).draw;
        let gpu = |t: GLuint| gl.ctx().objs().textures[t as usize].mem.gpu;
        assert_eq!((d.target_gpu, d.target_w, d.target_h), (gpu(t0), 8, 8));
        assert_eq!(d.num_targets, 2);
        assert_eq!((d.targets[1].gpu, d.targets[1].w, d.targets[1].h), (gpu(t1), 4, 4), "its own size");
    }

    #[test]
    fn a_draw_into_the_window_has_one_target() {
        let gl = Gl::new(8, 8);
        basic(&gl);
        let d = draw_tri(&gl).draw;
        assert_eq!(d.num_targets, 1);
        assert_eq!((d.targets[0].gpu, d.targets[0].w, d.targets[0].h), (d.target_gpu, 8, 8));
    }

    #[test]
    fn the_uniform_snapshot_is_the_programs_own_array() {
        /* upstream 98f7331: the GPU buffer's contents between the copy and
         * the backend's read are the program's business. */
        let gl = Gl::new(8, 8);
        let prog = basic(&gl);
        // SAFETY: plain GL calls.
        unsafe { glUniform4f(glGetUniformLocation(prog, c"u_color".as_ptr()), 0.5, 0.25, 1.0, 1.0) };
        let d = draw_tri(&gl).draw;
        let fs_uniforms = gl.ctx().objs().programs[obj_index(prog)].fs_uniforms;
        assert_eq!(d.fs_uniform_cpu, fs_uniforms as *const c_void);
        assert!(d.fs_uniform_dwords as usize <= gl.ctx().objs().programs[obj_index(prog)].fs_uniform_cap as usize + 4);
    }

    #[test]
    #[cfg(feature = "gpu-powervr")] /* the PowerVR table's own values */
    fn a_blend_variants_constants_reach_the_snapshot_inside_the_array() {
        /* upstream 98f7331 puts glBlendColor in the host-side array; the C
         * array is fs_uniform_cap dwords long and the DMA reads four more,
         * so the port keeps room for them (see GlProgram). */
        let gl = Gl::new(16, 16);
        let prog = basic(&gl);
        // SAFETY: plain GL calls.
        unsafe {
            glEnable(GL_BLEND);
            glBlendFunc(GL_CONSTANT_COLOR, GL_ONE_MINUS_CONSTANT_COLOR);
            glBlendColor(0.1, 0.2, 0.3, 0.4);
        }
        let d = draw_tri(&gl).draw;
        // SAFETY: the linked program's shader descriptor outlives the draw.
        let off = unsafe { (*d.fs).blend_consts_off };
        assert_ne!(off, SHC_NO_BLEND_CONSTS, "the variant reads the blend colour");
        let p = &gl.ctx().objs_ref().programs[obj_index(prog)];
        assert!(
            (off as usize) + 4 <= p.fs_uniform_cap as usize + context::BLEND_CONSTS_DWORDS,
            "the constants are inside the array the backend snapshots"
        );
        // SAFETY: the array holds fs_uniform_cap + BLEND_CONSTS_DWORDS dwords.
        let got = unsafe { core::slice::from_raw_parts(p.fs_uniforms.add(off as usize), 4) };
        assert_eq!(got, [0.1, 0.2, 0.3, 0.4]);
        assert_eq!(d.fs_uniform_cpu, p.fs_uniforms as *const c_void);
    }

    /// `vertex_array.draw_arrays_invalid_program` / `draw_elements_*`: a draw
    /// into an incomplete framebuffer is GL_INVALID_FRAMEBUFFER_OPERATION even
    /// with no program bound -- 4.4.5 outranks the missing program. The
    /// argument errors still come first.
    #[test]
    fn a_draw_into_an_incomplete_framebuffer_outranks_the_missing_program() {
        let gl = Gl::new(16, 16);
        let mut fbo: GLuint = 0;
        // SAFETY: room for one name; an FBO with no attachment is incomplete.
        unsafe {
            framebuffer::glGenFramebuffers(1, &mut fbo);
            framebuffer::glBindFramebuffer(GL_FRAMEBUFFER, fbo);
            super::super::program::glUseProgram(0);
            assert_eq!(gl.error(), GL_NO_ERROR);

            /* the argument errors come before the framebuffer */
            glDrawArrays(0xDEAD, 0, 1);
            assert_eq!(gl.error(), GL_INVALID_ENUM, "an unknown mode");
            glDrawArrays(GL_POINTS, 0, -1);
            assert_eq!(gl.error(), GL_INVALID_VALUE, "a negative count");
            glDrawElements(GL_POINTS, 1, GL_FLOAT, core::ptr::null());
            assert_eq!(gl.error(), GL_INVALID_ENUM, "an index type ES has not");

            /* then the framebuffer, ahead of "no current program" */
            glDrawArrays(GL_POINTS, 0, 1);
            assert_eq!(gl.error(), GL_INVALID_FRAMEBUFFER_OPERATION);
            glDrawElements(GL_POINTS, 1, GL_UNSIGNED_SHORT, core::ptr::null());
            assert_eq!(gl.error(), GL_INVALID_FRAMEBUFFER_OPERATION);

            /* complete again: the missing program is what is left */
            framebuffer::glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDrawArrays(GL_POINTS, 0, 1);
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
        }
    }

    /* ---- glClear with the write masks and the scissor box ------------------
     * GL applies to a clear the same state it applies to a draw. These pin
     * what reaches the backend; what the pixels end up as needs the GPU. */

    /// A buffer whose write mask is entirely off is not cleared at all.
    /// `glDepthMask(GL_FALSE); glClear(GL_DEPTH_BUFFER_BIT);` changes nothing.
    #[test]
    fn a_clear_of_a_fully_masked_buffer_does_nothing() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        // SAFETY: plain GL calls.
        unsafe {
            crate::gl::state::glDepthMask(GL_FALSE_B);
            crate::gl::state::glStencilMask(0);
            crate::gl::state::glColorMask(GL_FALSE_B, GL_FALSE_B, GL_FALSE_B, GL_FALSE_B);
            crate::gl::state::glClearDepthf(0.5);
            crate::gl::state::glClearStencil(3);
            crate::gl::state::glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT);
            crate::gl::state::glFlush();
        }
        assert_eq!(gl.draws().len(), 0, "nothing at all reaches the backend");
        assert_eq!(gl.error(), GL_NO_ERROR);

        /* and the clear does not turn up behind a later draw either */
        let d = draw_tri(&gl).draw;
        assert!(!d.do_clear && !d.do_clear_depth && !d.do_clear_stencil);
    }

    /// Each mask is read on its own: a masked depth buffer does not stop the
    /// colour clear.
    #[test]
    fn a_clear_masks_each_buffer_separately() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        // SAFETY: plain GL calls.
        unsafe {
            crate::gl::state::glDepthMask(GL_FALSE_B);
            crate::gl::state::glClearColor(0.25, 0.5, 0.75, 1.0);
            crate::gl::state::glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
            crate::gl::state::glFlush();
        }
        let d = one(&gl).draw;
        assert!(d.do_clear, "the colour clear still happens");
        assert_eq!(d.clear_rgba, [0.25, 0.5, 0.75, 1.0]);
        assert!(!d.do_clear_depth, "the depth mask is off");
        assert!(!d.clear_quad, "no scissor: the background object is the cheap path");
    }

    /// The stencil-mask idiom: clear, write the mask with colour writes off,
    /// then draw through it. The clear was issued with the colour mask on, so
    /// turning the mask off for the pass that follows must not cancel it.
    ///
    /// This is the exact shape of board cases 9 (stencil) and 13 (zls) on the
    /// i.MX8MP, and of every stencil-then-cover program: the clear is fused
    /// into the first draw, and the mask is read there instead of at glClear.
    #[test]
    fn a_clear_fused_into_a_draw_keeps_the_mask_it_was_called_with() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        // SAFETY: plain GL calls.
        unsafe {
            crate::gl::state::glClearColor(0.0, 0.0, 0.0, 1.0);
            crate::gl::state::glClear(GL_COLOR_BUFFER_BIT | GL_STENCIL_BUFFER_BIT);
            /* the mask belongs to the stencil pass, not to the clear above */
            crate::gl::state::glColorMask(0, 0, 0, 0);
        }
        let d = draw_tri(&gl).draw;
        assert!(
            d.do_clear,
            "the colour clear was issued with every channel writable; the mask set afterwards \
             for the stencil pass must not cancel it, or every pixel the draws do not cover \
             keeps whatever the target held"
        );
        assert_eq!(d.clear_rgba, [0.0, 0.0, 0.0, 1.0]);
    }

    /// The audit, at the GL core: every piece of state a glClear depends on
    /// is changed between the glClear and the draw it is fused into, and the
    /// draw still carries the clear as it was issued -- the colour, depth and
    /// stencil values, the colour mask, the stencil write mask, and the depth
    /// mask's say on whether depth is cleared at all (on at the glClear, off
    /// before the draw: it is cleared).
    #[test]
    fn a_fused_clear_carries_its_glclear_state_whatever_changes_before_the_draw() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        // SAFETY: plain GL calls.
        unsafe {
            crate::gl::state::glEnable(GL_STENCIL_TEST);
            crate::gl::state::glClearColor(0.25, 0.5, 0.75, 1.0);
            crate::gl::state::glClearDepthf(0.5);
            crate::gl::state::glClearStencil(7);
            crate::gl::state::glColorMask(1, 0, 1, 0);
            crate::gl::state::glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT);
            /* everything the clear used, changed before the draw */
            crate::gl::state::glClearColor(1.0, 0.0, 0.0, 0.0);
            crate::gl::state::glClearDepthf(1.0);
            crate::gl::state::glClearStencil(0);
            crate::gl::state::glColorMask(1, 1, 1, 1);
            crate::gl::state::glStencilMask(0x0f);
            crate::gl::state::glDepthMask(0);
        }
        let d = draw_tri(&gl).draw;
        assert!(d.do_clear && d.do_clear_depth && d.do_clear_stencil, "all three fused into the draw");
        assert_eq!(d.clear_rgba, [0.25, 0.5, 0.75, 1.0]);
        assert_eq!(d.clear_depth, 0.5);
        assert_eq!(d.clear_stencil, 7);
        assert_eq!(d.clear_color_mask, 0b0101);
        assert_eq!(d.clear_stencil_write_mask, 0xff);
        assert_eq!((d.color_mask, d.stencil_write_mask), (0xf, 0x0f), "the draw's own masks are the draw's");
    }

    /// A stencil clear under glStencilMask(0) changes no bit, so it reaches
    /// the backend as no stencil clear at all: `GlesDraw` uses
    /// `clear_stencil_write_mask == 0` for "not given" (the backend then
    /// falls back to the draw's mask), which is sound only because the GL
    /// core never sends a stencil clear with an empty mask. Pinned here, the
    /// producer's half of that contract (2026-09-24).
    #[test]
    fn a_stencil_clear_under_an_empty_mask_is_no_stencil_clear() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        /* the control: the same clear under 0xff does arrive, so the target
         * has stencil and a missing clear below is the mask's doing */
        // SAFETY: plain GL calls.
        unsafe {
            crate::gl::state::glClearStencil(7);
            crate::gl::state::glClear(GL_STENCIL_BUFFER_BIT);
        }
        let c = draw_tri(&gl).draw;
        assert!(c.do_clear_stencil && c.clear_stencil_write_mask == 0xff, "the control clear arrives");
        // SAFETY: plain GL calls.
        unsafe {
            crate::gl::state::glStencilMask(0);
            crate::gl::state::glClear(GL_STENCIL_BUFFER_BIT);
            crate::gl::state::glStencilMask(0xff);
        }
        let d = draw_tri(&gl).draw;
        assert!(!d.do_clear_stencil, "glStencilMask(0) + glClear(STENCIL) must write nothing");
        assert_eq!(d.clear_stencil_write_mask, 0, "and carries no mask the backend could take for one");
    }

    /// A stencil clear fused into the next draw carries the stencil write
    /// mask of its own glClear, not the draw's: gl_test 9 frame 5 clears
    /// under 0xff, sets glStencilMask(0xfe) and draws, and a clear taking the
    /// draw's 0xfe left bit 0 of the previous frame's stencil standing (the
    /// board read 0xff00ffff where the case wants 0xff000000, on Zephyr and
    /// Linux alike).
    #[test]
    fn a_stencil_clear_fused_into_a_draw_keeps_its_own_write_mask() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        // SAFETY: plain GL calls.
        unsafe {
            crate::gl::state::glEnable(GL_STENCIL_TEST);
            crate::gl::state::glClear(GL_COLOR_BUFFER_BIT | GL_STENCIL_BUFFER_BIT);
            crate::gl::state::glStencilMask(0xfe);
        }
        let d = draw_tri(&gl).draw;
        assert!(d.do_clear_stencil, "the clear is fused into the draw");
        assert_eq!(d.clear_stencil_write_mask, 0xff, "the clear's mask, as glStencilMask had it at the glClear");
        assert_eq!((d.stencil_write_mask, d.stencil_back_write_mask), (0xfe, 0xfe), "the draw's own mask");
    }

    /// A clear carries the colour mask of its own glClear to the backend, in
    /// `clear_color_mask` and not `color_mask`: fused into a draw, the draw's
    /// mask is whatever was set afterwards. dEQP's color_clear masked_* cases
    /// clear with a random partial mask and check every channel an earlier
    /// clear wrote; a clear that writes all four fails them.
    #[test]
    fn a_clear_carries_the_colour_mask_of_its_own_glclear() {
        /* on its own (glFlush): a clear-only kick */
        {
            let gl = Gl::new(16, 16);
            basic(&gl);
            // SAFETY: plain GL calls.
            unsafe {
                crate::gl::state::glColorMask(1, 0, 0, 1);
                crate::gl::state::glClear(GL_COLOR_BUFFER_BIT);
                crate::gl::state::glFlush();
            }
            let d = one(&gl).draw;
            assert!(d.do_clear);
            assert_eq!(d.clear_color_mask, 0b1001, "R and A, as glColorMask had them at the glClear");
        }
        /* fused into the next draw, with the mask changed in between */
        {
            let gl = Gl::new(16, 16);
            basic(&gl);
            // SAFETY: plain GL calls.
            unsafe {
                crate::gl::state::glColorMask(0, 1, 1, 0);
                crate::gl::state::glClear(GL_COLOR_BUFFER_BIT);
                crate::gl::state::glColorMask(1, 1, 1, 1);
            }
            let d = draw_tri(&gl).draw;
            assert!(d.do_clear);
            assert_eq!(d.clear_color_mask, 0b0110, "the clear's own mask");
            assert_eq!(d.color_mask, 0xf, "the draw's mask is the draw's");
        }
        /* every channel */
        {
            let gl = Gl::new(16, 16);
            basic(&gl);
            // SAFETY: plain GL calls.
            unsafe {
                crate::gl::state::glClear(GL_COLOR_BUFFER_BIT);
                crate::gl::state::glFlush();
            }
            let d = one(&gl).draw;
            assert_eq!(d.clear_color_mask, 0xf);
            assert!(d.do_clear && !d.clear_quad, "the full mask keeps the unchanged path: the background object");
        }
        /* an empty mask clears no colour: alone, nothing is kicked at all */
        {
            let gl = Gl::new(16, 16);
            basic(&gl);
            // SAFETY: plain GL calls.
            unsafe {
                crate::gl::state::glColorMask(0, 0, 0, 0);
                crate::gl::state::glClear(GL_COLOR_BUFFER_BIT);
                crate::gl::state::glFlush();
            }
            assert_eq!(gl.error(), GL_NO_ERROR);
            assert!(gl.draws().is_empty(), "a colour clear with every channel masked off does nothing");
        }
        /* ... and beside a depth clear, only the depth is cleared */
        {
            let gl = Gl::new(16, 16);
            basic(&gl);
            // SAFETY: plain GL calls.
            unsafe {
                crate::gl::state::glColorMask(0, 0, 0, 0);
                crate::gl::state::glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
                crate::gl::state::glFlush();
            }
            let d = one(&gl).draw;
            assert!(!d.do_clear && d.do_clear_depth, "depth only");
            assert_eq!(d.clear_color_mask, 0, "no colour clear, so no channel");
        }
    }

    /// An empty scissor box clears nothing; a box that holds the whole target
    /// is the same as no scissor and keeps the background object.
    #[test]
    fn a_clear_scissor_box_decides_between_nothing_the_background_and_a_quad() {
        /* empty */
        {
            let gl = Gl::new(16, 16);
            basic(&gl);
            // SAFETY: plain GL calls.
            unsafe {
                crate::gl::state::glEnable(GL_SCISSOR_TEST);
                crate::gl::state::glScissor(2, 2, 0, 4);
                crate::gl::state::glClear(GL_COLOR_BUFFER_BIT);
                crate::gl::state::glFlush();
            }
            assert_eq!(gl.draws().len(), 0, "an empty box clears nothing");
        }
        /* the whole target */
        {
            let gl = Gl::new(16, 16);
            basic(&gl);
            // SAFETY: plain GL calls.
            unsafe {
                crate::gl::state::glEnable(GL_SCISSOR_TEST);
                crate::gl::state::glScissor(0, 0, 16, 16);
                crate::gl::state::glClear(GL_COLOR_BUFFER_BIT);
                crate::gl::state::glFlush();
            }
            let d = one(&gl).draw;
            assert!(d.do_clear && !d.clear_quad, "the background object covers it");
        }
        /* narrower */
        {
            let gl = Gl::new(16, 16);
            basic(&gl);
            // SAFETY: plain GL calls.
            unsafe {
                crate::gl::state::glEnable(GL_SCISSOR_TEST);
                crate::gl::state::glScissor(2, 3, 4, 5);
                crate::gl::state::glClearColor(1.0, 0.0, 0.0, 1.0);
                crate::gl::state::glClear(GL_COLOR_BUFFER_BIT);
                crate::gl::state::glFlush();
            }
            let d = one(&gl).draw;
            assert!(d.clear_quad, "the background object has no scissor");
            assert!(d.scissor);
            assert_eq!(d.scissor_rect, [2, 3, 4, 5]);
            assert_eq!(d.clear_rgba, [1.0, 0.0, 0.0, 1.0]);
            assert!(d.color_write && d.do_clear);
            assert_eq!(d.index_count, 0, "the backend supplies the quad's geometry");
        }
    }

    /// A stencil clear that may change only some bits cannot be the frame's
    /// background object either, and the quad carries the write mask.
    #[test]
    fn a_stencil_clear_with_a_partial_write_mask_becomes_a_quad() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        // SAFETY: plain GL calls.
        unsafe {
            crate::gl::state::glStencilMask(0x0f);
            crate::gl::state::glClearStencil(7);
            crate::gl::state::glClear(GL_STENCIL_BUFFER_BIT);
            crate::gl::state::glFlush();
        }
        let d = one(&gl).draw;
        assert!(d.clear_quad);
        assert!(d.do_clear_stencil);
        assert_eq!(d.clear_stencil, 7);
        assert_eq!(d.stencil_write_mask, 0x0f);
        assert_eq!(d.stencil_back_write_mask, 0x0f);
        assert!(!d.do_clear, "colour is untouched");
    }

    /// A scissored clear in the middle of a frame is an object like any
    /// other: it is recorded where GL put it, ahead of the draw that follows,
    /// and does not force a flush.
    #[test]
    fn a_scissored_clear_is_recorded_before_the_draw_that_follows_it() {
        let gl = Gl::new(16, 16);
        basic(&gl);
        // SAFETY: the arrays the fixture set up are statics.
        unsafe {
            glDrawArrays(GL_TRIANGLES, 0, 3);
            crate::gl::state::glEnable(GL_SCISSOR_TEST);
            crate::gl::state::glScissor(1, 1, 2, 2);
            crate::gl::state::glClearColor(0.0, 1.0, 0.0, 1.0);
            crate::gl::state::glClear(GL_COLOR_BUFFER_BIT);
            glDrawArrays(GL_TRIANGLES, 0, 3);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        let v = gl.draws();
        assert_eq!(v.len(), 3, "draw, clear quad, draw");
        assert!(!v[0].draw.clear_quad && v[0].draw.index_count != 0);
        assert!(v[1].draw.clear_quad, "the clear sits between the two draws");
        assert_eq!(v[1].draw.scissor_rect, [1, 1, 2, 2]);
        assert!(!v[2].draw.clear_quad && v[2].draw.index_count != 0);
        assert!(!v[2].draw.do_clear, "the clear was not also fused into the draw");
    }

    #[test]
    fn a_sequence_of_scissored_clears_keeps_every_one_of_them() {
        /* dEQP-EGL's colour-clear op, verbatim: disable dither, scissor to a
         * rectangle, set the colour, clear, and turn the scissor off again --
         * repeated with different rectangles and colours, with no draw
         * between, and then read back. The reference applies them in order,
         * so every one of them has to reach the surface. */
        let gl = Gl::new(16, 16);
        basic(&gl);
        let ops = [
            ([1, 1, 4, 4], [1.0, 0.0, 0.0, 1.0]),
            ([2, 2, 8, 8], [0.0, 1.0, 0.0, 1.0]),
            ([0, 6, 16, 3], [0.0, 0.0, 1.0, 1.0]),
        ];
        // SAFETY: plain GL calls.
        unsafe {
            for (r, c) in ops {
                crate::gl::state::glDisable(GL_DITHER);
                crate::gl::state::glEnable(GL_SCISSOR_TEST);
                crate::gl::state::glScissor(r[0], r[1], r[2], r[3]);
                crate::gl::state::glClearColor(c[0], c[1], c[2], c[3]);
                crate::gl::state::glClear(GL_COLOR_BUFFER_BIT);
                crate::gl::state::glDisable(GL_SCISSOR_TEST);
            }
            /* glReadPixels is what follows in dEQP; it has to settle them */
            gl_flush_pending_clear();
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        let v = gl.draws();
        let clears: std::vec::Vec<_> = v.iter().filter(|d| d.draw.clear_quad || d.draw.do_clear).collect();
        assert_eq!(clears.len(), ops.len(), "one clear reached the surface per glClear, in order");
        for (i, (r, _)) in ops.iter().enumerate() {
            assert_eq!(clears[i].draw.scissor_rect, *r, "clear {i}");
        }
    }

    /* ---- GL's incomplete texture (GL ES 2.0 section 3.7.10) ---------------- */

    /// A minification filter that uses mipmaps needs the whole chain. Without
    /// it the texture is incomplete and samples (0,0,0,1); the core used to
    /// demote the filter and sample level 0 instead, which showed an
    /// application's mistake as a picture rather than as black.
    #[test]
    fn a_mipmap_filter_without_a_chain_is_an_incomplete_texture() {
        let gl = Gl::new(16, 16);
        textured(&gl);
        texture_rgba(4, 4); /* one level */
        // SAFETY: plain GL calls.
        unsafe { glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR_MIPMAP_LINEAR as GLint) };
        assert_eq!(draw_tri(&gl).draw.textures[0].incomplete, 1);

        /* a filter that does not use mipmaps makes it complete again */
        // SAFETY: plain GL calls.
        unsafe { glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR as GLint) };
        assert_eq!(draw_tri(&gl).draw.textures[0].incomplete, 0);
    }

    /// A cube map needs all six faces at one size.
    #[test]
    #[cfg_attr(
        all(feature = "no-shader-table", feature = "report-es2"),
        ignore = "GLSL ES 3.00 test shader (cube.vert) in a build that reports ES 2.0"
    )]
    fn a_cube_map_missing_a_face_is_incomplete() {
        let gl = Gl::new(16, 16);
        es3_prog(&gl, "cube.frag");
        let px = std::vec![0u8; 4 * 4 * 4];
        bind_new_tex(GL_TEXTURE_CUBE_MAP);
        // SAFETY: `px` holds each 4x4 face.
        unsafe {
            glTexParameteri(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_MIN_FILTER, GL_LINEAR as GLint);
            for f in 0..5u32 {
                glTexImage2D(
                    GL_TEXTURE_CUBE_MAP_POSITIVE_X + f,
                    0,
                    GL_RGBA as GLint,
                    4,
                    4,
                    0,
                    GL_RGBA,
                    GL_UNSIGNED_BYTE,
                    px.as_ptr() as *const c_void,
                );
            }
        }
        assert_eq!(draw_strip(&gl).draw.textures[0].incomplete, 1, "five faces of six");

        // SAFETY: `px` holds the sixth 4x4 face.
        unsafe {
            glTexImage2D(
                GL_TEXTURE_CUBE_MAP_NEGATIVE_Z,
                0,
                GL_RGBA as GLint,
                4,
                4,
                0,
                GL_RGBA,
                GL_UNSIGNED_BYTE,
                px.as_ptr() as *const c_void,
            )
        };
        assert_eq!(draw_strip(&gl).draw.textures[0].incomplete, 0, "all six at one size");
    }

    /// ES 2.0 without GL_OES_texture_npot: a non-power-of-two texture is
    /// complete only when it is clamped on both axes and filtered without
    /// mipmaps. ES 3.0 dropped the rule, and so does the extension, which
    /// the ES 2.0 surface claims (texture::NPOT_RESTRICTED).
    #[test]
    fn a_npot_texture_is_complete_only_where_the_version_allows_it() {
        let gl = Gl::new(16, 16);
        textured(&gl);
        texture_rgba(5, 4);
        // SAFETY: plain GL calls.
        unsafe { glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR as GLint) };

        /* repeat on both axes: ES 2.0 without the extension says incomplete */
        let want = u8::from(super::super::texture::NPOT_RESTRICTED);
        assert_eq!(draw_tri(&gl).draw.textures[0].incomplete, want, "NPOT, repeat");

        // SAFETY: plain GL calls.
        unsafe {
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE as GLint);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE as GLint);
        }
        assert_eq!(draw_tri(&gl).draw.textures[0].incomplete, 0, "NPOT, clamped on both axes");

        /* one axis left repeating is not enough */
        // SAFETY: plain GL calls.
        unsafe { glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_REPEAT as GLint) };
        assert_eq!(draw_tri(&gl).draw.textures[0].incomplete, want, "NPOT, T repeats");

        /* S alone, and mirrored repeat, are the same rule (dEQP
         * texture.completeness.2d.npot_s_repeat, npot_mirrored_repeat) */
        // SAFETY: plain GL calls.
        unsafe {
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE as GLint);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_REPEAT as GLint);
        }
        assert_eq!(draw_tri(&gl).draw.textures[0].incomplete, want, "NPOT, S repeats");
        // SAFETY: plain GL calls.
        unsafe {
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_MIRRORED_REPEAT as GLint);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_MIRRORED_REPEAT as GLint);
        }
        assert_eq!(draw_tri(&gl).draw.textures[0].incomplete, want, "NPOT, mirrored repeat");
    }

    /// Level `level` of a 2D texture, `w x h` texels of `format`.
    fn level_image(level: GLint, w: i32, h: i32, format: GLenum) {
        let bpp = match format {
            GL_RGBA => 4,
            GL_RGB => 3,
            GL_LUMINANCE_ALPHA => 2,
            _ => 1,
        };
        let texels = std::vec![0x40u8; (w * h * bpp).max(4) as usize];
        // SAFETY: `texels` holds the image at unpack alignment 1.
        unsafe {
            glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
            glTexImage2D(
                GL_TEXTURE_2D,
                level,
                format as GLint,
                w,
                h,
                0,
                format,
                GL_UNSIGNED_BYTE,
                texels.as_ptr() as *const c_void,
            );
        }
    }

    /// A `size x size` texture with a mipmap min filter and every level of
    /// its chain, except `skip`; level `other` in `other_format`. dEQP
    /// texture.completeness.2d's set-up, which `dEQP-GLES2.functional.
    /// texture.completeness.2d.*` on the i.MX8MP (99fc5c9d) found this GL
    /// core misjudging: 12 named cases sampled a picture where ES 2.0 3.7.10
    /// requires (0,0,0,1), or raised an error 3.7.1 does not have.
    fn mip_chain(size: i32, skip: Option<GLint>, other: Option<(GLint, GLenum)>) -> GLuint {
        let t = bind_new_tex(GL_TEXTURE_2D);
        // SAFETY: plain GL calls.
        unsafe {
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST_MIPMAP_NEAREST as GLint);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
        }
        let levels = 1 + (31 - (size as u32).leading_zeros()) as GLint;
        for l in 0..levels {
            if Some(l) == skip {
                continue;
            }
            let format = match other {
                Some((k, f)) if k == l => f,
                _ => GL_RGBA,
            };
            level_image(l, (size >> l).max(1), (size >> l).max(1), format);
        }
        t
    }

    /// A level of the chain never specified (missing_level_1, missing_level_3):
    /// incomplete until it is, whatever the highest level uploaded.
    ///
    /// Revert check: take `level_defined` out of `incomplete_reason` and the
    /// hole is invisible (`levels` is 7 either way).
    #[test]
    fn a_hole_in_the_mip_chain_is_incomplete() {
        let gl = Gl::new(16, 16);
        textured(&gl);
        for skip in [1, 3] {
            mip_chain(64, Some(skip), None);
            assert_eq!(gl.error(), GL_NO_ERROR);
            assert_eq!(draw_tri(&gl).draw.textures[0].incomplete, 1, "level {skip} missing");
            level_image(skip, 64 >> skip, 64 >> skip, GL_RGBA);
            assert_eq!(draw_tri(&gl).draw.textures[0].incomplete, 0, "level {skip} supplied");
        }
    }

    /// A level of another format than level 0 (format_mismatch_rgb_rgba and
    /// the three others): incomplete under a mipmap filter, no GL error.
    #[test]
    fn a_level_of_another_format_is_incomplete() {
        let gl = Gl::new(16, 16);
        textured(&gl);
        for (base, other) in [
            (GL_RGB, GL_RGBA),
            (GL_RGBA, GL_RGB),
            (GL_LUMINANCE, GL_LUMINANCE_ALPHA),
            (GL_LUMINANCE_ALPHA, GL_LUMINANCE),
        ] {
            let t = bind_new_tex(GL_TEXTURE_2D);
            // SAFETY: plain GL calls.
            unsafe { glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST_MIPMAP_NEAREST as GLint) };
            for l in 0..5 {
                level_image(l, 16 >> l, 16 >> l, if l == 1 { other } else { base });
            }
            assert_eq!(gl.error(), GL_NO_ERROR, "no error for a mismatched level (3.7.1)");
            assert_eq!(draw_tri(&gl).draw.textures[0].incomplete, 1, "{base:#x} with level 1 as {other:#x}");
            /* the same level in level 0's format makes it complete */
            level_image(1, 8, 8, base);
            assert_eq!(draw_tri(&gl).draw.textures[0].incomplete, 0, "texture {t}");
        }
    }

    /// A level 0 with a zero side (not_positive_level_0): the levels after it
    /// are no error, and the texture is incomplete.
    #[test]
    fn a_zero_sized_level_0_is_incomplete_and_later_levels_are_no_error() {
        let gl = Gl::new(16, 16);
        textured(&gl);
        bind_new_tex(GL_TEXTURE_2D);
        // SAFETY: plain GL calls.
        unsafe { glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST_MIPMAP_NEAREST as GLint) };
        level_image(0, 0, 0, GL_RGBA);
        assert_eq!(gl.error(), GL_NO_ERROR);
        for l in 1..5 {
            level_image(l, 16 >> l, 16 >> l, GL_RGBA);
            assert_eq!(gl.error(), GL_NO_ERROR, "level {l} after a 0x0 level 0 is no error");
        }
        /* A 0x0 level 0 lays out no storage, so the draw binds nothing for
         * the unit: the back end's unbound-sampler rule samples (0,0,0,1)
         * (gc-backend 00d8b62f). The later levels must not have changed
         * that by giving the texture storage of their own. */
        let t = &gl.ctx().objs().textures[gl.ctx().bound_texture_2d[0] as usize];
        assert_eq!((t.level_defined & 1, t.level_bad & 1, t.width, t.height), (1, 1, 0, 0));
        assert_eq!(draw_tri(&gl).draw.num_textures, 0, "no storage: left to the back end's black");
        /* and without a mipmap filter too: level 0 has nothing to sample */
        // SAFETY: plain GL calls.
        unsafe { glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint) };
        assert_eq!(draw_tri(&gl).draw.num_textures, 0);
    }

    /// A 1x1 image past the chain's last level (extra_level): ignored, as
    /// 3.7.10 ignores levels past q; no error, and the texture is complete.
    /// It was GL_INVALID_VALUE, which failed the dEQP case on the error.
    #[test]
    fn a_level_past_the_chain_is_ignored_and_no_error() {
        let gl = Gl::new(16, 16);
        textured(&gl);
        mip_chain(64, None, None);
        level_image(8, 1, 1, GL_RGBA);
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(draw_tri(&gl).draw.textures[0].incomplete, 0);
        /* one that is not 1x1 does not fit anything and is recorded as such */
        level_image(8, 2, 2, GL_RGBA);
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(draw_tri(&gl).draw.textures[0].incomplete, 0, "past the chain: not part of it");
    }

    /// A non-power-of-two base with a mipmap filter (npot_size): ES 2.0
    /// 3.7.1 without GL_OES_texture_npot refuses a level above 0 that is not
    /// a power of two with GL_INVALID_VALUE. With the extension (and in ES
    /// 3.0) the level is taken, and the texture is incomplete until its whole
    /// chain is there (15x15: 7x7, 3x3, 1x1), then complete under REPEAT.
    #[test]
    fn a_npot_level_above_0_follows_the_version() {
        let gl = Gl::new(16, 16);
        textured(&gl);
        bind_new_tex(GL_TEXTURE_2D);
        // SAFETY: plain GL calls.
        unsafe { glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST_MIPMAP_NEAREST as GLint) };
        level_image(0, 15, 15, GL_RGBA);
        assert_eq!(gl.error(), GL_NO_ERROR);
        level_image(1, 7, 7, GL_RGBA);
        let restricted = super::super::texture::NPOT_RESTRICTED;
        let want = if restricted { GL_INVALID_VALUE } else { GL_NO_ERROR };
        assert_eq!(gl.error(), want, "a NPOT level 1");
        assert_eq!(draw_tri(&gl).draw.textures[0].incomplete, 1, "levels 2 and 3 missing");
        level_image(2, 3, 3, GL_RGBA);
        level_image(3, 1, 1, GL_RGBA);
        assert_eq!(draw_tri(&gl).draw.textures[0].incomplete, u8::from(restricted), "the whole chain");
    }

    /// GL_OES_texture_npot, glmark2's terrain: an 800x600 RGBA texture,
    /// glGenerateMipmap, LINEAR_MIPMAP_LINEAR and REPEAT (or MIRRORED_REPEAT)
    /// on both axes is complete, with no GL error, in every build. Every
    /// backend holds the chain (`npot_mip_chains`) and the draw samples all
    /// ten levels with a linear mip filter: the GC7000's shape at its one
    /// 800-texel chain pitch, the PowerVR's at the 1024 of the power-of-two
    /// extent it lays the chain out in (texture.rs `pot_chain_layout`; it
    /// sampled the base level alone until 2026-10-08). The wrap is the
    /// application's either way.
    ///
    /// Revert check: put `crate::REPORT_ES2` back for NPOT_RESTRICTED in
    /// texture.rs and the report-es2 builds fail here on the error and on
    /// the incomplete flag.
    #[test]
    fn a_generated_npot_chain_is_complete_with_repeat() {
        for holds_chain in [false, true] {
            let gl = Gl::new(16, 16);
            if holds_chain {
                gl.backend_like_gc7000();
            }
            textured(&gl);
            texture_rgba(800, 600);
            // SAFETY: plain GL calls.
            unsafe {
                glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR_MIPMAP_LINEAR as GLint);
                glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_LINEAR as GLint);
                glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_REPEAT as GLint);
                glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_REPEAT as GLint);
            }
            assert_eq!(draw_tri(&gl).draw.textures[0].incomplete, 1, "{holds_chain}: no chain yet");
            // SAFETY: plain GL call.
            unsafe { glGenerateMipmap(GL_TEXTURE_2D) };
            assert_eq!(gl.error(), GL_NO_ERROR, "{holds_chain}: glGenerateMipmap of 800x600");
            let b = draw_tri(&gl).draw.textures[0];
            assert_eq!(b.incomplete, 0, "{holds_chain}: complete, mipmapped, REPEAT");
            assert_eq!((b.width, b.height), (800, 600));
            assert_eq!((b.wrap_s, b.wrap_t), (GLES_WRAP_REPEAT, GLES_WRAP_REPEAT));
            assert_eq!((b.linear_filter, b.min_linear), (1, 1));
            assert_eq!((b.mip_levels, b.mip_filter), (10, 2), "{holds_chain}: levels sampled, mip filter");
            assert_eq!(b.stride_texels, if holds_chain { 800 } else { 1024 }, "{holds_chain}: stride");
            // SAFETY: plain GL calls.
            unsafe {
                glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_MIRRORED_REPEAT as GLint);
                glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_MIRRORED_REPEAT as GLint);
            }
            let b = draw_tri(&gl).draw.textures[0];
            assert_eq!(b.incomplete, 0, "{holds_chain}: MIRRORED_REPEAT");
            assert_eq!((b.wrap_s, b.wrap_t), (GLES_WRAP_MIRRORED_REPEAT, GLES_WRAP_MIRRORED_REPEAT));
        }
    }

    /// The PowerVR's table is the shape `a_generated_npot_chain_is_complete_
    /// with_repeat` runs as the mock: the TPU walks the chain itself (packed,
    /// not a layered texture unit), so the GL core lays a NPOT 2D chain out
    /// as the chain of its power-of-two extent, Mesa pvr's layout (texture.rs
    /// `pot_chain_layout`): glmark2 terrain's 800x600 is ten levels in the
    /// 5,592,404 bytes of the 1024x1024 chain. It was one level, 800 * 600 *
    /// 4 bytes, until 2026-10-08.
    #[cfg(feature = "gpu-powervr")]
    #[test]
    fn the_powervr_lays_a_npot_chain_out_as_its_power_of_two_extent() {
        let pvr = &crate::pvr::backend::yakogl_backend_pvr;
        assert_eq!((pvr.mip_level_align, pvr.native_layered_textures), (0, false));
        let (_, levels, bytes, pitch) =
            super::super::texture::chain_layout_npot(800, 600, 1, 4, false, 0, true, true).unwrap();
        assert_eq!((levels, bytes, pitch), (10, 5_592_404, 0));
    }

    /// glmark2 terrain's path on the PowerVR's shape: a NPOT texture whose
    /// chain was laid out (glGenerateMipmap) is rendered into through a
    /// framebuffer. The render target is level 0 at the chain's power-of-two
    /// pitch (`GlesDraw::target_pitch` 16 for a 10-wide level 0 of the 16x8
    /// chain), and glReadPixels reads its rows at that pitch. Before, a
    /// level whose pitch was not its pair-padded width was no render target.
    #[test]
    fn a_powervr_npot_chain_renders_at_its_power_of_two_pitch() {
        let gl = Gl::new(16, 16);
        textured(&gl);
        let tex = texture_rgba(10, 6);
        // SAFETY: plain GL calls.
        unsafe {
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR_MIPMAP_LINEAR as GLint);
            glGenerateMipmap(GL_TEXTURE_2D);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        let (gpu, off1) = {
            let t = &gl.ctx().objs_ref().textures[tex as usize];
            assert_eq!((t.pot_chain, t.chain_levels, t.level_pitch_at(0)), (true, 4, 16));
            (t.mem.gpu, t.level_off[1])
        };
        let src = texture_rgba(4, 4);
        let mut fbo: GLuint = 0;
        // SAFETY: valid out pointer, plain GL calls.
        unsafe {
            glGenFramebuffers(1, &mut fbo);
            glBindFramebuffer(GL_FRAMEBUFFER, fbo);
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
            assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
            glBindTexture(GL_TEXTURE_2D, src);
        }
        let d = draw_tri(&gl).draw;
        assert_eq!((d.target_gpu, d.target_w, d.target_h, d.target_pitch), (gpu, 10, 6, 16));
        assert_eq!(crate::backend::draw_pitch(&d), 16);
        /* level 1 (5x3) is a target too, at its pitch of 8 */
        // SAFETY: plain GL calls.
        unsafe {
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 1);
            assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
        }
        let d = draw_tri(&gl).draw;
        assert_eq!((d.target_gpu, d.target_w, d.target_h, d.target_pitch), (gpu + off1 as u64, 5, 3, 8));
        /* what the pixel engine wrote at the 16-texel pitch reads back */
        // SAFETY: plain GL calls.
        unsafe { glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0) };
        let row1: Vec<u8> = (0..40u8).collect();
        gl.gpu_render(gpu + 16 * 4, &row1);
        let mut px = [0u8; 40];
        // SAFETY: `px` holds the 10x1 RGBA8 read.
        unsafe { glReadPixels(0, 1, 10, 1, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr() as *mut c_void) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(px[..], row1[..], "row 1 at the 16-texel pitch");
    }

    /// dEQP-GLES2 multisample's draws with alpha to coverage on a 4x target
    /// and alpha masked (the constancy cases): every draw, the first one
    /// too, runs the same fragment program, the sample-kill variant's blend
    /// variant for the colour mask -- the kill epilogue, never the program's
    /// own code -- whatever colour the draw's vertices carry. A first draw
    /// compiled without the epilogue would draw where its twin later does
    /// not (the constancy cases' green quad under the red one).
    #[cfg(all(feature = "shaderc-online", feature = "gpu-powervr"))]
    #[test]
    fn every_alpha_to_coverage_draw_runs_the_sample_kill_variant_from_the_first() {
        const VS: &str = "attribute highp vec4 p; attribute mediump vec4 a_color; varying mediump vec4 v_color; \
                          void main(){ gl_Position = p; v_color = a_color; }";
        const FS: &str = "varying mediump vec4 v_color;\nvoid main()\n{\n    gl_FragColor = v_color;\n}\n";
        let gl = Gl::new(16, 16);
        let _be = OnlineBackEnd::install();
        gl.ctx().default_samples = 4;
        let prog = online_program(VS, FS);
        let own = gl.ctx().objs().programs[obj_index(prog)].rfs.usc_words().to_vec();
        let colours: [[f32; 12]; 2] = [
            [0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.5, 0.0, 1.0, 0.0, 1.0],
            [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.5, 1.0, 0.0, 0.0, 1.0],
        ];
        // SAFETY: a NUL-terminated name.
        let ca = unsafe { glGetAttribLocation(prog, c"a_color".as_ptr()) };
        assert!(ca >= 0);
        // SAFETY: plain GL calls.
        unsafe {
            glEnable(GL_SAMPLE_ALPHA_TO_COVERAGE);
            glColorMask(1, 1, 1, 0);
            glEnableVertexAttribArray(ca as GLuint);
        }
        let key = shaderc::SHC_BLEND_KEY_REPLACE | shaderc::shc_blend_key_mask_off(0b1000);
        let mut first = None;
        for k in 0..8 {
            // SAFETY: as above.
            unsafe { glVertexAttribPointer(ca as GLuint, 4, GL_FLOAT, 0, 0, colours[k % 2].as_ptr() as *const c_void) };
            let d = draw_tri(&gl).draw;
            // SAFETY: the variant lives in the draw path's cache.
            let fs = unsafe { &*d.fs };
            assert_eq!((fs.uses_discard, fs.blend_key, d.alpha_to_coverage), (1, key, true), "draw {k}");
            assert_ne!(fs.usc_words(), &own[..], "draw {k}: not the program's own code");
            let at = (d.fs_usc_gpu, fs.usc_words().to_vec());
            assert_eq!(first.get_or_insert_with(|| at.clone()), &at, "draw {k}: the first draw's program");
        }
    }
}
