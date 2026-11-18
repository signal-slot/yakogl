// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GL_RGBA32F is stored in its own 32-bit class on the V3D path (plan #380,
//! issue #1261; images T05a and T06 of tools/cts/pi4-gles3-tree.txt). It was
//! stored as RGBA16F halves, and eight dEQP-GLES3 cases failed on that:
//!
//! * T05a `texture.format.sized.2d.rgba32f_pot`, `rgba32f_npot`;
//! * T06 `texture.format.sized.2d_array.rgba32f_pot`, `rgba32f_npot`,
//!   `texture.format.sized.3d.rgba32f_pot`, `rgba32f_npot`,
//!   `texture.format.sized.cube.rgba32f_pot`, `rgba32f_npot`.
//!
//! The cause was `sized_format` in src/gl/texture.rs (the line
//! `GL_RGBA32F => (GL_RGBA, GL_FLOAT, GLES_TEXFMT_RGBA16F), /* stored as
//! halves */`): the sized name resolves to the half-float class, and
//! `es3_formats.rs` has no row that gives GL_RGBA32F the 32-bit class
//! (`GLES_TEXFMT_RGBA32F`, which `F32_ST` lists only for GL_RG32F and
//! GL_RGB32F). A texel the client uploads is rounded to half precision on the
//! way in: 1.0e-8 (below the half range) is stored 0, 65520.0 (above the
//! largest half, 65504) 65504 (0x7bff), and 1/3 as 0x3555 (0.333251953125). The
//! back end sees `GLES_TEXFMT_RGBA16F` (the 16-bit TMU output, `return_32`
//! false), never the class it samples as 32 bits (`V3D_SAMPLED_TEXFMTS` has
//! RGBA32F; tests/es3_formats_v3d.rs pins that class). The path is the same on
//! 2D, 2D array, 3D and cube targets.
//!
//! What this pins, through the GL entry points over a recording stand-in back
//! end (the rig of tests/es3_tex_filtering.rs; the GL core's own store, the
//! bind a draw hands the back end, and the texels in the back end's memory):
//! the bind's format, the texels as stored, and what the V3D's own predicates
//! say about that bind. The fix (`rgba32f_own_class` opts in `gpu-v3d`):
//! GL_RGBA32F stored as `GLES_TEXFMT_RGBA32F`, four 32-bit words a texel, the
//! bits of the uploaded floats, `return_32` true. The controls (GL_RGBA16F,
//! values a half holds) stay. What it does not show: the picture dEQP compares with its reference.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_rgba32f_v3d
//! ```
#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-v3d", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::*;
use yakogl::gl::consts::*;
use yakogl::gl::draw::*;
use yakogl::gl::es3_formats::f32_to_half;
use yakogl::gl::framebuffer::*;
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
    name: c"es3-rgba32f-v3d".as_ptr() as *const c_char,
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

/// The values RGBA16F cannot hold exactly: below the half range, above it,
/// a third, and a tenth.
const VALUES: [f32; 4] = [1.0e-8, 65520.0, 1.0 / 3.0, 0.1];

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
}

/// A SIDE x SIDE (x 2 layers) texture of `internalformat`, every texel
/// `values`, NEAREST, complete; returns its name.
fn make(shape: Shape, internalformat: GLenum, values: [f32; 4]) -> GLuint {
    let layers = if matches!(shape, Shape::Array | Shape::D3) { 2 } else { 1 };
    let px: Vec<f32> = (0..SIDE * SIDE * layers).flat_map(|_| values).collect();
    let half: Vec<u16> = px.iter().map(|v| f32_to_half(*v)).collect();
    let (ty, data) = if internalformat == GL_RGBA16F {
        (GL_HALF_FLOAT, half.as_ptr() as *const c_void)
    } else {
        (GL_FLOAT, px.as_ptr() as *const c_void)
    };
    let mut name = 0;
    // SAFETY: plain GL calls; `data` holds SIDE x SIDE x layers RGBA texels.
    unsafe {
        glGenTextures(1, &mut name);
        glBindTexture(shape.target(), name);
        let (w, h, l, f) = (SIDE as GLsizei, SIDE as GLsizei, layers as GLsizei, internalformat as GLint);
        match shape {
            Shape::D2 => glTexImage2D(GL_TEXTURE_2D, 0, f, w, h, 0, GL_RGBA, ty, data),
            Shape::Array | Shape::D3 => glTexImage3D(shape.target(), 0, f, w, h, l, 0, GL_RGBA, ty, data),
            Shape::Cube => {
                for face in 0..6 {
                    glTexImage2D(GL_TEXTURE_CUBE_MAP_POSITIVE_X + face, 0, f, w, h, 0, GL_RGBA, ty, data);
                }
            }
        }
        for p in [GL_TEXTURE_MIN_FILTER, GL_TEXTURE_MAG_FILTER] {
            glTexParameteri(shape.target(), p, GL_NEAREST as GLint);
        }
    }
    assert_eq!(err(), GL_NO_ERROR, "{shape:?} {internalformat:#x}");
    name
}

/// The bind and the first texel's bytes of the one draw of a `shape` texture.
fn drawn(shape: Shape, internalformat: GLenum, values: [f32; 4]) -> (GlesTextureBind, Vec<u8>) {
    let t = make(shape, internalformat, values);
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

fn words(texel: &[u8]) -> [u32; 4] {
    core::array::from_fn(|i| u32::from_ne_bytes([texel[i * 4], texel[i * 4 + 1], texel[i * 4 + 2], texel[i * 4 + 3]]))
}

const SHAPES: [Shape; 4] = [Shape::D2, Shape::Array, Shape::D3, Shape::Cube];

/* ------------------------------------------------------------------------ */

/// The other half of the cause: the V3D samples the 32-bit class (RGBA32F is
/// in `V3D_SAMPLED_TEXFMTS`) and reads it at the 32-bit output, so the store
/// could have used it; it is the GL core's `sized_format` that does not.
#[test]
fn the_v3d_samples_the_32_bit_class_the_core_does_not_pick() {
    assert_ne!(sampled_texfmts() & (1 << GLES_TEXFMT_RGBA32F), 0);
    assert!(texture::return_32(&GlesTextureBind { format: GLES_TEXFMT_RGBA32F, ..GlesTextureBind::ZERO }));
}

/// The bind of a GL_RGBA32F texture of every target is the 32-bit class,
/// which the V3D reads at the 32-bit output.
#[test]
fn rgba32f_is_bound_as_its_own_32_bit_class() {
    let _g = context();
    for shape in SHAPES {
        let (b, _) = drawn(shape, GL_RGBA32F, VALUES);
        assert_eq!(b.format, GLES_TEXFMT_RGBA32F, "{shape:?}");
        assert!(texture::return_32(&b), "{shape:?}: the 32-bit TMU output");
        if !matches!(shape, Shape::Cube) {
            /* the stand-in's cube layout (faces stacked) is not the V3D's */
            assert_eq!(texture::refusal_for(&b, true), None, "{shape:?}: the 32-bit key takes it");
        }
        assert_eq!(b.incomplete, 0, "{shape:?}: complete");
    }
}

/// The texels as stored: four 32-bit words a texel, the bits of the floats
/// given (1.0e-8 and 65520.0 are kept, 1/3 is not rounded to a half).
#[test]
fn rgba32f_texels_are_stored_as_the_uploaded_floats() {
    let _g = context();
    for shape in SHAPES {
        let (b, texel) = drawn(shape, GL_RGBA32F, VALUES);
        assert_eq!(b.format, GLES_TEXFMT_RGBA32F, "{shape:?}");
        assert_eq!(words(&texel), VALUES.map(f32::to_bits), "{shape:?}: the f32 bit patterns of VALUES");
    }
}

/// The control: GL_RGBA16F is the half class by definition, bound and stored
/// the way GL_RGBA32F is now, but for the clamp (the fix changes only the RGBA32F row).
#[test]
fn rgba16f_is_the_same_class_and_bytes_control() {
    let _g = context();
    for shape in SHAPES {
        let (b, texel) = drawn(shape, GL_RGBA16F, VALUES);
        assert_eq!(b.format, GLES_TEXFMT_RGBA16F, "{shape:?}");
        assert!(!texture::return_32(&b), "{shape:?}");
        /* the client's own halves (65520.0 as infinity, by f32_to_half) */
        assert_eq!(halves(&texel), VALUES.map(f32_to_half), "{shape:?}");
    }
}

/// The narrowness: values a half holds are stored exactly in either class,
/// which is why only the cases whose reference values need more than a half
/// failed; now as 32-bit words.
#[test]
fn values_a_half_holds_are_stored_exactly() {
    let _g = context();
    let exact = [0.0f32, 0.5, 1.0, 2048.0];
    let (_, texel) = drawn(Shape::D2, GL_RGBA32F, exact);
    assert_eq!(words(&texel).map(f32::from_bits), exact);
}

/// A framebuffer attachment: the texture is still a GL_RGBA32F one, which is
/// not colour-renderable without GL_EXT_color_buffer_float (ES 3.0 4.4.4.1),
/// in its own class as it was as halves. GL_RGBA16F stays complete.
#[test]
fn rgba32f_attachment_stays_incomplete_and_rgba16f_complete() {
    let _g = context();
    for (internal, status) in
        [(GL_RGBA32F, GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT), (GL_RGBA16F, GL_FRAMEBUFFER_COMPLETE)]
    {
        let (mut fb, mut tex) = (0, 0);
        // SAFETY: room for one name each.
        unsafe {
            glGenTextures(1, &mut tex);
            glBindTexture(GL_TEXTURE_2D, tex);
            glTexStorage2D(GL_TEXTURE_2D, 1, internal, 4, 4);
            glGenFramebuffers(1, &mut fb);
            glBindFramebuffer(GL_FRAMEBUFFER, fb);
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
            assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), status, "{internal:#x}");
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteFramebuffers(1, &fb);
            glDeleteTextures(1, &tex);
        }
    }
}
