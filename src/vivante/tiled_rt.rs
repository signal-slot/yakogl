// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000 (i.MX8MP): 32-bit colour targets rendered into a TILED shadow, and
//! early depth for those draws (docs/notes/gc-tiled-wire.md W3-W5), behind the
//! `gc_tiled_rt` test knob (`gc_debug_set_tiled_rt`), off by default.
//!
//! Early depth needs a colour target that is not linear (Mesa's
//! `early_z_allowed`, `early_z.rs`), and the pixel engine of this part writes
//! a linear target only under SINGLE_BUFFER(1). So a draw that qualifies
//! ([`wanted`]) renders into a 4x4-tiled copy of its target under
//! SINGLE_BUFFER(2) with a super-tiled depth surface, and the resolve engine
//! copies the tiles back into the target's own linear storage **in the same
//! stream**: every reader of that storage (the texture unit, the CPU, the
//! mipmap chain, the window copy) is unchanged, because they already wait for
//! the render's kick and the kick holds the copy back.
//!
//! The shadow is never read by anything but the pixel engine and the resolve
//! engine. Its state in one stream ([`Phase`]):
//!
//! - `Active`: draws render into it; the storage is stale. The stream ends
//!   with (or the next draw that does not take the shadow is preceded by) the
//!   RS tiled to linear copy, which leaves it `InStep`.
//! - `InStep`: shadow and storage hold the same pixels; a draw that takes the
//!   shadow again needs no tile-in. A draw that writes the storage in linear
//!   form drops the shadow.
//! - `Sampled` (W10, `gc_tiled_sample`, off by default): draws sample the
//!   shadow straight ([`sampler_words_of`]) and the storage is stale. The copy
//!   back left out before them is owed: the stream ends with it, or the next
//!   draw that does not sample the shadow that way, or that draws into the
//!   storage, is preceded by it ([`needs_copy_back`]), as for `Active`.
//! - Nothing else carries over a kick: the next stream's first shadow draw
//!   tiles the storage in with the RS (linear to tiled), unless that draw
//!   opens with a whole colour clear.
//!
//! Where this differs from the design note: the note keeps a shadow in step
//! across kicks and tiles in only after a CPU write. This round-trips every
//! stream instead (one more RS copy a stream when the draw does not clear) and
//! needs no hook in the GL core to learn of CPU writes. The RS strides are
//! Mesa's (`etna_compile_rs_state`, etnaviv_rs.c:84-132): a TILED side's is
//! its row stride times 4 with RS_CONFIG's SOURCE_TILED or DEST_TILED, and no
//! TILING bit, which is the super-tiled sides'.

use super::backend::RsBlit;
use super::layout::{self, Layout};
use super::regs::*;
use crate::backend::{GlesDraw, GlesTextureBind, GLES_TEXFMT_RGBA8, GLES_TEXTYPE_2D};
use crate::osal::{self, OsalGpuMem, OSAL_MEM_DATA};
use crate::util::Global;

/// Bytes a pixel of a target this path takes.
pub const BPP: u32 = 4;
/// Where a shadow starts: a page, as every buffer Mesa hands this GPU does.
const BASE_ALIGN: usize = 4096;
/// Shadows kept at once.
const ENTRIES: usize = 4;
/// Words the closing tiled to linear copy needs ([`RsBlit`]'s block is 40
/// words): kept free in the stream while a shadow is `Active`.
pub const RESERVE_WORDS: u32 = 48;

/// The target a shadow stands for: a new value of any field is a new shadow.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Spec {
    /// The linear storage's GPU address (`GlesDraw::target_gpu`).
    pub gpu: u64,
    pub w: u32,
    pub h: u32,
    /// The linear storage's row pitch in bytes.
    pub pitch: u32,
}

impl Spec {
    pub const NONE: Spec = Spec { gpu: 0, w: 0, h: 0, pitch: 0 };
}

/// PE_COLOR_STRIDE of a shadow of a `w`-pixel-wide target: one row of the
/// 16-padded width (Mesa's `level->stride`).
pub const fn stride(w: u32) -> u32 {
    layout::stride(Layout::Tiled, w, BPP, true)
}

/// Where a shadow of `spec` stands in the stream being recorded.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Phase {
    /// No shadow tracked.
    None,
    /// Draws render into the shadow; the storage is stale.
    Active,
    /// Shadow and storage hold the same pixels.
    InStep,
    /// Draws sample the shadow straight (W10); the storage is stale and the
    /// copy back is owed, its words kept free as for `Active`.
    Sampled,
}

impl Phase {
    /// Does the stream owe this shadow's copy back (and keep
    /// [`RESERVE_WORDS`] free for it)?
    pub const fn owes_copy_back(self) -> bool {
        matches!(self, Phase::Active | Phase::Sampled)
    }
}

/// The shadow the open stream tracks.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Track {
    pub spec: Spec,
    /// The shadow's GPU address.
    pub shadow: u64,
    pub phase: Phase,
}

impl Track {
    pub const NONE: Track = Track { spec: Spec::NONE, shadow: 0, phase: Phase::None };

    /// Does a draw into `target` render into the shadow now?
    pub fn takes(&self, target: u64) -> Option<u64> {
        (self.phase == Phase::Active && self.spec.gpu == target && target != 0).then_some(self.shadow)
    }
}

/// The RS copy of the shadow into the storage (tiled to linear), the stream's
/// last act on a shadow.
pub(super) fn untile(spec: &Spec, shadow: u64) -> RsBlit {
    let fmt = GC_RS_FORMAT_A8R8G8B8;
    RsBlit {
        config: rs_source_format(fmt) | rs_dest_format(fmt) | GC_RS_SOURCE_TILED,
        src: shadow as u32,
        src_stride: stride(spec.w) << 2,
        dst: spec.gpu as u32,
        dst_stride: spec.pitch,
        w: spec.w,
        h: spec.h,
    }
}

/// The RS copy of the storage into the shadow (linear to tiled), before the
/// first draw that does not clear.
pub(super) fn tile_in(spec: &Spec, shadow: u64) -> RsBlit {
    let fmt = GC_RS_FORMAT_A8R8G8B8;
    RsBlit {
        config: rs_source_format(fmt) | rs_dest_format(fmt) | GC_RS_DEST_TILED,
        src: spec.gpu as u32,
        src_stride: spec.pitch,
        dst: shadow as u32,
        dst_stride: stride(spec.w) << 2,
        w: spec.w,
        h: spec.h,
    }
}

/// The target of `gl` as a shadow's spec, when its shape allows one: one
/// 4-byte colour target the resolve engine can address (width a multiple of
/// 16, height of 4, so the padded shadow is exactly the target and the RS
/// writes nothing past the GL core's rows), drawn straight. The format and
/// the depth rule are the caller's ([`super::backend`]'s `tiled_rt_spec`).
pub fn spec_of(gl: &GlesDraw) -> Option<Spec> {
    let (w, h) = (gl.target_w, gl.target_h);
    if gl.target_gpu == 0 || w == 0 || h == 0 || w % 16 != 0 || h % 4 != 0 {
        return None;
    }
    Some(Spec { gpu: gl.target_gpu, w, h, pitch: crate::backend::draw_pitch(gl) * BPP })
}

struct Entry {
    spec: Spec,
    mem: OsalGpuMem,
    /// The first page boundary in `mem`.
    gpu: u64,
    stamp: u32,
}

struct Shadows {
    entries: [Option<Entry>; ENTRIES],
    clock: u32,
}

static SHADOWS: Global<Shadows> = Global::new(Shadows { entries: [const { None }; ENTRIES], clock: 0 });

fn shadows() -> &'static mut Shadows {
    // SAFETY: single-threaded by contract; each entry point takes it once.
    unsafe { SHADOWS.get() }
}

/// The shadow of `spec`, made if it is new: its GPU address, 0 when there is
/// no memory for it. `busy` is the shadow the open stream tracks, which is
/// never the one given up.
pub fn shadow_of(spec: &Spec, busy: u64) -> u64 {
    let s = shadows();
    s.clock = s.clock.wrapping_add(1);
    let clock = s.clock;
    if let Some(e) = s.entries.iter_mut().flatten().find(|e| e.spec == *spec) {
        e.stamp = clock;
        return e.gpu;
    }
    /* the storage specified again at another size or pitch: the old shadow
     * is of no use */
    for e in s.entries.iter_mut() {
        if e.as_ref().is_some_and(|e| e.spec.gpu == spec.gpu && e.gpu != busy) {
            if let Some(mut old) = e.take() {
                osal::gpu_free(&mut old.mem);
            }
        }
    }
    let slot = s.entries.iter().position(|e| e.is_none()).or_else(|| {
        s.entries
            .iter()
            .enumerate()
            .filter(|(_, e)| e.as_ref().is_some_and(|e| e.gpu != busy))
            .max_by_key(|(_, e)| e.as_ref().map_or(0, |e| clock.wrapping_sub(e.stamp)))
            .map(|(i, _)| i)
    });
    let Some(slot) = slot else { return 0 };
    if let Some(mut old) = s.entries[slot].take() {
        osal::gpu_free(&mut old.mem);
    }
    let bytes = (stride(spec.w) * spec.h) as usize;
    let mut mem = OsalGpuMem::ZERO;
    if osal::gpu_alloc(super::backend::heap_data(), bytes + BASE_ALIGN - 1, OSAL_MEM_DATA, &mut mem) != 0 {
        crate::glog_once!("vivante: no GPU memory for a {bytes}-byte tiled colour target");
        return 0;
    }
    let gpu = mem.gpu + (mem.gpu.wrapping_neg() & (BASE_ALIGN as u64 - 1));
    s.entries[slot] = Some(Entry { spec: *spec, mem, gpu, stamp: clock });
    gpu
}

/// The GL core drops the storage `[base, base + bytes)`
/// (`YakoglBackend::texture_released`): the shadows of targets in it go too.
pub fn released(base: u64, bytes: u64) {
    for e in shadows().entries.iter_mut() {
        if e.as_ref().is_some_and(|e| e.spec.gpu >= base && e.spec.gpu < base.wrapping_add(bytes)) {
            if let Some(mut old) = e.take() {
                osal::gpu_free(&mut old.mem);
            }
        }
    }
}

/// Free every shadow (backend shutdown).
pub fn release_all() {
    let s = shadows();
    for e in s.entries.iter_mut() {
        if let Some(mut old) = e.take() {
            osal::gpu_free(&mut old.mem);
        }
    }
    s.clock = 0;
}

/// Shadows held (a test reads it).
#[cfg(test)]
pub fn held() -> usize {
    shadows().entries.iter().flatten().count()
}

/// The sampler descriptor words that read a shadow directly (gc-tiled-wire.md
/// W10): CONFIG0, CONFIG1, LINEAR_STRIDE, SLICE and the level 0 address. The
/// rest of the descriptor (size, log size, base LOD, 3D config) comes from the
/// texture as for any 2D one.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SamplerWords {
    pub config0: u32,
    pub config1: u32,
    pub linear_stride: u32,
    pub slice: u32,
    pub lod0: u64,
}

/// The words for sampling `shadow`, a single-level TILED 32-bit RGBA8 copy of
/// a `spec` target: Mesa's for a TILED resource the resolve engine may touch
/// (16x4-padded, HALIGN SIXTEEN). Everything but `lod0` equals the words
/// `texture.rs` writes for a tiled copy of the same size. `None` for a target
/// [`spec_of`] would refuse (`w % 16 != 0` or `h % 4 != 0`).
pub fn sampler_words(spec: &Spec, shadow: u64) -> Option<SamplerWords> {
    if spec.w == 0 || spec.h == 0 || !spec.w.is_multiple_of(16) || !spec.h.is_multiple_of(4) {
        return None;
    }
    let mut t = GlesTextureBind::ZERO;
    t.format = GLES_TEXFMT_RGBA8;
    t.width = spec.w;
    t.height = spec.h;
    let [config0, config1, _] = super::texture::format_words(&t, true);
    let linear_stride = spec.w * BPP;
    Some(SamplerWords {
        config0,
        config1: config1 | texdesc_halign(GC_TEXTURE_HALIGN_SIXTEEN),
        linear_stride,
        slice: linear_stride * spec.h,
        lod0: shadow,
    })
}

/// The sampler words that read binding `t` straight from `shadow`, the
/// shadow of `spec` (W10): [`sampler_words`], with CONFIG0 and CONFIG1 the
/// binding's own tiled words (its swizzle, BGRA order and alpha rule; the
/// shadow holds the storage's bytes, the RS copies them unchanged) under
/// HALIGN SIXTEEN. `None` unless `t` is the whole storage of `spec` as one
/// complete RGBA8 2D level at the base: then today's descriptor reads the
/// storage, which the caller keeps current.
pub fn sampler_words_of(t: &GlesTextureBind, spec: &Spec, shadow: u64) -> Option<SamplerWords> {
    if !samples_whole(t, spec) {
        return None;
    }
    let mut w = sampler_words(spec, shadow)?;
    let [config0, config1, _] = super::texture::format_words(t, true);
    w.config0 = config0;
    w.config1 = config1 | texdesc_halign(GC_TEXTURE_HALIGN_SIXTEEN);
    Some(w)
}

/// Is `t` the whole storage of `spec`, one complete RGBA8 2D level from its
/// base, which [`sampler_words_of`] reads from the shadow?
fn samples_whole(t: &GlesTextureBind, spec: &Spec) -> bool {
    spec.gpu != 0
        && t.base_gpu == spec.gpu
        && t.type_ == GLES_TEXTYPE_2D
        && t.format == GLES_TEXFMT_RGBA8
        && t.incomplete == 0
        && t.depth_id == 0
        && t.base_level == 0
        && t.mip_levels <= 1
        && t.layers <= 1
        && (t.width, t.height) == (spec.w, spec.h)
        && t.stride_texels * BPP == spec.pitch
}

/// Bytes past a binding's base its storage may span, for [`needs_copy_back`]:
/// level 0 (every layer) twice over, which holds a whole mip chain, and a
/// page. Generous on purpose: a wrong "far" would let a draw read stale
/// storage, a wrong "near" only costs the copy back W10 leaves out.
fn span_of(t: &GlesTextureBind) -> u64 {
    let level0 = u64::from(t.stride_texels.max(t.width)) * u64::from(t.height) * 16 * u64::from(t.layers.max(1));
    (u64::from(t.bytes).max(level0)) * 2 + 4096
}

/// W10's rule for a draw into another target while the stream owes the copy
/// back of the shadow of `spec`: does the draw need the storage current
/// first? Yes unless at least one of its fragment-stage bindings (`fs`)
/// samples the shadow straight ([`sampler_words_of`] has words for it), and
/// no other binding's storage reaches `spec`'s: a mip chain, a layer or
/// another format of it, and every vertex-stage binding (`vs`), whose tiled
/// copy the resolve engine makes from the storage (`tiled_copy.rs`).
/// `writes` is whether the draw writes the storage itself (one of its colour
/// targets is `spec.gpu`): then the copy back goes first too.
pub fn needs_copy_back(fs: &[GlesTextureBind], vs: &[GlesTextureBind], spec: &Spec, writes: bool) -> bool {
    let reaches = |t: &GlesTextureBind| t.base_gpu <= spec.gpu && spec.gpu < t.base_gpu.saturating_add(span_of(t));
    if writes || vs.iter().any(reaches) {
        return true;
    }
    let mut straight = false;
    for t in fs {
        if samples_whole(t, spec) && !super::tiled_copy::wanted(t) {
            straight = true;
        } else if reaches(t) {
            return true;
        }
    }
    !straight
}

/// Default threshold of [`long_program`], in instructions: a fragment
/// program of at least this many qualifies.
///
/// W6's fallback (`gc-tiled-wire.md`): if the RS resolve costs more than early
/// depth saves, the W4 rule narrows to draws with a long fragment program.
/// The threshold was set when the GC7000 compiler gave `terrain.frag` 80
/// instructions (148 in `gc-slow-host-counts.md`, before ranks T and 4a-4c),
/// the blur passes `tilt_*.frag` 57 and `bloom_*.frag` 32, `normalmap.frag`
/// 20, `luminance.frag` 7 and `overlay.frag` 2: 64 lay between the terrain
/// pass and the longest blur.
///
/// glmark2's fragment programs measured at f3f3dc52 (our GC7000 compiler;
/// `tests/gc_terrain_shader_count.rs` and `tests/gc_sr_shader_count.rs` pin
/// them, the latter with this verdict):
///
/// | scene   | fragment program  | instructions | at 64 |
/// |---------|-------------------|--------------|-------|
/// | terrain | `terrain.frag`    | 75           | long  |
/// | terrain | `tilt_*.frag`     | 29           | short |
/// | terrain | `normalmap.frag`  | 17           | short |
/// | terrain | `bloom_*.frag`    | 16           | short |
/// | terrain | `luminance.frag`  | 6            | short |
/// | terrain | `overlay.frag`    | 2            | short |
/// | refract | refract           | 55           | short |
/// | shadow  | ground            | 12           | short |
/// | shadow  | depth             | 2            | short |
/// | shadow  | horse             | 1            | short |
///
/// Only terrain's pass qualifies. Refract's 55 (64 before rank 3, a `mul`
/// read once by an `add` fused into a `mad`) is below the threshold; the
/// threshold was not derived from the shadow or refract programs.
pub const LONG_PROGRAM_INSTRS: u32 = 64;

/// Whether a fragment program of `usc_dw` dwords of shader code (the shader
/// output's `usc_dw`, four dwords an instruction) has at least
/// `threshold_instrs` instructions. Pure; nothing calls it until W6 asks.
#[allow(dead_code)]
pub const fn long_program(usc_dw: u32, threshold_instrs: u32) -> bool {
    usc_dw / 4 >= threshold_instrs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_program_splits_terrain_from_blits_and_overlays() {
        for (name, instrs, want) in [
            ("terrain.frag", 80, true),
            ("tilt_h.frag", 57, false),
            ("bloom_v.frag", 32, false),
            ("normalmap.frag", 20, false),
            ("luminance.frag", 7, false),
            ("overlay.frag", 2, false),
        ] {
            assert_eq!(long_program(instrs * 4, LONG_PROGRAM_INSTRS), want, "{name}");
        }
    }

    #[test]
    fn long_program_boundary_and_partial_dwords() {
        assert!(long_program(64 * 4, 64));
        assert!(!long_program(64 * 4 - 1, 64));
        assert!(long_program(0, 0));
        assert!(!long_program(0, 1));
    }

    #[test]
    fn terrains_256_square_shadow_samples_with_the_notes_words() {
        let spec = Spec { gpu: 0x1000, w: 256, h: 256, pitch: 1024 };
        let s = sampler_words(&spec, 0x20000).expect("256x256 qualifies");
        assert_eq!(s.config0 & texdesc_addressing_mode(!0), texdesc_addressing_mode(GC_TEX_ADDRESSING_TILED));
        assert_eq!(s.config1, 0x04321000);
        assert_eq!(s.linear_stride, 0x400);
        assert_eq!(s.slice, 0x40000);
        assert_eq!(s.lod0, 0x20000);
    }

    #[test]
    fn the_words_are_the_tiled_copys_but_for_lod0() {
        let spec = Spec { gpu: 0x1000, w: 64, h: 32, pitch: 256 };
        let a = sampler_words(&spec, 0x20000).unwrap();
        let b = sampler_words(&spec, 0x90000).unwrap();
        assert_eq!((a.config0, a.config1, a.linear_stride, a.slice), (b.config0, b.config1, b.linear_stride, b.slice));
        assert_ne!(a.lod0, b.lod0);
        let mut t = GlesTextureBind::ZERO;
        (t.width, t.height) = (64, 32);
        let [c0, c1, _] = super::super::texture::format_words(&t, true);
        assert_eq!(a.config0, c0);
        assert_eq!(a.config1, c1 | texdesc_halign(GC_TEXTURE_HALIGN_SIXTEEN));
        /* tiled_copy's descriptor: align4(w) * bpp, and align4(w) * align4(h) * bpp */
        assert_eq!(a.linear_stride, 64 * 4);
        assert_eq!(a.slice, 64 * 32 * 4);
    }

    /// A binding that is the whole storage of `spec` as one RGBA8 level.
    fn whole(spec: &Spec) -> GlesTextureBind {
        GlesTextureBind {
            format: GLES_TEXFMT_RGBA8,
            base_gpu: spec.gpu,
            width: spec.w,
            height: spec.h,
            stride_texels: spec.pitch / BPP,
            bytes: spec.pitch * spec.h,
            mip_levels: 1,
            layers: 1,
            ..GlesTextureBind::ZERO
        }
    }

    #[test]
    fn a_whole_single_level_binding_samples_the_shadow_with_its_own_format_words() {
        let spec = Spec { gpu: 0x10_0000, w: 256, h: 256, pitch: 1024 };
        let t = whole(&spec);
        let w = sampler_words_of(&t, &spec, 0x20_0000).expect("the whole storage");
        assert_eq!(Some(w), sampler_words(&spec, 0x20_0000), "an RGBA8 binding: the note's words");
        let bgra = GlesTextureBind { bgra: 1, ..t };
        let wb = sampler_words_of(&bgra, &spec, 0x20_0000).unwrap();
        let [c0, c1, _] = super::super::texture::format_words(&bgra, true);
        assert_eq!((wb.config0, wb.config1), (c0, c1 | texdesc_halign(GC_TEXTURE_HALIGN_SIXTEEN)));
        assert_eq!((wb.linear_stride, wb.slice, wb.lod0), (w.linear_stride, w.slice, w.lod0));
        for (what, o) in [
            ("mipmapped", GlesTextureBind { mip_levels: 9, ..t }),
            ("another base", GlesTextureBind { base_gpu: spec.gpu + 0x40, ..t }),
            ("another size", GlesTextureBind { width: 128, ..t }),
            ("another pitch", GlesTextureBind { stride_texels: 512, ..t }),
            ("a cube", GlesTextureBind { layers: 6, type_: crate::backend::GLES_TEXTYPE_CUBE, ..t }),
            ("incomplete", GlesTextureBind { incomplete: 1, ..t }),
            ("a depth view", GlesTextureBind { depth_id: 3, ..t }),
            ("a base level", GlesTextureBind { base_level: 1, ..t }),
            ("RGBA16F", GlesTextureBind { format: crate::backend::GLES_TEXFMT_RGBA16F, ..t }),
        ] {
            assert_eq!(sampler_words_of(&o, &spec, 0x20_0000), None, "{what}");
        }
    }

    #[test]
    fn only_a_draw_that_samples_the_shadow_straight_and_nothing_else_of_it_skips_the_copy_back() {
        let spec = Spec { gpu: 0x10_0000, w: 64, h: 64, pitch: 256 };
        let t = whole(&spec);
        let far = GlesTextureBind { base_gpu: 0x80_0000, ..t };
        assert!(!needs_copy_back(&[t], &[], &spec, false), "sampled straight");
        assert!(!needs_copy_back(&[far, t], &[], &spec, false), "and another texture far away");
        assert!(needs_copy_back(&[], &[], &spec, false), "samples nothing of it");
        assert!(needs_copy_back(&[far], &[], &spec, false), "samples something else");
        assert!(needs_copy_back(&[t], &[], &spec, true), "draws into the storage");
        assert!(needs_copy_back(&[t], &[t], &spec, false), "the vertex stage's copy is made from the storage");
        let chain = GlesTextureBind { mip_levels: 7, ..t };
        assert!(needs_copy_back(&[t, chain], &[], &spec, false), "a mip chain of the same storage");
        let below = GlesTextureBind { base_gpu: spec.gpu - 0x1000, layers: 6, ..t };
        assert!(needs_copy_back(&[t, below], &[], &spec, false), "a stack whose layer is the storage");
    }

    #[test]
    fn a_shadow_owes_its_copy_back_while_drawn_into_or_sampled() {
        assert!(Phase::Active.owes_copy_back());
        assert!(Phase::Sampled.owes_copy_back());
        assert!(!Phase::InStep.owes_copy_back());
        assert!(!Phase::None.owes_copy_back());
        let t = Track { spec: Spec { gpu: 0x1000, w: 16, h: 4, pitch: 64 }, shadow: 0x9000, phase: Phase::Sampled };
        assert_eq!(t.takes(0x1000), None, "a sampled shadow is no draw target");
    }

    #[test]
    fn a_size_the_resolve_engine_would_not_take_has_no_words() {
        for (w, h) in [(250, 256), (256, 250), (0, 0), (8, 4)] {
            let spec = Spec { gpu: 0x1000, w, h, pitch: w * 4 };
            assert_eq!(sampler_words(&spec, 0x20000), None, "{w}x{h}");
        }
    }

    #[test]
    fn the_strides_are_mesas_for_a_tiled_side() {
        let spec = Spec { gpu: 0x1000, w: 256, h: 256, pitch: 1024 };
        assert_eq!(stride(256), 1024, "a row of the 16-padded width");
        assert_eq!(stride(250), 256 * 4);
        let u = untile(&spec, 0x20000);
        assert_eq!(u.src_stride, 1024 << 2, "a tiled source's stride is times four, no TILING bit");
        assert_eq!(u.dst_stride, 1024, "the storage's own pitch");
        assert_eq!(u.config & GC_RS_SOURCE_TILED, GC_RS_SOURCE_TILED);
        assert_eq!(u.config & GC_RS_DEST_TILED, 0);
        let t = tile_in(&spec, 0x20000);
        assert_eq!(t.dst_stride, 1024 << 2);
        assert_eq!(t.config & GC_RS_DEST_TILED, GC_RS_DEST_TILED);
        assert_eq!((u.w, u.h), (256, 256));
    }

    #[test]
    fn only_a_target_the_resolve_engine_addresses_gets_a_spec() {
        let mut gl = GlesDraw::ZERO;
        assert!(spec_of(&gl).is_none(), "no target");
        gl.target_gpu = 0x10_0000;
        (gl.target_w, gl.target_h) = (250, 250);
        assert!(spec_of(&gl).is_none(), "250 is not a multiple of 16");
        (gl.target_w, gl.target_h) = (256, 250);
        assert!(spec_of(&gl).is_none(), "250 is not a multiple of 4");
        (gl.target_w, gl.target_h) = (256, 256);
        let s = spec_of(&gl).expect("a 256x256 target");
        assert_eq!((s.w, s.h, s.pitch), (256, 256, 1024));
    }

    #[test]
    fn a_track_hands_out_its_shadow_only_while_active() {
        let mut t = Track { spec: Spec { gpu: 0x1000, w: 16, h: 4, pitch: 64 }, shadow: 0x9000, phase: Phase::Active };
        assert_eq!(t.takes(0x1000), Some(0x9000));
        assert_eq!(t.takes(0x2000), None, "another target");
        t.phase = Phase::InStep;
        assert_eq!(t.takes(0x1000), None);
        assert_eq!(Track::NONE.takes(0), None);
    }
}
