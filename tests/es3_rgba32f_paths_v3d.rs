// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GL_RGBA32F on the V3D path reached through the specification entry points
//! tests/es3_rgba32f_v3d.rs does not cover (plan #380, issue #1261; images T05a,
//! T06 and T10-T13 of tools/cts/pi4-gles3-tree.txt). That file pins
//! `glTexImage2D/3D`; the dEQP-GLES3 groups `texture.specification.texstorage*`,
//! `texsubimage*` and `unpack_params` come in through
//!
//! * `glTexStorage2D` (2D and cube) and `glTexStorage3D` (2D array and 3D), the
//!   texels then given by `glTexSubImage2D/3D`;
//! * `glTexSubImage2D/3D` over a `glTexImage2D/3D` whose data is NULL;
//! * `GL_UNPACK_ROW_LENGTH`, `GL_UNPACK_SKIP_PIXELS`, `GL_UNPACK_SKIP_ROWS`,
//!   `GL_UNPACK_SKIP_IMAGES`, `GL_UNPACK_IMAGE_HEIGHT` and `GL_UNPACK_ALIGNMENT`
//!   on those sub-image uploads.
//!
//! Where the class is chosen (src/gl/texture.rs): every one of these paths takes
//! it from `sized_format`, whose `GL_RGBA32F` row answers `GLES_TEXFMT_RGBA32F`
//! only when `rgba32f_own_class()` holds (the GC7000UL, or a back end whose
//! RGBA32F surface is drawable, `framebuffer::ext_float_color_renderable`);
//! otherwise `GLES_TEXFMT_RGBA16F`, the half-float class. `glTexStorage2D` and
//! `glTexStorage3D` call `sized_format` and record the class in the
//! texture; `glTexSubImage2D/3D` do not choose one: they encode into the class the
//! specification made (`t.texfmt`). So the sub-image and unpack paths follow
//! whichever class the specification path picked, and a fix to `sized_format`
//! moves all of them together.
//!
//! What the integration tip does today, with this file's stand-in back end
//! (it draws no RGBA32F surface): every path stores GL_RGBA32F as RGBA16F halves.
//! 1.0e-8 is stored 0, 65520.0 as 65504 (0x7bff), 1/3 as 0x3555. The correct
//! value is the 32-bit class `GLES_TEXFMT_RGBA32F`, four 32-bit words a texel
//! holding the uploaded floats' own bits, `texture::return_32` true. The tests
//! named `..._flip_when_<path>_is_32_bit` assert today's halves and flip with
//! the fix. The unpack tests put 7.0 in every texel the parameters skip, so a
//! first texel equal to `VALUES` shows the skip was applied; the controls
//! (GL_RGBA16F) do not flip.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_rgba32f_paths_v3d
//! ```
#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-v3d", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::*;
use yakogl::gl::consts::*;
use yakogl::gl::draw::*;
use yakogl::gl::es3_formats::{f32_to_half, half_to_f32};
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::v3d::texture;

/* ---- a context on the host: the smallest OSAL and a recording back end ---- */

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

/// The first binding of every draw since the last take, with the first texel
/// at its base (read from the memory the OSAL handed the GL core: its GPU
/// address is the host pointer).
struct Seen(GlesTextureBind, Vec<u8>);
// SAFETY: the bind'"'"'s pointers are never dereferenced by the test.
unsafe impl Send for Seen {}
static DRAWS: Mutex<Vec<Seen>> = Mutex::new(Vec::new());
const TEXEL_BYTES: usize = 16;

unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    // SAFETY: the core passes a live job.
    let d = unsafe { &*d };
    let b = d.textures[0];
    // SAFETY: base_gpu is the address m_gpu_alloc gave (a leaked host
    // allocation holding at least the first texel of the level).
    let texels = unsafe { core::slice::from_raw_parts(b.base_gpu as *const u8, TEXEL_BYTES) }.to_vec();
    DRAWS.lock().unwrap_or_else(|e| e.into_inner()).push(Seen(b, texels));
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

/// A recording stand-in. The storage class the GL core picks does not depend
/// on the layout answers; the sampled classes are the V3D's (`gpu-v3d`).
static BACKEND: YakoglBackend = YakoglBackend {
    name: c"es3-rgba32f-paths-v3d".as_ptr() as *const c_char,
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
    max_samples: 0,
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
            g_backend = &BACKEND;
            yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            assert!(!yakogl::gl::context::gl_context_create(16, 16).is_null());
        }
    });
    // SAFETY: plain GL calls on the one context.
    unsafe {
        glUseProgram(0);
        glActiveTexture(GL_TEXTURE0);
        glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
        while glGetError() != GL_NO_ERROR {}
    }
    DRAWS.lock().unwrap_or_else(|e| e.into_inner()).clear();
    g
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

fn shader(ty: GLenum, text: &str) -> GLuint {
    let c = CString::new(text).unwrap();
    // SAFETY: plain GL calls; one NUL-terminated source.
    unsafe {
        let s = glCreateShader(ty);
        let p: [*const c_char; 1] = [c.as_ptr()];
        glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
        glCompileShader(s);
        let mut ok: GLint = 0;
        glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
        assert_eq!(ok, 1, "compile: {text}");
        s
    }
}

const VS: &str =
    "#version 300 es\nin vec4 p;\nout vec3 v;\nvoid main() { v = vec3(p.xy * 0.5 + 0.5, 0.5); gl_Position = p; }\n";

/// dEQP's renderQuad over `sampler` on unit 0: one draw.
fn draw_sampling(sampler: &str) {
    let fs = format!(
        "#version 300 es\nprecision highp float;\nprecision highp {sampler};\nuniform {sampler} s;\n\
         in vec3 v;\nout vec4 o;\nvoid main() {{ o = texture(s, v); }}\n"
    );
    let fs = if sampler == "sampler2D" { fs.replace("texture(s, v)", "texture(s, v.xy)") } else { fs };
    let (v, f) = (shader(GL_VERTEX_SHADER, VS), shader(GL_FRAGMENT_SHADER, &fs));
    const QUAD: [f32; 16] = [-1.0, -1.0, 0.5, 1.0, 1.0, -1.0, 0.5, 1.0, -1.0, 1.0, 0.5, 1.0, 1.0, 1.0, 0.5, 1.0];
    // SAFETY: plain GL calls; QUAD is a static.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glBindAttribLocation(p, 0, c"p".as_ptr());
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "link {sampler}");
        glUseProgram(p);
        glUniform1i(glGetUniformLocation(p, c"s".as_ptr()), 0);
        glViewport(0, 0, 16, 16);
        glEnableVertexAttribArray(0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, QUAD.as_ptr() as *const c_void);
        glDrawArrays(GL_TRIANGLE_STRIP, 0, 4);
        glUseProgram(0);
        glDeleteProgram(p);
    }
    assert_eq!(err(), GL_NO_ERROR, "the quad over {sampler}");
}

/* ------------------------------------------------------------------------ */

/// The values RGBA16F cannot hold exactly: below the half range, above it,
/// a third, and a tenth.
const VALUES: [f32; 4] = [1.0e-8, 65520.0, 1.0 / 3.0, 0.1];

/// What the GL core stores of `VALUES` as halves (65520.0 clamps to 0x7bff).
const STORED: [u16; 4] = [0x0000, 0x7bff, 0x3555, 0x2e66];

/// What the texels the unpack parameters skip hold.
const SKIPPED: [f32; 4] = [7.0; 4];

const SIDE: usize = 4;

#[derive(Clone, Copy, Debug)]
enum Shape {
    D2,
    Array,
    D3,
    Cube,
}

impl Shape {
    fn target(self) -> GLenum {
        match self {
            Shape::D2 => GL_TEXTURE_2D,
            Shape::Array => GL_TEXTURE_2D_ARRAY,
            Shape::D3 => GL_TEXTURE_3D,
            Shape::Cube => GL_TEXTURE_CUBE_MAP,
        }
    }
    fn sampler(self) -> &'static str {
        match self {
            Shape::D2 => "sampler2D",
            Shape::Array => "sampler2DArray",
            Shape::D3 => "sampler3D",
            Shape::Cube => "samplerCube",
        }
    }
    fn layers(self) -> usize {
        if matches!(self, Shape::Array | Shape::D3) {
            2
        } else {
            1
        }
    }
}

/// How the texture's storage is specified.
#[derive(Clone, Copy, Debug)]
enum Spec {
    /// glTexStorage2D / glTexStorage3D, one level
    Storage,
    /// glTexImage2D / glTexImage3D with NULL data
    NullImage,
}

/// The unpack state of an upload: the source holds `row_length` texels a row,
/// `image_height` rows an image, and the region starts `skip_*` in.
#[derive(Clone, Copy, Debug)]
struct Unpack {
    row_length: usize,
    image_height: usize,
    skip_pixels: usize,
    skip_rows: usize,
    skip_images: usize,
    alignment: GLint,
}

const TIGHT: Unpack =
    Unpack { row_length: SIDE, image_height: SIDE, skip_pixels: 0, skip_rows: 0, skip_images: 0, alignment: 1 };
const SKIPPING: Unpack =
    Unpack { row_length: 6, image_height: 5, skip_pixels: 1, skip_rows: 1, skip_images: 1, alignment: 1 };
/// `SKIPPING` for a 2D upload (no image skip).
const SKIPPING_2D: Unpack = Unpack { skip_images: 0, ..SKIPPING };

fn set_unpack(u: Unpack) {
    // SAFETY: plain GL calls.
    unsafe {
        glPixelStorei(GL_UNPACK_ROW_LENGTH, if u.row_length == SIDE { 0 } else { u.row_length as GLint });
        glPixelStorei(GL_UNPACK_IMAGE_HEIGHT, if u.image_height == SIDE { 0 } else { u.image_height as GLint });
        glPixelStorei(GL_UNPACK_SKIP_PIXELS, u.skip_pixels as GLint);
        glPixelStorei(GL_UNPACK_SKIP_ROWS, u.skip_rows as GLint);
        glPixelStorei(GL_UNPACK_SKIP_IMAGES, u.skip_images as GLint);
        glPixelStorei(GL_UNPACK_ALIGNMENT, u.alignment);
    }
}

/// The client memory of `layers` SIDE x SIDE images of `values` laid out as `u`
/// says, 7.0 in every texel outside the region.
fn source(layers: usize, values: [f32; 4], u: Unpack) -> Vec<f32> {
    let images = u.skip_images + layers;
    let mut v: Vec<f32> = (0..images * u.image_height * u.row_length).flat_map(|_| SKIPPED).collect();
    for l in 0..layers {
        for r in 0..SIDE {
            for c in 0..SIDE {
                let t = ((u.skip_images + l) * u.image_height + u.skip_rows + r) * u.row_length + u.skip_pixels + c;
                v[t * 4..t * 4 + 4].copy_from_slice(&values);
            }
        }
    }
    v
}

/// A SIDE x SIDE (x 2 layers) texture of `internalformat` specified by `spec`
/// (the base level only, NEAREST), then every texel set to `values` by
/// glTexSubImage2D/3D from client memory laid out as `u`; returns its name.
fn make(shape: Shape, internalformat: GLenum, spec: Spec, values: [f32; 4], u: Unpack) -> GLuint {
    let layers = shape.layers();
    let px = source(layers, values, u);
    let half: Vec<u16> = px.iter().map(|v| f32_to_half(*v)).collect();
    let (ty, data) = if internalformat == GL_RGBA16F {
        (GL_HALF_FLOAT, half.as_ptr() as *const c_void)
    } else {
        (GL_FLOAT, px.as_ptr() as *const c_void)
    };
    let (w, h, l, f) = (SIDE as GLsizei, SIDE as GLsizei, layers as GLsizei, internalformat as GLint);
    let null = core::ptr::null();
    let mut name = 0;
    // SAFETY: plain GL calls; `data` holds the region the unpack state names.
    unsafe {
        glGenTextures(1, &mut name);
        glBindTexture(shape.target(), name);
        set_unpack(TIGHT); /* the specification takes no data */
        match (spec, shape) {
            (Spec::Storage, Shape::D2 | Shape::Cube) => glTexStorage2D(shape.target(), 1, internalformat, w, h),
            (Spec::Storage, _) => glTexStorage3D(shape.target(), 1, internalformat, w, h, l),
            (Spec::NullImage, Shape::D2) => glTexImage2D(GL_TEXTURE_2D, 0, f, w, h, 0, GL_RGBA, GL_FLOAT, null),
            (Spec::NullImage, Shape::Array | Shape::D3) => {
                glTexImage3D(shape.target(), 0, f, w, h, l, 0, GL_RGBA, GL_FLOAT, null)
            }
            (Spec::NullImage, Shape::Cube) => {
                for face in 0..6 {
                    glTexImage2D(GL_TEXTURE_CUBE_MAP_POSITIVE_X + face, 0, f, w, h, 0, GL_RGBA, GL_FLOAT, null);
                }
            }
        }
        assert_eq!(err(), GL_NO_ERROR, "{shape:?} {spec:?} specification");
        set_unpack(u);
        match shape {
            Shape::D2 => glTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, w, h, GL_RGBA, ty, data),
            Shape::Array | Shape::D3 => glTexSubImage3D(shape.target(), 0, 0, 0, 0, w, h, l, GL_RGBA, ty, data),
            Shape::Cube => {
                for face in 0..6 {
                    glTexSubImage2D(GL_TEXTURE_CUBE_MAP_POSITIVE_X + face, 0, 0, 0, w, h, GL_RGBA, ty, data);
                }
            }
        }
        set_unpack(TIGHT);
        for p in [GL_TEXTURE_MIN_FILTER, GL_TEXTURE_MAG_FILTER] {
            glTexParameteri(shape.target(), p, GL_NEAREST as GLint);
        }
    }
    assert_eq!(err(), GL_NO_ERROR, "{shape:?} {spec:?} {internalformat:#x} sub-image {u:?}");
    name
}

/// The bind and the first texel's bytes of the one draw of the texture.
fn drawn(shape: Shape, internalformat: GLenum, spec: Spec, values: [f32; 4], u: Unpack) -> (GlesTextureBind, Vec<u8>) {
    let t = make(shape, internalformat, spec, values, u);
    draw_sampling(shape.sampler());
    // SAFETY: a name made above.
    unsafe { glDeleteTextures(1, &t) };
    let d = core::mem::take(&mut *DRAWS.lock().unwrap_or_else(|e| e.into_inner()));
    assert_eq!(d.len(), 1, "one draw of {shape:?}");
    let Seen(b, texel) = d.into_iter().next().unwrap();
    (b, texel)
}

fn halves(texel: &[u8]) -> [u16; 4] {
    core::array::from_fn(|i| u16::from_ne_bytes([texel[i * 2], texel[i * 2 + 1]]))
}

/// Today: the bind is the half-float class at the 16-bit TMU output and the
/// first texel is `VALUES` rounded to halves.
fn assert_halves_today(shape: Shape, spec: Spec, u: Unpack) {
    let (b, texel) = drawn(shape, GL_RGBA32F, spec, VALUES, u);
    let at = format!("{shape:?} {spec:?} {u:?}");
    assert_eq!(b.format, GLES_TEXFMT_RGBA16F, "{at}: flip to GLES_TEXFMT_RGBA32F");
    assert!(!texture::return_32(&b), "{at}: the 16-bit TMU output; flip to true");
    assert_eq!(b.incomplete, 0, "{at}: complete");
    assert_eq!(halves(&texel), STORED, "{at}: halves; flip to the f32 bit patterns of VALUES");
    let back = halves(&texel).map(half_to_f32);
    assert_eq!(back[0], 0.0, "{at}: 1.0e-8 is lost");
    assert_eq!(back[1], 65504.0, "{at}: 65520.0 saturates to the largest half");
    assert_ne!(back[2], VALUES[2], "{at}: 1/3 is not itself any more");
}

/* ------------------------------------------------------------------------ */

/// texture.specification.texstorage2d (and cube): glTexStorage2D of
/// GL_RGBA32F, then glTexSubImage2D. Today halves; the correct value is the
/// 32-bit class with the floats' own bits.
#[test]
fn texstorage2d_rgba32f_is_halves_flip_when_texstorage2d_is_32_bit() {
    let _g = context();
    for shape in [Shape::D2, Shape::Cube] {
        assert_halves_today(shape, Spec::Storage, TIGHT);
    }
}

/// texture.specification.texstorage3d: glTexStorage3D of GL_RGBA32F on a 2D
/// array and a 3D texture, then glTexSubImage3D. Today halves.
#[test]
fn texstorage3d_rgba32f_is_halves_flip_when_texstorage3d_is_32_bit() {
    let _g = context();
    for shape in [Shape::Array, Shape::D3] {
        assert_halves_today(shape, Spec::Storage, TIGHT);
    }
}

/// texture.specification.texsubimage2d: glTexSubImage2D over a glTexImage2D
/// with NULL data (2D and cube). The sub-image encodes into the class the
/// specification chose, so today halves.
#[test]
fn texsubimage2d_rgba32f_is_halves_flip_when_texsubimage2d_is_32_bit() {
    let _g = context();
    for shape in [Shape::D2, Shape::Cube] {
        assert_halves_today(shape, Spec::NullImage, TIGHT);
    }
}

/// texture.specification.texsubimage3d: glTexSubImage3D over a glTexImage3D
/// with NULL data (2D array and 3D). Today halves.
#[test]
fn texsubimage3d_rgba32f_is_halves_flip_when_texsubimage3d_is_32_bit() {
    let _g = context();
    for shape in [Shape::Array, Shape::D3] {
        assert_halves_today(shape, Spec::NullImage, TIGHT);
    }
}

/// texture.specification.unpack_params (row length, skip pixels, skip rows):
/// glTexSubImage2D from a source 6 texels wide with the region one texel and
/// one row in. The region is read right (the first texel is `VALUES`, not the
/// 7.0 before it) and is stored as halves today.
#[test]
fn unpack_row_length_and_skip_2d_rgba32f_is_halves_flip_when_unpack_is_32_bit() {
    let _g = context();
    for spec in [Spec::Storage, Spec::NullImage] {
        assert_halves_today(Shape::D2, spec, SKIPPING_2D);
    }
}

/// The same for glTexSubImage3D: row length, image height, skip pixels,
/// skip rows and skip images, over a 2D array and a 3D texture.
#[test]
fn unpack_row_length_and_skip_3d_rgba32f_is_halves_flip_when_unpack_is_32_bit() {
    let _g = context();
    for shape in [Shape::Array, Shape::D3] {
        for spec in [Spec::Storage, Spec::NullImage] {
            assert_halves_today(shape, spec, SKIPPING);
        }
    }
}

/// GL_UNPACK_ALIGNMENT 2, 4 and 8: a GL_RGBA/GL_FLOAT texel is 16 bytes, so
/// the alignment never pads a row and the result is that of the tight
/// upload. Today halves.
#[test]
fn unpack_alignment_rgba32f_is_halves_flip_when_unpack_is_32_bit() {
    let _g = context();
    for alignment in [2, 4, 8] {
        let u = Unpack { alignment, ..TIGHT };
        assert_halves_today(Shape::D2, Spec::Storage, u);
        assert_halves_today(Shape::D3, Spec::NullImage, u);
    }
}

/// The controls, which do not flip: GL_RGBA16F through the same paths is the
/// half class and holds the client's own halves (65520.0 is infinity there),
/// the skipped texels never reaching the first one.
#[test]
fn rgba16f_through_the_same_paths_is_the_half_class_control() {
    let _g = context();
    for shape in [Shape::D2, Shape::Array, Shape::D3, Shape::Cube] {
        let skipping = if shape.layers() > 1 { SKIPPING } else { SKIPPING_2D };
        for spec in [Spec::Storage, Spec::NullImage] {
            for u in [TIGHT, skipping] {
                let (b, texel) = drawn(shape, GL_RGBA16F, spec, VALUES, u);
                assert_eq!(b.format, GLES_TEXFMT_RGBA16F, "{shape:?} {spec:?} {u:?}");
                assert!(!texture::return_32(&b), "{shape:?} {spec:?} {u:?}");
                assert_eq!(halves(&texel), VALUES.map(f32_to_half), "{shape:?} {spec:?} {u:?}");
            }
        }
    }
}

/// The narrowness, through the storage path: values a half holds are stored
/// exactly. After the fix they are 32-bit words; flip the texel read.
#[test]
fn values_a_half_holds_are_stored_exactly_flip_when_texstorage_is_32_bit() {
    let _g = context();
    let exact = [0.0f32, 0.5, 1.0, 2048.0];
    let (_, texel) = drawn(Shape::D2, GL_RGBA32F, Spec::Storage, exact, SKIPPING_2D);
    assert_eq!(halves(&texel).map(half_to_f32), exact);
}
