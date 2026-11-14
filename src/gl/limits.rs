// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The ES 3.0 implementation-dependent limit minimums (ES 3.0.6 Tables 6.32,
//! 6.33) and the formula that ties the combined uniform components to the
//! default-block and uniform-block limits (section 2.11.4):
//!
//! `MAX_COMBINED_{VERTEX,FRAGMENT}_UNIFORM_COMPONENTS =
//!  MAX_{VERTEX,FRAGMENT}_UNIFORM_COMPONENTS
//!  + MAX_{VERTEX,FRAGMENT}_UNIFORM_BLOCKS * MAX_UNIFORM_BLOCK_SIZE / 4`.
//!
//! Pure arithmetic, no GPU: it compiles on every feature set.

/// ES 3.0 minimum of `MAX_UNIFORM_BLOCK_SIZE`, in bytes.
pub const ES3_MIN_MAX_UNIFORM_BLOCK_SIZE: usize = 16384;
/// ES 3.0 minimum of `MAX_VERTEX_UNIFORM_COMPONENTS`.
pub const ES3_MIN_MAX_VERTEX_UNIFORM_COMPONENTS: usize = 1024;
/// ES 3.0 minimum of `MAX_FRAGMENT_UNIFORM_COMPONENTS`.
pub const ES3_MIN_MAX_FRAGMENT_UNIFORM_COMPONENTS: usize = 896;
/// ES 3.0 minimum of `MAX_VERTEX_UNIFORM_BLOCKS`.
pub const ES3_MIN_MAX_VERTEX_UNIFORM_BLOCKS: usize = 12;
/// ES 3.0 minimum of `MAX_FRAGMENT_UNIFORM_BLOCKS`.
pub const ES3_MIN_MAX_FRAGMENT_UNIFORM_BLOCKS: usize = 12;
/// ES 3.0 minimum of `MAX_COMBINED_VERTEX_UNIFORM_COMPONENTS`.
pub const ES3_MIN_MAX_COMBINED_VERTEX_UNIFORM_COMPONENTS: usize = 50176;
/// ES 3.0 minimum of `MAX_COMBINED_FRAGMENT_UNIFORM_COMPONENTS`.
pub const ES3_MIN_MAX_COMBINED_FRAGMENT_UNIFORM_COMPONENTS: usize = 50048;

/// ES 3.0 minimum of `MAX_VARYING_VECTORS`.
pub const ES3_MIN_MAX_VARYING_VECTORS: usize = 15;
/// ES 3.0 minimum of `MAX_VARYING_COMPONENTS`.
pub const ES3_MIN_MAX_VARYING_COMPONENTS: usize = 60;
/// ES 3.0 minimum of `MAX_FRAGMENT_INPUT_COMPONENTS`.
pub const ES3_MIN_MAX_FRAGMENT_INPUT_COMPONENTS: usize = 60;
/// ES 3.0 minimum of `MAX_VERTEX_OUTPUT_COMPONENTS`.
pub const ES3_MIN_MAX_VERTEX_OUTPUT_COMPONENTS: usize = 64;

/// `MAX_COMBINED_*_UNIFORM_COMPONENTS` of one stage: the default block's
/// components plus `uniform_blocks` blocks of `block_size_bytes` bytes, each
/// counted in 4-byte components (`MAX_UNIFORM_BLOCK_SIZE / 4`).
pub const fn combined_uniform_components(
    default_block_components: usize,
    uniform_blocks: usize,
    block_size_bytes: usize,
) -> usize {
    default_block_components + uniform_blocks * (block_size_bytes / 4)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vertex_minimum_row() {
        assert_eq!(
            combined_uniform_components(
                ES3_MIN_MAX_VERTEX_UNIFORM_COMPONENTS,
                ES3_MIN_MAX_VERTEX_UNIFORM_BLOCKS,
                ES3_MIN_MAX_UNIFORM_BLOCK_SIZE
            ),
            ES3_MIN_MAX_COMBINED_VERTEX_UNIFORM_COMPONENTS
        );
        assert_eq!(ES3_MIN_MAX_COMBINED_VERTEX_UNIFORM_COMPONENTS, 50176);
    }

    #[test]
    fn fragment_minimum_row() {
        assert_eq!(
            combined_uniform_components(
                ES3_MIN_MAX_FRAGMENT_UNIFORM_COMPONENTS,
                ES3_MIN_MAX_FRAGMENT_UNIFORM_BLOCKS,
                ES3_MIN_MAX_UNIFORM_BLOCK_SIZE
            ),
            ES3_MIN_MAX_COMBINED_FRAGMENT_UNIFORM_COMPONENTS
        );
        assert_eq!(ES3_MIN_MAX_COMBINED_FRAGMENT_UNIFORM_COMPONENTS, 50048);
    }

    #[test]
    fn const_evaluable_and_other_sizes() {
        const C: usize = combined_uniform_components(1024, 12, 16384);
        assert_eq!(C, 50176);
        // The current 6144-byte block: 1024 + 12 * 1536.
        assert_eq!(combined_uniform_components(1024, 12, 6144), 19456);
        assert_eq!(combined_uniform_components(0, 0, 16384), 0);
    }

    #[test]
    fn varying_minimums() {
        assert_eq!(ES3_MIN_MAX_VARYING_VECTORS * 4, ES3_MIN_MAX_VARYING_COMPONENTS);
        assert_eq!(ES3_MIN_MAX_FRAGMENT_INPUT_COMPONENTS, 60);
        assert_eq!(ES3_MIN_MAX_VERTEX_OUTPUT_COMPONENTS, 64);
    }
}
