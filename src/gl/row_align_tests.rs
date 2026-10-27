// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The linear row alignment (`YakoglBackend::linear_row_align`): a backend
//! that reads and writes linear images only at a row pitch of whole
//! `linear_row_align` bytes (the Mali-G610: 64) gets every row the GL core
//! allocates so, and every reader and writer of that storage walks it at
//! that pitch. A backend with 0 keeps the pair-padded pitch: the other
//! GPUs' pitch and layout tests pin that, unchanged.

use std::vec;
use std::vec::Vec;

use super::consts::*;
use super::context::{linear_pitch, linear_surface_bytes, row_aligned, tex_pitch};
use super::framebuffer::{
    fb_target_pitched, glBindFramebuffer, glBindRenderbuffer, glFramebufferRenderbuffer, glGenFramebuffers,
    glGenRenderbuffers, glReadPixels, glRenderbufferStorage,
};
use super::texture::{glBindTexture, glCompressedTexImage2D, glGenTextures, glTexImage2D};
use crate::testutil::TestGl;

/// The rounding itself: the least pitch at or above the given one whose
/// bytes are a multiple of the alignment, for the pixel sizes the core
/// stores; 0 leaves it alone.
#[test]
fn a_row_is_rounded_up_to_whole_alignments() {
    for (pitch, unit, align, want) in [
        (8, 4, 64, 16),
        (16, 4, 64, 16),
        (600, 4, 64, 608),
        (8, 2, 64, 32),
        (64, 1, 64, 64),
        (2, 8, 64, 8),
        (2, 16, 64, 4),
        /* a 12-byte texel: 16 of them are the least whole 64 bytes */
        (2, 12, 64, 16),
        (600, 4, 0, 600),
        (7, 4, 0, 7),
    ] {
        assert_eq!(row_aligned(pitch, unit, align), want, "{pitch} x {unit} bytes on {align}");
    }
    let _gl = TestGl::new(4, 4);
    /* the PowerVR-shaped mock: the pair-padded pitch, as before */
    assert_eq!((linear_pitch(7, 4), linear_surface_bytes(7, 3, 4)), (tex_pitch(7), 8 * 3 * 4));
}

/// An 8x8 RGBA8 texture on the Mali's rows: 16 texels (64 bytes) a row,
/// glTexImage2D's rows written there, and the binding a draw gets names that
/// pitch, so the backend samples it in place.
#[test]
fn a_narrow_texture_is_stored_on_whole_64_byte_rows() {
    let gl = TestGl::new(4, 4);
    gl.backend_like_mali_rows();
    let texels: Vec<u8> = (0..8 * 8 * 4).map(|i| i as u8).collect();
    let mut tex = 0;
    // SAFETY: a current context; valid out-parameters and an 8x8 RGBA8 image.
    unsafe {
        glGenTextures(1, &mut tex);
        glBindTexture(GL_TEXTURE_2D, tex);
        glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA as i32, 8, 8, 0, GL_RGBA, GL_UNSIGNED_BYTE, texels.as_ptr().cast());
    }
    assert_eq!(gl.error(), GL_NO_ERROR);
    let t = gl.ctx().texture_ref(tex).expect("the texture");
    assert_eq!((t.chain_pitch, t.level_pitch(8)), (16, 16), "16 texels a row: 64 bytes");
    assert!(t.mem.size >= 16 * 8 * 4, "the storage holds the padded rows");
    // SAFETY: the storage's CPU view, at least `size` bytes.
    let mem = unsafe { core::slice::from_raw_parts(t.mem.cpu as *const u8, 16 * 8 * 4) };
    for y in 0..8 {
        assert_eq!(&mem[y * 64..y * 64 + 32], &texels[y * 32..y * 32 + 32], "row {y} at its 64-byte pitch");
    }
    let b = super::draw::texture_bind_2d(t);
    assert_eq!((b.stride_texels, b.width), (16, 8), "the draw is told the pitch");
}

/// A 16x8 ETC1 image on the Mali's rows: raster rows of 4 blocks of 8 bytes
/// (32), laid out 8 blocks (64 bytes) a row, every block uploaded there.
#[test]
fn compressed_raster_rows_are_whole_64_byte_rows() {
    let gl = TestGl::new(4, 4);
    gl.backend_like_mali_rows();
    let blocks: Vec<u8> = (0..4 * 2 * 8).map(|i| i as u8).collect();
    let mut tex = 0;
    // SAFETY: a current context; valid out-parameters and 8 ETC1 blocks.
    unsafe {
        glGenTextures(1, &mut tex);
        glBindTexture(GL_TEXTURE_2D, tex);
        glCompressedTexImage2D(GL_TEXTURE_2D, 0, 0x8D64, 16, 8, 0, blocks.len() as i32, blocks.as_ptr().cast());
    }
    assert_eq!(gl.error(), GL_NO_ERROR);
    let t = gl.ctx().texture_ref(tex).expect("the texture");
    assert_eq!((t.block_w, t.block_h, t.block_bytes), (8, 2, 8), "8 blocks a row: 64 bytes");
    // SAFETY: the storage's CPU view, at least one level of 2 rows.
    let mem = unsafe { core::slice::from_raw_parts(t.mem.cpu as *const u8, 2 * 64) };
    for by in 0..2 {
        assert_eq!(&mem[by * 64..by * 64 + 32], &blocks[by * 32..by * 32 + 32], "block row {by}");
    }
}

/// A 600x4 RGBA8 renderbuffer (femtovg's layer width) on the Mali's rows:
/// allocated at 608 pixels a row, the draw told that pitch, and
/// glReadPixels reading the rows there.
#[test]
fn a_renderbuffer_is_allocated_drawn_and_read_at_the_aligned_pitch() {
    let gl = TestGl::new(4, 4);
    gl.backend_like_mali_rows();
    let (mut fbo, mut rbo) = (0, 0);
    // SAFETY: a current context; valid out-parameters.
    unsafe {
        glGenFramebuffers(1, &mut fbo);
        glBindFramebuffer(GL_FRAMEBUFFER, fbo);
        glGenRenderbuffers(1, &mut rbo);
        glBindRenderbuffer(GL_RENDERBUFFER, rbo);
        glRenderbufferStorage(GL_RENDERBUFFER, GL_RGBA8_OES, 600, 4);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rbo);
    }
    assert_eq!(gl.error(), GL_NO_ERROR);
    let rb = gl.ctx().renderbuffer_ref(rbo).expect("the renderbuffer");
    assert_eq!(rb.mem.size, 608 * 4 * 4, "608 pixels a row");
    let (mem, w, h, _, pitch) = fb_target_pitched(gl.ctx()).expect("the target");
    assert_eq!((w, h, pitch), (600, 4, 608), "the draw renders at the aligned pitch");
    /* as the GPU leaves it: pixel (x, y) = (x, y, x >> 8, 0xff) at 608 a row */
    let mut px = vec![0u8; mem.size];
    for y in 0..4usize {
        for x in 0..600usize {
            px[(y * 608 + x) * 4..][..4].copy_from_slice(&[x as u8, y as u8, (x >> 8) as u8, 0xff]);
        }
    }
    crate::testutil::gpu_render(mem.gpu, &px);
    let mut out = vec![0u8; 600 * 4 * 4];
    // SAFETY: a buffer of the size read.
    unsafe { glReadPixels(0, 0, 600, 4, GL_RGBA, GL_UNSIGNED_BYTE, out.as_mut_ptr().cast()) };
    assert_eq!(gl.error(), GL_NO_ERROR);
    for y in 0..4usize {
        for x in [0usize, 1, 599] {
            assert_eq!(&out[(y * 600 + x) * 4..][..4], &[x as u8, y as u8, (x >> 8) as u8, 0xff], "({x}, {y})");
        }
    }
}
