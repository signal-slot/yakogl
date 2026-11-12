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
}
