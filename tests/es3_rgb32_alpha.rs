// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GL_RGB32F, GL_RGB32I and GL_RGB32UI textures keep alpha 1 in the 128-bit
//! class (item gc-w128-rgb32-alpha of plan #366). On the GC7000UL an RGBA32
//! texture is sampled as two G32R32 planes the shader joins as
//! (a.x, a.y, b.x, b.y): alpha is plane 1's .y, which a descriptor's A=ONE
//! (`texture_has_no_alpha`) cannot reach, so a three-component 32-bit format
//! is right only if the core stores alpha as 1 (1.0f bits, or integer 1;
//! ES 3.0 3.8: a missing alpha is one). docs/notes/gc-128bit-swizzle.md 3, 5.1.
//!
//! Every upload form is checked by reading the stored level texels back:
//! glTexImage2D, glTexStorage2D + glTexSubImage2D, glTexImage3D and
//! glTexSubImage3D on a 2D array, and glTexImage2D from a pixel unpack
//! buffer. GL_RGBA32UI/I must keep the alpha they were given, and so must
//! GL_RGBA32F, which every back end stores as RGBA16F today (`sized_format`;
//! pinned, the GC flip may move it). GL_RGB32F is stored in the 128-bit class
//! where the mask has it (gpu-powervr, gpu-v3d) and as RGBA16F where it has
//! not (gpu-vivante until the flip): the alpha is read in the class the core
//! chose, a half widened to its float, and must be 1.0 either way. The
//! integer formats are 128-bit classes; where the mask has none (gpu-vivante
//! until the flip) they are refused by name and the rows skip, with the
//! refusal pinned.
//!
//! Feature sets (a GPU is needed for the mask):
//!   cargo test --test es3_rgb32_alpha --features host-test,gpu-powervr
//!   cargo test --test es3_rgb32_alpha --features host-test,gpu-vivante
//!   cargo test --test es3_rgb32_alpha --features host-test,gpu-v3d,shaderc-online,no-shader-table
//! Under `report-es2` the formats do not exist and the file is empty.

#![cfg(all(feature = "host-test", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::buffer::*;
use yakogl::gl::consts::*;
use yakogl::gl::context;
use yakogl::gl::es3_formats::{self, half_to_f32};
use yakogl::gl::state::glGetError;
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};

/* ---- a context on the host: the smallest OSAL and backend that make one --- */

unsafe extern "C" fn m_gpu_alloc(_heap: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
    let mem = Box::leak(vec![0u8; size].into_boxed_slice());
    let p = mem.as_mut_ptr();
    *out = OsalGpuMem { cpu: p as *mut c_void, gpu: p as u64, phys: p as u64, size, impl_: p as *mut c_void };
    0
}
unsafe extern "C" fn m_gpu_free(_mem: *mut OsalGpuMem) -> i32 {
    0
}

/// Every range the GL wrote back from the CPU cache (address, bytes).
static FLUSHES: Mutex<Vec<(usize, usize)>> = Mutex::new(Vec::new());

unsafe extern "C" fn m_flush(cpu: *mut c_void, n: usize) {
    FLUSHES.lock().unwrap().push((cpu as usize, n));
}
unsafe extern "C" fn m_invd(_cpu: *mut c_void, _n: usize) {}
unsafe extern "C" fn b_init() -> i32 {
    0
}
unsafe extern "C" fn b_heap_base(_heap: GlesHeap) -> u64 {
    0x1000_0000
}
unsafe extern "C" fn b_ok() -> i32 {
    0
}
unsafe extern "C" fn b_draw(_d: *const GlesDraw) -> i32 {
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
    cache_flush: Some(m_flush),
    cache_invd: Some(m_invd),
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
    name: c"es3-rgb32-alpha".as_ptr() as *const c_char,
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
            assert!(!yakogl::gl::context::gl_context_create(8, 8).is_null());
        }
    });
    // SAFETY: plain GL calls.
    unsafe { while glGetError() != GL_NO_ERROR {} }
    g
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

/* ---- reading the stored texels ------------------------------------------- */

const W: usize = 4;
const H: usize = 2;
const LAYERS: usize = 2;

/// What the core stored: the class and the bytes of every texel.
struct Stored {
    texfmt: u8,
    texels: Vec<Vec<u8>>,
}

/// Level 0 of texture `name`: `layers` stacked images of W x H.
fn read_level0(name: GLuint, layers: usize) -> Stored {
    let c = context::current().expect("context");
    let t = &c.objs().textures[name as usize];
    assert!(!t.mem.cpu.is_null(), "texture storage has a CPU view");
    let (bpp, pitch) = (t.bpp as usize, t.level_pitch(W as u32) as usize);
    let base = t.level_off[0] as usize;
    let mut texels = Vec::new();
    for l in 0..layers {
        for y in 0..H {
            for x in 0..W {
                let off = base + (((l * H + y) * pitch) + x) * bpp;
                // SAFETY: the texel lies inside the level the core laid out.
                let s = unsafe { core::slice::from_raw_parts((t.mem.cpu as *const u8).add(off), bpp) };
                texels.push(s.to_vec());
            }
        }
    }
    Stored { texfmt: t.texfmt, texels }
}

fn new_tex(target: GLenum) -> GLuint {
    let mut name = 0;
    // SAFETY: room for one name.
    unsafe {
        glGenTextures(1, &mut name);
        glBindTexture(target, name);
    }
    name
}

/// The alpha word of a stored texel as a 32-bit pattern (a half of the
/// RGBA16F fall-back is widened to its float).
fn alpha_of(texfmt: u8, texel: &[u8]) -> u32 {
    match texfmt {
        backend::GLES_TEXFMT_RGBA32F | backend::GLES_TEXFMT_RGBA32UI | backend::GLES_TEXFMT_RGBA32I => {
            assert_eq!(texel.len(), 16);
            u32::from_ne_bytes(texel[12..16].try_into().unwrap())
        }
        backend::GLES_TEXFMT_RGBA16F => {
            assert_eq!(texel.len(), 8);
            half_to_f32(u16::from_ne_bytes(texel[6..8].try_into().unwrap())).to_bits()
        }
        t => panic!("class {t} is not one of the RGBA32 formats' storage"),
    }
}

/// The colour words of a stored texel as 32-bit patterns (halves widened).
fn rgb_of(texfmt: u8, texel: &[u8]) -> [u32; 3] {
    let mut o = [0u32; 3];
    for (k, v) in o.iter_mut().enumerate() {
        *v = if texfmt == backend::GLES_TEXFMT_RGBA16F {
            half_to_f32(u16::from_ne_bytes(texel[k * 2..k * 2 + 2].try_into().unwrap())).to_bits()
        } else {
            u32::from_ne_bytes(texel[k * 4..k * 4 + 4].try_into().unwrap())
        };
    }
    o
}

#[derive(Clone, Copy)]
struct Fmt {
    name: &'static str,
    internal: GLenum,
    rgba: GLenum,
    /// The upload format of the three- and four-component source.
    base3: GLenum,
    base4: GLenum,
    type_: GLenum,
    /// The alpha a missing one becomes: 1.0f bits or integer 1.
    one: u32,
}

const FMTS: [Fmt; 3] = [
    Fmt {
        name: "GL_RGB32F",
        internal: GL_RGB32F,
        rgba: GL_RGBA32F,
        base3: GL_RGB,
        base4: GL_RGBA,
        type_: GL_FLOAT,
        one: 0x3f80_0000,
    },
    Fmt {
        name: "GL_RGB32I",
        internal: GL_RGB32I,
        rgba: GL_RGBA32I,
        base3: GL_RGB_INTEGER,
        base4: GL_RGBA_INTEGER,
        type_: GL_INT,
        one: 1,
    },
    Fmt {
        name: "GL_RGB32UI",
        internal: GL_RGB32UI,
        rgba: GL_RGBA32UI,
        base3: GL_RGB_INTEGER,
        base4: GL_RGBA_INTEGER,
        type_: GL_UNSIGNED_INT,
        one: 1,
    },
];

fn class_of(f: &Fmt) -> u8 {
    match f.rgba {
        GL_RGBA32I => backend::GLES_TEXFMT_RGBA32I,
        GL_RGBA32UI => backend::GLES_TEXFMT_RGBA32UI,
        _ => backend::GLES_TEXFMT_RGBA32F,
    }
}

/// `n` source texels of `comps` components: distinct non-zero colour, alpha
/// 7 / 7.0 (not the 1 expected of a missing one). Returns the bytes, the
/// colour words and the alpha word.
fn source(f: &Fmt, comps: usize, n: usize) -> (Vec<u8>, Vec<[u32; 3]>, u32) {
    let float = f.type_ == GL_FLOAT;
    let val = |v: u32| if float { (v as f32).to_bits() } else { v };
    let alpha = val(7);
    let mut bytes = Vec::new();
    let mut rgb = Vec::new();
    for i in 0..n as u32 {
        let c = [val(10 + i), val(40 + i), val(70 + i)];
        for w in c.iter().copied().chain((comps == 4).then_some(alpha)) {
            bytes.extend_from_slice(&w.to_ne_bytes());
        }
        rgb.push(c);
    }
    (bytes, rgb, alpha)
}

/// The stored level of a three-component format holds the colour and alpha 1
/// (the format's `one`, widened from the half of the RGBA16F fall-back).
fn check_rgb(f: &Fmt, s: &Stored, rgb: &[[u32; 3]], what: &str) {
    assert_eq!(s.texels.len(), rgb.len(), "{} {what}", f.name);
    if s.texfmt == backend::GLES_TEXFMT_RGBA16F {
        assert_eq!(f.internal, GL_RGB32F, "{} {what}: only the float format falls back to halves", f.name);
    }
    let want_a = f.one;
    for (i, t) in s.texels.iter().enumerate() {
        assert_eq!(alpha_of(s.texfmt, t), want_a, "{} {what}: texel {i} alpha", f.name);
        assert_eq!(rgb_of(s.texfmt, t), rgb[i], "{} {what}: texel {i} colour", f.name);
    }
}

/// An RGBA32* format stores the alpha it was given. GL_RGBA32F is the one
/// sized name of the original tables that no back end stores in its 128-bit
/// class (`sized_format` holds it as RGBA16F: pinned here, and what the GC
/// flip has to change if it wants the class); the integer ones are 128-bit.
fn check_rgba(f: &Fmt, s: &Stored, rgb: &[[u32; 3]], alpha: u32, what: &str) {
    let want = if f.rgba == GL_RGBA32F { backend::GLES_TEXFMT_RGBA16F } else { class_of(f) };
    assert_eq!(s.texfmt, want, "{} {what}: storage class", f.rgba);
    assert_eq!(s.texels.len(), rgb.len(), "{} {what}", f.name);
    for (i, t) in s.texels.iter().enumerate() {
        assert_eq!(alpha_of(s.texfmt, t), alpha, "{} {what}: texel {i} alpha", f.name);
        assert_eq!(rgb_of(s.texfmt, t), rgb[i], "{} {what}: texel {i} colour", f.name);
    }
}

fn sub_image_2d(f: &Fmt, internal: GLenum, base: GLenum, pixels: *const c_void) {
    // SAFETY: `pixels` is W x H texels of `base` or a PBO offset.
    unsafe {
        glTexStorage2D(GL_TEXTURE_2D, 1, internal, W as i32, H as i32);
        glTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, W as i32, H as i32, base, f.type_, pixels);
    }
}

fn image_2d(f: &Fmt, internal: GLenum, base: GLenum, pixels: *const c_void) {
    // SAFETY: `pixels` is W x H texels of `base` or a PBO offset.
    unsafe { glTexImage2D(GL_TEXTURE_2D, 0, internal as GLint, W as i32, H as i32, 0, base, f.type_, pixels) };
}

fn image_3d(f: &Fmt, internal: GLenum, base: GLenum, pixels: &[u8]) {
    // SAFETY: `pixels` is W x H x LAYERS texels of `base`.
    unsafe {
        glTexImage3D(
            GL_TEXTURE_2D_ARRAY,
            0,
            internal as GLint,
            W as i32,
            H as i32,
            LAYERS as i32,
            0,
            base,
            f.type_,
            pixels.as_ptr().cast(),
        );
    }
}

fn sub_image_3d(f: &Fmt, internal: GLenum, base: GLenum, pixels: &[u8]) {
    // SAFETY: as above.
    unsafe {
        glTexStorage3D(GL_TEXTURE_2D_ARRAY, 1, internal, W as i32, H as i32, LAYERS as i32);
        glTexSubImage3D(
            GL_TEXTURE_2D_ARRAY,
            0,
            0,
            0,
            0,
            W as i32,
            H as i32,
            LAYERS as i32,
            base,
            f.type_,
            pixels.as_ptr().cast(),
        );
    }
}

/// An unpack buffer holding `bytes`, left bound.
fn unpack_buffer(bytes: &[u8]) {
    let mut buf = 0;
    // SAFETY: room for one name; `bytes` is the buffer's whole content.
    unsafe {
        glGenBuffers(1, &mut buf);
        glBindBuffer(GL_PIXEL_UNPACK_BUFFER, buf);
        glBufferData(GL_PIXEL_UNPACK_BUFFER, bytes.len() as isize, bytes.as_ptr().cast(), GL_STATIC_DRAW);
    }
}

fn unbind_unpack_buffer() {
    // SAFETY: plain GL call.
    unsafe { glBindBuffer(GL_PIXEL_UNPACK_BUFFER, 0) };
}

/// Does this back end's mask hold a class for `internal`? The GC7000UL's
/// has none of the 128-bit integer classes before the flip: its RGB32I/UI and
/// RGBA32I/UI are refused by name (GL_INVALID_OPERATION), which
/// `a_format_without_a_class_is_refused_by_name` pins; the rows then skip.
/// GL_RGBA32F is the original tables' (halves everywhere): no class to miss.
fn stored_here(internal: GLenum) -> bool {
    es3_formats::find(internal).is_none_or(|(_, e)| e.storage_here().is_some())
}

/* ---- the tests ------------------------------------------------------------- */

#[test]
fn rgb32_tex_image_2d_stores_alpha_one() {
    let _g = context();
    for f in FMTS {
        if !stored_here(f.internal) {
            continue;
        }
        let (bytes, rgb, _) = source(&f, 3, W * H);
        let name = new_tex(GL_TEXTURE_2D);
        image_2d(&f, f.internal, f.base3, bytes.as_ptr().cast());
        assert_eq!(err(), GL_NO_ERROR, "{}", f.name);
        check_rgb(&f, &read_level0(name, 1), &rgb, "glTexImage2D");
    }
}

#[test]
fn rgb32_tex_storage_then_sub_image_2d_stores_alpha_one() {
    let _g = context();
    for f in FMTS {
        if !stored_here(f.internal) {
            continue;
        }
        let (bytes, rgb, _) = source(&f, 3, W * H);
        let name = new_tex(GL_TEXTURE_2D);
        sub_image_2d(&f, f.internal, f.base3, bytes.as_ptr().cast());
        assert_eq!(err(), GL_NO_ERROR, "{}", f.name);
        check_rgb(&f, &read_level0(name, 1), &rgb, "glTexStorage2D + glTexSubImage2D");
    }
}

#[test]
fn rgb32_3d_uploads_store_alpha_one() {
    let _g = context();
    for f in FMTS {
        if !stored_here(f.internal) {
            continue;
        }
        let (bytes, rgb, _) = source(&f, 3, W * H * LAYERS);
        let name = new_tex(GL_TEXTURE_2D_ARRAY);
        image_3d(&f, f.internal, f.base3, &bytes);
        assert_eq!(err(), GL_NO_ERROR, "{}", f.name);
        check_rgb(&f, &read_level0(name, LAYERS), &rgb, "glTexImage3D");

        let name = new_tex(GL_TEXTURE_2D_ARRAY);
        sub_image_3d(&f, f.internal, f.base3, &bytes);
        assert_eq!(err(), GL_NO_ERROR, "{}", f.name);
        check_rgb(&f, &read_level0(name, LAYERS), &rgb, "glTexStorage3D + glTexSubImage3D");
    }
}

#[test]
fn rgb32_from_a_pixel_unpack_buffer_stores_alpha_one() {
    let _g = context();
    for f in FMTS {
        if !stored_here(f.internal) {
            continue;
        }
        let (bytes, rgb, _) = source(&f, 3, W * H);
        unpack_buffer(&bytes);
        let name = new_tex(GL_TEXTURE_2D);
        image_2d(&f, f.internal, f.base3, core::ptr::null());
        assert_eq!(err(), GL_NO_ERROR, "{}", f.name);
        check_rgb(&f, &read_level0(name, 1), &rgb, "glTexImage2D from a PBO");

        let name = new_tex(GL_TEXTURE_2D);
        sub_image_2d(&f, f.internal, f.base3, core::ptr::null());
        assert_eq!(err(), GL_NO_ERROR, "{}", f.name);
        check_rgb(&f, &read_level0(name, 1), &rgb, "glTexSubImage2D from a PBO");
        unbind_unpack_buffer();
    }
}

/// An RGBA32* format keeps the alpha it is given, in every upload form.
#[test]
fn rgba32_keeps_the_given_alpha() {
    let _g = context();
    for f in FMTS {
        if !stored_here(f.rgba) {
            continue;
        }
        let (bytes, rgb, alpha) = source(&f, 4, W * H * LAYERS);
        let rgb2 = &rgb[..W * H];

        let name = new_tex(GL_TEXTURE_2D);
        image_2d(&f, f.rgba, f.base4, bytes.as_ptr().cast());
        assert_eq!(err(), GL_NO_ERROR, "{}", f.rgba);
        check_rgba(&f, &read_level0(name, 1), rgb2, alpha, "glTexImage2D");

        let name = new_tex(GL_TEXTURE_2D);
        sub_image_2d(&f, f.rgba, f.base4, bytes.as_ptr().cast());
        assert_eq!(err(), GL_NO_ERROR, "{}", f.rgba);
        check_rgba(&f, &read_level0(name, 1), rgb2, alpha, "glTexStorage2D + glTexSubImage2D");

        let name = new_tex(GL_TEXTURE_2D_ARRAY);
        image_3d(&f, f.rgba, f.base4, &bytes);
        assert_eq!(err(), GL_NO_ERROR, "{}", f.rgba);
        check_rgba(&f, &read_level0(name, LAYERS), &rgb, alpha, "glTexImage3D");

        let name = new_tex(GL_TEXTURE_2D_ARRAY);
        sub_image_3d(&f, f.rgba, f.base4, &bytes);
        assert_eq!(err(), GL_NO_ERROR, "{}", f.rgba);
        check_rgba(&f, &read_level0(name, LAYERS), &rgb, alpha, "glTexStorage3D + glTexSubImage3D");

        unpack_buffer(&bytes);
        let name = new_tex(GL_TEXTURE_2D);
        sub_image_2d(&f, f.rgba, f.base4, core::ptr::null());
        unbind_unpack_buffer();
        assert_eq!(err(), GL_NO_ERROR, "{}", f.rgba);
        check_rgba(&f, &read_level0(name, 1), rgb2, alpha, "glTexSubImage2D from a PBO");
    }
}

/// A format whose 128-bit class the back end does not sample is refused by
/// name, never stored in a class that loses it: GL_INVALID_OPERATION from
/// glTexImage2D and glTexStorage2D, and no error from the formats it has.
#[test]
fn a_format_without_a_class_is_refused_by_name() {
    let _g = context();
    for f in FMTS {
        for internal in [f.internal, f.rgba] {
            let (bytes, _, _) = source(&f, if internal == f.internal { 3 } else { 4 }, W * H);
            let base = if internal == f.internal { f.base3 } else { f.base4 };
            new_tex(GL_TEXTURE_2D);
            image_2d(&f, internal, base, bytes.as_ptr().cast());
            let want = if stored_here(internal) { GL_NO_ERROR } else { GL_INVALID_OPERATION };
            assert_eq!(err(), want, "glTexImage2D 0x{internal:x}");
            new_tex(GL_TEXTURE_2D);
            // SAFETY: plain GL call.
            unsafe { glTexStorage2D(GL_TEXTURE_2D, 1, internal, W as i32, H as i32) };
            assert_eq!(err(), want, "glTexStorage2D 0x{internal:x}");
        }
    }
}
