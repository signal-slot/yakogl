// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The Valhall (Mali-G610) words of a 4x multisample framebuffer object, as
//! plain integers named after the genxml fields (docs/notes/
//! es3-msaa-renderbuffers.md (c4), (f4), (f5)). Not wired: `mali-msaa-fbo`
//! calls them from `src/mali`.
//!
//! The genxml numbers are copied from Mesa 26.2.1
//! `src/panfrost/genxml/v10.xml` (third_party/mesa-26.2.1). Mesa's
//! `pan_fb.c` and `pan_desc.c` are not vendored; their rules are cited as the
//! note and `src/mali/frame.rs` cite them.

use crate::msaa_store::SampleSize;

/// `Sample Count` of `Framebuffer Parameters` is a log2 field
/// (v10.xml:1689, `modifier="log2"`, default 1): 1 sample is 0, 4 samples 2.
pub const SAMPLE_COUNT_LOG2_1X: u32 = 0;
pub const SAMPLE_COUNT_LOG2_4X: u32 = 2;

/// `<enum name="Sample Pattern">` (v10.xml:1647-1653).
pub const SAMPLE_PATTERN_SINGLE_SAMPLED: u32 = 0; // v10.xml:1648
pub const SAMPLE_PATTERN_ORDERED_4X_GRID: u32 = 1; // v10.xml:1649
pub const SAMPLE_PATTERN_ROTATED_4X_GRID: u32 = 2; // v10.xml:1650

/// `<enum name="MSAA">` (v10.xml:405-413), the writeback's `MSAA` field.
pub const MSAA_SINGLE: u32 = 0; // v10.xml:406
/// N samples, 1 surface, resolved (v10.xml:407-408).
pub const MSAA_AVERAGE: u32 = 1;
/// N samples, 1 surface, unresolved (v10.xml:409-410).
pub const MSAA_MULTIPLE: u32 = 2;
/// N samples, N surfaces, one layer a sample (v10.xml:411-412).
pub const MSAA_LAYERED: u32 = 3;

/// Samples of a 4x framebuffer object.
pub const SAMPLES_4X: u32 = 4;

/// The `Framebuffer Parameters` words of a sample count: `Some((Sample Count
/// (log2), Sample Pattern))` for 1 and 4, `None` for any other count (the
/// frame builder refuses it, `NOT_BUILT_SAMPLES`). 4 samples take the
/// rotated 4x grid, as Mesa's `pan_sample_pattern` and `Target::sample_pattern`
/// in `src/mali/frame.rs:921-928` do.
pub const fn sample_words(samples: u32) -> Option<(u32, u32)> {
    match samples {
        1 => Some((SAMPLE_COUNT_LOG2_1X, SAMPLE_PATTERN_SINGLE_SAMPLED)),
        4 => Some((SAMPLE_COUNT_LOG2_4X, SAMPLE_PATTERN_ROTATED_4X_GRID)),
        _ => None,
    }
}

/// What a framebuffer attachment holds, for the choice of its resolve.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FormatClass {
    /// Float colour (RGBA16F, ...).
    Float,
    /// Unorm or sRGB colour (RGBA8, RGB10_A2, ...).
    Unorm,
    /// Integer colour (signed or unsigned).
    Integer,
    /// Depth, stencil or both.
    DepthStencil,
}

/// How a writeback resolves the samples of one attachment.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ResolveMode {
    /// `MALI_MSAA_AVERAGE`: the samples averaged into one surface.
    Average,
    /// `MALI_MSAA_LAYERED`: every sample kept, one layer a sample (the
    /// sample store; Mesa's multisampled image).
    Layered,
    /// One sample (sample 0) copied: not a `MSAA` enum value but Mesa's
    /// `SAMPLE_0` resolve, which a Z/S writeback uses.
    Sample0,
}

impl ResolveMode {
    /// The `MSAA` field of the writeback for this mode: `Sample0` is a
    /// single-sample writeback of sample 0 (`MALI_MSAA_SINGLE`).
    pub const fn msaa(self) -> u32 {
        match self {
            ResolveMode::Average => MSAA_AVERAGE,
            ResolveMode::Layered => MSAA_LAYERED,
            ResolveMode::Sample0 => MSAA_SINGLE,
        }
    }
}

/// The resolve a writeback of this class takes into the single-sample
/// target: float and unorm colour average (`pan_desc.c:26-41`
/// `mali_sampling_mode`, note (f4)), an integer copies one sample (a mean of
/// integers is not an integer; `u_blitter.c:1005-1009`), and Z/S uses
/// `SAMPLE_0` as `pan_fb.c:177-179` ("Z/S only supports SAMPLE_0").
///
/// Z/S: **board only**. `src/mali/frame.rs:2149-2154` writes the 4x D24S8
/// with AVERAGE today, and which sample the hardware keeps for either is not
/// measured (note (f4), (f5)); `Sample0` is Mesa's choice, not a board result.
pub const fn resolve_mode(class: FormatClass) -> ResolveMode {
    match class {
        FormatClass::Float | FormatClass::Unorm => ResolveMode::Average,
        FormatClass::Integer | FormatClass::DepthStencil => ResolveMode::Sample0,
    }
}

/// The per-sample store of a `w` x `h` attachment of `bytes_per_pixel`,
/// 4 samples (one `MALI_MSAA_LAYERED` layer each): the form
/// [`crate::msaa_store::MsaaStore::get_or_alloc`] takes. Packed: the back
/// end pads rows to 64 bytes itself (`sample_store` needs a stride that is a
/// multiple of 64, note (c4)).
pub const fn sample_size(w: u32, h: u32, bytes_per_pixel: u32) -> SampleSize {
    SampleSize { w, h, bytes_per_sample: bytes_per_pixel, samples: SAMPLES_4X }
}

// ---------------------------------------------------------------------------
// The load side: a 4x FBO drawn into again after a flush.
//
// Valhall has no fixed-function colour load. Mesa 26.2.1 preloads a tile
// buffer with a frame shader: a draw (DCD) that `Frame Shader DCDs`
// (v10.xml:1682) points at, run before the frame's own draws as `Pre Frame
// 0` (colour) or `Pre Frame 1` (depth/stencil) (v10.xml:1675-1676). Gallium's
// `pan_fb_preload.c` (not vendored) picks the DCD and its mode in
// `pan_preload_emit_pre_frame_dcd` (:1174-1252: `dcd_idx = zs ? 1 : 0`;
// colour INTERSECT, :1247-1249; Z/S EARLY_ZS_ALWAYS on v8-v12, :1228-1230),
// and sets the draw multisampled and run once a sample only when the image
// it reads has more than one sample (`pan_preload_is_ms`, :163-180, used at
// :1144-1145). The shader then fetches with `txf_ms` and the sample id
// (`GLSL_SAMPLER_DIM_MS`, :468, :489; `pan_fb_nir.c:402-420`
// `PAN_FB_MSAA_COPY_ALL`), or with a plain `txf` from a single-sampled image,
// whose one value lands in every covered sample (`PAN_FB_MSAA_COPY_SINGLE`,
// `pan_fb_nir.c:84-97` `reduce_msaa_op`). `src/mali/frame.rs`
// `colour_preload_dcd` is that draw for the 4x window.
//
// The two sources of `crate::msaa_store`'s contract (`Slot::needs_load`):
// the slot's samples (written by the previous job with `MALI_MSAA_LAYERED`,
// one plane a sample), or, when the store returned `None` or holds nothing
// yet, the resolved `rb.mem` broadcast into every sample.
// ---------------------------------------------------------------------------

/// `<enum name="Pre Post Frame Shader Mode">` (v10.xml:1661-1666).
pub const FRAME_SHADER_NEVER: u32 = 0; // v10.xml:1662
pub const FRAME_SHADER_ALWAYS: u32 = 1; // v10.xml:1663
pub const FRAME_SHADER_INTERSECT: u32 = 2; // v10.xml:1664
pub const FRAME_SHADER_EARLY_ZS_ALWAYS: u32 = 3; // v10.xml:1665

/// Index of the colour preload DCD in the `Frame Shader DCDs` array, run as
/// `Pre Frame 0` (v10.xml:1675; `pan_fb_preload.c:1179` `dcd_idx = zs ? 1 :
/// 0`).
pub const FRAME_SHADER_DCD_COLOUR: usize = 0;
/// Index of the depth/stencil preload DCD, run as `Pre Frame 1`
/// (v10.xml:1676).
pub const FRAME_SHADER_DCD_ZS: usize = 1;

/// `Sample Mask` of the preload DCD (`DCD Flags 1`, v10.xml:2142): every
/// sample, as `pan_fb_preload.c:1143` `cfg.flags_1.sample_mask = 0xFFFF`.
pub const PRELOAD_SAMPLE_MASK: u32 = 0xffff;

/// `Z Preload Enable` / `S Preload Enable` of `Framebuffer Parameters`
/// (v10.xml:1706, :1702) stay off: Mesa 26.2.1 never sets them on v10
/// (`pan_fb.c` and `pan_desc.c` write only `z_write_enable` /
/// `s_write_enable`, `pan_fb.c:750-757`, `pan_desc.c:1445-1446`); depth and
/// stencil are preloaded by the `Pre Frame 1` draw.
pub const ZS_PRELOAD_ENABLE: bool = false;

/// The row stride a sample store plane needs (note (c4): `sample_store`
/// returns `None` for a row stride that is not a multiple of 64).
pub const STORE_ROW_ALIGN: u32 = 64;

/// What the colour preload of a 4x job reads.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PreloadSource {
    /// The slot's four sample planes (`Slot::needs_load` true): each sample
    /// is reloaded from its own plane (Mesa `PAN_FB_MSAA_COPY_ALL`).
    SampleStore,
    /// The resolved single-sampled `rb.mem`, its value written to every
    /// sample (Mesa `PAN_FB_MSAA_COPY_SINGLE`): the store returned `None`,
    /// the slot is new or was evicted. The edge samples of earlier jobs are
    /// lost (note (c4)).
    Resolved,
}

/// The colour preload of a 4x job, from `msaa_store`'s contract: `None`
/// when the job starts from a whole clear (nothing to load), else the
/// store's samples when the job has a slot that `needs_load`, else the
/// resolved `rb.mem` (`slot_needs_load` is `None` when `get_or_alloc`
/// returned `None`).
pub const fn colour_preload_source(cleared_whole: bool, slot_needs_load: Option<bool>) -> Option<PreloadSource> {
    if cleared_whole {
        return None;
    }
    match slot_needs_load {
        Some(true) => Some(PreloadSource::SampleStore),
        Some(false) | None => Some(PreloadSource::Resolved),
    }
}

/// `pan_fix_frame_shader_mode` (Mesa `pan_desc.h:377-384`): a frame that
/// forces clean-tile writes (every tile written whole, as a resolving frame
/// does) loads every tile, so INTERSECT becomes ALWAYS.
pub const fn fix_frame_shader_mode(mode: u32, force_clean_tile: bool) -> u32 {
    if force_clean_tile && mode == FRAME_SHADER_INTERSECT {
        FRAME_SHADER_ALWAYS
    } else {
        mode
    }
}

/// The words of one preload draw and the image it reads, named after the
/// genxml fields.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PreloadRead {
    /// Its slot in `Frame Shader DCDs` ([`FRAME_SHADER_DCD_COLOUR`] or
    /// [`FRAME_SHADER_DCD_ZS`]).
    pub dcd_index: usize,
    /// `Pre Frame 0` or `Pre Frame 1` (v10.xml:1675-1676).
    pub pre_frame_mode: u32,
    /// How the surface it reads was written: [`MSAA_LAYERED`] for the sample
    /// store (one plane a sample, the store's writeback), [`MSAA_SINGLE`]
    /// for the resolved `rb.mem`. Never [`MSAA_MULTIPLE`]: the store is not
    /// one interleaved surface.
    pub source_msaa: u32,
    /// `Texture.Sample count` (v10.xml:1340, log2): 2 (4 samples) or 0.
    pub texture_sample_count_log2: u32,
    /// `Generic Plane.Size` (v10.xml:1363): the bytes the plane descriptor
    /// covers, all four sample planes for the store.
    pub plane_size: u32,
    /// `Generic Plane.Slice stride` (v10.xml:1366): one sample plane for the
    /// store, 0 for a single-sampled 2D image (`pan_texture.c:416-420`).
    pub slice_stride: u32,
    /// `DCD Flags 0.Multisample Enable` (v10.xml:2134).
    pub multisample_enable: bool,
    /// `DCD Flags 0.Evaluate per-sample` (v10.xml:2128): the shader runs
    /// once a sample and fetches the sample it runs for.
    pub evaluate_per_sample: bool,
    /// `DCD Flags 1.Sample Mask` (v10.xml:2142).
    pub sample_mask: u32,
}

/// One sample plane of the store, `row_stride * height` bytes: `None` when
/// the row stride is not a multiple of [`STORE_ROW_ALIGN`], a size is 0, or
/// the four planes do not fit the 32-bit `Size` field.
pub const fn store_surface_stride(row_stride: u32, height: u32) -> Option<u32> {
    if row_stride == 0 || height == 0 || !row_stride.is_multiple_of(STORE_ROW_ALIGN) {
        return None;
    }
    match row_stride.checked_mul(height) {
        Some(s) if s.checked_mul(SAMPLES_4X).is_some() => Some(s),
        _ => None,
    }
}

/// The colour preload that reloads each sample from the sample store: a
/// 4-sample texture over the four planes, the draw multisampled and run once
/// a sample (`pan_preload_is_ms` true). `force_clean_tile`: the frame writes
/// every tile whole (it resolves), see [`fix_frame_shader_mode`]. `None`
/// when [`store_surface_stride`] refuses the plane: the caller then takes
/// [`colour_preload_from_resolved`].
pub const fn colour_preload_from_store(row_stride: u32, height: u32, force_clean_tile: bool) -> Option<PreloadRead> {
    let plane = match store_surface_stride(row_stride, height) {
        Some(p) => p,
        None => return None,
    };
    Some(PreloadRead {
        dcd_index: FRAME_SHADER_DCD_COLOUR,
        pre_frame_mode: fix_frame_shader_mode(FRAME_SHADER_INTERSECT, force_clean_tile),
        source_msaa: MSAA_LAYERED,
        texture_sample_count_log2: SAMPLE_COUNT_LOG2_4X,
        plane_size: plane * SAMPLES_4X,
        slice_stride: plane,
        multisample_enable: true,
        evaluate_per_sample: true,
        sample_mask: PRELOAD_SAMPLE_MASK,
    })
}

/// The colour preload that broadcasts the resolved `rb.mem` into every
/// sample: a single-sampled texture, the draw neither multisampled nor run
/// a sample (`pan_preload_is_ms` false), so its one value lands in all four
/// covered samples. `None` when a size is 0 or `row_stride * height` does
/// not fit the plane's `Size`.
pub const fn colour_preload_from_resolved(row_stride: u32, height: u32, force_clean_tile: bool) -> Option<PreloadRead> {
    let size = match single_plane_size(row_stride, height) {
        Some(s) => s,
        None => return None,
    };
    Some(PreloadRead {
        dcd_index: FRAME_SHADER_DCD_COLOUR,
        pre_frame_mode: fix_frame_shader_mode(FRAME_SHADER_INTERSECT, force_clean_tile),
        source_msaa: MSAA_SINGLE,
        texture_sample_count_log2: SAMPLE_COUNT_LOG2_1X,
        plane_size: size,
        slice_stride: 0,
        multisample_enable: false,
        evaluate_per_sample: false,
        sample_mask: PRELOAD_SAMPLE_MASK,
    })
}

/// The colour preload of `source` (see [`colour_preload_source`]). A store
/// plane that [`store_surface_stride`] refuses falls back to the resolved
/// image, as `sample_store` returning `None` does (note (c4)).
pub const fn colour_preload(
    source: PreloadSource,
    row_stride: u32,
    height: u32,
    force_clean_tile: bool,
) -> Option<PreloadRead> {
    match source {
        PreloadSource::SampleStore => match colour_preload_from_store(row_stride, height, force_clean_tile) {
            Some(r) => Some(r),
            None => colour_preload_from_resolved(row_stride, height, force_clean_tile),
        },
        PreloadSource::Resolved => colour_preload_from_resolved(row_stride, height, force_clean_tile),
    }
}

/// The depth/stencil preload of a 4x job: always from the single-sampled
/// linear D24S8 plane into every sample, since the Mali keeps no depth
/// samples across jobs (note (c4)); `Pre Frame 1` EARLY_ZS_ALWAYS
/// (`pan_fb_preload.c:1228-1230`, v8-v12), not multisampled. `None` when a
/// size is 0 or does not fit.
pub const fn zs_preload_from_resolved(row_stride: u32, height: u32) -> Option<PreloadRead> {
    let size = match single_plane_size(row_stride, height) {
        Some(s) => s,
        None => return None,
    };
    Some(PreloadRead {
        dcd_index: FRAME_SHADER_DCD_ZS,
        pre_frame_mode: FRAME_SHADER_EARLY_ZS_ALWAYS,
        source_msaa: MSAA_SINGLE,
        texture_sample_count_log2: SAMPLE_COUNT_LOG2_1X,
        plane_size: size,
        slice_stride: 0,
        multisample_enable: false,
        evaluate_per_sample: false,
        sample_mask: PRELOAD_SAMPLE_MASK,
    })
}

const fn single_plane_size(row_stride: u32, height: u32) -> Option<u32> {
    if row_stride == 0 || height == 0 {
        return None;
    }
    row_stride.checked_mul(height)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_words_pin_genxml() {
        // v10.xml:1689 log2 count, :1647-1653 pattern values.
        assert_eq!(sample_words(1), Some((0, 0)));
        assert_eq!(sample_words(4), Some((2, 2)));
        for s in [0, 2, 3, 8, 16] {
            assert_eq!(sample_words(s), None);
        }
        assert_eq!(SAMPLE_PATTERN_ORDERED_4X_GRID, 1); // v10.xml:1649
    }

    #[test]
    fn msaa_values_pin_genxml() {
        // v10.xml:405-413.
        assert_eq!([MSAA_SINGLE, MSAA_AVERAGE, MSAA_MULTIPLE, MSAA_LAYERED], [0, 1, 2, 3]);
    }

    #[test]
    fn resolve_by_class() {
        assert_eq!(resolve_mode(FormatClass::Float), ResolveMode::Average);
        assert_eq!(resolve_mode(FormatClass::Unorm), ResolveMode::Average);
        assert_eq!(resolve_mode(FormatClass::Integer), ResolveMode::Sample0);
        // pan_fb.c:177-179; the hardware sample is board only.
        assert_eq!(resolve_mode(FormatClass::DepthStencil), ResolveMode::Sample0);
        assert_eq!(ResolveMode::Average.msaa(), 1);
        assert_eq!(ResolveMode::Layered.msaa(), 3);
        assert_eq!(ResolveMode::Sample0.msaa(), 0);
    }

    #[test]
    fn store_size() {
        let s = sample_size(64, 32, 4);
        assert_eq!(s, SampleSize { w: 64, h: 32, bytes_per_sample: 4, samples: 4 });
        assert_eq!(s.bytes(), Some(64 * 32 * 4 * 4));
        assert_eq!(sample_size(0, 8, 4).bytes(), None);
    }

    #[test]
    fn frame_shader_modes_pin_genxml() {
        // v10.xml:1661-1666.
        assert_eq!(
            [FRAME_SHADER_NEVER, FRAME_SHADER_ALWAYS, FRAME_SHADER_INTERSECT, FRAME_SHADER_EARLY_ZS_ALWAYS],
            [0, 1, 2, 3]
        );
        // pan_fb_preload.c:1179, v10.xml:1675-1676.
        assert_eq!((FRAME_SHADER_DCD_COLOUR, FRAME_SHADER_DCD_ZS), (0, 1));
        assert_eq!(PRELOAD_SAMPLE_MASK, 0xffff);
        const { assert!(!ZS_PRELOAD_ENABLE) };
    }

    #[test]
    fn fix_mode_as_pan_desc_h() {
        assert_eq!(fix_frame_shader_mode(FRAME_SHADER_INTERSECT, false), FRAME_SHADER_INTERSECT);
        assert_eq!(fix_frame_shader_mode(FRAME_SHADER_INTERSECT, true), FRAME_SHADER_ALWAYS);
        // only INTERSECT is promoted
        for m in [FRAME_SHADER_NEVER, FRAME_SHADER_ALWAYS, FRAME_SHADER_EARLY_ZS_ALWAYS] {
            assert_eq!(fix_frame_shader_mode(m, true), m);
        }
    }

    #[test]
    fn source_follows_the_store_contract() {
        // a whole clear loads nothing, whatever the slot holds
        assert_eq!(colour_preload_source(true, Some(true)), None);
        assert_eq!(colour_preload_source(true, None), None);
        // the slot holds samples of earlier jobs
        assert_eq!(colour_preload_source(false, Some(true)), Some(PreloadSource::SampleStore));
        // a new or cleared slot, or no slot (store returned None): rb.mem
        assert_eq!(colour_preload_source(false, Some(false)), Some(PreloadSource::Resolved));
        assert_eq!(colour_preload_source(false, None), Some(PreloadSource::Resolved));
    }

    #[test]
    fn preload_from_the_sample_store() {
        // 64x32 RGBA8: row stride 256, one plane 8192 bytes.
        let r = colour_preload_from_store(256, 32, false).unwrap();
        assert_eq!(
            r,
            PreloadRead {
                dcd_index: 0,
                pre_frame_mode: FRAME_SHADER_INTERSECT,
                source_msaa: MSAA_LAYERED,
                texture_sample_count_log2: 2,
                plane_size: 4 * 8192,
                slice_stride: 8192,
                multisample_enable: true,
                evaluate_per_sample: true,
                sample_mask: 0xffff,
            }
        );
        // the store is layered planes, not one interleaved surface
        assert_ne!(r.source_msaa, MSAA_MULTIPLE);
        // a resolving frame writes every tile: ALWAYS
        assert_eq!(colour_preload_from_store(256, 32, true).unwrap().pre_frame_mode, FRAME_SHADER_ALWAYS);
        // the store size agrees with msaa_store's packed size at a 64-byte row
        assert_eq!(Some(r.plane_size as usize), sample_size(64, 32, 4).bytes());
    }

    #[test]
    fn store_plane_refusals() {
        assert_eq!(store_surface_stride(256, 32), Some(8192));
        assert_eq!(store_surface_stride(100, 32), None, "row stride not a multiple of 64");
        assert_eq!(store_surface_stride(0, 32), None);
        assert_eq!(store_surface_stride(256, 0), None);
        // four planes must fit the 32-bit Size (v10.xml:1363)
        assert_eq!(store_surface_stride(1 << 16, 1 << 14), None);
        assert_eq!(store_surface_stride(1 << 16, (1 << 14) - 1), Some((1 << 30) - (1 << 16)));
        assert_eq!(colour_preload_from_store(100, 32, false), None);
    }

    #[test]
    fn preload_from_the_resolved_image() {
        // the store returned None: rb.mem into every sample
        let r = colour_preload_from_resolved(100, 32, false).unwrap();
        assert_eq!(
            r,
            PreloadRead {
                dcd_index: 0,
                pre_frame_mode: FRAME_SHADER_INTERSECT,
                source_msaa: MSAA_SINGLE,
                texture_sample_count_log2: 0,
                plane_size: 3200,
                slice_stride: 0,
                multisample_enable: false,
                evaluate_per_sample: false,
                sample_mask: 0xffff,
            }
        );
        assert_eq!(colour_preload_from_resolved(100, 32, true).unwrap().pre_frame_mode, FRAME_SHADER_ALWAYS);
        assert_eq!(colour_preload_from_resolved(0, 32, false), None);
        assert_eq!(colour_preload_from_resolved(1 << 16, 1 << 16, false), None);
    }

    #[test]
    fn colour_preload_dispatch_and_fallback() {
        assert_eq!(
            colour_preload(PreloadSource::SampleStore, 256, 32, false),
            colour_preload_from_store(256, 32, false)
        );
        assert_eq!(
            colour_preload(PreloadSource::Resolved, 256, 32, false),
            colour_preload_from_resolved(256, 32, false)
        );
        // a store plane that cannot be built reads rb.mem instead
        assert_eq!(
            colour_preload(PreloadSource::SampleStore, 100, 32, false),
            colour_preload_from_resolved(100, 32, false)
        );
    }

    #[test]
    fn zs_preload_is_always_single_sampled() {
        let r = zs_preload_from_resolved(256, 16).unwrap();
        assert_eq!(r.dcd_index, FRAME_SHADER_DCD_ZS);
        assert_eq!(r.pre_frame_mode, FRAME_SHADER_EARLY_ZS_ALWAYS);
        assert_eq!((r.source_msaa, r.texture_sample_count_log2), (MSAA_SINGLE, 0));
        assert_eq!((r.plane_size, r.slice_stride), (4096, 0));
        assert!(!r.multisample_enable && !r.evaluate_per_sample);
        assert_eq!(zs_preload_from_resolved(256, 0), None);
    }
}
