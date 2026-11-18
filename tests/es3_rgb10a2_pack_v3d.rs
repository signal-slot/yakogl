// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! glReadPixels of GL_RGB10_A2 renderbuffer and texture attachments with the
//! pack parameters and into a pixel pack buffer, on the V3D host back end,
//! through the GL entry points only (plan #370, rgb10a2-pack-api).
//!
//! 8428875e made glReadPixels read such an attachment as
//! GL_RGBA/GL_UNSIGNED_BYTE (decoded) and as
//! GL_RGBA/GL_UNSIGNED_INT_2_10_10_10_REV (the word unchanged); both are
//! 4 bytes a pixel. `es3_rgb10a2_readback_v3d` pins the plain read; this file
//! pins where the bytes go, for both attachment kinds and both types:
//!
//! - GL_PACK_ALIGNMENT 1/4/8 with an odd width (alignment 8 pads each row to
//!   16 bytes; the padding is not written),
//! - GL_PACK_ROW_LENGTH, GL_PACK_SKIP_PIXELS, GL_PACK_SKIP_ROWS (bytes
//!   outside the read rectangle are not written),
//! - a read into a GL_PIXEL_PACK_BUFFER at a non-zero offset, mapped back
//!   with glMapBufferRange,
//! - a sub-rectangle with a non-zero x/y, and one partly outside the
//!   surface (the pixels outside are not written).
//!
//! `expected` models ES 3.0 4.3.2: it lays out the bytes each read must
//! produce in a 0x77-filled destination. The implementation read pair is
//! not asserted here.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_rgb10a2_pack_v3d
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-v3d", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesHeap, YakoglBackend};
use yakogl::gl::buffer::*;
use yakogl::gl::consts::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::state::*;
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};

unsafe extern "C" fn m_gpu_alloc(_heap: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
    let mem = Box::leak(vec![0u8; size].into_boxed_slice());
    let p = mem.as_mut_ptr();
    *out = OsalGpuMem { cpu: p as *mut c_void, gpu: p as u64, phys: p as u64, size, impl_: p as *mut c_void };
    0
}
unsafe extern "C" fn m_gpu_free(_mem: *mut OsalGpuMem) -> i32 {
    0
}
unsafe extern "C" fn m_cache(_cpu: *mut c_void, _n: usize) {}
unsafe extern "C" fn b_init() -> i32 {
    0
}
unsafe extern "C" fn b_heap_base(_heap: GlesHeap) -> u64 {
    0x1000_0000
}
unsafe extern "C" fn b_ok() -> i32 {
    0
}
unsafe extern "C" fn b_ok_draw(_d: *const backend::GlesDraw) -> i32 {
    0
}

static OSAL: OsalOps = OsalOps {
    init: None,
    shutdown: None,
    gpu_alloc: Some(m_gpu_alloc),
    gpu_free: Some(m_gpu_free),
    mmu_map: None,
    map_phys: None,
    gpu_write: None,
    gpu_read: None,
    cache_flush: Some(m_cache),
    cache_invd: Some(m_cache),
    submit: None,
    wait: None,
    completed: None,
    reg_read: None,
    reg_write: None,
    query: None,
    log: None,
    wait_vblank: None,
    present: None,
    lock: None,
    unlock: None,
    thread_id: None,
    display_busy: None,
};

static BACKEND: YakoglBackend = YakoglBackend {
    name: c"es3-rgb10a2-pack".as_ptr() as *const c_char,
    native_attrib_formats: 0,
    rt_rgba8888_is_bgra: false,
    mip_level_align: 0,
    compressed_blocks_in_rows: false,
    native_layered_textures: false,
    front_facing_shader_variant: false,
    etc1_decoded_at_upload: false,
    init: Some(b_init),
    shutdown: None,
    draw: Some(b_ok_draw),
    flush: Some(b_ok),
    finish: Some(b_ok),
    finish_previous: None,
    kick_id: None,
    kick_pending: None,
    heap_base: Some(b_heap_base),
    counters: None,
    texture_released: None,
    surface_swapped: None,
    texture_uploaded: None,
    max_instance_divisor: 1,
    wait_kick: None,
    samples_texel_copies: false,
    attachment_gone: None,
    depth_resolve: None,
    query_begin: None,
    query_end: None,
    query_result: None,
    generate_mipmap: None,
    return_mipmap_chain: None,
    linear_row_align: 0,
    uniforms_host_only: false,
    ds_write: None,
    stencil_resolve: None,
    max_samples: 0,
};

static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

const W: GLsizei = 3;
const H: GLsizei = 3;

fn context() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    ONCE.call_once(|| {
        // SAFETY: both tables are statics; the lock above serialises the tests.
        unsafe {
            osal_set_ops(&OSAL);
            backend::g_backend = &BACKEND;
            yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            assert!(!yakogl::gl::context::gl_context_create(8, 8).is_null());
        }
    });
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glFinish();
    }
    while err() != GL_NO_ERROR {}
    g
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

/// The 10/10/10/2 word of (r, g, b, a), as GL_UNSIGNED_INT_2_10_10_10_REV
/// packs it.
fn a2(r: u32, g: u32, b: u32, a: u32) -> u32 {
    assert!(r < 1024 && g < 1024 && b < 1024 && a < 4);
    r | g << 10 | b << 20 | a << 30
}

/// A colour attachment: a texture at attachment 0 of its own framebuffer.
struct Att {
    fb: GLuint,
    tex: GLuint,
    w: GLsizei,
    h: GLsizei,
}

impl Drop for Att {
    fn drop(&mut self) {
        // SAFETY: names this test made.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteFramebuffers(1, &self.fb);
            glDeleteTextures(1, &self.tex);
        }
    }
}

fn attach(internal: GLenum, w: GLsizei, h: GLsizei, ty: GLenum, px: *const c_void) -> Att {
    let (mut fb, mut tex) = (0, 0);
    // SAFETY: room for one name each; `px` is null or holds w x h texels of `ty`.
    unsafe {
        glGenTextures(1, &mut tex);
        glBindTexture(GL_TEXTURE_2D, tex);
        glTexImage2D(GL_TEXTURE_2D, 0, internal as GLint, w, h, 0, GL_RGBA, ty, px);
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
        assert_eq!(err(), GL_NO_ERROR, "texture {internal:#x} {w}x{h}");
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "{internal:#x}");
    }
    Att { fb, tex, w, h }
}

/// An RGB10_A2 texture of `words` (w x h of them, or none: zeros).
fn tex10(w: GLsizei, h: GLsizei, words: Option<&[u32]>) -> Att {
    let zero = vec![0u32; (w * h) as usize];
    let words = words.unwrap_or(&zero);
    assert_eq!(words.len(), (w * h) as usize);
    attach(GL_RGB10_A2, w, h, GL_UNSIGNED_INT_2_10_10_10_REV, words.as_ptr() as *const c_void)
}

fn blit(src: &Att, s: [GLint; 4], dst: &Att, d: [GLint; 4], filter: GLenum) {
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, src.fb);
        glReadBuffer(GL_COLOR_ATTACHMENT0);
        glBindFramebuffer(GL_DRAW_FRAMEBUFFER, dst.fb);
        let draw = [GL_COLOR_ATTACHMENT0];
        glDrawBuffers(1, draw.as_ptr());
        glBlitFramebuffer(s[0], s[1], s[2], s[3], d[0], d[1], d[2], d[3], GL_COLOR_BUFFER_BIT, filter);
    }
    assert_eq!(err(), GL_NO_ERROR, "glBlitFramebuffer");
}

fn whole(a: &Att) -> [GLint; 4] {
    [0, 0, a.w, a.h]
}

/// Edge words first: all zero, all ones, each field alone, alpha 0..3, and
/// values off the byte rounding midpoints.
fn edge_words() -> Vec<u32> {
    let px = [
        (0, 0, 0, 0),
        (1023, 1023, 1023, 3),
        (1, 0, 0, 1),
        (0, 1, 0, 2),
        (0, 0, 1, 3),
        (512, 513, 2, 0),
        (341, 682, 1022, 1),
        (100, 900, 700, 2),
        (999, 3, 300, 3),
    ];
    px.iter().map(|&(r, g, b, a)| a2(r, g, b, a)).collect()
}

fn scale(v: u32, max: u32) -> u8 {
    ((v * 255 + max / 2) / max) as u8
}

fn decoded(w: u32) -> [u8; 4] {
    [scale(w & 0x3ff, 1023), scale((w >> 10) & 0x3ff, 1023), scale((w >> 20) & 0x3ff, 1023), scale(w >> 30, 3)]
}

/// An RGB10_A2 colour attachment (a texture, or a renderbuffer in `rb`).
struct Buf {
    att: Att,
    rb: GLuint,
}

impl Drop for Buf {
    fn drop(&mut self) {
        if self.rb != 0 {
            // SAFETY: a name this test made.
            unsafe {
                glBindFramebuffer(GL_FRAMEBUFFER, 0);
                glDeleteRenderbuffers(1, &self.rb);
            }
        }
    }
}

/// A texture attachment holding `words`, filled by a class to class blit.
fn filled_texture(words: &[u32]) -> Buf {
    let s = tex10(W, H, Some(words));
    let d = tex10(W, H, None);
    blit(&s, whole(&s), &d, whole(&d), GL_NEAREST);
    Buf { att: d, rb: 0 }
}

/// A renderbuffer attachment holding `words`, filled the same way.
fn filled_renderbuffer(words: &[u32]) -> Buf {
    let s = tex10(W, H, Some(words));
    let (mut fb, mut rb) = (0, 0);
    // SAFETY: room for one name each.
    unsafe {
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorage(GL_RENDERBUFFER, GL_RGB10_A2, W, H);
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb);
        assert_eq!(err(), GL_NO_ERROR, "renderbuffer storage");
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
    }
    let att = Att { fb, tex: 0, w: W, h: H };
    blit(&s, whole(&s), &att, whole(&att), GL_NEAREST);
    Buf { att, rb }
}

/// One read: the rectangle, the type and the pack state.
#[derive(Clone, Copy)]
struct Rd {
    x: GLint,
    y: GLint,
    w: GLsizei,
    h: GLsizei,
    ty: GLenum,
    align: GLint,
    row_len: GLint,
    skip_px: GLint,
    skip_rows: GLint,
}

fn rd(ty: GLenum) -> Rd {
    Rd { x: 0, y: 0, w: W, h: H, ty, align: 4, row_len: 0, skip_px: 0, skip_rows: 0 }
}

const FILL: u8 = 0x77;
const TYPES: [GLenum; 2] = [GL_UNSIGNED_BYTE, GL_UNSIGNED_INT_2_10_10_10_REV];

fn set_pack(r: &Rd) {
    // SAFETY: plain GL calls.
    unsafe {
        glPixelStorei(GL_PACK_ALIGNMENT, r.align);
        glPixelStorei(GL_PACK_ROW_LENGTH, r.row_len);
        glPixelStorei(GL_PACK_SKIP_PIXELS, r.skip_px);
        glPixelStorei(GL_PACK_SKIP_ROWS, r.skip_rows);
    }
    assert_eq!(err(), GL_NO_ERROR, "glPixelStorei");
}

fn reset_pack() {
    set_pack(&Rd { align: 4, ..rd(GL_UNSIGNED_BYTE) });
}

/// What ES 3.0 4.3.2 writes for `r` into `len` bytes of FILL, starting at
/// the destination's first byte: pixel (i, j) of the rectangle lands at row
/// `skip_rows + j` (rows `stride` bytes apart, GL_PACK_ROW_LENGTH or the
/// width, times 4, rounded up to the alignment), pixel `skip_px + i`. Pixels
/// outside the surface are not written. The surface row is the window row.
fn expected(words: &[u32], r: &Rd, len: usize) -> Vec<u8> {
    let mut out = vec![FILL; len];
    let a = r.align as usize;
    let row = if r.row_len > 0 { r.row_len } else { r.w } as usize * 4;
    let stride = row.div_ceil(a) * a;
    for j in 0..r.h {
        for i in 0..r.w {
            let (sx, sy) = (r.x + i, r.y + j);
            if !(0..W).contains(&sx) || !(0..H).contains(&sy) {
                continue;
            }
            let word = words[(sy * W + sx) as usize];
            let px: [u8; 4] = if r.ty == GL_UNSIGNED_BYTE { decoded(word) } else { word.to_le_bytes() };
            let at = (r.skip_rows + j) as usize * stride + (r.skip_px + i) as usize * 4;
            out[at..at + 4].copy_from_slice(&px);
        }
    }
    out
}

const LEN: usize = 160;

/// glReadPixels of `r` into client memory of LEN bytes of FILL.
fn read_client(b: &Buf, r: &Rd) -> Vec<u8> {
    let mut px = vec![FILL; LEN];
    set_pack(r);
    // SAFETY: LEN holds every row any case here writes.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, b.att.fb);
        glReadBuffer(GL_COLOR_ATTACHMENT0);
        glReadPixels(r.x, r.y, r.w, r.h, GL_RGBA, r.ty, px.as_mut_ptr() as *mut c_void);
    }
    let e = err();
    reset_pack();
    assert_eq!(e, GL_NO_ERROR, "glReadPixels");
    px
}

/// glReadPixels of `r` into a pixel pack buffer of LEN bytes of FILL at
/// `offset`, mapped back whole.
fn read_pbo(b: &Buf, r: &Rd, offset: usize) -> Vec<u8> {
    let fill = [FILL; LEN];
    let mut pbo = 0;
    set_pack(r);
    // SAFETY: plain GL calls; the buffer holds LEN bytes.
    let out = unsafe {
        glGenBuffers(1, &mut pbo);
        glBindBuffer(GL_PIXEL_PACK_BUFFER, pbo);
        glBufferData(GL_PIXEL_PACK_BUFFER, LEN as isize, fill.as_ptr() as *const c_void, GL_STREAM_READ);
        assert_eq!(err(), GL_NO_ERROR, "glBufferData");
        glBindFramebuffer(GL_READ_FRAMEBUFFER, b.att.fb);
        glReadBuffer(GL_COLOR_ATTACHMENT0);
        glReadPixels(r.x, r.y, r.w, r.h, GL_RGBA, r.ty, offset as *mut c_void);
        let e = err();
        reset_pack();
        assert_eq!(e, GL_NO_ERROR, "glReadPixels into the pack buffer");
        let p = glMapBufferRange(GL_PIXEL_PACK_BUFFER, 0, LEN as isize, GL_MAP_READ_BIT) as *const u8;
        assert_eq!(err(), GL_NO_ERROR, "glMapBufferRange");
        assert!(!p.is_null());
        let v = std::slice::from_raw_parts(p, LEN).to_vec();
        glUnmapBuffer(GL_PIXEL_PACK_BUFFER);
        glBindBuffer(GL_PIXEL_PACK_BUFFER, 0);
        glDeleteBuffers(1, &pbo);
        v
    };
    assert_eq!(err(), GL_NO_ERROR, "unmap");
    out
}

fn check_client(b: &Buf, words: &[u32], r: Rd, what: &str) {
    assert_eq!(read_client(b, &r), expected(words, &r, LEN), "{what}, type {:#x}", r.ty);
}

/// The same read into a pack buffer at `offset`: the bytes before the offset
/// are untouched and the rest is `expected` shifted by it.
fn check_pbo(b: &Buf, words: &[u32], r: Rd, offset: usize, what: &str) {
    let mut want = vec![FILL; offset];
    want.extend(expected(words, &r, LEN - offset));
    assert_eq!(read_pbo(b, &r, offset), want, "{what} at offset {offset}, type {:#x}", r.ty);
}

fn alignment(b: &Buf, w: &[u32]) {
    for ty in TYPES {
        for align in [1, 4, 8] {
            check_client(b, w, Rd { align, ..rd(ty) }, &format!("alignment {align}"));
        }
        /* alignment 8, a 3 wide row is 12 bytes: padded to 16 */
        let r = Rd { align: 8, ..rd(ty) };
        let got = read_client(b, &r);
        assert!(got[12..16].iter().all(|&v| v == FILL), "row padding untouched, type {ty:#x}");
    }
}

fn row_skip(b: &Buf, w: &[u32]) {
    for ty in TYPES {
        check_client(b, w, Rd { row_len: 5, ..rd(ty) }, "row length 5");
        check_client(b, w, Rd { w: 2, h: 2, row_len: 7, ..rd(ty) }, "2x2 in row length 7");
        check_client(b, w, Rd { skip_px: 2, ..rd(ty) }, "skip pixels 2");
        check_client(b, w, Rd { skip_rows: 2, ..rd(ty) }, "skip rows 2");
        check_client(b, w, Rd { row_len: 6, skip_px: 1, skip_rows: 2, ..rd(ty) }, "all three");
        check_client(
            b,
            w,
            Rd { x: 1, y: 1, w: 2, h: 2, row_len: 5, skip_px: 1, skip_rows: 1, align: 8, ..rd(ty) },
            "all three, alignment 8, sub-rectangle",
        );
    }
}

fn pbo_reads(b: &Buf, w: &[u32]) {
    for ty in TYPES {
        check_pbo(b, w, rd(ty), 0, "plain");
        check_pbo(b, w, rd(ty), 20, "plain");
        check_pbo(b, w, Rd { align: 8, ..rd(ty) }, 20, "alignment 8");
        check_pbo(b, w, Rd { row_len: 5, skip_px: 1, skip_rows: 1, ..rd(ty) }, 24, "row length, skips");
        check_pbo(b, w, Rd { x: 1, y: 1, w: 2, h: 2, ..rd(ty) }, 12, "sub-rectangle");
    }
}

fn sub_rect(b: &Buf, w: &[u32]) {
    for ty in TYPES {
        check_client(b, w, Rd { x: 1, y: 0, w: 2, h: 3, ..rd(ty) }, "x 1");
        check_client(b, w, Rd { x: 0, y: 1, w: 3, h: 2, ..rd(ty) }, "y 1");
        check_client(b, w, Rd { x: 2, y: 2, w: 1, h: 1, ..rd(ty) }, "last pixel");
        check_client(b, w, Rd { x: 1, y: 1, w: 2, h: 2, align: 8, ..rd(ty) }, "x 1, y 1, alignment 8");
        /* partly outside: left, bottom, right and top, and all of them */
        check_client(b, w, Rd { x: -1, y: 0, w: 3, h: 3, ..rd(ty) }, "left edge outside");
        check_client(b, w, Rd { x: 0, y: -2, w: 3, h: 4, ..rd(ty) }, "bottom edge outside");
        check_client(b, w, Rd { x: 2, y: 1, w: 3, h: 3, ..rd(ty) }, "right and top edges outside");
        check_client(b, w, Rd { x: -1, y: -1, w: 5, h: 5, row_len: 6, skip_px: 1, ..rd(ty) }, "surrounded");
    }
}

macro_rules! rows {
    ($rb:ident, $tex:ident, $f:ident) => {
        #[test]
        fn $rb() {
            let _g = context();
            let w = edge_words();
            $f(&filled_renderbuffer(&w), &w);
        }
        #[test]
        fn $tex() {
            let _g = context();
            let w = edge_words();
            $f(&filled_texture(&w), &w);
        }
    };
}

rows!(renderbuffer_pack_alignment, texture_pack_alignment, alignment);
rows!(renderbuffer_pack_row_length_and_skips, texture_pack_row_length_and_skips, row_skip);
rows!(renderbuffer_reads_into_pack_buffer, texture_reads_into_pack_buffer, pbo_reads);
rows!(renderbuffer_sub_rectangles, texture_sub_rectangles, sub_rect);
