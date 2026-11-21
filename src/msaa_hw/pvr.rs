// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The PowerVR (BXS-4-64, AM62P, `src/pvr`) words of a 4x multisample
//! framebuffer object and of its job-end downscale resolve, as plain
//! integers named after the csbgen fields (docs/notes/es3-msaa-renderbuffers.md
//! (c1), (f2), (f5)). Not wired: `pvr-msaa-fbo` calls them from `src/pvr`.
//!
//! Every value is the one `src/pvr/backend.rs` already programs for the 4x
//! window (the `isp_aa`, `isp_ctl`, `pbe_reg_words`, `ms_bytes`, `ds_bytes`,
//! `tex_desc_set_samples` and `fs_sample_rate` there, cited by line at each
//! item), with the field positions of Mesa 26.2.1's csbgen XML
//! (`third_party/mesa-26.2.1/src/imagination/csbgen/rogue/{cr,pbestate,texstate}.xml`,
//! read for the bit numbers only). The tests below recompute the window's
//! words from these and compare them with the constants `src/pvr` pins
//! (`ISP_CTL_DEFAULT` / `ISP_CTL_MSAA4`, `src/pvr/backend.rs:10721-10723`;
//! `PBE_WORD0_DOWNSCALE`, `src/pvr/backend_tests.rs:5406`).
//!
//! What `pvr-msaa-fbo` must do with these ((c1), (f5)):
//!
//! 1. Draw a 4x FBO as the window: [`isp_aa_mode`], [`isp_ctl`] (two tiles in
//!    flight at four registers a pixel, [`isp_tiles_in_flight`]), one colour
//!    attachment only ([`MAX_COLOUR_ATTACHMENTS_4X`]).
//! 2. Resolve at every job end through the PBE: the target's REG_WORD0 with
//!    [`pbe_reg_word0_resolve_bits`] (downscale, and gamma for sRGB), and,
//!    because an FBO kick never ends a swap ([`stores_samples`]), a second
//!    emit of the same words without downscale into the sample store of
//!    [`colour_store_bytes`] ([`pbe_reg_word0_store_bits`]).
//! 3. Load the store in the next kick as a TPU image of [`smpcnt`] samples
//!    ([`tex_image_word0_smpcnt_bits`]) at the full sample rate
//!    ([`load_sample_rate`]).
//! 4. Depth/stencil stays in the ISP's 2x2-scaled plane of [`ds_bytes`];
//!    its resolve takes the word [`ds_sample0_word`] names, not the PBE
//!    ((f2): Mesa resolves Z/S with a transfer pass; which sample that word
//!    holds is **board only**, (e)6).
//!
//! **Unmeasured on the board** (#1289, (e)8): that the PBE's unresolved
//! layout is the TPU's `smpcnt` layout over the same stride, and the order
//! of averaging and sRGB encode in the PBE's downscale ((e)7).

use crate::msaa_store::SampleSize;

/// Samples of a 4x framebuffer object.
pub const SAMPLES_4X: u32 = 4;

/// Colour attachments a 4x frame may have on this back end: one. A wider
/// pixel at 4x (8 registers) is 8192 dwords a tile, the whole reserved
/// partition store, so multi-attachment 4x is refused by name (note (c1)
/// "Tile and memory", (e)5).
pub const MAX_COLOUR_ATTACHMENTS_4X: u32 = 1;

/* ---- ISP ------------------------------------------------------------- */

/// `ISP_AA_MODE_TYPE` (cr.xml:47-52; `src/pvr/csbgen.rs:22-25`).
pub const ISP_AA_MODE_AA_NONE: u32 = 0;
pub const ISP_AA_MODE_AA_2X: u32 = 1;
pub const ISP_AA_MODE_AA_4X: u32 = 2;

/// CR_ISP_AA for a frame of `samples` (its only field, `mode`, bits 0..1,
/// cr.xml:523-525): 4 is `AA_4X`, 2 `AA_2X`, anything else `AA_NONE`, as
/// `isp_aa` (`src/pvr/backend.rs:646-653`).
pub const fn isp_aa_mode(samples: u32) -> u32 {
    match samples {
        4 => ISP_AA_MODE_AA_4X,
        2 => ISP_AA_MODE_AA_2X,
        _ => ISP_AA_MODE_AA_NONE,
    }
}

/// The screen tile is 16x16 pixels (bxs-4-64.h `tile_size_x/y`, as
/// `TILE_DWORDS` in `src/pvr/backend.rs:606`).
pub const TILE_SIZE: u32 = 16;
/// The BXS-4-64's partitions (`BXS_MAX_PARTITIONS`, `src/pvr/backend.rs:541`).
pub const MAX_PARTITIONS: u32 = 16;
/// The USC's reserved partition store in dwords
/// (`RESERVED_PARTITION_DWORDS`, `src/pvr/backend.rs:565`).
pub const RESERVED_PARTITION_DWORDS: u32 = 8192;
/// `isp_max_tiles_in_flight` (`src/pvr/backend.rs:607`).
pub const ISP_MAX_TILES_IN_FLIGHT: u32 = 6;
/// `usc_min_output_registers_per_pix` (`src/pvr/backend.rs:605`).
pub const USC_MIN_OUTPUT_REGS: u32 = 2;

/// The USC partition one tile of a frame of `samples` takes, in dwords, for
/// a pixel of `pixel_regs` output registers: the register width (at least
/// 2, a power of two) times the samples (1..4) times the tile's pixels, as
/// `isp_tiles_in_flight` (`src/pvr/backend.rs:604-616`) and note (c1).
pub const fn partition_dwords(samples: u32, pixel_regs: u32) -> u32 {
    let regs = if pixel_regs < USC_MIN_OUTPUT_REGS { USC_MIN_OUTPUT_REGS } else { pixel_regs };
    let s = if samples < 1 {
        1
    } else if samples > 4 {
        4
    } else {
        samples
    };
    regs.next_power_of_two() * s * TILE_SIZE * TILE_SIZE
}

/// The ISP's tiles in flight for a frame of `samples` whose pixels have
/// `pixel_regs` output registers: as many partitions as the reserved store
/// holds, at most 6, at least 1 (`isp_tiles_in_flight`,
/// `src/pvr/backend.rs:604-616`). 6 at 1x and 2 at 4x with four registers.
pub const fn isp_tiles_in_flight(samples: u32, pixel_regs: u32) -> u32 {
    let p = partition_dwords(samples, pixel_regs);
    let all = p * MAX_PARTITIONS;
    let usable = if RESERVED_PARTITION_DWORDS < all { RESERVED_PARTITION_DWORDS } else { all };
    let avail = if usable / p < MAX_PARTITIONS { usable / p } else { MAX_PARTITIONS };
    let t = if avail < ISP_MAX_TILES_IN_FLIGHT { avail } else { ISP_MAX_TILES_IN_FLIGHT };
    if t < 1 {
        1
    } else {
        t
    }
}

/// CR_ISP_CTL fields `src/pvr` sets (cr.xml:527-546): `skip_init_hdrs`
/// bit 31, `dbias_is_int` bit 20, `process_empty_tiles` bit 17,
/// `sample_pos` bit 16, `pipe_enable` bits 12..15.
pub const ISP_CTL_SKIP_INIT_HDRS: u32 = 1 << 31;
pub const ISP_CTL_DBIAS_IS_INT: u32 = 1 << 20;
pub const ISP_CTL_PROCESS_EMPTY_TILES: u32 = 1 << 17;
pub const ISP_CTL_SAMPLE_POS: u32 = 1 << 16;
pub const ISP_CTL_PIPE_ENABLE_SHIFT: u32 = 12;

/// CR_ISP_CTL as `isp_ctl` packs it (`src/pvr/backend.rs:630-643`): the
/// three fixed bits, `dbias_is_int` for an integer ZLS format, and
/// `pipe_enable` = tiles in flight - 1.
pub const fn isp_ctl(samples: u32, pixel_regs: u32, dbias_is_int: bool) -> u32 {
    ISP_CTL_SKIP_INIT_HDRS
        | ISP_CTL_PROCESS_EMPTY_TILES
        | ISP_CTL_SAMPLE_POS
        | if dbias_is_int { ISP_CTL_DBIAS_IS_INT } else { 0 }
        | (isp_tiles_in_flight(samples, pixel_regs) - 1) << ISP_CTL_PIPE_ENABLE_SHIFT
}

/// The window's CR_ISP_CTL at four registers a pixel: 1x and 4x
/// (`ISP_CTL_DEFAULT`, `ISP_CTL_MSAA4`, `src/pvr/backend.rs:10721-10723`,
/// pinned by `src/pvr/backend_csbgen_tests.rs:315-319`).
pub const ISP_CTL_WINDOW_1X: u32 = 0x8003_5000;
pub const ISP_CTL_WINDOW_4X: u32 = 0x8003_1000;

/* ---- PBE: the end-of-tile store and its downscale resolve -------------- */

/// PBESTATE_REG_WORD0 `downscale` (pbestate.xml:255, bit 56 of the 64-bit
/// word; `PBE_WORD0_DOWNSCALE`, `src/pvr/backend_tests.rs:5406`): the PBE
/// averages the tile's samples on emit, the job-end resolve (note (c1)
/// "Resolve", (f2): Mesa `pvr_arch_job_common.c:282`).
pub const PBE_REG_WORD0_DOWNSCALE: u64 = 1 << 56;
/// PBESTATE_REG_WORD0 `gamma` (pbestate.xml:268, bit 4): the PBE encodes
/// R, G, B to sRGB on store (`pbe_reg_words` sets it for an sRGB target,
/// `src/pvr/backend.rs:942-963`, `gamma: gles_surf_is_srgb(fmt)`).
pub const PBE_REG_WORD0_GAMMA: u64 = 1 << 4;

/// Does the PBE resolve a frame of `samples` on its end-of-tile store? Any
/// multisampled frame: the window's call passes `b.samples > 1`
/// (`src/pvr/backend.rs:10022`).
pub const fn pbe_downscale(samples: u32) -> bool {
    samples > 1
}

/// The REG_WORD0 bits of the resolved emit into the single-sample target
/// (the renderbuffer's `rb.mem`): `downscale` on a multisampled frame,
/// `gamma` for an sRGB target (`pbe_reg_words`, `src/pvr/backend.rs:942-963`,
/// called at `:10022`). The averaging is in the F16 source, before the
/// encode, by Mesa's reading (note (f2)); on this board: unmeasured ((e)7).
pub const fn pbe_reg_word0_resolve_bits(samples: u32, srgb: bool) -> u64 {
    (if pbe_downscale(samples) { PBE_REG_WORD0_DOWNSCALE } else { 0 }) | if srgb { PBE_REG_WORD0_GAMMA } else { 0 }
}

/// The REG_WORD0 bits of the unresolved emit into the sample store: the
/// target's words without `downscale`, same stride and layout, untwiddled
/// (`src/pvr/backend.rs:9972-9987`; `src/pvr/backend_tests.rs:5451-5453`).
pub const fn pbe_reg_word0_store_bits(srgb: bool) -> u64 {
    pbe_reg_word0_resolve_bits(1, srgb)
}

/// Does a kick of `samples` keep its unresolved samples (the second emit)?
/// A multisampled kick that does not end a swap (`MsStore`,
/// `src/pvr/backend.rs:5361-5375`). An FBO never ends a swap, so every 4x
/// FBO kick takes the two-store path (note (c1) "Sample storage").
pub const fn stores_samples(samples: u32, ends_swap: bool) -> bool {
    samples > 1 && !ends_swap
}

/// PBE emits a kick of `samples` makes for its one colour attachment: the
/// resolved target, plus the sample store when [`stores_samples`]
/// (`num_rt` 2, `src/pvr/backend_tests.rs:5444`).
pub const fn pbe_emits(samples: u32, ends_swap: bool) -> u32 {
    if stores_samples(samples, ends_swap) {
        2
    } else {
        1
    }
}

/* ---- TPU: the next kick's load of the sample store --------------------- */

/// TEXSTATE_IMAGE_WORD0 `smpcnt` starts at bit 62, two bits (texstate.xml:273).
pub const TEX_IMAGE_WORD0_SMPCNT_SHIFT: u32 = 62;

/// `smpcnt` of an image of `samples` a texel: log2 of the count (0 for 1x,
/// 2 for 4x), as `tex_desc_set_samples` (`src/pvr/backend.rs:5481-5488`)
/// and the test reading it back (`src/pvr/backend_tests.rs:5471`).
pub const fn smpcnt(samples: u32) -> u32 {
    if samples <= 1 {
        0
    } else {
        samples.ilog2()
    }
}

/// The IMAGE_WORD0 bits of a sample-store image of `samples`.
pub const fn tex_image_word0_smpcnt_bits(samples: u32) -> u64 {
    (smpcnt(samples) as u64) << TEX_IMAGE_WORD0_SMPCNT_SHIFT
}

/// PDS DOUTU sample rate (`src/pvr/backend.rs:667-668`): per pixel or per
/// covered sample.
pub const PDS_DOUTU_SAMPLE_RATE_INSTANCE: u32 = 0;
pub const PDS_DOUTU_SAMPLE_RATE_FULL: u32 = 2;

/// The rate the load quad reading the sample store runs at: FULL on a
/// multisampled frame, so each instance reads and writes its own sample
/// (`fs_sample_rate` with `loads_samples`, `src/pvr/backend.rs:690-696`,
/// `:8616`).
pub const fn load_sample_rate(samples: u32) -> u32 {
    if samples > 1 {
        PDS_DOUTU_SAMPLE_RATE_FULL
    } else {
        PDS_DOUTU_SAMPLE_RATE_INSTANCE
    }
}

/* ---- sample-buffer geometry ------------------------------------------- */

/// Bytes of the colour sample store of an `h`-row target whose rows are
/// `pitch` pixels apart, `bytes_per_pixel` a pixel, at `samples`: the TPU's
/// multisampled image, `samples` single-sampled images' worth (`ms_bytes`,
/// `src/pvr/backend.rs:5399-5404`). `None` on overflow.
pub const fn colour_store_bytes(h: u32, pitch: u32, bytes_per_pixel: u32, samples: u32) -> Option<u64> {
    // pitch * h fits a u64; the next two products may not.
    match ((pitch as u64) * (h as u64)).checked_mul(bytes_per_pixel as u64) {
        Some(b) => b.checked_mul(samples as u64),
        None => None,
    }
}

/// The row pitch, in pixels, of a renderbuffer `w` wide in the GL core's
/// storage: whole pixel pairs (`tex_pitch`, `src/gl/context.rs:296-298`;
/// the PBE's linestride counts pairs, `src/pvr/backend.rs:942-952`).
pub const fn colour_pitch(w: u32) -> u32 {
    w.wrapping_add(1) & !1
}

/// The shared store's form of a `w` x `h` attachment of `bytes_per_pixel`
/// at 4x, for [`crate::msaa_store::MsaaStore::get_or_alloc`]: at the GL
/// core's pitch ([`colour_pitch`]), so its packed bytes are
/// [`colour_store_bytes`].
pub const fn sample_size(w: u32, h: u32, bytes_per_pixel: u32) -> SampleSize {
    SampleSize { w: colour_pitch(w), h, bytes_per_sample: bytes_per_pixel, samples: SAMPLES_4X }
}

/// The ISP's Z load/store tile: 32 pixels (`ZLS_TILE`,
/// `src/pvr/backend.rs:730`, the 16x16 tile doubled).
pub const ZLS_TILE: u32 = 32;
/// Bytes a depth/stencil sample: packed D24S8, 4 bytes a pixel (`src/pvr/backend.rs:721-722`).
pub const DS_BYTES_PER_SAMPLE: u64 = 4;

/// The ISP's scale of a frame of `samples`: `(x, y)` samples a pixel spans,
/// 2 by 2 at 4x, 1 by 2 at 2x (`ds_sample0`, `src/pvr/backend.rs:5331-5336`;
/// `ds_bytes` `:5344-5349`).
pub const fn isp_scale(samples: u32) -> (u32, u32) {
    match samples {
        4 => (2, 2),
        2 => (1, 2),
        _ => (1, 1),
    }
}

/// Bytes of the packed D24S8 plane of a `w` x `h` target at `samples`:
/// whole ZLS tiles, the width and height scaled as [`isp_scale`]
/// (`ds_bytes`, `src/pvr/backend.rs:5344-5349`). `None` on overflow.
pub const fn ds_bytes(w: u32, h: u32, samples: u32) -> Option<u64> {
    let (sx, sy) = isp_scale(samples);
    let aw = (w.div_ceil(ZLS_TILE) as u64) * (ZLS_TILE as u64) * (sx as u64);
    let ah = (h.div_ceil(ZLS_TILE) as u64) * (ZLS_TILE as u64) * (sy as u64);
    match aw.checked_mul(ah) {
        Some(px) => px.checked_mul(DS_BYTES_PER_SAMPLE),
        None => None,
    }
}

/// The D24S8 word a resolve reads for pixel (x, y) of a `w`-wide target at
/// `samples`: the top-left of its spread, the row the ZLS-aligned width
/// times the x scale (`ds_sample0`, `src/pvr/backend.rs:5331-5340`). Which
/// sample the hardware stores there is **board only** (note (e)6).
pub const fn ds_sample0_word(w: u32, samples: u32, x: u32, y: u32) -> u64 {
    let (sx, sy) = isp_scale(samples);
    let row = (w.div_ceil(ZLS_TILE) as u64) * (ZLS_TILE as u64) * (sx as u64);
    (y as u64) * (sy as u64) * row + (x as u64) * (sx as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn isp_aa_mode_is_the_window_mode() {
        // src/pvr/backend_csbgen_tests.rs:320-322.
        for (s, mode) in [(0, 0), (1, 0), (2, 1), (4, 2), (8, 0)] {
            assert_eq!(isp_aa_mode(s), mode, "{s} samples");
        }
        assert_eq!(isp_aa_mode(SAMPLES_4X), ISP_AA_MODE_AA_4X);
    }

    #[test]
    fn tiles_in_flight_and_partitions() {
        // src/pvr/backend_tests.rs:5273: (1, 4, 6), (4, 4, 2).
        assert_eq!(isp_tiles_in_flight(1, 4), 6);
        assert_eq!(isp_tiles_in_flight(4, 4), 2);
        assert_eq!(partition_dwords(4, 4), 4096);
        // note (c1): 8 registers at 4x is 8192 dwords a tile, one tile.
        assert_eq!(partition_dwords(4, 8), 8192);
        assert_eq!(isp_tiles_in_flight(4, 8), 1);
        // the register floor and the sample clamp.
        assert_eq!(partition_dwords(0, 1), partition_dwords(1, 2));
        assert_eq!(partition_dwords(8, 4), partition_dwords(4, 4));
        for s in [1, 2, 4] {
            for r in [1, 2, 4, 8] {
                let t = isp_tiles_in_flight(s, r);
                assert!(t >= 1 && t * partition_dwords(s, r) <= RESERVED_PARTITION_DWORDS, "{s} {r}");
            }
        }
    }

    #[test]
    fn isp_ctl_matches_the_window_words() {
        // src/pvr/backend_csbgen_tests.rs:314-319.
        for s in [0, 1] {
            assert_eq!(isp_ctl(s, 4, false), ISP_CTL_WINDOW_1X);
            assert_eq!(isp_ctl(s, 4, true), ISP_CTL_WINDOW_1X | ISP_CTL_DBIAS_IS_INT);
        }
        assert_eq!(isp_ctl(4, 4, false), ISP_CTL_WINDOW_4X);
        assert_eq!(isp_ctl(4, 4, true), ISP_CTL_WINDOW_4X | ISP_CTL_DBIAS_IS_INT);
        assert_eq!((isp_ctl(4, 4, false) >> ISP_CTL_PIPE_ENABLE_SHIFT) & 0xf, 1);
    }

    #[test]
    fn pbe_resolve_and_store_bits() {
        // src/pvr/backend_tests.rs:5406.
        assert_eq!(PBE_REG_WORD0_DOWNSCALE, 1u64 << 56);
        assert!(pbe_downscale(4) && pbe_downscale(2));
        assert!(!pbe_downscale(1) && !pbe_downscale(0));
        assert_eq!(pbe_reg_word0_resolve_bits(4, false), PBE_REG_WORD0_DOWNSCALE);
        assert_eq!(pbe_reg_word0_resolve_bits(4, true), PBE_REG_WORD0_DOWNSCALE | PBE_REG_WORD0_GAMMA);
        assert_eq!(pbe_reg_word0_resolve_bits(1, false), 0);
        assert_eq!(pbe_reg_word0_resolve_bits(1, true), PBE_REG_WORD0_GAMMA);
        // the store's emit is the target's words without downscale
        // (src/pvr/backend_tests.rs:5453).
        for srgb in [false, true] {
            assert_eq!(pbe_reg_word0_store_bits(srgb), pbe_reg_word0_resolve_bits(4, srgb) & !PBE_REG_WORD0_DOWNSCALE);
        }
    }

    #[test]
    fn emits_per_kick() {
        // an FBO never ends a swap: every 4x FBO kick stores its samples.
        assert!(stores_samples(4, false));
        assert_eq!(pbe_emits(4, false), 2);
        // the window's swap kick and every 1x kick: the target only.
        assert!(!stores_samples(4, true));
        assert_eq!(pbe_emits(4, true), 1);
        assert_eq!(pbe_emits(1, false), 1);
    }

    #[test]
    fn smpcnt_and_load_rate() {
        // src/pvr/backend_tests.rs:5471: smpcnt of 4 samples is 2.
        assert_eq!(smpcnt(4), 2);
        assert_eq!(smpcnt(2), 1);
        assert_eq!(smpcnt(1), 0);
        assert_eq!(smpcnt(0), 0);
        assert_eq!(tex_image_word0_smpcnt_bits(4) >> 62, 2);
        assert_eq!(tex_image_word0_smpcnt_bits(1), 0);
        assert_eq!(load_sample_rate(4), PDS_DOUTU_SAMPLE_RATE_FULL);
        assert_eq!(load_sample_rate(1), PDS_DOUTU_SAMPLE_RATE_INSTANCE);
        assert_eq!((PDS_DOUTU_SAMPLE_RATE_INSTANCE, PDS_DOUTU_SAMPLE_RATE_FULL), (0, 2));
    }

    #[test]
    fn colour_store_geometry() {
        // note (c): 128x128 RGBA8 at 4x is 256 KiB of samples.
        assert_eq!(colour_store_bytes(128, colour_pitch(128), 4, 4), Some(256 * 1024));
        assert_eq!(colour_store_bytes(128, 128, 4, 1), Some(64 * 1024));
        // an odd width stores whole pixel pairs.
        assert_eq!(colour_pitch(65), 66);
        assert_eq!(colour_pitch(64), 64);
        assert_eq!(colour_store_bytes(10, colour_pitch(65), 8, 4), Some(66 * 10 * 8 * 4));
        assert_eq!(colour_store_bytes(u32::MAX, u32::MAX, 16, 4), None);
        let s = sample_size(65, 10, 4);
        assert_eq!(s, SampleSize { w: 66, h: 10, bytes_per_sample: 4, samples: 4 });
        assert_eq!(s.bytes().map(|b| b as u64), colour_store_bytes(10, 66, 4, 4));
    }

    #[test]
    fn ds_geometry() {
        // whole ZLS tiles: 1x 64x64 is 64*64*4; 4x doubles both sides.
        assert_eq!(ds_bytes(64, 64, 1), Some(64 * 64 * 4));
        assert_eq!(ds_bytes(64, 64, 4), Some(128 * 128 * 4));
        assert_eq!(ds_bytes(64, 64, 2), Some(64 * 128 * 4));
        assert_eq!(ds_bytes(33, 1, 1), Some(64 * 32 * 4));
        assert_eq!(ds_bytes(33, 1, 4), Some(128 * 64 * 4));
        // sample 0's word: (2x, 2y) of the 4x plane, (x, y) of the 1x one.
        assert_eq!(ds_sample0_word(64, 1, 3, 5), 5 * 64 + 3);
        assert_eq!(ds_sample0_word(64, 4, 3, 5), 10 * 128 + 6);
        assert_eq!(ds_sample0_word(64, 2, 3, 5), 10 * 64 + 3);
        // the last pixel's word lies inside the plane.
        for s in [1, 2, 4] {
            let (w, h) = (100, 70);
            let words = ds_bytes(w, h, s).unwrap() / DS_BYTES_PER_SAMPLE;
            assert!(ds_sample0_word(w, s, w - 1, h - 1) < words, "{s} samples");
        }
    }
}
