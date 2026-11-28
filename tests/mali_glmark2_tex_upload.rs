// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GL-level host test of the texture uploads glmark2-es2 makes on the Mali
//! back end (plan #367): its image textures are GL_RGB / GL_UNSIGNED_BYTE (3
//! bytes a texel) or GL_RGBA, often with NPOT widths whose rows are not
//! 4-byte aligned, under GL_UNPACK_ALIGNMENT 1 or 4. Covered: glTexImage2D of
//! GL_RGB at widths 1, 3, 5 and 7 under alignment 1 and 4, GL_RGBA, GL_LUMINANCE
//! and GL_ALPHA, and glTexSubImage2D into a sub-rectangle.
//!
//! Each texture is set to GL_NEAREST and drawn on a quad the size of the
//! target. The host has no Mali OSAL, so (as in `mali_glmark2_scene_draw.rs`)
//! the stand-in back end hands the draw to `mali_draw`, which ends at
//! `-ENODEV`: no shader runs and no pixel is written. The check is therefore
//! on what the Mali is handed: the draw raises no GL error, the texture
//! arrives complete with its width, height and `no_alpha` flag, and every
//! texel of the stored image (read through the bind's `base_gpu` and
//! `stride_texels`, which the stand-in allocator makes host pointers) equals
//! the RGBA the upload must produce. A row-pitch error (alignment) or a
//! swizzle error shows as a texel mismatch. glReadPixels of the target is
//! checked to raise no error.
//!
//! Not a tools/check.sh row.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_glmark2_tex_upload
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-mali", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend, *};
use yakogl::gl::buffer::glBindBuffer;
use yakogl::gl::consts::*;
use yakogl::gl::draw::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::{glEnableVertexAttribArray, glVertexAttribPointer};
use yakogl::mali::backend::{mali_draw, yakogl_backend_mali};
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::util::ENODEV;

/* ---- a context on the host: the smallest OSAL and back end that make one -- */

unsafe extern "C" fn m_gpu_alloc(_heap: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
    let p = Box::leak(vec![0u8; size].into_boxed_slice()).as_mut_ptr();
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

/// What the back end was handed for the draw's texture, and what the Mali
/// made of the draw.
#[derive(Clone, Debug)]
#[allow(dead_code)]
struct Seen {
    type_: u8,
    width: u32,
    mip_levels: u8,
    mip_filter: u8,
    incomplete: u8,
    mali: i32,
    height: u32,
    stride: u32,
    no_alpha: u8,
    texels: Vec<u8>,
}

static SEEN: Mutex<Vec<Seen>> = Mutex::new(Vec::new());

unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    if d.index_count == 0 {
        return 0;
    }
    let t = &d.textures[0];
    let mali = mali_draw(d);
    // SAFETY: the stand-in allocator's gpu address is the host pointer of a
    // buffer of at least stride * height texels.
    let texels = unsafe {
        core::slice::from_raw_parts(t.base_gpu as usize as *const u8, (t.stride_texels * t.height * 4) as usize)
            .to_vec()
    };
    SEEN.lock().unwrap().push(Seen {
        height: t.height,
        stride: t.stride_texels,
        no_alpha: t.no_alpha,
        texels,
        type_: t.type_,
        width: t.width,
        mip_levels: t.mip_levels,
        mip_filter: t.mip_filter,
        incomplete: t.incomplete,
        mali,
    });
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

/// The Mali's table with the stand-in's own entries (a draw that records, a
/// heap base, no hardware to start or wait for).
fn stand_in() -> &'static YakoglBackend {
    let m = &yakogl_backend_mali;
    Box::leak(Box::new(YakoglBackend {
        name: m.name,
        native_attrib_formats: m.native_attrib_formats,
        rt_rgba8888_is_bgra: m.rt_rgba8888_is_bgra,
        mip_level_align: m.mip_level_align,
        compressed_blocks_in_rows: m.compressed_blocks_in_rows,
        native_layered_textures: m.native_layered_textures,
        front_facing_shader_variant: m.front_facing_shader_variant,
        etc1_decoded_at_upload: m.etc1_decoded_at_upload,
        init: Some(b_init),
        shutdown: None,
        draw: Some(b_draw),
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
        max_instance_divisor: m.max_instance_divisor,
        wait_kick: None,
        samples_texel_copies: m.samples_texel_copies,
        attachment_gone: None,
        depth_resolve: None,
        query_begin: None,
        query_end: None,
        query_result: None,
        generate_mipmap: m.generate_mipmap,
        return_mipmap_chain: m.return_mipmap_chain,
        linear_row_align: m.linear_row_align,
        uniforms_host_only: m.uniforms_host_only,
        ds_write: None,
        stencil_resolve: None,
        max_samples: m.max_samples,
    }))
}

/// One context for the whole file; the tests take turns on it.
static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

fn context() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    ONCE.call_once(|| {
        // SAFETY: the tables are statics or leaked; the lock above serialises
        // the tests.
        unsafe {
            osal_set_ops(&OSAL);
            backend::g_backend = stand_in();
            yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            assert!(!yakogl::gl::context::gl_context_create(8, 8).is_null());
        }
    });
    SEEN.lock().unwrap().clear();
    // SAFETY: plain GL call.
    while unsafe { glGetError() } != GL_NO_ERROR {}
    g
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

fn shader(ty: GLenum, text: &str) -> GLuint {
    let text = CString::new(text).unwrap();
    // SAFETY: plain GL calls; one NUL-terminated source.
    unsafe {
        let s = glCreateShader(ty);
        let p: [*const c_char; 1] = [text.as_ptr()];
        glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
        glCompileShader(s);
        let mut ok: GLint = 0;
        glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
        let mut log = [0u8; 512];
        let mut n: GLsizei = 0;
        glGetShaderInfoLog(s, log.len() as GLsizei, &mut n, log.as_mut_ptr().cast());
        assert_eq!(ok, 1, "{text:?} did not compile: {}", String::from_utf8_lossy(&log[..n as usize]));
        s
    }
}

fn program(vs: &str, fs: &str, attribs: &[&core::ffi::CStr]) -> GLuint {
    let (v, f) = (shader(GL_VERTEX_SHADER, vs), shader(GL_FRAGMENT_SHADER, fs));
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        for (i, a) in attribs.iter().enumerate() {
            glBindAttribLocation(p, i as GLuint, a.as_ptr());
        }
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "link failed");
        glUseProgram(p);
        p
    }
}

/// An RGBA8 texture as the colour attachment of a framebuffer.
fn target(w: i32, h: i32) -> (GLuint, GLuint) {
    let (mut fbo, mut tex) = (0, 0);
    // SAFETY: room for one name each; null texels allocate the image.
    unsafe {
        glGenTextures(1, &mut tex);
        glBindTexture(GL_TEXTURE_2D, tex);
        glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA as GLint, w, h, 0, GL_RGBA, GL_UNSIGNED_BYTE, core::ptr::null());
        glGenFramebuffers(1, &mut fbo);
        glBindFramebuffer(GL_FRAMEBUFFER, fbo);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
        glViewport(0, 0, w, h);
        glClearColor(0.0, 0.0, 0.0, 1.0);
        glClear(GL_COLOR_BUFFER_BIT);
    }
    assert_eq!(err(), GL_NO_ERROR, "target set-up");
    (fbo, tex)
}

/// Two triangles over the whole target: position (xyz) and a second
/// attribute (normal or coordinate, 3 floats).
static POS: [f32; 18] =
    [-1.0, -1.0, 0.0, 1.0, -1.0, 0.0, -1.0, 1.0, 0.0, -1.0, 1.0, 0.0, 1.0, -1.0, 0.0, 1.0, 1.0, 0.0];
static UV: [f32; 12] = [0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0, 1.0];

/// Draw the quad with `second` as attribute 1 (`n` floats per vertex), read
/// back one pixel; returns the draw's and the read's GL errors, and what the
/// Mali was handed.
fn draw(second: &[f32], n: i32) -> (GLenum, GLenum, Seen) {
    SEEN.lock().unwrap().clear();
    let mut px = [0u8; 4];
    // SAFETY: the arrays are static; `px` holds one RGBA8 pixel.
    unsafe {
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 3, GL_FLOAT, 0, 0, POS.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glVertexAttribPointer(1, n as GLint, GL_FLOAT, 0, 0, second.as_ptr() as *const c_void);
        glEnableVertexAttribArray(1);
        glDrawArrays(GL_TRIANGLES, 0, 6);
        glFinish();
    }
    let de = err();
    // SAFETY: one pixel into `px`.
    unsafe { glReadPixels(4, 4, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr().cast()) };
    let re = err();
    let s = core::mem::take(&mut *SEEN.lock().unwrap());
    (de, re, s.last().expect("the draw reached the back end").clone())
}

const TEX_VS: &str = "attribute vec3 a_pos;
attribute vec2 a_uv;
varying mediump vec2 v_uv;
void main()
{
    gl_Position = vec4(a_pos, 1.0);
    v_uv = a_uv;
}
";
const TEX_FS: &str = "precision mediump float;
uniform sampler2D u_tex;
varying mediump vec2 v_uv;
void main()
{
    gl_FragColor = texture2D(u_tex, v_uv);
}
";

/// Source byte of texel `i`, channel `c`: distinct for every position.
fn val(i: usize, c: usize) -> u8 {
    (i * 37 + c * 11 + 5) as u8
}

/// Client image `w` x `h` with `bpt` bytes a texel, rows padded to `align`;
/// padding bytes are 0xee so that a wrong pitch reads them.
fn client(w: usize, h: usize, bpt: usize, align: usize) -> Vec<u8> {
    let pitch = (w * bpt).div_ceil(align) * align;
    let mut v = vec![0xeeu8; pitch * h];
    for y in 0..h {
        for x in 0..w {
            for c in 0..bpt {
                v[y * pitch + x * bpt + c] = val(y * w + x, c);
            }
        }
    }
    v
}

/// The RGBA a texel of `fmt` made of source bytes `b` must store.
fn expect(fmt: GLenum, b: &[u8]) -> [u8; 4] {
    match fmt {
        GL_RGBA => [b[0], b[1], b[2], b[3]],
        GL_RGB => [b[0], b[1], b[2], 255],
        GL_LUMINANCE => [b[0], b[0], b[0], 255],
        GL_ALPHA => [0, 0, 0, b[0]],
        _ => unreachable!(),
    }
}

fn bpt(fmt: GLenum) -> usize {
    match fmt {
        GL_RGBA => 4,
        GL_RGB => 3,
        _ => 1,
    }
}

/// Draw `tex` (bound, program in use) on the quad; returns what the Mali saw.
fn draw_checked() -> Seen {
    let (de, re, s) = draw(&UV, 2);
    assert_eq!((de, re), (GL_NO_ERROR, GL_NO_ERROR), "draw / read errors");
    assert_eq!(s.mali, -ENODEV, "the Mali takes the draw");
    assert_eq!((s.type_, s.incomplete), (GLES_TEXTYPE_2D, 0), "the texture arrives complete");
    s
}

fn check_texels(s: &Seen, fmt: GLenum, w: usize, h: usize, want: &dyn Fn(usize, usize) -> [u8; 4], what: &str) {
    assert_eq!((s.width as usize, s.height as usize), (w, h), "{what}: size");
    for y in 0..h {
        for x in 0..w {
            let o = (y * s.stride as usize + x) * 4;
            assert_eq!(&s.texels[o..o + 4], &want(x, y), "{what}: texel ({x},{y})");
        }
    }
    let alpha_free = matches!(fmt, GL_RGB | GL_LUMINANCE);
    assert_eq!(s.no_alpha != 0, alpha_free, "{what}: no_alpha");
}

/// Upload a `w` x `h` image of `fmt` under `align`, draw it, check every texel.
fn upload_case(fmt: GLenum, w: usize, h: usize, align: GLint) {
    let _g = context();
    let (fbo, tex) = target(w as i32, h as i32);
    let p = program(TEX_VS, TEX_FS, &[c"a_pos", c"a_uv"]);
    let img = client(w, h, bpt(fmt), align as usize);
    let mut src = 0;
    // SAFETY: `img` covers w x h texels at the pitch the alignment gives.
    unsafe {
        glGenTextures(1, &mut src);
        glActiveTexture(GL_TEXTURE0);
        glBindTexture(GL_TEXTURE_2D, src);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE as GLint);
        glPixelStorei(GL_UNPACK_ALIGNMENT, align);
        glTexImage2D(GL_TEXTURE_2D, 0, fmt as GLint, w as i32, h as i32, 0, fmt, GL_UNSIGNED_BYTE, img.as_ptr().cast());
        glPixelStorei(GL_UNPACK_ALIGNMENT, 4);
        glUniform1i(glGetUniformLocation(p, c"u_tex".as_ptr()), 0);
    }
    assert_eq!(err(), GL_NO_ERROR, "upload");
    let s = draw_checked();
    let pitch = (w * bpt(fmt)).div_ceil(align as usize) * align as usize;
    let what = format!("{fmt:#x} {w}x{h} align {align}");
    check_texels(
        &s,
        fmt,
        w,
        h,
        &|x, y| {
            let o = y * pitch + x * bpt(fmt);
            expect(fmt, &img[o..o + bpt(fmt)])
        },
        &what,
    );
    // SAFETY: names this test made.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fbo);
        glDeleteTextures(1, &tex);
        glDeleteTextures(1, &src);
    }
}

#[test]
fn rgb_widths_align1() {
    for w in [1, 3, 5, 7] {
        upload_case(GL_RGB, w, 3, 1);
    }
}

#[test]
fn rgb_widths_align4() {
    for w in [1, 3, 5, 7] {
        upload_case(GL_RGB, w, 3, 4);
    }
}

#[test]
fn rgba_widths_both_alignments() {
    for w in [1, 3, 5, 7] {
        for a in [1, 4] {
            upload_case(GL_RGBA, w, 3, a);
        }
    }
}

#[test]
fn luminance_widths_both_alignments() {
    for w in [1, 3, 5, 7] {
        for a in [1, 4] {
            upload_case(GL_LUMINANCE, w, 3, a);
        }
    }
}

#[test]
fn alpha_widths_both_alignments() {
    for w in [1, 3, 5, 7] {
        for a in [1, 4] {
            upload_case(GL_ALPHA, w, 3, a);
        }
    }
}

/// glTexSubImage2D of a 3x2 RGB block at (2,1) into a 7x4 RGBA texture
/// under alignment 1: the rest keeps the first upload.
#[test]
fn sub_image_rgb_into_rgba() {
    let _g = context();
    let (fbo, tex) = target(7, 4);
    let p = program(TEX_VS, TEX_FS, &[c"a_pos", c"a_uv"]);
    let base = client(7, 4, 4, 1);
    let blk = client(3, 2, 3, 1);
    let mut src = 0;
    // SAFETY: the client images cover what the calls read.
    unsafe {
        glGenTextures(1, &mut src);
        glActiveTexture(GL_TEXTURE0);
        glBindTexture(GL_TEXTURE_2D, src);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
        glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
        glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA as GLint, 7, 4, 0, GL_RGBA, GL_UNSIGNED_BYTE, base.as_ptr().cast());
        glTexSubImage2D(GL_TEXTURE_2D, 0, 2, 1, 3, 2, GL_RGB, GL_UNSIGNED_BYTE, blk.as_ptr().cast());
        glPixelStorei(GL_UNPACK_ALIGNMENT, 4);
        glUniform1i(glGetUniformLocation(p, c"u_tex".as_ptr()), 0);
    }
    assert_eq!(err(), GL_NO_ERROR, "upload");
    let s = draw_checked();
    check_texels(
        &s,
        GL_RGBA,
        7,
        4,
        &|x, y| {
            if (2..5).contains(&x) && (1..3).contains(&y) {
                let o = (y - 1) * 9 + (x - 2) * 3;
                expect(GL_RGB, &blk[o..o + 3])
            } else {
                let o = (y * 7 + x) * 4;
                expect(GL_RGBA, &base[o..o + 4])
            }
        },
        "sub-image rgb into rgba",
    );
    // SAFETY: names this test made.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fbo);
        glDeleteTextures(1, &tex);
        glDeleteTextures(1, &src);
    }
}

/// glTexSubImage2D of a 3x2 RGB block under alignment 4 (row pitch 12, not 9)
/// into a 5x3 RGB texture.
#[test]
fn sub_image_rgb_align4() {
    let _g = context();
    let (fbo, tex) = target(5, 3);
    let p = program(TEX_VS, TEX_FS, &[c"a_pos", c"a_uv"]);
    let base = client(5, 3, 3, 1);
    let blk = client(3, 2, 3, 4);
    let mut src = 0;
    // SAFETY: the client images cover what the calls read.
    unsafe {
        glGenTextures(1, &mut src);
        glActiveTexture(GL_TEXTURE0);
        glBindTexture(GL_TEXTURE_2D, src);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
        glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
        glTexImage2D(GL_TEXTURE_2D, 0, GL_RGB as GLint, 5, 3, 0, GL_RGB, GL_UNSIGNED_BYTE, base.as_ptr().cast());
        glPixelStorei(GL_UNPACK_ALIGNMENT, 4);
        glTexSubImage2D(GL_TEXTURE_2D, 0, 1, 1, 3, 2, GL_RGB, GL_UNSIGNED_BYTE, blk.as_ptr().cast());
        glUniform1i(glGetUniformLocation(p, c"u_tex".as_ptr()), 0);
    }
    assert_eq!(err(), GL_NO_ERROR, "upload");
    let s = draw_checked();
    check_texels(
        &s,
        GL_RGB,
        5,
        3,
        &|x, y| {
            if (1..4).contains(&x) && (1..3).contains(&y) {
                let o = (y - 1) * 12 + (x - 1) * 3;
                expect(GL_RGB, &blk[o..o + 3])
            } else {
                let o = (y * 5 + x) * 3;
                expect(GL_RGB, &base[o..o + 3])
            }
        },
        "sub-image rgb align 4",
    );
    // SAFETY: names this test made.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fbo);
        glDeleteTextures(1, &tex);
        glDeleteTextures(1, &src);
    }
}
