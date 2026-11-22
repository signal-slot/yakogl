// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Per-draw-buffer blend and colour-mask state for GL_EXT/OES_draw_buffers_indexed
//! (Draft 1 parts (1)-(3) of docs/notes/v3d-draw-buffers-indexed.md).
//!
//! The state model only: it is not wired into `GlesDraw`, no `gl*iEXT` entry
//! point calls it and the extension is not listed. Errors are returned as GL
//! error codes (`Err(GL_INVALID_VALUE)`), which the caller records.
//!
//! The array holds [`GLES_MAX_TARGETS`] buffers; an index is valid below
//! [`backend::draw_targets`] (GL_MAX_DRAW_BUFFERS).

use crate::backend::{self, GLES_MAX_TARGETS};
use crate::gl::consts::*;

/// Blend and colour-mask state of one draw buffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DrawBufferState {
    pub blend_enable: bool,
    pub eq_rgb: u32,
    pub eq_alpha: u32,
    pub src_rgb: u32,
    pub dst_rgb: u32,
    pub src_alpha: u32,
    pub dst_alpha: u32,
    /// R, G, B, A.
    pub color_mask: [bool; 4],
}

impl DrawBufferState {
    /// The initial state: blending off, FUNC_ADD, ONE/ZERO, all channels written.
    pub const INITIAL: DrawBufferState = DrawBufferState {
        blend_enable: false,
        eq_rgb: GL_FUNC_ADD,
        eq_alpha: GL_FUNC_ADD,
        src_rgb: GL_ONE,
        dst_rgb: GL_ZERO,
        src_alpha: GL_ONE,
        dst_alpha: GL_ZERO,
        color_mask: [true; 4],
    };
}

/// The per-draw-buffer array.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DrawBuffersIndexed {
    bufs: [DrawBufferState; GLES_MAX_TARGETS],
}

impl Default for DrawBuffersIndexed {
    fn default() -> Self {
        Self::new()
    }
}

impl DrawBuffersIndexed {
    pub const fn new() -> Self {
        DrawBuffersIndexed { bufs: [DrawBufferState::INITIAL; GLES_MAX_TARGETS] }
    }

    fn check(index: u32) -> Result<usize, u32> {
        let i = index as usize;
        if i >= backend::draw_targets() {
            Err(GL_INVALID_VALUE)
        } else {
            Ok(i)
        }
    }

    /// The state of buffer `index`, or GL_INVALID_VALUE.
    pub fn get(&self, index: u32) -> Result<DrawBufferState, u32> {
        Self::check(index).map(|i| self.bufs[i])
    }

    /* ---- non-indexed: every buffer ---- */

    pub fn set_blend_enable(&mut self, on: bool) {
        self.bufs.iter_mut().for_each(|b| b.blend_enable = on);
    }
    pub fn set_blend_equation(&mut self, mode: u32) {
        self.set_blend_equation_separate(mode, mode);
    }
    pub fn set_blend_equation_separate(&mut self, rgb: u32, alpha: u32) {
        self.bufs.iter_mut().for_each(|b| {
            b.eq_rgb = rgb;
            b.eq_alpha = alpha;
        });
    }
    pub fn set_blend_func(&mut self, src: u32, dst: u32) {
        self.set_blend_func_separate(src, dst, src, dst);
    }
    pub fn set_blend_func_separate(&mut self, src_rgb: u32, dst_rgb: u32, src_alpha: u32, dst_alpha: u32) {
        self.bufs.iter_mut().for_each(|b| {
            b.src_rgb = src_rgb;
            b.dst_rgb = dst_rgb;
            b.src_alpha = src_alpha;
            b.dst_alpha = dst_alpha;
        });
    }
    pub fn set_color_mask(&mut self, mask: [bool; 4]) {
        self.bufs.iter_mut().for_each(|b| b.color_mask = mask);
    }

    /* ---- indexed (the *iEXT calls) ---- */

    /// Enablei: `cap` other than GL_BLEND is GL_INVALID_ENUM.
    pub fn enablei(&mut self, cap: u32, index: u32) -> Result<(), u32> {
        self.set_capi(cap, index, true)
    }
    /// Disablei.
    pub fn disablei(&mut self, cap: u32, index: u32) -> Result<(), u32> {
        self.set_capi(cap, index, false)
    }
    fn set_capi(&mut self, cap: u32, index: u32, on: bool) -> Result<(), u32> {
        if cap != GL_BLEND {
            return Err(GL_INVALID_ENUM);
        }
        let i = Self::check(index)?;
        self.bufs[i].blend_enable = on;
        Ok(())
    }
    /// IsEnabledi.
    pub fn is_enabledi(&self, cap: u32, index: u32) -> Result<bool, u32> {
        if cap != GL_BLEND {
            return Err(GL_INVALID_ENUM);
        }
        Ok(self.get(index)?.blend_enable)
    }
    pub fn blend_equationi(&mut self, index: u32, mode: u32) -> Result<(), u32> {
        self.blend_equation_separatei(index, mode, mode)
    }
    pub fn blend_equation_separatei(&mut self, index: u32, rgb: u32, alpha: u32) -> Result<(), u32> {
        let i = Self::check(index)?;
        self.bufs[i].eq_rgb = rgb;
        self.bufs[i].eq_alpha = alpha;
        Ok(())
    }
    pub fn blend_funci(&mut self, index: u32, src: u32, dst: u32) -> Result<(), u32> {
        self.blend_func_separatei(index, src, dst, src, dst)
    }
    pub fn blend_func_separatei(
        &mut self,
        index: u32,
        src_rgb: u32,
        dst_rgb: u32,
        src_alpha: u32,
        dst_alpha: u32,
    ) -> Result<(), u32> {
        let i = Self::check(index)?;
        let b = &mut self.bufs[i];
        b.src_rgb = src_rgb;
        b.dst_rgb = dst_rgb;
        b.src_alpha = src_alpha;
        b.dst_alpha = dst_alpha;
        Ok(())
    }
    pub fn color_maski(&mut self, index: u32, mask: [bool; 4]) -> Result<(), u32> {
        let i = Self::check(index)?;
        self.bufs[i].color_mask = mask;
        Ok(())
    }

    /* ---- getters ---- */

    /// glGetIntegeri_v of GL_BLEND_EQUATION_RGB/ALPHA and
    /// GL_BLEND_SRC/DST_RGB/ALPHA (other pnames: GL_INVALID_ENUM).
    pub fn get_integeri(&self, pname: u32, index: u32) -> Result<u32, u32> {
        if !matches!(
            pname,
            GL_BLEND_EQUATION_RGB
                | GL_BLEND_EQUATION_ALPHA
                | GL_BLEND_SRC_RGB
                | GL_BLEND_DST_RGB
                | GL_BLEND_SRC_ALPHA
                | GL_BLEND_DST_ALPHA
        ) {
            return Err(GL_INVALID_ENUM);
        }
        let b = self.get(index)?;
        Ok(match pname {
            GL_BLEND_EQUATION_RGB => b.eq_rgb,
            GL_BLEND_EQUATION_ALPHA => b.eq_alpha,
            GL_BLEND_SRC_RGB => b.src_rgb,
            GL_BLEND_DST_RGB => b.dst_rgb,
            GL_BLEND_SRC_ALPHA => b.src_alpha,
            _ => b.dst_alpha,
        })
    }
    /// glGetBooleani_v of GL_COLOR_WRITEMASK.
    pub fn get_color_maski(&self, index: u32) -> Result<[bool; 4], u32> {
        Ok(self.get(index)?.color_mask)
    }
    /// The non-indexed integer getter: buffer 0's value.
    pub fn get_integer(&self, pname: u32) -> Result<u32, u32> {
        self.get_integeri(pname, 0)
    }
    /// The non-indexed GL_COLOR_WRITEMASK: buffer 0.
    pub fn color_mask0(&self) -> [bool; 4] {
        self.bufs[0].color_mask
    }
    /// The non-indexed GL_BLEND: buffer 0.
    pub fn blend_enabled0(&self) -> bool {
        self.bufs[0].blend_enable
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TestGl;

    #[test]
    fn index_three_is_valid_at_four_targets() {
        let gl = TestGl::new(8, 8);
        gl.backend_draws_targets(4);
        let mut s = DrawBuffersIndexed::new();
        assert_eq!(s.enablei(GL_BLEND, 3), Ok(()));
        assert_eq!(s.blend_equation_separatei(3, GL_FUNC_SUBTRACT, GL_MAX), Ok(()));
        assert_eq!(s.blend_func_separatei(3, GL_SRC_ALPHA, GL_ONE, GL_ZERO, GL_DST_ALPHA), Ok(()));
        assert_eq!(s.color_maski(3, [true, false, true, false]), Ok(()));
        assert_eq!(s.is_enabledi(GL_BLEND, 3), Ok(true));
        assert_eq!(s.get_integeri(GL_BLEND_EQUATION_RGB, 3), Ok(GL_FUNC_SUBTRACT));
        assert_eq!(s.get_integeri(GL_BLEND_EQUATION_ALPHA, 3), Ok(GL_MAX));
        assert_eq!(s.get_integeri(GL_BLEND_SRC_RGB, 3), Ok(GL_SRC_ALPHA));
        assert_eq!(s.get_integeri(GL_BLEND_DST_RGB, 3), Ok(GL_ONE));
        assert_eq!(s.get_integeri(GL_BLEND_SRC_ALPHA, 3), Ok(GL_ZERO));
        assert_eq!(s.get_integeri(GL_BLEND_DST_ALPHA, 3), Ok(GL_DST_ALPHA));
        assert_eq!(s.get_color_maski(3), Ok([true, false, true, false]));
        /* buffer 0 and the non-indexed getters are untouched */
        assert_eq!(s.is_enabledi(GL_BLEND, 0), Ok(false));
        assert!(!s.blend_enabled0());
        assert_eq!(s.get_integer(GL_BLEND_EQUATION_RGB), Ok(GL_FUNC_ADD));
        assert_eq!(s.color_mask0(), [true; 4]);
        assert_eq!(s.get_integeri(GL_BLEND_EQUATION_RGB, 4), Err(GL_INVALID_VALUE));
    }

    #[test]
    fn index_three_is_invalid_at_three_targets() {
        let gl = TestGl::new(8, 8);
        gl.backend_draws_targets(3);
        let mut s = DrawBuffersIndexed::new();
        assert_eq!(s.enablei(GL_BLEND, 3), Err(GL_INVALID_VALUE));
        assert_eq!(s.disablei(GL_BLEND, 3), Err(GL_INVALID_VALUE));
        assert_eq!(s.blend_equationi(3, GL_FUNC_ADD), Err(GL_INVALID_VALUE));
        assert_eq!(s.blend_funci(3, GL_ONE, GL_ONE), Err(GL_INVALID_VALUE));
        assert_eq!(s.color_maski(3, [false; 4]), Err(GL_INVALID_VALUE));
        assert_eq!(s.is_enabledi(GL_BLEND, 3), Err(GL_INVALID_VALUE));
        assert_eq!(s.get_color_maski(3), Err(GL_INVALID_VALUE));
        assert_eq!(s.enablei(GL_BLEND, 2), Ok(()));
        assert_eq!(s.is_enabledi(GL_BLEND, 2), Ok(true));
        assert_eq!(s.get_color_maski(2), Ok([true; 4]));
    }

    #[test]
    fn non_indexed_setters_set_every_buffer_and_getter_answers_zero() {
        let gl = TestGl::new(8, 8);
        gl.backend_draws_targets(4);
        let mut s = DrawBuffersIndexed::new();
        s.set_blend_enable(true);
        s.set_blend_equation_separate(GL_FUNC_REVERSE_SUBTRACT, GL_MIN);
        s.set_blend_func_separate(GL_ONE, GL_ONE, GL_ZERO, GL_ONE);
        s.set_color_mask([false, true, false, true]);
        for i in 0..4 {
            let b = s.get(i).unwrap();
            assert!(b.blend_enable);
            assert_eq!((b.eq_rgb, b.eq_alpha), (GL_FUNC_REVERSE_SUBTRACT, GL_MIN));
            assert_eq!((b.src_rgb, b.dst_rgb, b.src_alpha, b.dst_alpha), (GL_ONE, GL_ONE, GL_ZERO, GL_ONE));
            assert_eq!(b.color_mask, [false, true, false, true]);
        }
        s.color_maski(2, [true; 4]).unwrap();
        assert_eq!(s.color_mask0(), [false, true, false, true]);
        assert_eq!(s.get_integer(GL_BLEND_EQUATION_ALPHA), Ok(GL_MIN));
    }

    #[test]
    fn enablei_other_cap_is_invalid_enum() {
        let gl = TestGl::new(8, 8);
        gl.backend_draws_targets(4);
        let mut s = DrawBuffersIndexed::new();
        assert_eq!(s.enablei(GL_DEPTH_TEST, 0), Err(GL_INVALID_ENUM));
        assert_eq!(s.disablei(GL_CULL_FACE, 9), Err(GL_INVALID_ENUM));
        assert_eq!(s.is_enabledi(GL_DEPTH_TEST, 0), Err(GL_INVALID_ENUM));
        assert_eq!(s.get_integeri(GL_VIEWPORT, 0), Err(GL_INVALID_ENUM));
    }
}
