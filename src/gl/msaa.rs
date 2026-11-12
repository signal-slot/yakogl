// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The multisample renderbuffer rules of ES 3.0 as pure functions
//! (docs/notes/es3-msaa-renderbuffers.md, (a) and (b) items 1-3 and 10).
//!
//! Each takes the back end's `max_samples` (0 = single-sampled only, 4 = the
//! only other value) as a plain argument, so the rules are tested without a
//! context. GL_MAX_SAMPLES (state.rs) asks [`max_samples_query`] with
//! [`backend_max_samples`]; `gl-msaa-cap` wires the rest into
//! glRenderbufferStorageMultisample, glGetInternalformativ, the storage cap
//! and the framebuffer completeness check.

use super::consts::*;
use super::framebuffer::{integer_surf_format, renderbuffer_surf_format};
use super::types::GLenum;
use crate::backend;

/// The sample count of a 4x renderbuffer, the only one a back end keeps.
pub(crate) const MS_SAMPLES: u32 = 4;

/// Most pixels a multisampled renderbuffer may have: its samples are four
/// times the surface per attachment (64 MiB at RGBA8), note (b)3.
pub(crate) const MS_MAX_PIXELS: u64 = 2048 * 2048;

/// The samples the back end keeps for a multisampled renderbuffer: 0 (only
/// single-sampled storage) on every back end today. gl-msaa-cap makes it
/// return the back end's `YakoglBackend.max_samples` field (0 or 4).
pub(crate) fn backend_max_samples() -> u32 {
    0
}

/// GL_MAX_SAMPLES: never below 1, whatever the back end keeps.
pub(crate) fn max_samples_query(max: u32) -> i32 {
    max.max(1) as i32
}

/// The `GL_RENDERBUFFER_SAMPLES` glRenderbufferStorageMultisample stores for
/// a request, or the error it raises (ES 3.0 4.4.2). `integer_format` is the
/// ES 3.0 colour-renderable integer class, which has no multisampled storage.
///
/// 1..=max gives 4 (ES 3.0 4.4.2.1 allows "at least"). With `max` 0 a request
/// of 1 keeps what the code stores today (1: sample buffers for completeness,
/// one sample in fact); any larger count is refused as today.
pub(crate) fn stored_samples(requested: i32, max: u32, integer_format: bool) -> Result<u32, GLenum> {
    if requested < 0 {
        return Err(GL_INVALID_VALUE);
    }
    let requested = requested as u32;
    if requested == 0 {
        return Ok(0);
    }
    if requested > max.max(1) || integer_format {
        return Err(GL_INVALID_OPERATION);
    }
    Ok(if max >= MS_SAMPLES { MS_SAMPLES } else { 1 })
}

/// Is a multisampled renderbuffer of this size GL_OUT_OF_MEMORY?
pub(crate) fn oom(samples: u32, w: u32, h: u32) -> bool {
    samples > 1 && u64::from(w) * u64::from(h) > MS_MAX_PIXELS
}

/// Can a renderbuffer of this format have samples? The ES 3.0 colour list
/// and the depth/stencil formats, plus the half-float names the back end
/// draws single-sampled; the 32F ones stay out (note (b)2). Integer formats
/// are out: ES 3.0 has no multisample integer format.
pub(crate) fn multisample_renderable(internalformat: GLenum) -> bool {
    match internalformat {
        GL_RGBA8
        | GL_SRGB8_ALPHA8
        | GL_RGB10_A2
        | GL_RGB8
        | GL_RGB565
        | GL_RGBA4
        | GL_RGB5_A1
        | GL_R8
        | GL_RG8
        | GL_DEPTH_COMPONENT16
        | GL_DEPTH_COMPONENT24
        | GL_DEPTH_COMPONENT32F
        | GL_DEPTH24_STENCIL8
        | GL_DEPTH32F_STENCIL8
        | GL_STENCIL_INDEX8 => true,
        GL_RGBA16F | GL_RGB16F | GL_RG16F | GL_R16F => {
            backend::rt_format_drawable(renderbuffer_surf_format(internalformat))
        }
        _ => false,
    }
}

/// glGetInternalformativ GL_SAMPLES: the counts the format supports, largest
/// first; GL_NUM_SAMPLE_COUNTS is its length.
pub(crate) fn internalformat_sample_counts(internalformat: GLenum, max: u32) -> &'static [i32] {
    if max >= MS_SAMPLES && multisample_renderable(internalformat) {
        &[MS_SAMPLES as i32]
    } else if max == 0 && integer_surf_format(internalformat).is_none() {
        /* a single-sampled back end: the one count, 1, of every format the
         * query accepts except the integer ones (they have none) */
        &[1]
    } else {
        &[]
    }
}

/// Completeness (GL_FRAMEBUFFER_INCOMPLETE_MULTISAMPLE): every attachment has
/// the same count, a texture counting 0.
pub(crate) fn attachments_complete(mut samples: impl Iterator<Item = u32>) -> bool {
    let Some(first) = samples.next() else { return true };
    samples.all(|s| s == first)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn max_query_is_at_least_one() {
        assert_eq!(max_samples_query(0), 1);
        assert_eq!(max_samples_query(1), 1);
        assert_eq!(max_samples_query(4), 4);
    }

    #[test]
    fn stored_samples_negative_is_invalid_value() {
        for max in [0, 4] {
            assert_eq!(stored_samples(-1, max, false), Err(GL_INVALID_VALUE));
            assert_eq!(stored_samples(-1, max, true), Err(GL_INVALID_VALUE));
        }
    }

    #[test]
    fn stored_samples_zero_is_zero() {
        for max in [0, 4] {
            assert_eq!(stored_samples(0, max, false), Ok(0));
            assert_eq!(stored_samples(0, max, true), Ok(0));
        }
    }

    #[test]
    fn stored_samples_rounds_up_to_four() {
        for n in 1..=4 {
            assert_eq!(stored_samples(n, 4, false), Ok(4));
        }
    }

    #[test]
    fn stored_samples_past_max_is_invalid_operation() {
        assert_eq!(stored_samples(5, 4, false), Err(GL_INVALID_OPERATION));
        assert_eq!(stored_samples(8, 4, false), Err(GL_INVALID_OPERATION));
    }

    #[test]
    fn stored_samples_max_zero_keeps_todays_behaviour() {
        assert_eq!(stored_samples(1, 0, false), Ok(1));
        assert_eq!(stored_samples(2, 0, false), Err(GL_INVALID_OPERATION));
        assert_eq!(stored_samples(4, 0, false), Err(GL_INVALID_OPERATION));
        assert_eq!(stored_samples(1, 0, true), Err(GL_INVALID_OPERATION));
    }

    #[test]
    fn stored_samples_integer_format_is_invalid_operation() {
        assert_eq!(stored_samples(1, 4, true), Err(GL_INVALID_OPERATION));
        assert_eq!(stored_samples(4, 4, true), Err(GL_INVALID_OPERATION));
        assert_eq!(stored_samples(8, 4, true), Err(GL_INVALID_OPERATION));
    }

    #[test]
    fn oom_needs_samples_and_more_than_the_cap() {
        assert!(!oom(0, 4096, 4096));
        assert!(!oom(1, 4096, 4096));
        assert!(!oom(4, 2048, 2048));
        assert!(oom(4, 2049, 2048));
        assert!(oom(4, 4096, 4096));
        assert!(!oom(4, 0, 4096));
    }

    #[test]
    fn renderable_formats() {
        for f in [
            GL_RGBA8,
            GL_SRGB8_ALPHA8,
            GL_RGB10_A2,
            GL_RGB8,
            GL_RGB565,
            GL_RGBA4,
            GL_RGB5_A1,
            GL_R8,
            GL_RG8,
            GL_DEPTH_COMPONENT16,
            GL_DEPTH_COMPONENT24,
            GL_DEPTH_COMPONENT32F,
            GL_DEPTH24_STENCIL8,
            GL_DEPTH32F_STENCIL8,
            GL_STENCIL_INDEX8,
            GL_RGBA16F,
            GL_RGB16F,
            GL_RG16F,
            GL_R16F,
        ] {
            assert!(multisample_renderable(f), "{f:#x}");
        }
        for f in [GL_R32F, GL_R11F_G11F_B10F, GL_R8I, GL_RGBA8UI, GL_RGB10_A2UI, GL_RGBA32F, GL_RGBA, GL_NONE] {
            assert!(!multisample_renderable(f), "{f:#x}");
        }
    }

    #[test]
    fn sample_counts() {
        assert_eq!(internalformat_sample_counts(GL_RGBA8, 4), &[4]);
        assert_eq!(internalformat_sample_counts(GL_DEPTH24_STENCIL8, 4), &[4]);
        assert_eq!(internalformat_sample_counts(GL_RGBA8, 0), &[1]);
        assert_eq!(internalformat_sample_counts(GL_DEPTH24_STENCIL8, 0), &[1]);
        assert_eq!(internalformat_sample_counts(GL_RGB10_A2, 0), &[1]);
        assert_eq!(internalformat_sample_counts(GL_R32F, 0), &[1]);
        assert!(internalformat_sample_counts(GL_R8I, 0).is_empty());
        assert!(internalformat_sample_counts(GL_RGBA32UI, 0).is_empty());
        assert!(internalformat_sample_counts(GL_RGB10_A2UI, 0).is_empty());
        assert!(internalformat_sample_counts(GL_R32F, 4).is_empty());
        assert!(internalformat_sample_counts(GL_RGBA8UI, 4).is_empty());
    }

    #[test]
    fn completeness_compares_all_counts() {
        assert!(attachments_complete([].into_iter()));
        assert!(attachments_complete([0].into_iter()));
        assert!(attachments_complete([4, 4, 4].into_iter()));
        assert!(attachments_complete([0, 0].into_iter()));
        assert!(!attachments_complete([4, 0].into_iter()));
        assert!(!attachments_complete([0, 4].into_iter()));
        assert!(!attachments_complete([4, 4, 1].into_iter()));
    }
}
