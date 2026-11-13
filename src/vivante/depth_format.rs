// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The depth surface format and size of a GL depth attachment, as a pure
//! function (docs/notes/gc-slow-terrain-stream-diff.md, rank 6).
//!
//! Terrain's `DEPTH_COMPONENT16` renderbuffer is stored and tested as D24S8
//! today (stride 0xd00 at 800 wide, Mesa's 0x680): twice the depth bytes in
//! the terrain draw, its clear and the overlay draw. This file says what the
//! surface should be; nothing calls it yet. Core only, no crate imports.
//!
//! Mesa 26.2.1 etnaviv, which this follows (line numbers are those of the
//! tree the notes cite; the etnaviv driver is not vendored, so they are not
//! checkable from this repository):
//! - `translate_depth_format` (etnaviv_translate.h): `Z16_UNORM` is
//!   `ETNA_DEPTH_FORMAT D16`; `Z24X8`, `Z24_UNORM_S8_UINT` and `S8_UINT`
//!   (no D16 stencil form exists) are `D24S8`.
//! - `etna_resource` sizing (etnaviv_resource.c, `etna_resource_alloc`):
//!   stride = align(width, layout padding) * bytes a pixel, size =
//!   stride * align(height, layout padding), the padding of the layout
//!   (4x4 tiled 4x4, super-tiled 64x64 here).
//! - `translate_clear_depth_stencil` (etnaviv_translate.h): D16 clears with
//!   `depth * 0xffff` in both halves of the word; D24S8 with
//!   `(depth * 0xffffff) << 8 | stencil`.
//!
//! Consumers that must change before the format can switch (all still assume
//! 4 bytes a pixel and D24S8):
//! - `ds_cache.rs`: `select` sizes every surface `align64(w) * 4` and keeps
//!   the plane bookkeeping (`stride`, `rows`, retention) in those units;
//!   `resolve_depth` and `depth-sample-gc` read 4-byte words
//!   (`resolve_read_bytes` below says 2 for D16);
//! - the depth sample path that turns a surface into a depth texture;
//! - `backend.rs`: `GC_PE_DEPTH_FORMAT_D24S8` in `PE_DEPTH_CONFIG`, the
//!   surface's stride/size registers, and the clear's value;
//! - polygon offset: `GC_SE_DEPTH_BIAS` is scaled by 1 / 16777215 (24 bits);
//!   a 16-bit buffer needs 1 / 65535.

/// What the pixel engine is told the depth buffer holds. The caller maps it
/// to `GC_PE_DEPTH_FORMAT_*`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PeDepthFormat {
    D16,
    D24S8,
}

/// A GL depth/stencil attachment format.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GlDepthAttachment {
    /// No depth or stencil attachment.
    None,
    DepthComponent16,
    DepthComponent24,
    Depth24Stencil8,
    /// `STENCIL_INDEX8` alone.
    StencilIndex8,
}

/// What a depth attachment is stored as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DepthSurface {
    pub format: PeDepthFormat,
    /// Bytes a pixel in memory.
    pub bytes_per_pixel: u32,
    /// Bytes a depth resolve or depth sample must read a pixel: 2 for D16,
    /// 4 for D24S8.
    pub resolve_read_bytes: u32,
}

const fn align_up(v: u32, to: u32) -> u32 {
    v.div_ceil(to) * to
}

/// The surface for an attachment; `None` for no attachment (nothing to
/// allocate).
pub const fn depth_surface(a: GlDepthAttachment) -> Option<DepthSurface> {
    match a {
        GlDepthAttachment::None => None,
        GlDepthAttachment::DepthComponent16 => {
            Some(DepthSurface { format: PeDepthFormat::D16, bytes_per_pixel: 2, resolve_read_bytes: 2 })
        }
        GlDepthAttachment::DepthComponent24 | GlDepthAttachment::Depth24Stencil8 | GlDepthAttachment::StencilIndex8 => {
            Some(DepthSurface { format: PeDepthFormat::D24S8, bytes_per_pixel: 4, resolve_read_bytes: 4 })
        }
    }
}

impl DepthSurface {
    /// Bytes a row with each row padded to `pad` pixels (64 for the
    /// super-tiled layout).
    pub const fn stride(&self, width: u32, pad: u32) -> u32 {
        align_up(width, pad) * self.bytes_per_pixel
    }

    /// Bytes the surface takes: stride times the height padded to `pad`.
    pub const fn size(&self, width: u32, height: u32, pad: u32) -> u32 {
        self.stride(width, pad) * align_up(height, pad)
    }

    /// The 32-bit word the clear writes to every pixel, for a depth in
    /// [0, 1] (clamped) and a stencil value (D24S8 only; ignored by D16).
    pub fn clear_value(&self, depth: f32, stencil: u8) -> u32 {
        let d = if depth.is_nan() { 0.0 } else { depth.clamp(0.0, 1.0) };
        match self.format {
            PeDepthFormat::D16 => {
                let v = (d as f64 * 65535.0 + 0.5) as u32;
                v | v << 16
            }
            PeDepthFormat::D24S8 => {
                let v = (d as f64 * 16_777_215.0 + 0.5) as u32;
                v << 8 | stencil as u32
            }
        }
    }
}
