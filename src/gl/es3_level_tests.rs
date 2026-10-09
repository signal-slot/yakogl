// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Mip levels of non-power-of-two textures and depth uploads from a pixel
//! unpack buffer, through the GL entry points on the mock back end (dEQP-GLES3
//! texture.specification texstorage2d / texstorage3d / texsubimage3d_depth /
//! teximage2d_depth_pbo / teximage3d_depth_pbo). They are in the crate and not
//! in tests/: an integration test cannot make a context (`TestGl`).
//!
//! ES 3.0 3.8.14: level i of a texture is max(1, floor(size / 2^i)) on every
//! side; a 2D array's layer count is the same at every level.

use core::ffi::c_void;
use std::vec;
use std::vec::Vec;

use super::buffer::*;
use super::consts::*;
use super::state::glPixelStorei;
use super::texture::*;
use super::types::*;
use crate::testutil::TestGl;

fn bind_new(target: GLenum) -> GLuint {
    let mut name: GLuint = 0;
    // SAFETY: room for one name; plain GL calls.
    unsafe {
        glGenTextures(1, &mut name);
        glBindTexture(target, name);
    }
    name
}

/// The levels of a chain whose largest side is `size`: 57 -> 6.
fn levels_of(size: u32) -> u32 {
    size.ilog2() + 1
}

#[test]
fn every_level_of_a_npot_2d_array_takes_an_image() {
    let gl = TestGl::new(4, 4);
    bind_new(GL_TEXTURE_2D_ARRAY);
    let (w, h, d) = (57u32, 44u32, 5u32);
    assert_eq!(levels_of(w.max(h)), 6);
    for l in 0..levels_of(w.max(h)) {
        let (lw, lh) = ((w >> l).max(1), (h >> l).max(1));
        let px = vec![l as u8; (lw * lh * d * 4) as usize];
        // SAFETY: `px` holds lw x lh x d RGBA8 texels.
        unsafe {
            glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
            glTexImage3D(
                GL_TEXTURE_2D_ARRAY,
                l as GLint,
                GL_RGBA8 as GLint,
                lw as GLsizei,
                lh as GLsizei,
                d as GLsizei,
                0,
                GL_RGBA,
                GL_UNSIGNED_BYTE,
                px.as_ptr() as *const c_void,
            )
        };
        assert_eq!(gl.error(), GL_NO_ERROR, "teximage3d level {l} {lw}x{lh}x{d}");
        // SAFETY: as above.
        unsafe {
            glTexSubImage3D(
                GL_TEXTURE_2D_ARRAY,
                l as GLint,
                0,
                0,
                0,
                lw as GLsizei,
                lh as GLsizei,
                d as GLsizei,
                GL_RGBA,
                GL_UNSIGNED_BYTE,
                px.as_ptr() as *const c_void,
            )
        };
        assert_eq!(gl.error(), GL_NO_ERROR, "texsubimage3d level {l}");
    }
    /* the layer count of a level is the base level's */
    // SAFETY: plain GL call; nothing is read.
    unsafe {
        glTexImage3D(
            GL_TEXTURE_2D_ARRAY,
            1,
            GL_RGBA8 as GLint,
            28,
            22,
            2,
            0,
            GL_RGBA,
            GL_UNSIGNED_BYTE,
            core::ptr::null(),
        )
    };
    assert_eq!(gl.error(), GL_INVALID_VALUE, "level 1 with 2 layers");
}

#[test]
fn tex_storage_gives_every_level_of_a_npot_texture() {
    let gl = TestGl::new(4, 4);
    for (target, w, h, d) in [
        (GL_TEXTURE_2D, 117u32, 97u32, 1u32),
        (GL_TEXTURE_CUBE_MAP, 57, 57, 1),
        (GL_TEXTURE_2D_ARRAY, 57, 44, 5),
        (GL_TEXTURE_3D, 57, 44, 5),
    ] {
        bind_new(target);
        let n = levels_of(w.max(h)) as GLsizei;
        // SAFETY: plain GL calls.
        unsafe {
            if d == 1 {
                glTexStorage2D(target, n, GL_RGBA8, w as GLsizei, h as GLsizei);
            } else {
                glTexStorage3D(target, n, GL_RGBA8, w as GLsizei, h as GLsizei, d as GLsizei);
            }
        }
        assert_eq!(gl.error(), GL_NO_ERROR, "storage of target 0x{target:x}");
        for l in 0..n as u32 {
            let (lw, lh) = ((w >> l).max(1), (h >> l).max(1));
            let ld = if target == GL_TEXTURE_3D { (d >> l).max(1) } else { d };
            let px = vec![0x55u8; (lw * lh * ld * 4) as usize];
            let p = px.as_ptr() as *const c_void;
            // SAFETY: `px` holds the level's texels.
            unsafe {
                glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
                match target {
                    GL_TEXTURE_2D | GL_TEXTURE_CUBE_MAP => {
                        let face = if target == GL_TEXTURE_2D { GL_TEXTURE_2D } else { GL_TEXTURE_CUBE_MAP_NEGATIVE_Y };
                        glTexSubImage2D(
                            face,
                            l as GLint,
                            0,
                            0,
                            lw as GLsizei,
                            lh as GLsizei,
                            GL_RGBA,
                            GL_UNSIGNED_BYTE,
                            p,
                        )
                    }
                    _ => glTexSubImage3D(
                        target,
                        l as GLint,
                        0,
                        0,
                        0,
                        lw as GLsizei,
                        lh as GLsizei,
                        ld as GLsizei,
                        GL_RGBA,
                        GL_UNSIGNED_BYTE,
                        p,
                    ),
                }
            };
            assert_eq!(gl.error(), GL_NO_ERROR, "target 0x{target:x} sub-image level {l} {lw}x{lh}x{ld}");
        }
    }
}

/// ES 2.0 3.7.7 with OES_texture_npot: the levels of a NPOT 2D texture, which
/// every build keeps.
#[test]
fn npot_2d_levels_keep_working() {
    let gl = TestGl::new(4, 4);
    bind_new(GL_TEXTURE_2D);
    for (l, (lw, lh)) in [(0, (57usize, 44usize)), (1, (28, 22)), (2, (14, 11)), (3, (7, 5)), (4, (3, 2)), (5, (1, 1))]
    {
        let px = vec![l as u8; lw * lh * 4];
        // SAFETY: `px` holds the level's texels.
        unsafe {
            glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
            glTexImage2D(
                GL_TEXTURE_2D,
                l,
                GL_RGBA as GLint,
                lw as GLsizei,
                lh as GLsizei,
                0,
                GL_RGBA,
                GL_UNSIGNED_BYTE,
                px.as_ptr() as *const c_void,
            )
        };
        assert_eq!(gl.error(), GL_NO_ERROR, "level {l}");
    }
}

/// teximage2d_depth_pbo / teximage3d_depth_pbo: the depth image is specified
/// with a null pointer from a bound unpack buffer holding the data, then the
/// buffer is deleted (es3fTextureSpecificationTests.cpp).
#[test]
fn depth_uploads_from_a_unpack_buffer() {
    if crate::REPORT_ES2 {
        return; /* ES 3.0's formats */
    }
    let gl = TestGl::new(4, 4);
    for (ifmt, type_, bytes, fmt) in [
        (GL_DEPTH_COMPONENT32F, GL_FLOAT, 4usize, GL_DEPTH_COMPONENT),
        (GL_DEPTH_COMPONENT24, GL_UNSIGNED_INT, 4, GL_DEPTH_COMPONENT),
        (GL_DEPTH_COMPONENT16, GL_UNSIGNED_SHORT, 2, GL_DEPTH_COMPONENT),
        (GL_DEPTH32F_STENCIL8, GL_FLOAT_32_UNSIGNED_INT_24_8_REV, 8, GL_DEPTH_STENCIL),
        (GL_DEPTH24_STENCIL8, GL_UNSIGNED_INT_24_8, 4, GL_DEPTH_STENCIL),
    ] {
        let (w, h) = (64usize, 128usize);
        for array in [false, true] {
            /* exactly the texels: the last byte of the buffer is the image's last */
            let data: Vec<u8> = (0..w * h * bytes * if array { 3 } else { 1 }).map(|i| (i * 7 + 3) as u8).collect();
            let depth = if array { 3 } else { 1 };
            let mut tex = [0 as GLuint; 2];
            let mut buf: GLuint = 0;
            let target = if array { GL_TEXTURE_2D_ARRAY } else { GL_TEXTURE_2D };
            let image = |p: *const c_void| {
                // SAFETY: `p` is an offset into the bound unpack buffer or `data`.
                unsafe {
                    if array {
                        glTexImage3D(target, 0, ifmt as GLint, w as GLsizei, h as GLsizei, depth, 0, fmt, type_, p);
                    } else {
                        glTexImage2D(target, 0, ifmt as GLint, w as GLsizei, h as GLsizei, 0, fmt, type_, p);
                    }
                }
            };
            // SAFETY: plain GL calls; `data` is valid for its length.
            unsafe {
                glPixelStorei(GL_UNPACK_ROW_LENGTH, w as GLint);
                glPixelStorei(GL_UNPACK_SKIP_ROWS, 0);
                glPixelStorei(GL_UNPACK_SKIP_PIXELS, 0);
                glPixelStorei(GL_UNPACK_ALIGNMENT, 4);
            }
            /* from client memory (which passes on the boards) ... */
            tex[0] = bind_new(target);
            image(data.as_ptr() as *const c_void);
            assert_eq!(gl.error(), GL_NO_ERROR, "client memory, format 0x{ifmt:x} array {array}");
            /* ... and from a buffer, as dEQP does: a null offset, then the buffer is deleted */
            // SAFETY: plain GL calls; `data` is valid for its length.
            unsafe {
                glGenBuffers(1, &mut buf);
                glBindBuffer(GL_PIXEL_UNPACK_BUFFER, buf);
                glBufferData(
                    GL_PIXEL_UNPACK_BUFFER,
                    data.len() as GLsizeiptr,
                    data.as_ptr() as *const c_void,
                    GL_STATIC_DRAW,
                );
            }
            tex[1] = bind_new(target);
            image(core::ptr::null());
            // SAFETY: plain GL call.
            unsafe { glDeleteBuffers(1, &buf) };
            assert_eq!(gl.error(), GL_NO_ERROR, "buffer, format 0x{ifmt:x} array {array}");
            assert_eq!(gl.ctx().bound_pixel_unpack_buffer, 0, "deleting the bound buffer unbinds it");
            let bytes_of = |n: GLuint| {
                let m = gl.ctx().objs().textures[n as usize].mem;
                gl.gpu_bytes(m.gpu, m.size)
            };
            assert_eq!(bytes_of(tex[0]), bytes_of(tex[1]), "the same texels, format 0x{ifmt:x} array {array}");
            // SAFETY: plain GL call.
            unsafe { glPixelStorei(GL_UNPACK_ROW_LENGTH, 0) };
        }
    }
}
