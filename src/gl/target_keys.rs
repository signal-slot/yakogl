// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The per-target keys of a context's draw buffers, for the mixed integer /
//! float colour-target key (docs/notes/pvr-int-mixed-targets.md, sections
//! 2.1, 2.4 and 4). The list is the dense one `draw::resolve_targets` builds:
//! it starts at the frame's own attachment ([`framebuffer::frame_attachment`])
//! and slot `i` is attachment `frame_attachment + i`.

use super::context::GlContext;
use super::framebuffer;
use crate::backend::GLES_MAX_TARGETS;
use crate::target_key::{gles_surf_target_key, shc_blend_key_mixed, SHC_MIXED_MAX_TARGETS};

/// The per-target keys of the context's draw buffers, 0.. of the dense list,
/// and how many. `None` when the list is not dense (a `GL_NONE` or empty
/// slot before a later present one), when a target has no per-target key
/// (sRGB), or when there are more than [`SHC_MIXED_MAX_TARGETS`]. A single
/// target gives a list of one.
pub fn draw_target_keys(c: &GlContext) -> Option<([u32; SHC_MIXED_MAX_TARGETS], usize)> {
    let lead = framebuffer::frame_attachment(c);
    let mut keys = [0u32; SHC_MIXED_MAX_TARGETS];
    let mut n = 0usize;
    let mut gap = false;
    for i in 0..GLES_MAX_TARGETS.saturating_sub(lead as usize) {
        let att = lead + i as u8;
        let fmt = framebuffer::fb_target_format_n(c, att);
        let present = if i == 0 {
            /* slot 0 keeps its attachment with no memory when draw buffer 0
             * names GL_NONE and nothing else writes (resolve_targets) */
            !(lead == 0 && framebuffer::draw_buffer0_none(c) && !framebuffer::colour_writes_off(c))
        } else {
            framebuffer::fb_target_n(c, att).is_some()
        };
        if !present {
            gap = true;
            continue;
        }
        /* a present slot after an absent one: not dense */
        if gap {
            return None;
        }
        if n == SHC_MIXED_MAX_TARGETS {
            return None;
        }
        keys[n] = gles_surf_target_key(fmt)?;
        n += 1;
    }
    if n == 0 {
        return None;
    }
    Some((keys, n))
}

/// The mixed key of the context's draw buffers
/// ([`shc_blend_key_mixed`] of [`draw_target_keys`]); `None` for one target,
/// no integer target, one integer layout everywhere, or a list that has no
/// keys.
pub fn draw_mixed_key(c: &GlContext) -> Option<u32> {
    let (keys, n) = draw_target_keys(c)?;
    shc_blend_key_mixed(&keys[..n])
}

#[cfg(all(test, not(feature = "report-es2")))]
mod tests {
    use super::*;
    use crate::gl::consts::*;
    use crate::gl::framebuffer::{
        glBindFramebuffer, glBindRenderbuffer, glDrawBuffers, glFramebufferRenderbuffer, glGenFramebuffers,
        glGenRenderbuffers, glRenderbufferStorage,
    };
    use crate::gl::types::*;
    use crate::testutil::TestGl;

    /// A framebuffer with a `formats[i]` renderbuffer at colour attachment
    /// `i` (`GL_NONE` leaves it empty), bound to GL_FRAMEBUFFER, and every
    /// attachment named by `draw_buffers` as its draw buffers.
    fn fbo(formats: &[GLenum], draw_buffers: &[GLenum]) {
        let mut fb = 0;
        // SAFETY: plain GL calls with room for one name each.
        unsafe {
            glGenFramebuffers(1, &mut fb);
            glBindFramebuffer(GL_FRAMEBUFFER, fb);
            for (i, f) in formats.iter().enumerate() {
                if *f == GL_NONE {
                    continue;
                }
                let mut rb = 0;
                glGenRenderbuffers(1, &mut rb);
                glBindRenderbuffer(GL_RENDERBUFFER, rb);
                glRenderbufferStorage(GL_RENDERBUFFER, *f, 4, 4);
                glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0 + i as u32, GL_RENDERBUFFER, rb);
            }
            glDrawBuffers(draw_buffers.len() as i32, draw_buffers.as_ptr());
        }
    }

    const A0: GLenum = GL_COLOR_ATTACHMENT0;
    const A1: GLenum = GL_COLOR_ATTACHMENT0 + 1;
    const A2: GLenum = GL_COLOR_ATTACHMENT0 + 2;

    #[test]
    fn rgba8_beside_r32ui_is_the_mixed_key() {
        let gl = TestGl::new(8, 8);
        fbo(&[GL_RGBA8, GL_R32UI], &[A0, A1]);
        assert_eq!(draw_mixed_key(gl.ctx()), Some(0x0204_C000));
        let (keys, n) = draw_target_keys(gl.ctx()).unwrap();
        assert_eq!(n, 2);
        assert_eq!(keys[0], gles_surf_target_key(crate::backend::GLES_SURF_RGBA8888).unwrap());
    }

    #[test]
    fn two_integer_layouts_are_the_mixed_key() {
        let gl = TestGl::new(8, 8);
        fbo(&[GL_R32UI, GL_R32I], &[A0, A1]);
        assert_eq!(draw_mixed_key(gl.ctx()), Some(0x020C_A400));
    }

    #[test]
    fn three_targets_list_and_key() {
        let gl = TestGl::new(8, 8);
        fbo(&[GL_RGBA8, GL_R32UI, GL_R32I], &[A0, A1, A2]);
        assert_eq!(draw_target_keys(gl.ctx()).map(|(_, n)| n), Some(3));
        assert!(draw_mixed_key(gl.ctx()).is_some());
    }

    #[test]
    fn not_a_mix_has_no_mixed_key() {
        let gl = TestGl::new(8, 8);
        fbo(&[GL_R32UI, GL_R32UI], &[A0, A1]);
        assert_eq!(draw_mixed_key(gl.ctx()), None);
        assert_eq!(draw_target_keys(gl.ctx()).map(|(_, n)| n), Some(2));
    }

    #[test]
    fn one_target_has_no_mixed_key() {
        let gl = TestGl::new(8, 8);
        fbo(&[GL_R32UI], &[A0]);
        assert_eq!(draw_mixed_key(gl.ctx()), None);
    }

    #[test]
    fn two_rgba8_have_no_mixed_key() {
        let gl = TestGl::new(8, 8);
        fbo(&[GL_RGBA8, GL_RGBA8], &[A0, A1]);
        assert_eq!(draw_mixed_key(gl.ctx()), None);
    }

    #[test]
    fn a_none_gap_has_no_list() {
        let gl = TestGl::new(8, 8);
        fbo(&[GL_RGBA8, GL_NONE, GL_R32UI], &[A0, GL_NONE, A2]);
        assert_eq!(draw_target_keys(gl.ctx()), None);
        assert_eq!(draw_mixed_key(gl.ctx()), None);
    }

    #[test]
    fn an_srgb_target_has_no_list() {
        let gl = TestGl::new(8, 8);
        fbo(&[GL_SRGB8_ALPHA8, GL_R32UI], &[A0, A1]);
        assert_eq!(draw_target_keys(gl.ctx()), None);
    }
}
