// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000UL (i.MX8MP): glReadPixels of a narrow colour attachment, R8, RG8,
//! RGB8 and RGB565, which the GL core keeps as RGBA8888 texels
//! (`renderbuffer_surf_format`, texture.rs `sized_format`). ES 3.0 4.3.2
//! converts the read buffer's components to RGBA, a component the format
//! lacks reading 0 (alpha: 1), whatever the RGBA8 store holds in its byte.
//!
//! The store's spare bytes are not 0 / 255 on the board: a plain-colour draw
//! or clear writes G and B into an R8 buffer (the PE masks only alpha), and a
//! renderbuffer is zero-filled at storage and no draw into an alpha-less
//! target writes its alpha byte. Before the fix the unsigned-byte path of
//! `read_pixels` (src/gl/framebuffer.rs) copied those bytes out raw, while
//! the float path already forced the absent channels. dEQP-GLES3 cases this
//! read pins the front-end half of (imx8mp-gles3-causes.tsv,
//! `gc-narrow-target`): fbo.render.shared_colorbuffer{,_clear}.rbo_{r8,rg8}*,
//! fbo.render.recreate_color.rbo_{r8,rg8,rgb8,rgb565}_*,
//! fbo.color.blend.{r8,rg8,rgb8,rgb565}_src_over.
//!
//! The test writes a known pattern into the attachment's store through
//! `gl_fb_target` (standing in for what the PE left there) and reads it back:
//! the channels the format has must equal those an RGBA8 attachment with the
//! same bytes reads (the B,G,R,A order of a rendered GC target is the back
//! end's, not pinned here), the others 0, 0, 255.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test es3_narrow_fb_gc
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online", not(feature = "report-es2")))]

use core::ffi::c_void;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Mutex;
use yakogl::gl::consts::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::state::*;
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::vivante::kick::{GcKick, GC_KICK_MAGIC};

/* ---- the stand-in OSAL: host memory, kicks accepted and completed at once ---------- */

static BUMP: Mutex<BTreeMap<u64, u64>> = Mutex::new(BTreeMap::new());
static FENCE: AtomicI64 = AtomicI64::new(0);
/// Serialises the tests: the OSAL pointer, the back end and `g_backend` are process-global.
static SERIAL: Mutex<()> = Mutex::new(());

unsafe extern "C" fn p_gpu_alloc(heap_base: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
    if size == 0 || size > 1 << 30 {
        return -12;
    }
    let mem = Box::leak(vec![0u64; size.div_ceil(8)].into_boxed_slice());
    let mut b = BUMP.lock().unwrap_or_else(|e| e.into_inner());
    let at = b.entry(heap_base).or_insert(0);
    let gpu = heap_base + 0x1000 + *at;
    *at += (size as u64).div_ceil(4096) * 4096;
    let cpu = mem.as_mut_ptr() as *mut c_void;
    // SAFETY: the caller passes a writable descriptor.
    unsafe { *out = OsalGpuMem { cpu, gpu, phys: gpu, size, impl_: cpu } };
    0
}
unsafe extern "C" fn p_gpu_free(mem: *mut OsalGpuMem) -> i32 {
    // SAFETY: the caller passes a live descriptor; the memory is leaked on purpose.
    unsafe { *mem = OsalGpuMem::ZERO };
    0
}
unsafe extern "C" fn p_cache(_cpu: *mut c_void, _n: usize) {}
unsafe extern "C" fn p_submit(kick: *const c_void, len: usize) -> i64 {
    if len != core::mem::size_of::<GcKick>() {
        return -22;
    }
    // SAFETY: the back end passes a live GcKick of that size.
    let k = unsafe { *(kick as *const GcKick) };
    if k.magic != GC_KICK_MAGIC {
        return -22;
    }
    FENCE.fetch_add(1, Ordering::SeqCst) + 1
}
unsafe extern "C" fn p_wait(_fence: i64, _timeout_us: i64) -> i32 {
    0
}
unsafe extern "C" fn p_completed() -> i64 {
    FENCE.load(Ordering::SeqCst)
}
unsafe extern "C" fn p_query(what: u32, out: *mut c_void, out_sz: usize) -> i32 {
    use yakogl::osal::{OSAL_QUERY_DRAW_STATS, OSAL_QUERY_GC_FRAME_OPEN};
    if (what == OSAL_QUERY_GC_FRAME_OPEN && out_sz == 4 && !out.is_null()) || what == OSAL_QUERY_DRAW_STATS {
        return 0;
    }
    -22
}

static OSAL: OsalOps = OsalOps {
    init: None,
    shutdown: None,
    gpu_alloc: Some(p_gpu_alloc),
    gpu_free: Some(p_gpu_free),
    mmu_map: None,
    map_phys: None,
    gpu_write: None,
    gpu_read: None,
    cache_flush: Some(p_cache),
    cache_invd: Some(p_cache),
    submit: Some(p_submit),
    wait: Some(p_wait),
    completed: Some(p_completed),
    reg_read: None,
    reg_write: None,
    query: Some(p_query),
    log: None,
    wait_vblank: None,
    present: None,
    lock: None,
    unlock: None,
    thread_id: None,
    display_busy: None,
};

/// Run `case` with the Vivante back end up on the stand-in OSAL and a fresh 64x64 context.
fn on_gc7000(case: impl FnOnce()) {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    // SAFETY: the guard serialises the tests; the tables are statics.
    let ctx = unsafe {
        osal_set_ops(&OSAL);
        yakogl::backend::g_backend = &yakogl::vivante::backend::yakogl_backend_vivante;
        yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
        assert_eq!(yakogl::vivante::backend::gc_init(), 0, "gc_init");
        let c = yakogl::gl::context::gl_context_create(64, 64);
        assert!(!c.is_null(), "context pool exhausted");
        while glGetError() != GL_NO_ERROR {}
        c
    };
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(case));
    // SAFETY: the context made above.
    unsafe {
        yakogl::gl::context::gl_context_destroy(ctx);
        yakogl::vivante::backend::gc_shutdown();
    }
    if let Err(e) = r {
        std::panic::resume_unwind(e);
    }
}

/* ---- GL plumbing -------------------------------------------------------------------- */

const W: u32 = 8;
const H: u32 = 4;
/// What the PE left in every texel of the store: every byte neither 0 nor 255.
const PATTERN: [u8; 4] = [0x40, 0x80, 0xc0, 0x20];

#[derive(Clone, Copy, Debug)]
enum Image {
    Renderbuffer(GLenum),
    /// glTexImage2D's (internalformat, format, type).
    Texture(GLenum, GLenum, GLenum),
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

/// A framebuffer with a fresh `W`x`H` image of `img` as its colour
/// attachment, bound; `fill` writes `PATTERN` into the whole store first.
fn attach(img: Image, fill: bool) -> (GLuint, GLuint) {
    let mut fb = 0;
    let mut name = 0;
    // SAFETY: plain GL calls on the current context; one name each.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        match img {
            Image::Renderbuffer(f) => {
                glGenRenderbuffers(1, &mut name);
                glBindRenderbuffer(GL_RENDERBUFFER, name);
                glRenderbufferStorage(GL_RENDERBUFFER, f, W as GLsizei, H as GLsizei);
                glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, name);
            }
            Image::Texture(i, f, t) => {
                glGenTextures(1, &mut name);
                glBindTexture(GL_TEXTURE_2D, name);
                glTexImage2D(GL_TEXTURE_2D, 0, i as GLint, W as GLsizei, H as GLsizei, 0, f, t, core::ptr::null());
                glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, name, 0);
            }
        }
        assert_eq!(err(), GL_NO_ERROR, "{img:?} attached");
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "{img:?} complete");
        if fill {
            let (mut w, mut h, mut off) = (0u32, 0u32, 0u32);
            let m = gl_fb_target(&mut w, &mut h, &mut off);
            assert!(!m.is_null(), "{img:?} has a target");
            assert_eq!((w, h), (W, H), "{img:?} target size");
            let m = *m;
            assert!(!m.cpu.is_null(), "{img:?} store is CPU-mapped");
            /* RGBA8888 texels from the image's offset to the end of its store */
            let store = core::slice::from_raw_parts_mut((m.cpu as *mut u8).add(off as usize), m.size - off as usize);
            for px in store.as_chunks_mut::<4>().0 {
                px.copy_from_slice(&PATTERN);
            }
        }
    }
    (fb, name)
}

fn release(img: Image, (fb, name): (GLuint, GLuint)) {
    // SAFETY: plain GL calls; the names made by `attach`.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fb);
        match img {
            Image::Renderbuffer(_) => glDeleteRenderbuffers(1, &name),
            Image::Texture(..) => glDeleteTextures(1, &name),
        }
    }
}

/// glReadPixels RGBA/UNSIGNED_BYTE of the whole bound framebuffer.
fn read() -> Vec<[u8; 4]> {
    let mut px = vec![[0x5au8; 4]; (W * H) as usize];
    // SAFETY: `px` holds W x H RGBA8 pixels (pack alignment 4).
    unsafe {
        glReadPixels(0, 0, W as GLsizei, H as GLsizei, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr() as *mut c_void)
    };
    assert_eq!(err(), GL_NO_ERROR, "glReadPixels");
    px
}

/// `img` filled with `PATTERN`, read back.
fn read_filled(img: Image) -> Vec<[u8; 4]> {
    let names = attach(img, true);
    let px = read();
    release(img, names);
    px
}

/// The pixel an image of `img` with channels `has` (R, G, B, A) must read
/// when its store holds `PATTERN`: `full`'s channels where it has them, 0
/// (alpha 255) where not.
fn expect(full: [u8; 4], has: [bool; 4]) -> [u8; 4] {
    core::array::from_fn(|ch| {
        if has[ch] {
            full[ch]
        } else if ch == 3 {
            255
        } else {
            0
        }
    })
}

const R: [bool; 4] = [true, false, false, false];
const RG: [bool; 4] = [true, true, false, false];
const RGB: [bool; 4] = [true, true, true, false];

#[test]
fn narrow_renderbuffers_read_their_absent_channels_as_0_0_1() {
    on_gc7000(|| {
        /* the control: an RGBA8 renderbuffer hands every byte back */
        let full = read_filled(Image::Renderbuffer(GL_RGBA8));
        assert!(full.iter().all(|&p| p == full[0]), "uniform RGBA8 read {:02x?}", full[0]);
        let mut sorted = full[0];
        sorted.sort_unstable();
        let mut pattern = PATTERN;
        pattern.sort_unstable();
        assert_eq!(sorted, pattern, "RGBA8 reads the stored bytes, some order");

        for (f, has, name) in
            [(GL_R8, R, "R8"), (GL_RG8, RG, "RG8"), (GL_RGB8, RGB, "RGB8"), (GL_RGB565, RGB, "RGB565")]
        {
            let px = read_filled(Image::Renderbuffer(f));
            if f == GL_RGB565 && px[0][..3] != full[0][..3] {
                /* a back end that keeps RGB565 in its own 16-bit surface:
                 * only the alpha is this test's */
                assert!(px.iter().all(|p| p[3] == 255), "{name}: alpha reads 255, got {:02x?}", px[0]);
                continue;
            }
            let want = expect(full[0], has);
            for (i, p) in px.iter().enumerate() {
                assert_eq!(*p, want, "{name} renderbuffer pixel {i}: ES 3.0 4.3.2 absent channels read 0, 0, 1");
            }
        }
    });
}

#[test]
fn narrow_textures_read_their_absent_channels_as_0_0_1() {
    on_gc7000(|| {
        let full = read_filled(Image::Texture(GL_RGBA8, GL_RGBA, GL_UNSIGNED_BYTE));
        assert!(full.iter().all(|&p| p == full[0]), "uniform RGBA8 read {:02x?}", full[0]);
        for (img, has, name) in [
            (Image::Texture(GL_R8, GL_RED, GL_UNSIGNED_BYTE), R, "R8"),
            (Image::Texture(GL_RG8, GL_RG, GL_UNSIGNED_BYTE), RG, "RG8"),
            (Image::Texture(GL_RGB8, GL_RGB, GL_UNSIGNED_BYTE), RGB, "RGB8"),
        ] {
            let want = expect(full[0], has);
            for (i, p) in read_filled(img).iter().enumerate() {
                assert_eq!(*p, want, "{name} texture pixel {i}: ES 3.0 4.3.2 absent channels read 0, 0, 1");
            }
        }
    });
}

/// fbo.render.recreate_color.rbo_*: a renderbuffer made again under its name
/// is zero-filled, and a draw into an alpha-less target leaves its alpha
/// byte alone; its alpha still reads 255.
#[test]
fn a_fresh_alpha_less_renderbuffer_reads_alpha_255() {
    on_gc7000(|| {
        for f in [GL_R8, GL_RG8, GL_RGB8, GL_RGB565] {
            let img = Image::Renderbuffer(f);
            let names = attach(img, false);
            let px = read();
            release(img, names);
            assert!(
                px.iter().all(|p| *p == [0, 0, 0, 255]),
                "{f:#x}: a zero store reads 0, 0, 0, 255, got {:02x?}",
                px[0]
            );
        }
    });
}

/// The formats that have every channel keep the store's alpha byte.
#[test]
fn rgba_attachments_keep_their_alpha() {
    on_gc7000(|| {
        for img in [Image::Renderbuffer(GL_RGBA8), Image::Texture(GL_RGBA8, GL_RGBA, GL_UNSIGNED_BYTE)] {
            let px = read_filled(img);
            assert!(
                px.iter().all(|p| p[3] == PATTERN[3]),
                "{img:?}: alpha byte {:02x} kept, got {:02x?}",
                PATTERN[3],
                px[0]
            );
        }
    });
}
