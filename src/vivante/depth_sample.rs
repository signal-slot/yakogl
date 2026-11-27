// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The depth-sampling experiment (docs/notes/gc-depth-sample.md, case
//! `dtpu`; plan #328, #1204): can the texture unit read the row-linear D24S8
//! surface the PE wrote, without the CPU resolve (`ds_cache::resolve_depth`)?
//!
//! A test knob selects the mode, `gc_debug_set_depth_tpu(mode)`, and only
//! gl_test and the host tests set it. The product default
//! ([`DEPTH_TPU_DEFAULT`], the only mode without `test-knobs`) is mode 4,
//! [`DEPTH_TPU_D24X8_FLUSH`]: gl_test `dtpu` on the Armadillo-X2 (measured
//! there in place of the Verdin iMX8MP) answered verdict code 3 with check 7
//! failing, outcome "(a) plus a texture-cache flush" of
//! docs/notes/gc-depth-sample-wire.md. Mode 0 is the CPU resolve:
//! [`gc_depth_sampled`] answers no, the GL core resolves the depth into the
//! R32F texels on the CPU, and nothing here runs. With mode 1, 2 or 4
//! [`gc_depth_sampled`] answers yes for a Live, single-sampled depth
//! plane that covers the texture, the GL core puts the id into the binding
//! (`GlesTextureBind::depth_id`), and `texture::emit_slot` samples the surface
//! through the binding [`view_binding`] makes:
//!
//! - mode 1, route 2 of the note (Mesa's): `TEXTURE_FORMAT_D24X8` (0x11),
//!   swizzle (R, R, R, 1), no INT_FILTER (Mesa F:320);
//! - mode 2, the raw view: A8R8G8B8 with the identity swizzle and INT_FILTER,
//!   so the texel is (D1, D0, S, D2) of the word `D2 D1 D0 S`.
//!
//! (Mode 3 samples a copy instead: section "Outcome (b).1, mode 3".)
//!
//! Both are LINEAR, one level, LOD 0 at the surface past the guard, and
//! `LINEAR_STRIDE` the surface's row stride; mode 2 is NEAREST. A binding whose
//! texture does not fit inside the live surface (or whose surface went away
//! between the answer and the draw) fails the draw with -ENOTSUP, by name: the
//! texels were left stale on the strength of the answer, so sampling them
//! would be wrong. Modes 1, 2 and 3 are not product paths: the release
//! archive has no knob (tools/check.sh, "no test knob in the artefact"), so
//! it runs mode 4 alone.
//!
//! # Outcome (b).1, mode 3
//!
//! If `dtpu` answers (b) (checks 4 and 5 fail, check 6 passes: the TPU
//! addresses the surface's bytes, but D24X8 does not decode a LINEAR surface),
//! gc-depth-sample-wire.md "(b).1 Primary: an RS copy into a supertiled depth
//! view" copies the surface on the GPU into a SUPER_TILED buffer of the same
//! shape and samples that as D24X8, Mesa's own sample of a depth resource.
//! [`copy_blit`] builds that copy's resolve-engine words, [`copy_bytes`] the
//! buffer's size, and [`copy_sampler_words`] the descriptor words that sample
//! the buffer. Knob mode 3 ([`DEPTH_TPU_RSCOPY`]) wires them: the answer
//! ([`gc_depth_sampled`]) makes the buffer (`depth_copy::CopyPool`, keyed by
//! id and surface generation), `emit_copies` records the copy before the
//! draw, [`binding`] points the descriptor at the buffer as
//! [`GC_TEXFMT_DEPTH_COPY`] and stamps it with the draw's kick,
//! `gc_attachment_gone` retires it and `finish` / `finish_previous` reap it.
//! A surface the copy refuses, or no buffer, answers no: the CPU resolve.
//! tests/gc_depth_copy_knob.rs proves it on the host; the board arm that
//! checks it (case 222's knob 3) is gc-depth-sample-wire.md's "Board check".
//!
//! Mesa 26.2.1 citations are `src/gallium/drivers/etnaviv/<file>:<line>` and
//! `src/etnaviv/hw/<header>:<line>` of the tree tools/vivante/README.md builds
//! in (the deps cache's `mesa-pan/mesa-26.2.1`; `third_party/mesa-26.2.1`
//! holds no etnaviv).

use crate::backend::{swizzle_source, GlesTextureBind, GLES_TEXFMT_RGBA8, GLES_TEXTYPE_2D};
use crate::util::ENOTSUP;

/// `TEXTURE_FORMAT_D24X8` (Mesa `common_3d.xml.h`, X:60; etnaviv_format.c
/// F:179-180 for X8Z24_UNORM and S8_UINT_Z24_UNORM): a CONFIG0 format.
pub const GC_TEX_FORMAT_D24X8: u32 = 0x11;

/// A backend-private texel class: a binding that reads a depth surface in
/// place as D24X8 ([`view_binding`] mode 1). Never a `GLES_TEXFMT_*`.
pub const GC_TEXFMT_DEPTH_VIEW: u8 = 0xf5;

/// A backend-private texel class: a binding that reads outcome (b).1's
/// SUPER_TILED copy of a depth surface as D24X8 ([`copy_binding`], mode 3).
/// Never a `GLES_TEXFMT_*`.
pub const GC_TEXFMT_DEPTH_COPY: u8 = 0xf4;

/// Knob modes: 0 the CPU resolve, 1 D24X8, 2 the raw A8R8G8B8 view, 3 an RS
/// copy into a SUPER_TILED buffer sampled as D24X8 (outcome (b).1, section
/// "Mode 3" below).
pub const DEPTH_TPU_OFF: i32 = 0;
pub const DEPTH_TPU_D24X8: i32 = 1;
pub const DEPTH_TPU_RAW8888: i32 = 2;
pub const DEPTH_TPU_RSCOPY: i32 = 3;

/// The product default mode: what [`mode`] answers in a build without
/// `test-knobs`, and the knob's start value. [`DEPTH_TPU_D24X8_FLUSH`], the
/// D24X8 view with a texture-cache flush in front of the sampling draw:
/// gl_test `dtpu`'s verdict code 3 with check 7 failing (stale texels at
/// index 32 with no wait or flush), measured on the Armadillo-X2 in place of
/// the Verdin iMX8MP, selects outcome "(a) plus a texture-cache flush" of
/// docs/notes/gc-depth-sample-wire.md. Plain mode 1 is not safe there (check
/// 7). Exact-depth readers ([`exact_depth_reader`]) and planes that are not
/// row-linear (`ds_cache::depth_view`) never take the view.
pub const DEPTH_TPU_DEFAULT: i32 = DEPTH_TPU_D24X8_FLUSH;

/// The experiment's mode ([`mode`]): [`DEPTH_TPU_DEFAULT`] unless gl_test set it.
#[cfg(feature = "test-knobs")]
#[no_mangle]
pub static mut gc_debug_depth_tpu: i32 = DEPTH_TPU_DEFAULT;

/// Set [`gc_debug_depth_tpu`]; returns the previous value (gl_test's `dtpu`
/// runs each arm and puts it back).
#[cfg(feature = "test-knobs")]
#[no_mangle]
pub extern "C" fn gc_debug_set_depth_tpu(mode: i32) -> i32 {
    // SAFETY: single-threaded by contract; plain loads and stores of an int.
    unsafe { core::ptr::addr_of_mut!(gc_debug_depth_tpu).replace(mode) }
}

/// The current mode: [`DEPTH_TPU_DEFAULT`] ([`DEPTH_TPU_D24X8_FLUSH`]) in every build without `test-knobs`, and in one that has not set
/// the knob.
pub fn mode() -> i32 {
    #[cfg(feature = "test-knobs")]
    {
        // SAFETY: single-threaded by contract; a plain read of an int only
        // the setter writes.
        unsafe { core::ptr::addr_of!(gc_debug_depth_tpu).read() }
    }
    #[cfg(not(feature = "test-knobs"))]
    {
        DEPTH_TPU_DEFAULT
    }
}

/// Where a depth plane lies: the GPU address of its first word (past the
/// guard), the surface's row stride in bytes and its rows (both 64-pixel
/// aligned, `ds_cache::Surface`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct DepthView {
    pub gpu: u64,
    pub stride: u32,
    pub rows: u32,
}

impl DepthView {
    /// Does a `w` x `h` texture sampled from it lie inside the surface?
    pub fn covers(&self, w: u32, h: u32) -> bool {
        w != 0 && h != 0 && u64::from(w) * 4 <= u64::from(self.stride) && h <= self.rows
    }
}

/// The live depth plane of GL object `id`; `kick` != 0 keeps the surface
/// until that kick completed.
fn view(id: u32, kick: u32) -> Option<DepthView> {
    let (gpu, stride, rows) = super::ds_cache::depth_view(id, super::backend::GC_DEPTH_GUARD_BYTES, kick)?;
    Some(DepthView { gpu, stride, rows })
}

/// `crate::backend::samples_rendered_depth`'s GC answer: no in mode 0 (the
/// CPU resolve, set by the knob only); otherwise yes when `id`'s depth plane is Live here,
/// single-sampled, and covers `w` x `h`. In mode 3 the RS copy must also be
/// possible: a surface [`copy_bytes`] takes and its buffer in the pool (made
/// now, so a refused shape or no memory leaves the draw on the CPU resolve).
/// No wait and no CPU access of the surface.
pub fn gc_depth_sampled(id: u32, w: u32, h: u32) -> bool {
    match mode() {
        DEPTH_TPU_OFF => false,
        DEPTH_TPU_RSCOPY => id != 0 && copy_ready(id, w, h),
        _ => id != 0 && view(id, 0).is_some_and(|v| v.covers(w, h)),
    }
}

/// Does `t` ask for a depth compare that reads the depth's value
/// (GL_TEXTURE_COMPARE_MODE GL_COMPARE_REF_TO_TEXTURE with a function other
/// than GL_NEVER and GL_ALWAYS; `GlesTextureBind::compare_op`, src/gl/draw.rs
/// `compare_op`)? Such a reader needs the exact 24 bits the CPU resolve
/// converts, and a D24X8 view may return fewer (gl_test `dtpu` verdict code 3,
/// docs/notes/gc-depth-sample-wire.md). NEVER (op 0) and ALWAYS (op 7) give a
/// constant whatever the depth, so they take the view; so does a binding with
/// compare_op 0 that was built from `GlesTextureBind::ZERO`.
pub fn exact_depth_reader(t: &GlesTextureBind) -> bool {
    t.compare_op & 16 == 0 && !matches!(t.compare_op & 7, 0 | 7)
}

/// Is `t`'s GL_TEXTURE_SWIZZLE_* the identity (every component reads its
/// own channel, [`swizzle_source`])?
fn identity_swizzle(t: &GlesTextureBind) -> bool {
    (0..4).all(|k| swizzle_source(k, t.swizzle[k]) == k)
}

/// The binding `emit_slot` describes instead of `t` (whose texels are
/// stale) in `mode`, for the surface `v`. -ENOTSUP when there is no surface,
/// the texture does not fit inside it, or the mode is not 1, 2 or 4 (4 is
/// mode 1's view); and, in
/// either mode, for an [`exact_depth_reader`] (a shadow compare must read the
/// resolve's 24 bits, not a view that may hold fewer), and in mode 2 for a
/// non-identity swizzle (the raw view's channels are bytes of the word, not
/// components a GL swizzle names). Mode 1 keeps `t`'s swizzle: `emit_slot`'s
/// `texture::descriptor_swizzle` composes it with the view's own (R, R, R, 1),
/// as it composes the R32F texels' swizzle with `depth_luminance`, so an
/// ES 2.0 depth texture (`depth_luminance` 1, base (d, d, d, 1)) reads through
/// the view what its resolved texels read, whatever its swizzle. (An ES 3.0
/// binding, `depth_luminance` 0, reads (d, 0, 0, 1) from its texels and
/// (d, d, d, 1) from the view, swizzled or not, as before this guard; a
/// shader that reads only `.r`, glmark2's included, sees no difference.)
///
/// A refusal here fails the draw by name: the GL core already left the
/// texels stale on [`gc_depth_sampled`]'s yes, which cannot see the compare
/// mode (`samples_rendered_depth(id, w, h)` carries neither it nor the unit
/// whose sampler object may set it). Keeping such a reader on the CPU resolve
/// needs the GL core to ask with the binding's compare (src/gl, #397).
/// CPU readbacks are not affected: every CPU read or write of the texels
/// resolves them first (`gl::texture::settle_texture` calls `resolve_depth`).
pub fn view_binding(t: &GlesTextureBind, mode: i32, v: Option<DepthView>) -> Result<GlesTextureBind, i32> {
    let Some(v) = v.filter(|v| v.covers(t.width, t.height)) else {
        return Err(-ENOTSUP);
    };
    if exact_depth_reader(t) || (mode == DEPTH_TPU_RAW8888 && !identity_swizzle(t)) {
        return Err(-ENOTSUP);
    }
    /* one 2D level at the surface: LINEAR (no `cpu`, so no tiled copy),
     * LINEAR_STRIDE = stride_texels * 4 = the surface's stride */
    let at = GlesTextureBind {
        type_: GLES_TEXTYPE_2D,
        layers: 1,
        base_gpu: v.gpu,
        stride_texels: v.stride / 4,
        bytes: v.stride * v.rows,
        mip_filter: 0,
        mip_levels: 1,
        bgra: 0,
        depth_id: 0,
        cpu: core::ptr::null(),
        swizzle: [0; 4],
        ..*t
    };
    match mode {
        /* (d, d, d, 1): R in G and B (`depth_luminance`), A 1, with the
         * texture's swizzle on top; mode 4 is the same view (its flush is
         * in the stream, section "Mode 4") */
        DEPTH_TPU_D24X8 | DEPTH_TPU_D24X8_FLUSH => Ok(GlesTextureBind {
            format: GC_TEXFMT_DEPTH_VIEW,
            depth_luminance: 1,
            no_alpha: 1,
            swizzle: t.swizzle,
            ..at
        }),
        /* the word's bytes as they are: NEAREST (a filter across bytes is
         * not a filter of the depth), identity swizzle */
        DEPTH_TPU_RAW8888 => Ok(GlesTextureBind {
            format: GLES_TEXFMT_RGBA8,
            bgra: 1,
            depth_luminance: 0,
            no_alpha: 0,
            linear_filter: 0,
            min_linear: 0,
            ..at
        }),
        _ => Err(-ENOTSUP),
    }
}

/// [`view_binding`] for a binding with `depth_id != 0` in the current mode,
/// keeping the surface for `kick`. A refusal is logged once.
pub fn binding(t: &GlesTextureBind, kick: u32) -> Result<GlesTextureBind, i32> {
    let mode = mode();
    if mode == DEPTH_TPU_RSCOPY {
        return copy_binding_live(t, kick);
    }
    let v = view(t.depth_id, kick);
    let b = view_binding(t, mode, v);
    match (&b, v) {
        (Ok(_), Some(v)) => crate::glog_once!(
            "vivante: dtpu mode {mode}: a {}x{} depth texture sampled from its depth surface (stride {})",
            t.width,
            t.height,
            v.stride
        ),
        (Err(_), Some(v)) if v.covers(t.width, t.height) => crate::glog_once!(
            "vivante: dtpu mode {mode}: a {}x{} depth texture with a depth compare (op {}) or a swizzle the view cannot keep; the draw is refused",
            t.width,
            t.height,
            t.compare_op
        ),
        _ => crate::glog_once!(
            "vivante: dtpu mode {mode}: a {}x{} depth texture does not fit a live depth surface; the draw is refused",
            t.width,
            t.height
        ),
    }
    b
}

/* ---- Mode 4: mode 1 with a flush in front of the sampling draw ---- */

/// Knob mode 4, for verdict code 2 of gl_test `dtpu` (check 7 fails: the
/// view reads stale depth; gc-depth-sample-wire.md (a).10, "Check 7 fails").
/// Everything of [`DEPTH_TPU_D24X8`] (the answer, the D24X8 view, its
/// refusals), and in front of the first draw that samples a depth view whose
/// surface a draw wrote or cleared since the last such flush, the words of
/// [`emit_view_flush`]. A board arm of case 222 that sets 4 tells whether
/// that flush cures check 7 (gl_test does not set 4 today; the arm is
/// driver/samples/zephyr's). The X2's code 3 with check 7 failing made it
/// the product default ([`DEPTH_TPU_DEFAULT`]).
pub const DEPTH_TPU_D24X8_FLUSH: i32 = 4;

/// How many written depth objects [`Written`] names before it gives up and
/// counts every one as written.
const WRITTEN_MAX: usize = 8;

/// The depth objects a recorded draw wrote or cleared since the last flush
/// of [`emit_view_flush`] (mode 4 only). A flush writes every plane back, so
/// it empties the whole set; `all` stands for a set that ran out of room.
struct Written {
    ids: [u32; WRITTEN_MAX],
    n: usize,
    all: bool,
}

static WRITTEN: crate::util::Global<Written> =
    crate::util::Global::new(Written { ids: [0; WRITTEN_MAX], n: 0, all: false });

fn written() -> &'static mut Written {
    // SAFETY: single-threaded by contract; no caller holds a reference
    // across a call that takes another.
    unsafe { WRITTEN.get() }
}

/// Does draw `d` sample (in either stage) a depth view whose surface was
/// written since the last flush? No in every mode but 4.
fn needs_view_flush(d: &crate::backend::GlesDraw) -> bool {
    if mode() != DEPTH_TPU_D24X8_FLUSH {
        return false;
    }
    let w = written();
    let fs = &d.textures[..(d.num_textures as usize).min(d.textures.len())];
    let vs = &d.vs_textures[..(d.num_vs_textures as usize).min(d.vs_textures.len())];
    fs.iter().chain(vs).any(|t| t.depth_id != 0 && (w.all || w.ids[..w.n].contains(&t.depth_id)))
}

/// Mode 4: record into `c`, in front of draw `d`'s own state (after any
/// restore clear `backend::draw_one` records for it), the flush that makes a
/// depth write visible to the texture unit, when [`needs_view_flush`]:
///
/// 1. `FLUSH_CACHE DEPTH`: the PE's depth cache written back (the note's
///    `FLUSH_DEPTH`; Mesa flushes DEPTH for a depth-state or framebuffer
///    change, etnaviv_emit.c:264-265, :274-275);
/// 2. the FE-PE semaphore and the front end's STALL: nothing behind it is
///    fetched before the pixel engine drained (the note's FE-PE stall; Mesa's
///    `stall_before_flush` of a texture-cache flush, etnaviv_emit.c:269-270,
///    :284-285, as `etna_stall` writes it, :66-72);
/// 3. `FLUSH_CACHE TEXTURE`, then 4. `FLUSH_CACHE TEXTUREVS` on its own:
///    no texture line read before the write stays (Mesa's
///    `ETNA_DIRTY_TEXTURE_CACHES`, etnaviv_emit.c:266-268, :287-290, and
///    `etna_texture_barrier`, etnaviv_texture.c:414-418);
/// 5. the RA-PE semaphore and stall token behind the flush (etnaviv_emit.c:291,
///    etnaviv_texture.c:419).
///
/// The draw's own `emit_flushes` follows unchanged. Read-only on the set, so
/// a draw recorded again after an overflow records the flush again;
/// [`view_drawn`] empties it once the draw is in the stream.
pub(super) fn emit_view_flush(d: &crate::backend::GlesDraw, c: &mut super::cmd::GcCmd) {
    use super::regs::{
        GC_FLUSH_DEPTH, GC_FLUSH_TEXTURE, GC_FLUSH_TEXTUREVS, GC_GL_FLUSH_CACHE, GC_SYNC_FE, GC_SYNC_PE, GC_SYNC_RA,
    };
    if !needs_view_flush(d) {
        return;
    }
    c.state(GC_GL_FLUSH_CACHE, GC_FLUSH_DEPTH);
    c.stall(GC_SYNC_FE, GC_SYNC_PE);
    c.state(GC_GL_FLUSH_CACHE, GC_FLUSH_TEXTURE);
    c.state(GC_GL_FLUSH_CACHE, GC_FLUSH_TEXTUREVS);
    c.stall(GC_SYNC_RA, GC_SYNC_PE);
}

/// Mode 4: draw `d` is in the stream. If [`emit_view_flush`] recorded the
/// flush for it, every write before is visible and the set is emptied; then
/// depth object `depth_id` (the draw's depth attachment, 0 for none) joins
/// it when the draw `wrote` its depth plane (a write, a clear or a restore).
/// Nothing in any other mode.
pub(super) fn view_drawn(d: &crate::backend::GlesDraw, depth_id: u32, wrote: bool) {
    if mode() != DEPTH_TPU_D24X8_FLUSH {
        return;
    }
    let flushed = needs_view_flush(d);
    let w = written();
    if flushed {
        w.n = 0;
        w.all = false;
    }
    if !wrote || depth_id == 0 || w.all || w.ids[..w.n].contains(&depth_id) {
        return;
    }
    if w.n == WRITTEN_MAX {
        w.all = true;
    } else {
        w.ids[w.n] = depth_id;
        w.n += 1;
    }
}

/* ---- Outcome (b).1: the RS copy into a supertiled depth view ---- */

use super::backend::RsBlit;
use super::layout::{self, Layout};
use super::regs::{
    rs_dest_format, rs_source_format, texdesc_addressing_mode, texdesc_format, texdesc_halign, texdesc_size_height,
    texdesc_size_width, texdesc_swizzle_a, texdesc_swizzle_b, texdesc_swizzle_g, texdesc_swizzle_r, texdesc_type,
    GC_RS_DEST_STRIDE_SUPER_TILED_NEW, GC_RS_DEST_STRIDE_TILING, GC_RS_DEST_TILED, GC_RS_FORMAT_A8R8G8B8, GC_SWIZZLE_1,
    GC_SWIZZLE_R, GC_TEXDESC_CONFIG2_BASE, GC_TEXTURE_HALIGN_SUPER_TILED, GC_TEX_ADDRESSING_TILED, GC_TEX_TYPE_2D,
};

/// Bytes of one D24S8 word, and of one A8R8G8B8 pixel the RS moves it as.
const DS_BPP: u32 = 4;

/// Where the copy and the surface must start: 64 bytes, the PE's and the
/// texture unit's level alignment (`ETNA_PE_ALIGNMENT`,
/// etnaviv_resource.c:351-352; `GC_TEXDESC_LOD_ALIGN`). The surface already
/// is (`ds_cache`'s allocation, gc-depth-sample-wire.md (a).2).
pub const COPY_ALIGN: u64 = 64;

/// Bytes of (b).1's buffer for surface `v`: `stride * rows`, the surface's own
/// shape (gc-depth-sample-wire.md (b).1, "The copy"). A SUPER_TILED level is
/// padded to 64x64 (etnaviv_resource.c:389-392), its stride is
/// `padded_width * bpp` and its size `stride * padded_height`
/// (etnaviv_resource.c:327-331), and `ds_cache`'s surface is 64-pixel
/// aligned both ways, so the same bytes hold it. `None` for a shape
/// [`copy_blit`] refuses.
pub fn copy_bytes(v: &DepthView) -> Option<usize> {
    shape_ok(v).then(|| v.stride as usize * v.rows as usize)
}

/// A surface the copy can take: a non-zero row of whole super-tiles (the
/// stride a multiple of `64 * 4` bytes, so also of the RS's 16-pixel width,
/// etnaviv_rs.c:91-92, etnaviv_internal.h:55), whole super-tile rows, a
/// window the 16-bit RS_WINDOW_SIZE fields hold, and an address the 32-bit RS
/// address registers hold.
fn shape_ok(v: &DepthView) -> bool {
    let row = layout::SUPER_TILE * DS_BPP;
    v.stride != 0
        && v.rows != 0
        && v.stride.is_multiple_of(row)
        && v.rows.is_multiple_of(layout::SUPER_TILE)
        && v.stride / DS_BPP <= 0xffff
        && v.rows <= 0xffff
        && v.gpu.is_multiple_of(COPY_ALIGN)
        && v.gpu + u64::from(v.stride) * u64::from(v.rows) <= 1 << 32
}

/// (b).1's resolve-engine copy of the row-linear D24S8 surface `v` into the
/// SUPER_TILED buffer at `dst` ([`copy_bytes`] long), for
/// `super::backend::emit_rs` (its flush, RA-PE stall, dither off and
/// SINGLE_BUFFER around the kicker are that function's, as for every RS copy
/// here). Each word:
///
/// - `config`: `RS_FORMAT_A8R8G8B8` (6, state_3d.xml.h:82) as source and
///   destination format, Mesa's "compatible" 4-byte format for a copy
///   (`etna_compatible_rs_format`, etnaviv_rs.c:55-66, used for a blit at
///   :746): the 32-bit words move unchanged, depth in bits 31..8, stencil in
///   7..0. `DEST_TILED` (0x4000, state_3d.xml.h:1360) because the destination
///   is tiled (`ETNA_LAYOUT_BIT_TILE`, etnaviv_rs.c:80, :95-102); no
///   `SOURCE_TILED`, no downsample, no `SWAP_RB`, no flip.
/// - `src_stride`: the surface's row stride in bytes as it is, a LINEAR side
///   shifting by 0 (etnaviv_rs.c:77, :104-106).
/// - `dst_stride`: the buffer's stride times 4 (a tiled side's shift 2,
///   etnaviv_rs.c:78, :130), `TILING` (0x8000_0000, state_3d.xml.h:1385)
///   for a super-tiled side (`ETNA_LAYOUT_BIT_SUPER`, :82, :130-132) and
///   `SUPER_TILED_NEW` (0x0800_0000, state_3d.xml.h:1383) on this
///   CACHE128B256BPERLINE part (:134-135). This is
///   gc-depth-sample-wire.md (b).1's `(S << 2) | TILING | SUPER_TILED_NEW`,
///   and Mesa's own resolve source word 0xa8000400 for a 64-wide super-tiled
///   surface less its `TS_MODE` (tools/vivante/README.md, the ts-golden
///   table, "the resolve").
/// - `w`, `h`: the whole surface, `S / 4` by `rows` (the note's window), a
///   multiple of 16 wide and 4 high (etnaviv_internal.h:55-57).
///
/// `None` for a surface [`shape_ok`] refuses or a `dst` that is not on
/// [`COPY_ALIGN`] or past 32 bits.
pub(super) fn copy_blit(v: &DepthView, dst: u64) -> Option<RsBlit> {
    let bytes = copy_bytes(v)? as u64;
    if !dst.is_multiple_of(COPY_ALIGN) || dst + bytes > 1 << 32 {
        return None;
    }
    let fmt = GC_RS_FORMAT_A8R8G8B8;
    Some(RsBlit {
        config: rs_source_format(fmt) | rs_dest_format(fmt) | GC_RS_DEST_TILED,
        src: v.gpu as u32,
        src_stride: v.stride,
        dst: dst as u32,
        dst_stride: (v.stride << 2) | GC_RS_DEST_STRIDE_TILING | GC_RS_DEST_STRIDE_SUPER_TILED_NEW,
        w: v.stride / DS_BPP,
        h: v.rows,
    })
}

/// Every `(register, value)` pair `super::backend::emit_rs` writes for
/// [`copy_blit`]'s copy of `v` to `dst`, in order (the flush and stall in front
/// included, as LOAD_STATE writes; a STALL is not a register write and is
/// left out), for host tests. Read-only: it records the stream into a scratch
/// buffer and changes no state. `-ENOTSUP` when [`copy_blit`] refuses.
#[cfg(any(test, feature = "host-test"))]
pub fn copy_rs_writes(v: &DepthView, dst: u64) -> Result<std::vec::Vec<(u32, u32)>, i32> {
    use super::cmd::GcCmd;
    let b = copy_blit(v, dst).ok_or(-ENOTSUP)?;
    let mut buf = std::vec![0u32; 256];
    let mut c = GcCmd { words: buf.as_mut_ptr(), used: 0, cap: buf.len() as u32, overflow: false };
    super::backend::emit_rs(&mut c, &b);
    if c.overflow {
        return Err(-ENOTSUP);
    }
    let words = &buf[..c.used as usize];
    let mut out = std::vec::Vec::new();
    let mut i = 0usize;
    while i < words.len() {
        let w = words[i];
        if w & 0xf800_0000 != super::regs::GC_OP_LOAD_STATE {
            return Err(-ENOTSUP);
        }
        let count = ((w >> 16) & 0x3ff) as usize;
        let base = (w & 0xffff) * 4;
        for k in 0..count {
            out.push((base + 4 * k as u32, words[i + 1 + k]));
        }
        i += 1 + count + usize::from(count & 1 == 0);
    }
    Ok(out)
}

/// The texture descriptor words that sample (b).1's copy: CONFIG0, CONFIG1,
/// CONFIG2, LINEAR_STRIDE, SLICE, SIZE and the level 0 address. The rest of
/// the descriptor (log size, base LOD 0 with max 0, 3D config, the other LOD
/// addresses 0) and the sampler state (gc-depth-sample-wire.md (a).8:
/// `samp_ctrl0` unchanged, NEAREST, MIP NONE, no INT_FILTER) are the
/// texture's, as for any one-level 2D texture.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct CopySamplerWords {
    pub config0: u32,
    pub config1: u32,
    pub config2: u32,
    pub linear_stride: u32,
    pub slice: u32,
    pub size: u32,
    pub lod0: u64,
}

impl CopySamplerWords {
    /// The layout these words address: SUPER_TILED when ADDRESSING is TILED
    /// and HALIGN is SUPER_TILED (super-tiling is told by HALIGN, not by the
    /// addressing mode: etnaviv_texture_desc.c:182-186, tex_layout.rs).
    pub fn layout(&self) -> Option<Layout> {
        let addressing = (self.config0 >> 20) & 3;
        let halign = (self.config1 >> 26) & 7;
        match (addressing, halign) {
            (GC_TEX_ADDRESSING_TILED, GC_TEXTURE_HALIGN_SUPER_TILED) => Some(Layout::SuperTiled),
            _ => None,
        }
    }

    /// The GPU address of texel `(x, y)` of level 0 through these words.
    pub fn texel_addr(&self, x: u32, y: u32) -> Option<u64> {
        let l = self.layout()?;
        Some(self.lod0 + layout::offset(l, x, y, DS_BPP, self.linear_stride) as u64)
    }
}

/// The words for sampling texture `t` (its `width` x `height`, wraps and
/// filters stay the texture's) from (b).1's copy at `copy` of surface `v`.
/// Each word (Mesa `etna_emit_texture_desc` builds them,
/// etnaviv_texture_desc.c:178-205):
///
/// - CONFIG0: TYPE 2D (`GC_TEX_TYPE_2D`), FORMAT `TEXTURE_FORMAT_D24X8`
///   (0x11, common_3d.xml.h:60; etnaviv_format.c:179-180 for X8Z24_UNORM and
///   S8_UINT_Z24_UNORM), ADDRESSING TILED (0, common_3d.xml.h:130): the
///   LINEAR mode is written only for a LINEAR resource
///   (etnaviv_texture_desc.c:180-182), and Mesa's depth resource is
///   SUPER_TILED (etnaviv_resource.c:572-576). The same as `texture.rs`'s
///   `GC_TEXFMT_DEPTH_VIEW` class with `tiled` set.
/// - CONFIG1: no FORMAT_EXT (D24X8 is a CONFIG0 format), HALIGN
///   `TEXTURE_HALIGN_SUPER_TILED` (2, common_3d.xml.h:125; the resource's
///   halign, etnaviv_resource.c:392, written at etnaviv_texture_desc.c:186;
///   gc-depth-sample-wire.md (b).1, "The descriptor"), swizzle (R, R, R, 1):
///   [`view_binding`]'s mode 1, the ES 2.0 depth texture as luminance
///   (gc-depth-sample-wire.md (a).6's table, its first row).
/// - CONFIG2: `0x00030000` (etnaviv_texture_desc.c:187), no signed-integer
///   bit (D24X8 is not an integer format).
/// - LINEAR_STRIDE: the copy's row stride, `S`, which is
///   `levels[0].stride` of a 64-padded super-tiled level
///   (etnaviv_texture_desc.c:190, etnaviv_resource.c:329; the note's
///   "LINEAR_STRIDE stays S").
/// - SLICE: `levels[0].layer_stride`, `S * align(height, 64)`
///   (etnaviv_texture_desc.c:192, etnaviv_resource.c:331).
/// - SIZE: the texture's `width | height << 16` (etnaviv_texture_desc.c:202-203).
/// - LOD 0: `copy` (etnaviv_texture_desc.c:204-205, offset 0).
///
/// -ENOTSUP when the texture does not lie inside the surface
/// ([`DepthView::covers`]), the surface's shape is one [`copy_blit`] refuses,
/// or `copy` is not on [`COPY_ALIGN`].
pub fn copy_sampler_words(t: &GlesTextureBind, v: &DepthView, copy: u64) -> Result<CopySamplerWords, i32> {
    if !v.covers(t.width, t.height) || !shape_ok(v) || !copy.is_multiple_of(COPY_ALIGN) {
        return Err(-ENOTSUP);
    }
    let config0 = texdesc_type(GC_TEX_TYPE_2D)
        | texdesc_format(GC_TEX_FORMAT_D24X8)
        | texdesc_addressing_mode(GC_TEX_ADDRESSING_TILED);
    let config1 = texdesc_swizzle_r(GC_SWIZZLE_R)
        | texdesc_swizzle_g(GC_SWIZZLE_R)
        | texdesc_swizzle_b(GC_SWIZZLE_R)
        | texdesc_swizzle_a(GC_SWIZZLE_1)
        | texdesc_halign(GC_TEXTURE_HALIGN_SUPER_TILED);
    Ok(CopySamplerWords {
        config0,
        config1,
        config2: GC_TEXDESC_CONFIG2_BASE,
        linear_stride: v.stride,
        slice: v.stride * layout::align_up(t.height, layout::SUPER_TILE),
        size: texdesc_size_width(t.width) | texdesc_size_height(t.height),
        lod0: copy,
    })
}

/* ---- Mode 3: (b).1 wired behind the knob ---- */

use super::cmd::GcCmd;
use super::depth_copy::{CopyPool, OsalHost};
use crate::backend::GlesDraw;
use crate::util::Global;

/// The (b).1 buffers of mode 3, one per sampled depth object
/// (`depth_copy.rs`). Empty in every other mode.
static POOL: Global<CopyPool> = Global::new(CopyPool::new());

fn pool() -> &'static mut CopyPool {
    // SAFETY: single-threaded by contract; no caller holds a reference
    // across a call that takes another.
    unsafe { POOL.get() }
}

/// The live depth plane of `id` and its generation; `kick` != 0 keeps the
/// surface until that kick completed.
fn view_gen(id: u32, kick: u32) -> Option<(DepthView, u32)> {
    let (gpu, stride, rows, gen) = super::ds_cache::depth_view_gen(id, super::backend::GC_DEPTH_GUARD_BYTES, kick)?;
    Some((DepthView { gpu, stride, rows }, gen))
}

/// Mode 3's answer for `id`: its live surface covers `w` x `h`, the RS can
/// copy it, and its buffer is in the pool. Each refusal is logged once and
/// leaves the draw on the CPU resolve.
fn copy_ready(id: u32, w: u32, h: u32) -> bool {
    let Some((v, gen)) = view_gen(id, 0).filter(|(v, _)| v.covers(w, h)) else {
        return false;
    };
    if copy_bytes(&v).is_none() {
        crate::glog_once!(
            "vivante: dtpu mode 3: a {w}x{h} depth surface (stride {}, {} rows) the RS cannot copy; the CPU resolves it",
            v.stride,
            v.rows
        );
        return false;
    }
    match pool().buffer(&mut OsalHost, id, gen, &v) {
        Ok(_) => true,
        Err(e) => {
            crate::glog_once!(
                "vivante: dtpu mode 3: no depth-copy buffer for a {w}x{h} depth texture ({e}); the CPU resolves it"
            );
            false
        }
    }
}

/// The binding `emit_slot` describes in mode 3 instead of `t` (whose texels
/// are stale): the D24X8 view of mode 1 ([`view_binding`], so `t`'s
/// swizzle and the [`exact_depth_reader`] refusal are the same), at `copy`,
/// (b).1's SUPER_TILED copy of the surface `v`, as [`GC_TEXFMT_DEPTH_COPY`]
/// (`texture::build_descriptor` writes ADDRESSING TILED, HALIGN SUPER_TILED
/// and the copy's SLICE for it, the words of [`copy_sampler_words`]).
/// -ENOTSUP when there is no surface or no copy, the texture does not fit,
/// the shape or the address is one the copy refuses, or for an exact-depth
/// reader.
pub fn copy_binding(t: &GlesTextureBind, v: Option<DepthView>, copy: Option<u64>) -> Result<GlesTextureBind, i32> {
    let (Some(v), Some(copy)) = (v, copy) else {
        return Err(-ENOTSUP);
    };
    copy_sampler_words(t, &v, copy)?;
    let b = view_binding(t, DEPTH_TPU_D24X8, Some(v))?;
    Ok(GlesTextureBind { format: GC_TEXFMT_DEPTH_COPY, base_gpu: copy, ..b })
}

/// [`binding`] in mode 3: the live buffer of `t.depth_id` at its surface's
/// generation, stamped with `kick` (not freed before that kick completed).
fn copy_binding_live(t: &GlesTextureBind, kick: u32) -> Result<GlesTextureBind, i32> {
    let id = t.depth_id;
    let vg = view_gen(id, kick);
    let copy = vg.and_then(|(_, gen)| pool().live(id).filter(|l| l.1 == gen).map(|l| l.0));
    let b = copy_binding(t, vg.map(|(v, _)| v), copy);
    match (&b, vg) {
        (Ok(_), Some((v, _))) => {
            pool().note_read(id, kick);
            crate::glog_once!(
                "vivante: dtpu mode 3: a {}x{} depth texture sampled from its RS copy (stride {})",
                t.width,
                t.height,
                v.stride
            );
        }
        _ => crate::glog_once!(
            "vivante: dtpu mode 3: a {}x{} depth texture has no RS copy it can sample (no live surface or buffer, a depth compare, or a shape the copy refuses); the draw is refused",
            t.width,
            t.height
        ),
    }
    b
}

/// Mode 3: record into `c`, before draw `d` records anything, one RS copy of
/// each depth surface its bindings sample (each object once) into its pool
/// buffer (the one [`gc_depth_sampled`] made for the surface's generation),
/// and stamp the buffer and the surface with `kick`, the kick being
/// recorded (`backend::draw_one`'s `record_all`, so a draw recorded again in
/// a new frame records the copy again). `emit_rs`'s colour and depth flush
/// and RA-PE stall put the depth the PE wrote in memory first; the draw's own
/// `emit_flushes` (texture cache included) and stall come after the copy. A
/// binding with no copy here is refused by name at [`binding`]. Nothing in
/// any other mode.
pub(super) fn emit_copies(d: &GlesDraw, c: &mut GcCmd, kick: u32) {
    if mode() != DEPTH_TPU_RSCOPY {
        return;
    }
    let fs = &d.textures[..(d.num_textures as usize).min(d.textures.len())];
    let vs = &d.vs_textures[..(d.num_vs_textures as usize).min(d.vs_textures.len())];
    let mut done = [0u32; 64];
    let mut n = 0usize;
    for t in fs.iter().chain(vs) {
        let id = t.depth_id;
        if id == 0 || t.incomplete != 0 || done[..n].contains(&id) {
            continue;
        }
        if n < done.len() {
            done[n] = id;
            n += 1;
        }
        /* the buffer [`gc_depth_sampled`] made: no allocation and no kick
         * query here, with the stream open */
        let Some((v, gen)) = view_gen(id, kick) else { continue };
        let Some((dst, _, _)) = pool().live(id).filter(|l| l.1 == gen) else { continue };
        let Some(b) = copy_blit(&v, dst) else { continue };
        super::backend::emit_rs(c, &b);
        pool().note_read(id, kick);
    }
}

/// The depth contents of `id` are gone (`gc_attachment_gone`): its copy
/// buffer is retired, and freed once no pending kick reads it.
pub(super) fn copy_gone(id: u32) {
    pool().gone(&mut OsalHost, id);
}

/// Free the retired copy buffers whose kicks completed (`finish`,
/// `finish_previous`).
pub(super) fn reap_copies() {
    pool().reap(&mut OsalHost);
}

/// Free every copy buffer (backend shutdown).
pub(super) fn release_copies() {
    pool().release_all(&mut OsalHost);
}

/// Mode 3's buffer of depth object `id` (host tests): its GPU address, the
/// surface generation it was made for, and the kick that last read it.
pub fn copy_live(id: u32) -> Option<(u64, u32, u32)> {
    pool().live(id)
}

/// Mode 3's buffers held, live and retired (host tests).
pub fn copies_held() -> usize {
    pool().held()
}

#[cfg(test)]
mod copy_tests {
    use super::*;

    /// The product default is the D24X8 view with its texture-cache flush
    /// (mode 4): the Armadillo-X2's `dtpu` verdict code 3 with check 7
    /// failing (docs/notes/gc-depth-sample-wire.md). Changing it is a
    /// deliberate edit of this test.
    #[test]
    fn product_default_is_depth_view_with_flush() {
        assert_eq!(DEPTH_TPU_DEFAULT, DEPTH_TPU_D24X8_FLUSH);
        assert_eq!(DEPTH_TPU_DEFAULT, 4);
        #[cfg(not(feature = "test-knobs"))]
        assert_eq!(mode(), DEPTH_TPU_D24X8_FLUSH, "no knob: the default");
    }
    use crate::ds_retention::unorm24_depth;
    use crate::vivante::cmd::GcCmd;
    use crate::vivante::regs::*;
    use crate::vivante::rs_interp::{Memory, RsInterp};
    use std::vec::Vec;

    const SURF: u64 = 0x0010_0000;
    const COPY: u64 = 0x0040_0000;

    /// `ds_cache`'s surface for a `w` x `h` depth texture: 64-pixel aligned
    /// row stride and rows (tests/gc_depth_resolve.rs: 90x70 is 128 words by
    /// 128 rows).
    fn surface(w: u32, h: u32) -> DepthView {
        DepthView { gpu: SURF, stride: layout::align_up(w, 64) * 4, rows: layout::align_up(h, 64) }
    }

    fn texture(w: u32, h: u32) -> GlesTextureBind {
        GlesTextureBind {
            width: w,
            height: h,
            type_: GLES_TEXTYPE_2D,
            layers: 1,
            mip_levels: 1,
            ..GlesTextureBind::ZERO
        }
    }

    /// The D24S8 word at `(x, y)`: a depth of 24 varied bits, a stencil byte
    /// that changes too, so a copy that dropped or moved either would show.
    fn word(x: u32, y: u32) -> u32 {
        let depth = (x.wrapping_mul(0x9e37) ^ y.wrapping_mul(0x7f4a_7c15) ^ (x << 13)) & 0xff_ffff;
        (depth << 8) | ((x * 3 + y * 5) & 0xff)
    }

    /// ds_cache's CPU resolve of a word, written out as tests/gc_depth_resolve.rs's
    /// bit-exact reference states it: `(word >> 8)` over 2^24 - 1, as f32.
    fn cpu_resolve(w: u32) -> f32 {
        ((w >> 8) as f64 / 16_777_215.0) as f32
    }

    /// Every `(register, value)` a LOAD_STATE of `words` writes, in order (a
    /// STALL from the FE is not in an RS copy).
    fn writes(words: &[u32]) -> Vec<(u32, u32)> {
        let mut out = Vec::new();
        let mut i = 0usize;
        while i < words.len() {
            let w = words[i];
            assert_eq!(w & 0xf800_0000, GC_OP_LOAD_STATE, "word {i}: only LOAD_STATEs expected");
            let count = ((w >> 16) & 0x3ff) as usize;
            let base = (w & 0xffff) * 4;
            for k in 0..count {
                out.push((base + 4 * k as u32, words[i + 1 + k]));
            }
            i += 1 + count + if count & 1 == 0 { 1 } else { 0 };
        }
        out
    }

    /// The stream `emit_rs` writes for `b`, decoded.
    fn stream(b: &RsBlit) -> Vec<(u32, u32)> {
        let mut buf = vec![0u32; 256];
        let mut c = GcCmd { words: buf.as_mut_ptr(), used: 0, cap: buf.len() as u32, overflow: false };
        super::super::backend::emit_rs(&mut c, b);
        assert!(!c.overflow);
        writes(&buf[..c.used as usize])
    }

    #[test]
    fn the_copy_words_are_the_notes_for_a_64_square_surface() {
        let v = surface(64, 64);
        assert_eq!((v.stride, v.rows), (256, 64));
        assert_eq!(copy_bytes(&v), Some(256 * 64));
        let b = copy_blit(&v, COPY).expect("a 64x64 surface is copied");
        assert_eq!(b.config, 0x0000_4606, "A8R8G8B8 both sides (6 | 6 << 8), DEST_TILED, nothing else");
        assert_eq!(b.config, rs_source_format(6) | rs_dest_format(6) | GC_RS_DEST_TILED);
        assert_eq!(b.config & (GC_RS_SOURCE_TILED | GC_RS_SWAP_RB | GC_RS_CONFIG_FLIP), 0);
        assert_eq!(b.config & (GC_RS_DOWNSAMPLE_X | GC_RS_DOWNSAMPLE_Y), 0);
        assert_eq!(b.src_stride, 256, "a LINEAR source: the row stride as it is");
        assert_eq!(b.dst_stride, (256 << 2) | 0x8000_0000 | 0x0800_0000);
        /* Mesa's resolve of a 64-wide super-tiled surface, less TS_MODE */
        assert_eq!(b.dst_stride, 0xa800_0400 & !0x2000_0000);
        assert_eq!((b.src, b.dst), (SURF as u32, COPY as u32));
        assert_eq!((b.w, b.h), (64, 64));
    }

    #[test]
    fn the_window_is_the_whole_padded_surface() {
        /* glmark2's 1920x1080 shadow and refract maps, a 90x70 one, a 1x1 one */
        for (w, h, sw, sh) in [(1920, 1080, 1920, 1088), (90, 70, 128, 128), (1, 1, 64, 64)] {
            let v = surface(w, h);
            let b = copy_blit(&v, COPY).unwrap();
            assert_eq!((b.w, b.h), (sw, sh), "{w}x{h}");
            assert!(b.w.is_multiple_of(16) && b.h.is_multiple_of(4), "an RS window");
            assert_eq!(b.dst_stride & 0x0003_ffff, v.stride << 2);
            assert_eq!(copy_bytes(&v), Some((sw * 4 * sh) as usize));
        }
    }

    #[test]
    fn a_shape_or_address_the_copy_cannot_take_has_no_words() {
        let ok = surface(64, 64);
        for v in [
            DepthView { stride: 0, ..ok },
            DepthView { rows: 0, ..ok },
            DepthView { stride: 4 * 48, ..ok },
            DepthView { rows: 60, ..ok },
            DepthView { gpu: SURF + 4, ..ok },
            DepthView { gpu: 0xffff_f000, ..ok },
            DepthView { stride: 4 * 64 * 1024, ..ok },
        ] {
            assert_eq!(copy_bytes(&v), None, "{v:?}");
            assert_eq!(copy_blit(&v, COPY), None, "{v:?}");
            assert_eq!(copy_sampler_words(&texture(1, 1), &v, COPY), Err(-ENOTSUP), "{v:?}");
        }
        assert_eq!(copy_blit(&ok, COPY + 32), None, "a misaligned copy");
        assert_eq!(copy_blit(&ok, (1 << 32) - 0x1000), None, "a copy past 32 bits");
        assert_eq!(copy_sampler_words(&texture(64, 64), &ok, COPY + 32), Err(-ENOTSUP));
        assert_eq!(copy_sampler_words(&texture(65, 64), &ok, COPY), Err(-ENOTSUP), "wider than the surface");
        assert_eq!(copy_sampler_words(&texture(64, 65), &ok, COPY), Err(-ENOTSUP), "taller than the surface");
        assert_eq!(copy_sampler_words(&texture(0, 64), &ok, COPY), Err(-ENOTSUP));
    }

    #[test]
    fn the_sampler_words_are_d24x8_super_tiled_at_the_copy() {
        let v = surface(90, 70);
        let s = copy_sampler_words(&texture(90, 70), &v, COPY).unwrap();
        assert_eq!((s.config0 >> 13) & 0x1f, 0x11, "FORMAT D24X8");
        assert_eq!((s.config0 >> 20) & 3, GC_TEX_ADDRESSING_TILED, "ADDRESSING TILED");
        assert_eq!(s.config0 & 7, GC_TEX_TYPE_2D, "TYPE 2D");
        assert_eq!(s.config1 & 0x3f, 0, "no FORMAT_EXT");
        assert_eq!((s.config1 >> 26) & 7, 2, "HALIGN SUPER_TILED");
        assert_eq!(s.config1, texdesc_swizzle_a(GC_SWIZZLE_1) | texdesc_halign(2), "swizzle (R, R, R, 1)");
        assert_eq!(s.config2, 0x0003_0000);
        assert_eq!(s.linear_stride, 512);
        assert_eq!(s.slice, 512 * 128);
        assert_eq!(s.size, 90 | 70 << 16);
        assert_eq!(s.lod0, COPY);
        assert_eq!(s.layout(), Some(Layout::SuperTiled));
    }

    /// The words equal `texture.rs`'s for the in-place view of mode 1
    /// ([`view_binding`]) described as tiled, with HALIGN SUPER_TILED: the
    /// one difference the note's (b).1 names besides the address.
    #[test]
    fn the_sampler_words_are_the_depth_views_but_for_the_layout() {
        let v = surface(64, 32);
        let t = texture(64, 32);
        let s = copy_sampler_words(&t, &v, COPY).unwrap();
        let view = view_binding(&t, DEPTH_TPU_D24X8, Some(v)).unwrap();
        let [c0, c1, c2] = super::super::texture::format_words(&view, true);
        assert_eq!((s.config0, s.config1, s.config2), (c0, c1 | texdesc_halign(2), c2));
        let [l0, ..] = super::super::texture::format_words(&view, false);
        assert_eq!(l0 & !texdesc_addressing_mode(3), s.config0, "only the addressing differs from the LINEAR view");
        assert_eq!(s.linear_stride, view.stride_texels * 4, "the view's stride");
    }

    /// The copy run through `rs_interp` from `emit_rs`'s stream, then each
    /// texel read through the sampler words: its depth is ds_cache's CPU
    /// resolve of the surface's word, bit for bit. This proves the strides,
    /// the tiling bits and the address arithmetic agree with layout.rs; what
    /// the hardware does is the board's (gc-depth-sample-wire.md (b).1,
    /// "Board check").
    #[test]
    fn a_copied_surface_samples_the_cpu_resolves_depth() {
        for (w, h) in [(90u32, 70u32), (64, 64), (130, 3), (1, 1)] {
            let v = surface(w, h);
            let bytes = copy_bytes(&v).unwrap();
            let mut lin = vec![0u8; bytes];
            for y in 0..v.rows {
                for x in 0..v.stride / 4 {
                    let o = layout::offset(Layout::Linear, x, y, 4, v.stride);
                    lin[o..o + 4].copy_from_slice(&word(x, y).to_le_bytes());
                }
            }
            let mut mem = Memory::new();
            mem.map(SURF as u32, lin.clone());
            mem.map(COPY as u32, vec![0xcd; bytes]);
            let b = copy_blit(&v, COPY).unwrap();
            let wr = stream(&b);
            assert_eq!(wr.iter().filter(|(r, _)| *r == GC_RS_KICKER).count(), 1, "one RS job");
            let mut rs = RsInterp::new();
            rs.run(&wr, &mut mem);
            assert_eq!(rs.jobs.len(), 1);
            assert_eq!(rs.jobs[0].src.layout, Layout::Linear);
            assert_eq!(rs.jobs[0].dst.layout, Layout::SuperTiled, "the destination is super-tiled");
            assert_eq!(rs.jobs[0].dst.stride, v.stride, "the stride divided back by 4 is the surface's");
            assert_eq!(mem.region(SURF as u32), &lin[..], "the source is untouched");
            /* every byte of the buffer is the surface's, super-tiled by layout.rs */
            let mut want = vec![0u8; bytes];
            for y in 0..v.rows {
                for x in 0..v.stride / 4 {
                    let o = layout::offset(Layout::SuperTiled, x, y, 4, v.stride);
                    want[o..o + 4].copy_from_slice(&word(x, y).to_le_bytes());
                }
            }
            let copy = mem.region(COPY as u32).to_vec();
            assert!(copy == want, "{w}x{h}: the buffer is the super-tiled surface");
            assert!(copy != lin, "{w}x{h}: the copy is not the linear bytes");
            let s = copy_sampler_words(&texture(w, h), &v, COPY).unwrap();
            for y in 0..h {
                for x in 0..w {
                    let a = s.texel_addr(x, y).unwrap();
                    let got = u32::from_le_bytes(mem.read(a, 4).try_into().unwrap());
                    let want = word(x, y);
                    assert_eq!(got, want, "{w}x{h}: word ({x}, {y})");
                    /* D24X8 reads the depth in bits 31..8 (the word's stencil
                     * byte is X); the CPU resolve reads the same bits */
                    assert_eq!(unorm24_depth(got >> 8).to_bits(), cpu_resolve(want).to_bits(), "({x}, {y})");
                }
            }
        }
    }
}

/* ---- The view's guards: the texture's swizzle and exact-depth readers ---- */

#[cfg(test)]
mod guard_tests {
    use super::*;
    use crate::backend::{
        GLES_SWIZZLE_ALPHA, GLES_SWIZZLE_BLUE, GLES_SWIZZLE_GREEN, GLES_SWIZZLE_ONE, GLES_SWIZZLE_RED,
        GLES_SWIZZLE_ZERO, GLES_TEXFMT_R32F,
    };
    use crate::vivante::regs::{GC_SWIZZLE_0, GC_SWIZZLE_1, GC_SWIZZLE_R};
    use crate::vivante::texture::{descriptor_swizzle, gc_texfmt};

    const SURFACE: DepthView = DepthView { gpu: 0x0010_0000, stride: 64 * 4, rows: 64 };
    /* compare_op as the GL core sets it for a DEPTH24_STENCIL8 texture
     * (src/gl/draw.rs `compare_op`): bit 3 fixed-point, bit 4 no compare */
    const FIXED: u8 = 8;
    const NO_COMPARE: u8 = 16 | FIXED;

    /// The GL core's ES 2.0 depth texture binding (`depth_luminance` 1).
    fn depth_texture(compare_op: u8, swizzle: [u8; 4]) -> GlesTextureBind {
        GlesTextureBind {
            format: GLES_TEXFMT_R32F,
            type_: GLES_TEXTYPE_2D,
            layers: 1,
            width: 64,
            height: 64,
            mip_levels: 1,
            no_alpha: 1,
            depth_luminance: 1,
            depth_id: (1 << 30) | 1,
            compare_op,
            swizzle,
            ..GlesTextureBind::ZERO
        }
    }

    /// CONFIG1's four swizzle fields `emit_slot` writes for a binding.
    fn words(b: &GlesTextureBind) -> [u32; 4] {
        descriptor_swizzle(&gc_texfmt(b.format), b)
    }

    /// What the texels' R32F binding reads in GC terms (R the depth), the
    /// CPU resolve's answer the view must give.
    fn cpu_words(t: &GlesTextureBind) -> [u32; 4] {
        words(&GlesTextureBind { depth_id: 0, ..*t })
    }

    #[test]
    fn the_identity_swizzle_keeps_the_views_own_descriptor() {
        let b = view_binding(&depth_texture(NO_COMPARE, [0; 4]), DEPTH_TPU_D24X8, Some(SURFACE)).expect("it fits");
        assert_eq!(words(&b), [GC_SWIZZLE_R, GC_SWIZZLE_R, GC_SWIZZLE_R, GC_SWIZZLE_1], "(d, d, d, 1)");
        assert_eq!(words(&b), cpu_words(&depth_texture(NO_COMPARE, [0; 4])));
    }

    /// Mode 1 keeps the texture's swizzle: an ES 2.0 depth texture's view
    /// reads what its resolved texels would, for every component source.
    #[test]
    fn a_swizzled_depth_view_reads_what_the_texels_would() {
        let (r, g, b, a) = (GLES_SWIZZLE_RED, GLES_SWIZZLE_GREEN, GLES_SWIZZLE_BLUE, GLES_SWIZZLE_ALPHA);
        let (z, o) = (GLES_SWIZZLE_ZERO, GLES_SWIZZLE_ONE);
        for swizzle in [[0; 4], [a, b, g, r], [z, o, r, a], [g, g, a, o], [0, 0, 0, z], [a, 0, z, 0]] {
            let t = depth_texture(NO_COMPARE, swizzle);
            let v = view_binding(&t, DEPTH_TPU_D24X8, Some(SURFACE)).expect("mode 1 keeps a swizzle");
            assert_eq!((v.format, v.swizzle), (GC_TEXFMT_DEPTH_VIEW, swizzle));
            assert_eq!(words(&v), cpu_words(&t), "swizzle {swizzle:?}");
        }
        /* written out: (A, B, G, R) of (d, d, d, 1) is (1, d, d, d), and
         * (0, 1, R, A) is (0, 1, d, 1) */
        let v = view_binding(&depth_texture(NO_COMPARE, [a, b, g, r]), DEPTH_TPU_D24X8, Some(SURFACE)).unwrap();
        assert_eq!(words(&v), [GC_SWIZZLE_1, GC_SWIZZLE_R, GC_SWIZZLE_R, GC_SWIZZLE_R]);
        let v = view_binding(&depth_texture(NO_COMPARE, [z, o, r, a]), DEPTH_TPU_D24X8, Some(SURFACE)).unwrap();
        assert_eq!(words(&v), [GC_SWIZZLE_0, GC_SWIZZLE_1, GC_SWIZZLE_R, GC_SWIZZLE_1]);
    }

    /// The raw view's channels are bytes of the D24S8 word, which no GL
    /// swizzle names: it takes the identity alone.
    #[test]
    fn the_raw_view_refuses_a_swizzle() {
        let raw = |swizzle| view_binding(&depth_texture(NO_COMPARE, swizzle), DEPTH_TPU_RAW8888, Some(SURFACE));
        let explicit = [GLES_SWIZZLE_RED, GLES_SWIZZLE_GREEN, GLES_SWIZZLE_BLUE, GLES_SWIZZLE_ALPHA];
        for identity in [[0; 4], explicit] {
            assert_eq!(raw(identity).expect("the identity").swizzle, [0; 4]);
        }
        assert_eq!(raw([GLES_SWIZZLE_ALPHA, 0, 0, 0]).err(), Some(-ENOTSUP));
        assert_eq!(raw([0, 0, 0, GLES_SWIZZLE_ONE]).err(), Some(-ENOTSUP));
    }

    /// A shadow compare that reads the depth (LESS .. NOTEQUAL) needs the
    /// resolve's 24 bits: both modes refuse the view. No compare, NEVER and
    /// ALWAYS (a constant whatever the depth) take it, as does a binding built
    /// from `ZERO` (compare_op 0).
    #[test]
    fn a_depth_compare_is_refused_the_view() {
        for mode in [DEPTH_TPU_D24X8, DEPTH_TPU_RAW8888, DEPTH_TPU_D24X8_FLUSH] {
            for op in 1..=6u8 {
                for fixed in [0, FIXED] {
                    let t = depth_texture(op | fixed, [0; 4]);
                    assert!(exact_depth_reader(&t));
                    assert_eq!(view_binding(&t, mode, Some(SURFACE)).err(), Some(-ENOTSUP), "mode {mode}, op {op}");
                }
            }
            for op in [NO_COMPARE, 16, FIXED, FIXED | 7, 7, 0] {
                let t = depth_texture(op, [0; 4]);
                assert!(!exact_depth_reader(&t), "op {op}");
                assert!(view_binding(&t, mode, Some(SURFACE)).is_ok(), "mode {mode}, op {op}");
            }
        }
    }

    /// Mode 0 is the CPU resolve: the view is never made, whatever the binding.
    #[test]
    fn mode_0_makes_no_view() {
        for op in [NO_COMPARE, FIXED | 1] {
            for swizzle in [[0; 4], [GLES_SWIZZLE_ALPHA, 0, 0, 0]] {
                let t = depth_texture(op, swizzle);
                assert_eq!(view_binding(&t, DEPTH_TPU_OFF, Some(SURFACE)).err(), Some(-ENOTSUP));
            }
        }
        #[cfg(not(feature = "test-knobs"))]
        assert!(!gc_depth_sampled((1 << 30) | 1, 64, 64), "no live surface: the CPU resolve");
    }
}
