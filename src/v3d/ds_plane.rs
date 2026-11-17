// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Read-modify-write helpers for the V3D's depth/stencil planes, as pure
//! functions over raster scratches (docs/notes/v3d-fbo-blit.md 3.4).
//!
//! The V3D keeps one `ZsBuf` per depth/stencil id in two layouts: D24S8
//! (4 bytes a pixel, depth unorm24 in bits 31..8, stencil in bits 7..0) and a
//! separate S8 buffer (1 byte a pixel). `v3d_tex_load_box` / `v3d_tex_store_box`
//! convert the UIF level to and from a raster scratch; these functions work on
//! that scratch only, rows top-down in the layout `v3d_depth_resolve` reads.
//!
//! `src/v3d/backend.rs` includes it (`#[path]`) for `v3d_ds_write` and
//! `v3d_stencil_resolve`; `tests/v3d_ds_plane.rs` includes it the same way
//! and tests it on its own (`cargo test --test v3d_ds_plane`). It names
//! nothing of the crate.

/// A rectangle or slice the helpers refuse; the wiring maps it to `-EINVAL`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlaneError {
    /// The rectangle reaches past the buffer (or `w > pitch`).
    OutOfBounds,
    /// A slice is shorter than its rows need.
    ShortSlice,
}

/// Depth `v` (f32) as unorm24: clamped to [0, 1] (NaN is 0), rounded to
/// nearest of `v * (2^24 - 1)`; the inverse of `v3d_depth_resolve`'s
/// `(word >> 8) as f32 / 16_777_215.0` (docs/notes/v3d-fbo-blit.md 3.4).
pub fn depth_unorm24(v: f32) -> u32 {
    let d = if v >= 0.0 { v.min(1.0) } else { 0.0 };
    (d as f64 * 16_777_215.0 + 0.5) as u32
}

/// Elements a `w` x `h` rectangle at `pitch` needs: `(h - 1) * pitch + w`.
fn needed(w: u32, h: u32, pitch: u32) -> usize {
    if w == 0 || h == 0 {
        0
    } else {
        (h as usize - 1) * pitch as usize + w as usize
    }
}

/// Validates the `w` x `h` rectangle at (`x`, `y`) of a `bw` x `bh` buffer
/// of `buf_len` elements (rows `bw` apart) and a source or destination of
/// `other_len` elements at `pitch` (docs/notes/v3d-fbo-blit.md 3.4). An empty
/// rectangle inside the buffer is accepted and writes nothing.
#[allow(clippy::too_many_arguments)]
pub fn check_rect(
    bw: u32,
    bh: u32,
    buf_len: usize,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    pitch: u32,
    other_len: usize,
) -> Result<(), PlaneError> {
    if w > pitch || x as u64 + w as u64 > bw as u64 || y as u64 + h as u64 > bh as u64 {
        return Err(PlaneError::OutOfBounds);
    }
    if buf_len < bw as usize * bh as usize || other_len < needed(w, h, pitch) {
        return Err(PlaneError::ShortSlice);
    }
    Ok(())
}

/// Writes `src` (f32 depth, `pitch` elements a row) into the depth bits of
/// the `w` x `h` rectangle at (`x`, `y`) of D24S8 `words` (`bw` x `bh`),
/// keeping each word's stencil byte (docs/notes/v3d-fbo-blit.md 3.4).
#[allow(clippy::too_many_arguments)]
pub fn write_depth_d24s8(
    words: &mut [u32],
    bw: u32,
    bh: u32,
    src: &[f32],
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    pitch: u32,
) -> Result<(), PlaneError> {
    check_rect(bw, bh, words.len(), x, y, w, h, pitch, src.len())?;
    for row in 0..h as usize {
        for col in 0..w as usize {
            let word = &mut words[(y as usize + row) * bw as usize + x as usize + col];
            *word = (*word & 0xff) | depth_unorm24(src[row * pitch as usize + col]) << 8;
        }
    }
    Ok(())
}

/// Writes `src` (u8 stencil, `pitch` elements a row) into the low byte of
/// the rectangle of D24S8 `words`, keeping the depth bits
/// (docs/notes/v3d-fbo-blit.md 3.4).
#[allow(clippy::too_many_arguments)]
pub fn write_stencil_d24s8(
    words: &mut [u32],
    bw: u32,
    bh: u32,
    src: &[u8],
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    pitch: u32,
) -> Result<(), PlaneError> {
    check_rect(bw, bh, words.len(), x, y, w, h, pitch, src.len())?;
    for row in 0..h as usize {
        for col in 0..w as usize {
            let word = &mut words[(y as usize + row) * bw as usize + x as usize + col];
            *word = (*word & !0xff) | u32::from(src[row * pitch as usize + col]);
        }
    }
    Ok(())
}

/// Writes `src` (u8 stencil, `pitch` elements a row) into the rectangle of
/// the separate S8 scratch `bytes` (`bw` x `bh`, 1 byte a pixel)
/// (docs/notes/v3d-fbo-blit.md 3.4).
#[allow(clippy::too_many_arguments)]
pub fn write_stencil_s8(
    bytes: &mut [u8],
    bw: u32,
    bh: u32,
    src: &[u8],
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    pitch: u32,
) -> Result<(), PlaneError> {
    check_rect(bw, bh, bytes.len(), x, y, w, h, pitch, src.len())?;
    for row in 0..h as usize {
        let at = (y as usize + row) * bw as usize + x as usize;
        bytes[at..at + w as usize].copy_from_slice(&src[row * pitch as usize..][..w as usize]);
    }
    Ok(())
}

/// The part of a `w` x `h` read the `bw` x `bh` buffer covers, as
/// `v3d_depth_resolve` clips it: `(w.min(bw), h.min(bh))`.
fn read_clip(bw: u32, bh: u32, w: u32, h: u32) -> (u32, u32) {
    (w.min(bw), h.min(bh))
}

/// `stencil_resolve` from D24S8: the low byte of each word of the top-left
/// `w` x `h` (clipped to the buffer like `v3d_depth_resolve`) into `dst`,
/// `pitch` elements a row; the rest of `dst` is left alone
/// (docs/notes/v3d-fbo-blit.md 3.4).
pub fn read_stencil_d24s8(
    words: &[u32],
    bw: u32,
    bh: u32,
    dst: &mut [u8],
    w: u32,
    h: u32,
    pitch: u32,
) -> Result<(), PlaneError> {
    let (rw, rh) = read_clip(bw, bh, w, h);
    if w > pitch {
        return Err(PlaneError::OutOfBounds);
    }
    check_rect(bw, bh, words.len(), 0, 0, rw, rh, pitch, dst.len())?;
    for row in 0..rh as usize {
        for col in 0..rw as usize {
            dst[row * pitch as usize + col] = words[row * bw as usize + col] as u8;
        }
    }
    Ok(())
}

/// `stencil_resolve` from a separate S8 scratch: the bytes of the top-left
/// `w` x `h` (clipped to the buffer) into `dst`, `pitch` elements a row
/// (docs/notes/v3d-fbo-blit.md 3.4).
pub fn read_stencil_s8(
    bytes: &[u8],
    bw: u32,
    bh: u32,
    dst: &mut [u8],
    w: u32,
    h: u32,
    pitch: u32,
) -> Result<(), PlaneError> {
    let (rw, rh) = read_clip(bw, bh, w, h);
    if w > pitch {
        return Err(PlaneError::OutOfBounds);
    }
    check_rect(bw, bh, bytes.len(), 0, 0, rw, rh, pitch, dst.len())?;
    for row in 0..rh as usize {
        dst[row * pitch as usize..][..rw as usize].copy_from_slice(&bytes[row * bw as usize..][..rw as usize]);
    }
    Ok(())
}
