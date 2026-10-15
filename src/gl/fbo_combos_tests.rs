// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The 81 dEQP-GLES2 `functional.fbo.completeness.attachment_combinations`
//! cases on the GL core (docs/investigations/es2-v3d-fbo-combos.md, section 2
//! and follow-up 2).
//!
//! The 27 Pi 4 cases ending in `_tex` were NotSupported only because the V3D
//! build did not claim GL_OES_packed_depth_stencil and GL_OES_depth_texture:
//! dEQP's `getDefaultFormat` found no stencil-attachable texture format. The
//! core completes those framebuffers; this pins it, so that a change to the
//! extension list or to the completeness rules cannot silently reopen them.
//!
//! The generator is `glsFboCompletenessTests.cpp:923-933`: a four-deep loop
//! over colour 0, colour N, depth and stencil, each `none`, `rbo` or `tex`,
//! the case name being the four joined by `_`. Each framebuffer is built as
//! `AttachmentTest::build` does (`:812-832`): colour 0, colour N on the points
//! 1..GL_MAX_COLOR_ATTACHMENTS, then `makeDepthAndStencil`, which on ES 2.0
//! (no core format is both depth- and stencil-renderable) attaches the stencil
//! image, then the depth image, always two separate images. Every image is
//! 64x64. Default formats (`getDefaultFormat`): RGBA4 renderbuffer or an RGBA
//! texture for colour, DEPTH_COMPONENT16 / STENCIL_INDEX8 renderbuffers, and
//! the textures the extensions give: DEPTH_COMPONENT/UNSIGNED_SHORT
//! (GL_OES_depth_texture) for depth, DEPTH_STENCIL/UNSIGNED_INT_24_8 (both
//! GL_OES_packed_depth_stencil and GL_OES_depth_texture) for stencil.

use core::ffi::CStr;
use core::ptr;
use std::string::String;
use std::vec::Vec;

use super::consts::*;
use super::framebuffer::*;
use super::state::*;
use super::texture::*;
use super::types::*;
use crate::testutil::TestGl;

const SIZE: GLsizei = 64;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    None,
    Rbo,
    Tex,
}

impl Kind {
    const ALL: [Kind; 3] = [Kind::None, Kind::Rbo, Kind::Tex];
    fn name(self) -> &'static str {
        match self {
            Kind::None => "none",
            Kind::Rbo => "rbo",
            Kind::Tex => "tex",
        }
    }
}

/// Whether this build's GL_EXTENSIONS holds `token` (what dEQP's
/// `addExtFormats` looks at).
fn claims(token: &str) -> bool {
    // SAFETY: a current context (the TestGl guard); GL_EXTENSIONS is a
    // NUL-terminated static string.
    let s = unsafe { CStr::from_ptr(glGetString(GL_EXTENSIONS) as *const _) };
    s.to_str().unwrap().split(' ').any(|t| t == token)
}

fn gen_rbo(format: GLenum) -> GLuint {
    let mut rb: GLuint = 0;
    // SAFETY: room for one name; plain GL calls.
    unsafe {
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorage(GL_RENDERBUFFER, format, SIZE, SIZE);
    }
    rb
}

fn gen_tex(internal: GLenum, type_: GLenum) -> GLuint {
    let mut t: GLuint = 0;
    // SAFETY: room for one name; null pixels only allocate.
    unsafe {
        glGenTextures(1, &mut t);
        glBindTexture(GL_TEXTURE_2D, t);
        glTexImage2D(GL_TEXTURE_2D, 0, internal as GLint, SIZE, SIZE, 0, internal, type_, ptr::null());
    }
    t
}

/// Attach a new image of `kind` to `point`: a renderbuffer of `rbo_format`
/// or a texture of the unsized format and type in `tex`.
fn attach(point: GLenum, kind: Kind, rbo_format: GLenum, tex: (GLenum, GLenum)) {
    // SAFETY: plain GL calls.
    unsafe {
        match kind {
            Kind::None => {}
            Kind::Rbo => glFramebufferRenderbuffer(GL_FRAMEBUFFER, point, GL_RENDERBUFFER, gen_rbo(rbo_format)),
            Kind::Tex => {
                let t = gen_tex(tex.0, tex.1);
                glFramebufferTexture2D(GL_FRAMEBUFFER, point, GL_TEXTURE_2D, t, 0)
            }
        }
    }
}

fn status_name(s: GLenum) -> &'static str {
    match s {
        GL_FRAMEBUFFER_COMPLETE => "COMPLETE",
        GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT => "INCOMPLETE_ATTACHMENT",
        GL_FRAMEBUFFER_INCOMPLETE_MISSING_ATTACHMENT => "INCOMPLETE_MISSING_ATTACHMENT",
        GL_FRAMEBUFFER_INCOMPLETE_DIMENSIONS => "INCOMPLETE_DIMENSIONS",
        GL_FRAMEBUFFER_UNSUPPORTED => "UNSUPPORTED",
        _ => "other",
    }
}

/// dEQP-GLES2.functional.fbo.completeness.attachment_combinations.*, on the
/// shared core, in whichever extension configuration this build claims.
#[test]
fn attachment_combinations_follow_deqp_gles2() {
    let mut names: Vec<String> = Vec::new();
    let mut not_supported: Vec<String> = Vec::new();
    let mut failures: Vec<String> = Vec::new();
    let mut pair_claimed = false;
    for c0 in Kind::ALL {
        for cn in Kind::ALL {
            for depth in Kind::ALL {
                for stencil in Kind::ALL {
                    let name = std::format!("{}_{}_{}_{}", c0.name(), cn.name(), depth.name(), stencil.name());
                    names.push(name.clone());
                    // A fresh context per case: the doc says it is required.
                    let gl = TestGl::new(64, 64);
                    /* getDefaultFormat throws NotSupported when the extension
                     * entry that gives the texture format is not claimed. */
                    let depth_tex_ok = claims("GL_OES_depth_texture");
                    let stencil_tex_ok = depth_tex_ok && claims("GL_OES_packed_depth_stencil");
                    pair_claimed = stencil_tex_ok;
                    if (depth == Kind::Tex && !depth_tex_ok) || (stencil == Kind::Tex && !stencil_tex_ok) {
                        not_supported.push(name);
                        continue;
                    }
                    let mut fb: GLuint = 0;
                    // SAFETY: room for one name; plain GL calls.
                    unsafe {
                        glGenFramebuffers(1, &mut fb);
                        glBindFramebuffer(GL_FRAMEBUFFER, fb);
                    }
                    let colour_tex = (GL_RGBA, GL_UNSIGNED_BYTE);
                    attach(GL_COLOR_ATTACHMENT0, c0, GL_RGBA4, colour_tex);
                    let mut max_colour: GLint = 0;
                    // SAFETY: a writable GLint.
                    unsafe { glGetIntegerv(GL_MAX_COLOR_ATTACHMENTS, &mut max_colour) };
                    if cn != Kind::None {
                        assert!(max_colour > 1, "{name}: GL_NV_fbo_color_attachments claims more than one point");
                        for i in 1..max_colour as GLenum {
                            attach(GL_COLOR_ATTACHMENT0 + i, cn, GL_RGBA4, colour_tex);
                        }
                    }
                    /* makeDepthAndStencil: separate images, stencil first */
                    attach(GL_STENCIL_ATTACHMENT, stencil, GL_STENCIL_INDEX8, (GL_DEPTH_STENCIL, GL_UNSIGNED_INT_24_8));
                    attach(GL_DEPTH_ATTACHMENT, depth, GL_DEPTH_COMPONENT16, (GL_DEPTH_COMPONENT, GL_UNSIGNED_SHORT));
                    let err = gl.error();
                    // SAFETY: plain GL call.
                    let st = unsafe { glCheckFramebufferStatus(GL_FRAMEBUFFER) };
                    let all_none = [c0, cn, depth, stencil].iter().all(|k| *k == Kind::None);
                    /* The reference: no attachment is MISSING_ATTACHMENT. The
                     * rest may be COMPLETE, and UNSUPPORTED is always allowed
                     * on ES 2.0 (es2fFboCompletenessTests.cpp:231); texture
                     * formats are not required renderable, so dEQP also
                     * tolerates INCOMPLETE_ATTACHMENT for them with a quality
                     * warning, which is a regression here, not a pass. */
                    let ok = if all_none {
                        st == GL_FRAMEBUFFER_INCOMPLETE_MISSING_ATTACHMENT
                    } else {
                        st == GL_FRAMEBUFFER_COMPLETE || st == GL_FRAMEBUFFER_UNSUPPORTED
                    };
                    if err != GL_NO_ERROR || !ok {
                        failures.push(std::format!(
                            "dEQP-GLES2.functional.fbo.completeness.attachment_combinations.{name}: \
                             status {} (0x{st:x}), GL error 0x{err:x}",
                            status_name(st)
                        ));
                    }
                }
            }
        }
    }
    assert_eq!(names.len(), 81, "3x3x3x3 cases");
    let mut sorted = names.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(sorted.len(), 81, "the case names are distinct");
    std::println!(
        "attachment_combinations: {} of 81 ran, {} NotSupported (GL_OES_depth_texture/GL_OES_packed_depth_stencil not \
         claimed), {} failed",
        81 - not_supported.len(),
        not_supported.len(),
        failures.len()
    );
    /* The 27 Pi 4 cases are the ones whose stencil part is a texture: with
     * the pair claimed none of them is NotSupported. */
    if pair_claimed {
        assert!(not_supported.is_empty(), "the extension pair is claimed, yet NotSupported: {not_supported:?}");
    }
    assert!(failures.is_empty(), "{} of 81 cases fail:\n{}", failures.len(), failures.join("\n"));
}
