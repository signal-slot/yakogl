// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! PowerVR (AM62P): GL_TEXTURE_BASE_LEVEL, GL_TEXTURE_MIN_LOD and
//! GL_TEXTURE_MAX_LOD reach the sampler descriptor a draw binds for the 2D,
//! cube and 2D-array targets, shadow binds included (dEQP-GLES3
//! `texture.mipmap.*.{base_level,min_lod,max_lod}.*`).
//!
//! Two halves:
//!
//! - The descriptor words. `yakogl::pvr::backend::texture_descriptor` is
//!   the draw's own builder input (`pvr::texture::TexLod::of_bind`): a bind
//!   whose `lod_set` carries MIN_LOD/MAX_LOD and whose `base_level` is set
//!   packs TEXSTATE_SAMPLER_WORD0 minlod [13:22] / maxlod [23:32] (u4.6,
//!   Mesa 26.2.1 pvr_arch_sampler.c `util_unsigned_fixed(CLAMP(lod, 0,
//!   959/64), 6)`, inside the levels from the base), and the image from the
//!   base level: TEXSTATE_IMAGE_WORD1.baselevel [60:63] for the native cube
//!   and native 2D array, a linear 2D image's texaddr moved to it. GL's
//!   defaults (a zeroed bind, or -1000/1000 stated) give today's words.
//!   The native 3D texture keeps its own view (`gl::native_vol::View`): its
//!   words do not move with the bind's fields.
//! - The GL core. A real draw on a stand-in back end with the PowerVR's
//!   flags hands over the texture's MIN_LOD/MAX_LOD (a bound sampler
//!   object's in its place, ES 3.0.6 3.8.2) and the effective base level
//!   in the bind, for a `sampler2D`, `samplerCube` and `sampler2DArray`.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_pvr_texlod
//! ```
//!
//! HOST-ONLY: no picture is checked; the words are what the TPU reads.
//! The GL half needs the in-process compiler and ES 3.0 (not report-es2).

#![cfg(feature = "gpu-powervr")]

use yakogl::backend::{
    GlesTextureBind, GLES_TEXFMT_RGBA8, GLES_TEXTYPE_2D, GLES_TEXTYPE_2D_ARRAY_TWIDDLED, GLES_TEXTYPE_3D_TWIDDLED,
    GLES_TEXTYPE_CUBE_TWIDDLED_PROBE,
};
use yakogl::pvr::backend::{texture_descriptor, texture_descriptor_lod};
use yakogl::pvr::texture::TexLod;

const BASE: u64 = 0x80_0010_0000;

/// A 64x64 RGBA8 bind of `type_` with a seven-level chain under
/// LINEAR_MIPMAP_LINEAR (or one level without a mipmap filter).
fn bind(type_: u8, mip: bool) -> GlesTextureBind {
    let layers = match type_ {
        GLES_TEXTYPE_2D => 1,
        GLES_TEXTYPE_CUBE_TWIDDLED_PROBE => 6,
        _ => 4,
    };
    GlesTextureBind {
        format: GLES_TEXFMT_RGBA8,
        type_,
        layers,
        base_gpu: BASE,
        width: 64,
        height: 64,
        stride_texels: 64,
        bytes: 0x8000 * u32::from(layers),
        linear_filter: 1,
        min_linear: 1,
        mip_filter: if mip { 2 } else { 0 },
        mip_levels: if mip { 7 } else { 1 },
        wrap_s: 1,
        wrap_t: 2,
        ..GlesTextureBind::ZERO
    }
}

fn word(w: &[u32], i: usize) -> u64 {
    u64::from(w[i]) | (u64::from(w[i + 1]) << 32)
}

fn bits(v: u64, lo: u32, hi: u32) -> u64 {
    (v >> lo) & ((1u64 << (hi - lo + 1)) - 1)
}

/// SAMPLER_WORD0 (minlod, maxlod) in u4.6.
fn lods(w: &[u32]) -> (u64, u64) {
    let sw0 = word(w, 8);
    (bits(sw0, 13, 22), bits(sw0, 23, 32))
}

/// IMAGE_WORD1 of the twiddled word: (baselevel, num_mip_levels).
fn twiddled_levels(w: &[u32]) -> (u64, u64) {
    let iw1 = word(w, 2);
    (bits(iw1, 60, 63), bits(iw1, 0, 3))
}

fn texaddr(w: &[u32]) -> u64 {
    bits(word(w, 2), 16, 53) << 2
}

const TYPES: [(u8, &str); 3] =
    [(GLES_TEXTYPE_2D, "2D"), (GLES_TEXTYPE_CUBE_TWIDDLED_PROBE, "cube"), (GLES_TEXTYPE_2D_ARRAY_TWIDDLED, "2D array")];

/* ---- the descriptor words ---- */

/// GL's defaults, zeroed or stated (and the saturated +-127 the bind
/// stores for -1000/1000), give the words the draw built before the fields
/// were carried, for every target, with and without a chain, shadow binds
/// included.
#[test]
fn defaults_give_todays_words() {
    for (ty, name) in TYPES {
        for mip in [false, true] {
            for compare in [0u8, 4] {
                let mut t = bind(ty, mip);
                t.compare_op = compare;
                let before = texture_descriptor_lod(&t, &TexLod::DEFAULT);
                assert_eq!(texture_descriptor(&t), before, "{name} mip {mip} compare {compare}: zeroed bind");
                t.set_lod_range(-1000.0, 1000.0);
                assert_eq!(t.lod_set, 1);
                assert_eq!(texture_descriptor(&t), before, "{name} mip {mip} compare {compare}: defaults stated");
            }
        }
    }
    /* the pinned defaults of today: LOD 0..6 over the chain; one level is
     * 0..0, or 0..1 when the min and mag filters differ (the TPU's min/mag
     * choice on the clamped LOD, tex_desc_sampler_lod) */
    assert_eq!(lods(&texture_descriptor(&bind(GLES_TEXTYPE_2D, true))), (0, 6 * 64));
    assert_eq!(lods(&texture_descriptor(&bind(GLES_TEXTYPE_2D, false))), (0, 0));
    let t = GlesTextureBind { min_linear: 0, ..bind(GLES_TEXTYPE_2D, false) };
    assert_eq!(lods(&texture_descriptor(&t)), (0, 64));
}

/// MIN_LOD 1.5 and MAX_LOD 3.25 clamp the sampler of every target: u4.6
/// minlod 96, maxlod 208; the image words do not move.
#[test]
fn min_and_max_lod_reach_the_sampler() {
    for (ty, name) in TYPES {
        for compare in [0u8, 4] {
            let mut t = bind(ty, true);
            t.compare_op = compare;
            let before = texture_descriptor(&t);
            t.set_lod_range(1.5, 3.25);
            let w = texture_descriptor(&t);
            assert_eq!(lods(&w), (96, 208), "{name} compare {compare}");
            assert_eq!(w[0..8], before[0..8], "{name}: the image words stay");
            assert_eq!(w[10..], before[10..], "{name}: the rest stays");
            /* the words are TexLod's, spelled out */
            let l = TexLod { min_lod: 1.5, max_lod: 3.25, ..TexLod::DEFAULT };
            assert_eq!(w, texture_descriptor_lod(&bind_with_compare(ty, compare), &l), "{name}");
        }
    }
}

fn bind_with_compare(ty: u8, compare: u8) -> GlesTextureBind {
    GlesTextureBind { compare_op: compare, ..bind(ty, true) }
}

/// MAX_LOD past the chain is the chain's last level; MAX_LOD below MIN_LOD
/// pulls minlod down to it; a negative MIN_LOD is 0; MAX_LOD 0 holds a
/// one-level sampler whose min and mag filters differ at LOD 0 (magnify
/// only, as GL clamps lambda to 0).
#[test]
fn lod_edges() {
    let mut t = bind(GLES_TEXTYPE_2D, true);
    t.set_lod_range(-2.0, 40.0);
    assert_eq!(lods(&texture_descriptor(&t)), (0, 6 * 64));
    t.set_lod_range(4.0, 2.0);
    assert_eq!(lods(&texture_descriptor(&t)), (128, 128));
    let mut t = GlesTextureBind { min_linear: 0, ..bind(GLES_TEXTYPE_2D, false) };
    assert_eq!(lods(&texture_descriptor(&t)), (0, 64));
    t.set_lod_range(-1000.0, 0.0);
    assert_eq!(lods(&texture_descriptor(&t)), (0, 0));
    /* 1/256 steps: 0.3 is 76/256 rounded down, which u4.6 reads as 19/64 */
    let mut t = bind(GLES_TEXTYPE_CUBE_TWIDDLED_PROBE, true);
    t.set_lod_range(0.3, 5.0);
    assert_eq!(lods(&texture_descriptor(&t)), (19, 320));
}

/// GL_TEXTURE_BASE_LEVEL 1 moves the image to level 1 and the LOD range to
/// the six levels from it: the linear 2D image's address moves by level
/// 0's bytes and its extent halves; the native cube and 2D array name
/// baselevel 1 over the copy's whole chain (num_mip_levels 7, the stride).
#[test]
fn base_level_moves_the_image_and_the_range() {
    let mut t = bind(GLES_TEXTYPE_2D, true);
    t.base_level = 1;
    let w = texture_descriptor(&t);
    assert_eq!(texaddr(&w), BASE + 64 * 64 * 4, "texaddr at level 1");
    let iw0 = word(&w, 0);
    assert_eq!((bits(iw0, 34, 47) + 1, bits(iw0, 48, 61) + 1), (32, 32), "level 1's extent");
    assert_eq!(bits(word(&w, 2), 60, 63), 6, "STRIDE num_mip_levels: levels 1..6");
    assert_eq!(lods(&w), (0, 5 * 64));

    for ty in [GLES_TEXTYPE_CUBE_TWIDDLED_PROBE, GLES_TEXTYPE_2D_ARRAY_TWIDDLED] {
        let mut t = bind(ty, true);
        let before = texture_descriptor(&t);
        t.base_level = 1;
        let w = texture_descriptor(&t);
        assert_eq!(twiddled_levels(&before), (0, 7));
        assert_eq!(twiddled_levels(&w), (1, 7), "type {ty}: baselevel 1 over the copy's chain");
        assert_eq!(texaddr(&w), BASE);
        assert_eq!(lods(&w), (0, 5 * 64), "type {ty}");
        assert_eq!(w[4..8], before[4..8], "type {ty}: the layer stride stays the copy's");
    }
}

/// Base level 1 with MIN_LOD 1.5 and MAX_LOD 3.25: the range counts from
/// the base level (ES 3.0.6 3.8.10.4: level = base + lambda), so the words
/// are the same u4.6 96..208 over the image stated from level 1.
#[test]
fn base_level_and_lod_range_together() {
    for (ty, name) in TYPES {
        let mut t = bind(ty, true);
        t.base_level = 1;
        t.set_lod_range(1.5, 3.25);
        assert_eq!(lods(&texture_descriptor(&t)), (96, 208), "{name}");
        t.set_lod_range(0.0, 100.0);
        assert_eq!(lods(&texture_descriptor(&t)), (0, 5 * 64), "{name}: max past the chain from the base");
    }
}

/// The native 3D texture's words do not follow the bind's fields: its copy
/// carries the volume's own view (gl::native_vol::View). Unchanged.
#[test]
fn native_3d_is_unchanged() {
    let mut t = bind(GLES_TEXTYPE_3D_TWIDDLED, true);
    let before = texture_descriptor(&t);
    t.base_level = 1;
    t.set_lod_range(1.5, 3.25);
    assert_eq!(texture_descriptor(&t), before);
}

/// The bind's fixed point: 1/256 of a level, rounded down, saturated at
/// +-127 levels, a NaN 0; a zeroed bind reads GL's defaults.
#[test]
fn lod_fields_round_trip() {
    let mut t = GlesTextureBind::ZERO;
    assert_eq!(t.lod_range(), (-1000.0, 1000.0));
    t.set_lod_range(1.5, -0.25);
    assert_eq!((t.lod_set, t.min_lod, t.max_lod), (1, 384, -64));
    assert_eq!(t.lod_range(), (1.5, -0.25));
    t.set_lod_range(-1000.0, 1000.0);
    assert_eq!((t.min_lod, t.max_lod), (-127 * 256, 127 * 256));
    t.set_lod_range(f32::NAN, 0.3);
    assert_eq!((t.min_lod, t.max_lod), (0, 76));
    /* rounded down on the negative side too: -0.3 is -76.8/256 */
    t.set_lod_range(-0.3, -2.0);
    assert_eq!((t.min_lod, t.max_lod), (-77, -512));
}

/* ---- the GL core fills the bind ---- */

#[cfg(all(feature = "host-test", feature = "shaderc-online", not(feature = "report-es2")))]
mod gl {
    use core::ffi::{c_char, c_void};
    use std::ffi::CString;
    use std::sync::Mutex;
    use yakogl::backend::{GlesDraw, GlesHeap, GlesTextureBind, YakoglBackend};
    use yakogl::gl::consts::*;
    use yakogl::gl::draw::*;
    use yakogl::gl::es3::{glBindSampler, glGenSamplers, glSamplerParameterf, glSamplerParameteri};
    use yakogl::gl::program::*;
    use yakogl::gl::shader::*;
    use yakogl::gl::state::*;
    use yakogl::gl::texture::*;
    use yakogl::gl::types::*;
    use yakogl::gl::vertex::*;
    use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};

    // SAFETY (Send): GlesTextureBind holds a raw `cpu` pointer; the tests
    // read only its plain fields.
    struct SendBind(GlesTextureBind);
    unsafe impl Send for SendBind {}
    static BIND_SLOT: Mutex<Option<SendBind>> = Mutex::new(None);

    unsafe extern "C" fn b_init() -> i32 {
        0
    }
    unsafe extern "C" fn b_heap_base(_h: GlesHeap) -> u64 {
        0x1000_0000
    }
    unsafe extern "C" fn b_ok() -> i32 {
        0
    }
    unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
        // SAFETY: the core passes a live job.
        let d = unsafe { &*d };
        if d.num_textures > 0 {
            *BIND_SLOT.lock().unwrap_or_else(|e| e.into_inner()) = Some(SendBind(d.textures[0]));
        }
        0
    }

    /* Built from zero with the fields used here set, so a field another
     * change adds to YakoglBackend needs no edit (CLAUDE.md). Zero is the
     * PowerVR's shape for the flags that matter: packed levels, no layered
     * TPU walk. */
    static PVR_SHAPE: YakoglBackend = {
        // SAFETY: all-zero is a valid YakoglBackend (null, None, false, 0).
        let mut b: YakoglBackend = unsafe { core::mem::zeroed() };
        b.name = c"pvr texlod stand-in".as_ptr() as *const c_char;
        b.init = Some(b_init);
        b.draw = Some(b_draw);
        b.flush = Some(b_ok);
        b.finish = Some(b_ok);
        b.heap_base = Some(b_heap_base);
        b.max_instance_divisor = 1;
        b
    };

    unsafe extern "C" fn o_gpu_alloc(_heap: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
        let mem = Box::leak(vec![0u64; size.div_ceil(8)].into_boxed_slice());
        let cpu = mem.as_mut_ptr() as *mut c_void;
        // SAFETY: `out` is the core's.
        unsafe { *out = OsalGpuMem { cpu, gpu: cpu as u64, phys: cpu as u64, size, impl_: cpu } };
        0
    }
    unsafe extern "C" fn o_gpu_free(_m: *mut OsalGpuMem) -> i32 {
        0
    }
    unsafe extern "C" fn o_cache(_cpu: *mut c_void, _n: usize) {}

    static OSAL: OsalOps = {
        // SAFETY: all-zero OsalOps is every hook None.
        let mut o: OsalOps = unsafe { core::mem::zeroed() };
        o.gpu_alloc = Some(o_gpu_alloc);
        o.gpu_free = Some(o_gpu_free);
        o.cache_flush = Some(o_cache);
        o.cache_invd = Some(o_cache);
        o
    };

    static SERIAL: Mutex<()> = Mutex::new(());

    fn on_powervr(case: impl FnOnce()) {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        // SAFETY: the guard serialises the tests; the tables are statics.
        let ctx = unsafe {
            osal_set_ops(&OSAL);
            yakogl::backend::g_backend = &PVR_SHAPE;
            yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            let c = yakogl::gl::context::gl_context_create(64, 64);
            assert!(!c.is_null(), "context pool exhausted");
            c
        };
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(case));
        // SAFETY: the context made above.
        unsafe { yakogl::gl::context::gl_context_destroy(ctx) };
        if let Err(e) = r {
            std::panic::resume_unwind(e);
        }
    }

    fn no_error(what: &str) {
        // SAFETY: a plain GL call.
        let e = unsafe { glGetError() };
        assert_eq!(e, GL_NO_ERROR, "{what}: 0x{e:x}");
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
            assert_eq!(ok, 1, "{text} did not compile");
            s
        }
    }

    const VS: &str =
        "#version 300 es\nin vec4 p;\nout vec3 v;\nvoid main() { v = vec3(p.xy, 1.0); gl_Position = p; }\n";
    fn fs(sampler: &str) -> String {
        format!(
            "#version 300 es\nprecision mediump float;\nuniform mediump {sampler} s;\nin vec3 v;\nout vec4 o;\n\
             void main() {{ o = texture(s, {}); }}\n",
            if sampler == "sampler2D" { "v.xy" } else { "v" }
        )
    }
    static QUAD: [f32; 16] = [-1.0, -1.0, 0.5, 1.0, 1.0, -1.0, 0.5, 1.0, -1.0, 1.0, 0.5, 1.0, 1.0, 1.0, 0.5, 1.0];

    fn program(sampler: &str) -> GLuint {
        // SAFETY: plain GL calls.
        unsafe {
            let p = glCreateProgram();
            glAttachShader(p, shader(GL_VERTEX_SHADER, VS));
            glAttachShader(p, shader(GL_FRAGMENT_SHADER, &fs(sampler)));
            glBindAttribLocation(p, 0, c"p".as_ptr());
            glLinkProgram(p);
            let mut ok: GLint = 0;
            glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
            assert_eq!(ok, 1, "link failed");
            glUseProgram(p);
            glUniform1i(glGetUniformLocation(p, c"s".as_ptr()), 0);
            glEnableVertexAttribArray(0);
            glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, QUAD.as_ptr() as *const c_void);
            p
        }
    }

    fn draw() -> GlesTextureBind {
        *BIND_SLOT.lock().unwrap_or_else(|e| e.into_inner()) = None;
        // SAFETY: plain GL call; the program and attribute were set up.
        unsafe { glDrawArrays(GL_TRIANGLE_STRIP, 0, 4) };
        no_error("the quad");
        BIND_SLOT.lock().unwrap_or_else(|e| e.into_inner()).take().expect("drawn with a texture").0
    }

    const FACES: [GLenum; 6] = [
        GL_TEXTURE_CUBE_MAP_POSITIVE_X,
        GL_TEXTURE_CUBE_MAP_NEGATIVE_X,
        GL_TEXTURE_CUBE_MAP_POSITIVE_Y,
        GL_TEXTURE_CUBE_MAP_NEGATIVE_Y,
        GL_TEXTURE_CUBE_MAP_POSITIVE_Z,
        GL_TEXTURE_CUBE_MAP_NEGATIVE_Z,
    ];

    /// A complete 16x16 five-level RGBA8 texture of `target`, bound to unit
    /// 0, LINEAR_MIPMAP_LINEAR.
    fn texture(target: GLenum) {
        let mut name = 0;
        // SAFETY: plain GL calls; each level's pixels are a live Vec.
        unsafe {
            glGenTextures(1, &mut name);
            glBindTexture(target, name);
            for l in 0..5 {
                let s = 16 >> l;
                let px = vec![0x80u8; s * s * 4 * 4];
                let ptr = px.as_ptr() as *const c_void;
                match target {
                    GL_TEXTURE_2D => glTexImage2D(
                        target,
                        l,
                        GL_RGBA as GLint,
                        s as GLsizei,
                        s as GLsizei,
                        0,
                        GL_RGBA,
                        GL_UNSIGNED_BYTE,
                        ptr,
                    ),
                    GL_TEXTURE_CUBE_MAP => {
                        for f in FACES {
                            glTexImage2D(
                                f,
                                l,
                                GL_RGBA as GLint,
                                s as GLsizei,
                                s as GLsizei,
                                0,
                                GL_RGBA,
                                GL_UNSIGNED_BYTE,
                                ptr,
                            );
                        }
                    }
                    _ => glTexImage3D(
                        target,
                        l,
                        GL_RGBA8 as GLint,
                        s as GLsizei,
                        s as GLsizei,
                        4,
                        0,
                        GL_RGBA,
                        GL_UNSIGNED_BYTE,
                        ptr,
                    ),
                }
            }
            glTexParameteri(target, GL_TEXTURE_MIN_FILTER, GL_LINEAR_MIPMAP_LINEAR as GLint);
            glTexParameteri(target, GL_TEXTURE_MAG_FILTER, GL_LINEAR as GLint);
        }
        no_error("the texture");
    }

    const TARGETS: [(GLenum, &str); 3] =
        [(GL_TEXTURE_2D, "sampler2D"), (GL_TEXTURE_CUBE_MAP, "samplerCube"), (GL_TEXTURE_2D_ARRAY, "sampler2DArray")];

    /// The texture's MIN_LOD/MAX_LOD and base level are in the bind of every
    /// target; GL's defaults are the saturated +-127 levels.
    #[test]
    fn the_draw_fills_the_bind() {
        for (target, sampler) in TARGETS {
            on_powervr(|| {
                texture(target);
                program(sampler);
                let t = draw();
                assert_eq!((t.lod_set, t.min_lod, t.max_lod, t.base_level), (1, -127 * 256, 127 * 256, 0), "{sampler}");
                // SAFETY: plain GL calls.
                unsafe {
                    glTexParameterf(target, GL_TEXTURE_MIN_LOD, 1.5);
                    glTexParameterf(target, GL_TEXTURE_MAX_LOD, 3.25);
                    glTexParameteri(target, GL_TEXTURE_BASE_LEVEL, 1);
                }
                let t = draw();
                assert_eq!((t.lod_set, t.min_lod, t.max_lod, t.base_level), (1, 384, 832, 1), "{sampler}");
                assert_eq!(t.lod_range(), (1.5, 3.25), "{sampler}");
            });
        }
    }

    /// A sampler object's MIN_LOD/MAX_LOD replace the texture's while it is
    /// bound (ES 3.0.6 3.8.2); unbinding it brings the texture's back.
    #[test]
    fn a_sampler_object_replaces_the_texture_lods() {
        for (target, sampler) in TARGETS {
            on_powervr(|| {
                texture(target);
                program(sampler);
                let mut s = 0;
                // SAFETY: plain GL calls.
                unsafe {
                    glTexParameterf(target, GL_TEXTURE_MIN_LOD, 1.0);
                    glGenSamplers(1, &mut s);
                    glSamplerParameteri(s, GL_TEXTURE_MIN_FILTER, GL_LINEAR_MIPMAP_LINEAR as GLint);
                    glSamplerParameterf(s, GL_TEXTURE_MIN_LOD, 2.0);
                    glSamplerParameterf(s, GL_TEXTURE_MAX_LOD, 2.5);
                    glBindSampler(0, s);
                }
                no_error("the sampler");
                assert_eq!(draw().lod_range(), (2.0, 2.5), "{sampler}: the sampler's");
                // SAFETY: plain GL call.
                unsafe { glBindSampler(0, 0) };
                assert_eq!(draw().lod_range(), (1.0, 127.0), "{sampler}: the texture's again");
            });
        }
    }
}
