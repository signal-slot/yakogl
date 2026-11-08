// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! PowerVR (AM62P): R and RG integer textures are complete, drawable colour
//! attachments (dEQP fbo.color, fbo.render and
//! fbo.completeness.renderable.texture.color0.*, RC17).
//!
//! `es3_formats` holds an R or RG integer texture (R8I .. RG32UI, twelve
//! formats) widened to the four-channel store of the same channel type and
//! width (R8I in RGBA8I, RG32UI in RGBA32UI): the PowerVR samples only those.
//! The GL core used to answer GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT for them,
//! because the surface of the format (`GLES_SURF_R8I` ..) is not the store's
//! layout. Now the PowerVR build draws such a texture as the wide surface of
//! its store (src/gl/framebuffer.rs `integer_texture_internalformat`,
//! `WIDE_INT_RT`), which its PBE writes. What is held, through the GL API on a
//! stand-in back end and the in-process compiler:
//!
//! - each of the twelve is COMPLETE at GL_COLOR_ATTACHMENT0, as layer 1 of a
//!   2D array and as level 1 of a mipmapped texture;
//! - a draw with an `ivec4` / `uvec4` output hands the back end one target of
//!   the wide surface at the attachment's address, the write mask of the
//!   format's channels (`GlesDraw::color_mask`), and a fragment program
//!   compiled for the wide layout with no colour mask in its key (the
//!   channels the format lacks are not the application's to mask);
//! - glReadPixels of the attachment gives the format's channels and 0, 0, 1
//!   for the others (ES 3.0 4.3.2), whatever the store holds there.
//!
//! What it cannot see: the GPU. The PowerVR's fragment program writes all
//! four channels of the wide store (the PBE has no per-channel mask, and the
//! compiler refuses an integer colour mask: rc1-int-mask); a texture sampled
//! after the draw reads the shader's values in the channels the format lacks
//! until it writes 0, 0, 1 there.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_pvr_fbo_int_rg
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-powervr", not(feature = "report-es2")))]
// the draw needs the in-process compiler; without it its half of the record is idle
#![cfg_attr(not(feature = "shaderc-online"), allow(dead_code))]

use core::ffi::{c_char, c_void};
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
use yakogl::gl::es3::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::state::*;
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};

/* ---- a context on the host: the smallest OSAL and back end that make one -- */

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

/// What the back end was handed, one entry a draw (or clear).
#[derive(Clone, Debug)]
struct Seen {
    /// An application draw (vertices), not a clear alone.
    draws: bool,
    /// `targets[..num_targets]`: (gpu, w, h, format).
    targets: Vec<(u64, u32, u32, u8)>,
    color_mask: u8,
    /// The fragment program's (`ShcShader::blend_key`), 0 without one.
    blend_key: u32,
}

static SEEN: Mutex<Vec<Seen>> = Mutex::new(Vec::new());

/// Records each draw; a clear of an integer target stores the low bits of
/// the four raw values in the target's layout, little endian, rows at the
/// pair-padded pitch (every channel the surface has, as the PBE would).
unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    let n = d.num_targets.max(1) as usize;
    SEEN.lock().unwrap().push(Seen {
        draws: d.index_count != 0,
        targets: d.targets[..n].iter().map(|t| (t.gpu, t.w, t.h, t.format)).collect(),
        color_mask: d.color_mask,
        blend_key: d.fs.as_ref().map_or(0, |fs| fs.blend_key),
    });
    let t = &d.targets[0];
    let Some(l) = backend::gles_surf_int_layout(t.format) else { return 0 };
    if !d.do_clear || t.gpu == 0 {
        return 0;
    }
    let bpp = backend::gles_surf_bytes_per_pixel(t.format) as usize;
    let pitch = ((t.w + 1) & !1) as usize;
    let v = d.clear_rgba.map(f32::to_bits);
    let step = l.bits as usize / 8;
    let mut px = vec![0u8; bpp];
    for ch in 0..l.channels as usize {
        px[ch * step..ch * step + step].copy_from_slice(&v[ch].to_le_bytes()[..step]);
    }
    let base = t.gpu as *mut u8;
    for y in 0..t.h as usize {
        for x in 0..t.w as usize {
            core::ptr::copy_nonoverlapping(px.as_ptr(), base.add((y * pitch + x) * bpp), bpp);
        }
    }
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
    name: c"es3-pvr-fbo-int-rg".as_ptr() as *const c_char,
    native_attrib_formats: 0,
    rt_rgba8888_is_bgra: false,
    mip_level_align: 0,
    compressed_blocks_in_rows: false,
    native_layered_textures: false,
    front_facing_shader_variant: false,
    etc1_decoded_at_upload: false,
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
};

/// One context for the whole file; the tests take turns on it.
static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

fn context() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    ONCE.call_once(|| {
        // SAFETY: both tables are statics; the lock above serialises the tests.
        unsafe {
            osal_set_ops(&OSAL);
            backend::g_backend = &BACKEND;
            #[cfg(feature = "shaderc-online")]
            {
                yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            }
            assert!(!yakogl::gl::context::gl_context_create(8, 8).is_null());
        }
    });
    /* what an earlier test left recorded is settled before this one counts */
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glFinish();
    }
    SEEN.lock().unwrap().clear();
    // SAFETY: plain GL call.
    while unsafe { glGetError() } != GL_NO_ERROR {}
    g
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

/// The twelve R and RG integer formats: (format, name, channels, bits,
/// signed, the four-channel surface their store is drawn as).
const R_RG_INT: [(GLenum, &str, u8, u8, bool, u8); 12] = [
    (GL_R8I, "R8I", 1, 8, true, backend::GLES_SURF_RGBA8I),
    (GL_R8UI, "R8UI", 1, 8, false, backend::GLES_SURF_RGBA8UI),
    (GL_R16I, "R16I", 1, 16, true, backend::GLES_SURF_RGBA16I),
    (GL_R16UI, "R16UI", 1, 16, false, backend::GLES_SURF_RGBA16UI),
    (GL_R32I, "R32I", 1, 32, true, backend::GLES_SURF_RGBA32I),
    (GL_R32UI, "R32UI", 1, 32, false, backend::GLES_SURF_RGBA32UI),
    (GL_RG8I, "RG8I", 2, 8, true, backend::GLES_SURF_RGBA8I),
    (GL_RG8UI, "RG8UI", 2, 8, false, backend::GLES_SURF_RGBA8UI),
    (GL_RG16I, "RG16I", 2, 16, true, backend::GLES_SURF_RGBA16I),
    (GL_RG16UI, "RG16UI", 2, 16, false, backend::GLES_SURF_RGBA16UI),
    (GL_RG32I, "RG32I", 2, 32, true, backend::GLES_SURF_RGBA32I),
    (GL_RG32UI, "RG32UI", 2, 32, false, backend::GLES_SURF_RGBA32UI),
];

/// A `w` x `w` 2D texture of `f` with `levels` levels.
fn texture_2d(f: GLenum, w: GLsizei, levels: GLsizei) -> GLuint {
    let mut tex = 0;
    // SAFETY: room for one name.
    unsafe {
        glGenTextures(1, &mut tex);
        glBindTexture(GL_TEXTURE_2D, tex);
        glTexStorage2D(GL_TEXTURE_2D, levels, f, w, w);
    }
    assert_eq!(err(), GL_NO_ERROR, "glTexStorage2D {f:#x}");
    tex
}

/// A framebuffer with `tex` level `level` at GL_COLOR_ATTACHMENT0, bound.
fn framebuffer_2d(tex: GLuint, level: GLint) -> GLuint {
    let mut fb = 0;
    // SAFETY: room for one name; tex is a level the framebuffer takes.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, level);
    }
    fb
}

fn release(fb: GLuint, tex: GLuint) {
    // SAFETY: names made by the test.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fb);
        glDeleteTextures(1, &tex);
    }
}

fn status() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glCheckFramebufferStatus(GL_FRAMEBUFFER) }
}

/// dEQP fbo.completeness.renderable.texture.color0.{r,rg}{8,16,32}{i,ui}:
/// complete at attachment 0, at layer 1 of a 2D array and at level 1.
#[test]
fn r_and_rg_integer_textures_are_complete_attachments() {
    let _g = context();
    for (f, name, ..) in R_RG_INT {
        let tex = texture_2d(f, 8, 1);
        let fb = framebuffer_2d(tex, 0);
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "{name}: level 0 of a 2D texture");
        release(fb, tex);

        let tex = texture_2d(f, 8, 3);
        let fb = framebuffer_2d(tex, 1);
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "{name}: level 1 of a mipmapped 2D texture");
        release(fb, tex);

        let (mut tex, mut fb) = (0, 0);
        // SAFETY: room for one name each; layer 1 of 3 is there.
        unsafe {
            glGenTextures(1, &mut tex);
            glBindTexture(GL_TEXTURE_2D_ARRAY, tex);
            glTexStorage3D(GL_TEXTURE_2D_ARRAY, 1, f, 8, 8, 3);
            assert_eq!(glGetError(), GL_NO_ERROR, "{name}: glTexStorage3D");
            glGenFramebuffers(1, &mut fb);
            glBindFramebuffer(GL_FRAMEBUFFER, fb);
            glFramebufferTextureLayer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, tex, 0, 1);
        }
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "{name}: layer 1 of a 2D array");
        release(fb, tex);
    }
}

/// The attachment's size queries are the format's, not the store's: no
/// blue or alpha bits for either, no green for an R format.
#[test]
fn the_attachment_reports_the_format_s_bits() {
    let _g = context();
    for (f, name, channels, bits, ..) in R_RG_INT {
        let tex = texture_2d(f, 8, 1);
        let fb = framebuffer_2d(tex, 0);
        let q = |pname| {
            let mut v = -1;
            // SAFETY: one GLint.
            unsafe { glGetFramebufferAttachmentParameteriv(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, pname, &mut v) };
            v
        };
        let green = if channels == 2 { bits as GLint } else { 0 };
        assert_eq!(
            [q(GL_FRAMEBUFFER_ATTACHMENT_RED_SIZE), q(GL_FRAMEBUFFER_ATTACHMENT_GREEN_SIZE)],
            [bits as GLint, green],
            "{name}"
        );
        assert_eq!([q(GL_FRAMEBUFFER_ATTACHMENT_BLUE_SIZE), q(GL_FRAMEBUFFER_ATTACHMENT_ALPHA_SIZE)], [0, 0], "{name}");
        release(fb, tex);
    }
}

/// glClearBuffer stores four values in the wide store; glReadPixels gives
/// the format's channels and 0, 0, 1 for the rest (ES 3.0 4.3.2).
#[test]
fn read_back_gives_0_0_1_for_the_channels_the_format_lacks() {
    let _g = context();
    for (f, name, channels, _, signed, wide) in R_RG_INT {
        let tex = texture_2d(f, 4, 1);
        let fb = framebuffer_2d(tex, 0);
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "{name}");
        let v: [GLint; 4] = [5, 6, 7, 8];
        let mut px = vec![0u32; 4 * 4 * 4];
        // SAFETY: four values; room for 4x4 pixels of four 32-bit values.
        unsafe {
            if signed {
                glClearBufferiv(GL_COLOR, 0, v.as_ptr());
            } else {
                glClearBufferuiv(GL_COLOR, 0, v.as_ptr() as *const GLuint);
            }
            glReadPixels(
                0,
                0,
                4,
                4,
                GL_RGBA_INTEGER,
                if signed { GL_INT } else { GL_UNSIGNED_INT },
                px.as_mut_ptr() as *mut c_void,
            );
        }
        assert_eq!(err(), GL_NO_ERROR, "{name}");
        let clear = SEEN.lock().unwrap().iter().rev().find(|s| !s.draws).cloned().expect("the clear reached");
        assert_eq!(clear.targets[0].3, wide, "{name}: the clear writes the wide surface");
        let want = if channels == 1 { [5, 0, 0, 1] } else { [5, 6, 0, 1] };
        for (n, p) in px.chunks(4).enumerate() {
            assert_eq!(p, want, "{name}: pixel {n}");
        }
        release(fb, tex);
    }
}

#[cfg(feature = "shaderc-online")]
mod draw {
    use super::*;
    use std::ffi::CString;
    use yakogl::gl::buffer::glBindBuffer;
    use yakogl::gl::draw::*;
    use yakogl::gl::program::*;
    use yakogl::gl::shader::*;
    use yakogl::gl::vertex::{glEnableVertexAttribArray, glVertexAttribPointer};
    use yakogl::shaderc::{shc_blend_key_int_rt, ShcIntRt};

    const VS: &str = "#version 300 es\nin vec4 p;\nvoid main() { gl_Position = p; }\n";

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
            assert_eq!(ok, 1, "{text:?} did not compile");
            s
        }
    }

    /// A linked, current program of `VS` and `fs`.
    fn program(fs: &str) -> GLuint {
        let (v, f) = (shader(GL_VERTEX_SHADER, VS), shader(GL_FRAGMENT_SHADER, fs));
        // SAFETY: plain GL calls.
        unsafe {
            let p = glCreateProgram();
            glAttachShader(p, v);
            glAttachShader(p, f);
            glLinkProgram(p);
            let mut ok: GLint = 0;
            glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
            assert_eq!(ok, 1);
            glUseProgram(p);
            p
        }
    }

    fn draw_triangle() {
        static TRI: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 3.0, -1.0, 0.0, 1.0, -1.0, 3.0, 0.0, 1.0];
        // SAFETY: TRI is static and outlives the draw.
        unsafe {
            glBindBuffer(GL_ARRAY_BUFFER, 0);
            glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, TRI.as_ptr() as *const c_void);
            glEnableVertexAttribArray(0);
            glDrawArrays(GL_TRIANGLES, 0, 3);
            glFinish();
        }
    }

    /// The draw dEQP fbo.color / fbo.render makes into each of the twelve:
    /// one target of the wide surface at the texture's address, the write
    /// mask of the format's channels, and a program packed for the wide
    /// layout with no mask in its key. A glColorMask on a channel the format
    /// lacks changes neither; one on its own channels still reaches the key.
    #[test]
    fn a_draw_writes_the_wide_surface_with_the_format_s_channels() {
        let _g = context();
        for (f, name, channels, bits, signed, wide) in R_RG_INT {
            let tex = texture_2d(f, 8, 1);
            let fb = framebuffer_2d(tex, 0);
            assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "{name}");
            let (ty, val) = if signed { ("ivec4", "ivec4(1, -2, 3, -4)") } else { ("uvec4", "uvec4(1u, 2u, 3u, 4u)") };
            let p = program(&format!(
                "#version 300 es\nprecision highp int;\nlayout(location = 0) out highp {ty} o;\n\
                 void main() {{ o = {val}; }}\n"
            ));
            SEEN.lock().unwrap().clear();
            draw_triangle();
            assert_eq!(err(), GL_NO_ERROR, "{name}: the draw");
            let d = SEEN.lock().unwrap().iter().rev().find(|s| s.draws).cloned().expect("the draw reached");
            assert_eq!(d.targets.len(), 1, "{name}");
            assert_eq!((d.targets[0].1, d.targets[0].2, d.targets[0].3), (8, 8, wide), "{name}: the wide surface");
            assert_ne!(d.targets[0].0, 0, "{name}: the texture's store");
            let own = (1u8 << channels) - 1;
            assert_eq!(d.color_mask, own, "{name}: the format's channels");
            let key = shc_blend_key_int_rt(ShcIntRt { channels: 4, bits, signed });
            assert_eq!(d.blend_key, key, "{name}: packed for the wide layout, no mask");

            /* blue and alpha masked: the format has neither */
            // SAFETY: plain GL calls.
            unsafe { glColorMask(1, 1, 0, 0) };
            SEEN.lock().unwrap().clear();
            draw_triangle();
            let d = SEEN.lock().unwrap().iter().rev().find(|s| s.draws).cloned().expect("the masked draw reached");
            assert_eq!((d.color_mask, d.blend_key), (own, key), "{name}: a mask of channels it lacks");
            // SAFETY: plain GL calls.
            unsafe {
                glColorMask(1, 1, 1, 1);
                glUseProgram(0);
                glDeleteProgram(p);
            }
            release(fb, tex);
        }
    }
}
