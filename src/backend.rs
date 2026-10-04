// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The boundary between the GL core and a GPU command backend (`backend.h`).
//!
//! `gl::draw` resolves the current GL context into one [`GlesDraw`] (the
//! whole state a draw needs: target, clears, depth/stencil, blend, textures,
//! vertex streams, shaders) and hands it to the backend; the backend records
//! draws into the current frame and submits the frame when the target
//! changes, on glFlush/glFinish or when its own limits are hit. The backend
//! owns the GPU memory layout (heaps) and the command encoding; the GL core
//! never sees a register. Everything here is GPU-agnostic; `pvr` implements
//! it for the PowerVR BXS. One backend is linked per build (`g_backend`).
//!
//! The types are `#[repr(C)]` and the table is a C ABI (the on-silicon test
//! suite drives the backend directly); C enums are `u32` aliases so that any
//! value a C caller passes is representable.

use crate::shaderc::ShcShader;
use core::ffi::{c_char, c_void};

/* enum gles_prim */
pub type GlesPrim = u32;
pub const GLES_PRIM_TRIANGLES: GlesPrim = 0;
pub const GLES_PRIM_TRI_STRIP: GlesPrim = 1;
pub const GLES_PRIM_TRI_FAN: GlesPrim = 2;
pub const GLES_PRIM_LINES: GlesPrim = 3;
pub const GLES_PRIM_LINE_STRIP: GlesPrim = 4;
pub const GLES_PRIM_LINE_LOOP: GlesPrim = 5;
pub const GLES_PRIM_POINTS: GlesPrim = 6;

/* enum gles_cmp (TA_CMPMODE order) */
pub type GlesCmp = u32;
pub const GLES_CMP_NEVER: GlesCmp = 0;
pub const GLES_CMP_LESS: GlesCmp = 1;
pub const GLES_CMP_EQUAL: GlesCmp = 2;
pub const GLES_CMP_LEQUAL: GlesCmp = 3;
pub const GLES_CMP_GREATER: GlesCmp = 4;
pub const GLES_CMP_NOTEQUAL: GlesCmp = 5;
pub const GLES_CMP_GEQUAL: GlesCmp = 6;
pub const GLES_CMP_ALWAYS: GlesCmp = 7;

/* enum gles_cull */
pub type GlesCull = u32;
pub const GLES_CULL_NONE: GlesCull = 0;
pub const GLES_CULL_CW: GlesCull = 1;
pub const GLES_CULL_CCW: GlesCull = 2;

/* enum gles_attrib_format: source formats of a vertex attribute array
 * (glVertexAttribPointer type). A backend advertises the ones its vertex
 * fetch reads natively in `YakoglBackend::native_attrib_formats`; the GL core
 * converts every other array to float32 into a staging buffer at draw time. */
pub type GlesAttribFormat = u8;
pub const GLES_ATTRIBF_FLOAT32: GlesAttribFormat = 0;
pub const GLES_ATTRIBF_BYTE: GlesAttribFormat = 1;
pub const GLES_ATTRIBF_UBYTE: GlesAttribFormat = 2;
pub const GLES_ATTRIBF_SHORT: GlesAttribFormat = 3;
pub const GLES_ATTRIBF_USHORT: GlesAttribFormat = 4;
pub const GLES_ATTRIBF_FIXED: GlesAttribFormat = 5;
pub const GLES_ATTRIBF_COUNT: usize = 6;

/// `GLES_ATTRIBF_BIT(f)`: the bit of one format in `native_attrib_formats`.
pub const fn gles_attribf_bit(f: GlesAttribFormat) -> u32 {
    1u32 << f
}

/* enum gles_stencil_op (ISPB encoding order = Vulkan's) */
pub type GlesStencilOp = u32;
pub const GLES_SOP_KEEP: GlesStencilOp = 0;
pub const GLES_SOP_ZERO: GlesStencilOp = 1;
pub const GLES_SOP_REPLACE: GlesStencilOp = 2;
pub const GLES_SOP_INCR_SAT: GlesStencilOp = 3;
pub const GLES_SOP_DECR_SAT: GlesStencilOp = 4;
pub const GLES_SOP_INVERT: GlesStencilOp = 5;
pub const GLES_SOP_INCR_WRAP: GlesStencilOp = 6;
pub const GLES_SOP_DECR_WRAP: GlesStencilOp = 7;

/* enum gles_heap: device-VA heaps the GL core allocates in */
pub type GlesHeap = u32;
pub const GLES_HEAP_DATA: GlesHeap = 0;
pub const GLES_HEAP_SHADER: GlesHeap = 1;
pub const GLES_HEAP_PROGRAM: GlesHeap = 2;

/* enum gles_texfmt: texel formats the TPU decodes */
pub const GLES_TEXFMT_RGBA8: u8 = 0;
pub const GLES_TEXFMT_ETC2_RGB: u8 = 1;
pub const GLES_TEXFMT_ETC2_RGBA: u8 = 2;
pub const GLES_TEXFMT_ETC2_RGB_PT: u8 = 3;
/// Linear 16-bit R5G6B5 (a native buffer or surface).
pub const GLES_TEXFMT_RGB565: u8 = 4;
/* OpenGL ES 3.0 float texels, linear rows like RGBA8: 8 bytes (four halves),
 * 2 bytes (one half, reads (r,0,0,1)), 4 bytes (one float). HDR light probes
 * and Qt Quick 3D's shadow maps. */
pub const GLES_TEXFMT_RGBA16F: u8 = 5;
pub const GLES_TEXFMT_R16F: u8 = 6;
pub const GLES_TEXFMT_R32F: u8 = 7;
/* ES 2.0's unsized luminance/alpha texels stored narrow (1, 1 and 2 bytes)
 * where the backend samples them ([`narrow_la_texels`]); elsewhere they are
 * expanded into RGBA8 as (L,L,L,1), (0,0,0,A) and (L,L,L,A). */
pub const GLES_TEXFMT_L8: u8 = 8;
pub const GLES_TEXFMT_A8: u8 = 9;
pub const GLES_TEXFMT_LA8: u8 = 10;

/// Bytes per texel of a linear (non-block) texel format.
pub const fn gles_texfmt_bytes(f: u8) -> u32 {
    if f == GLES_TEXFMT_RGBA16F {
        8
    } else if f == GLES_TEXFMT_R16F || f == GLES_TEXFMT_RGB565 || f == GLES_TEXFMT_LA8 {
        2
    } else if f == GLES_TEXFMT_L8 || f == GLES_TEXFMT_A8 {
        1
    } else {
        4
    }
}

/* enum gles_surf_format: memory formats of a colour surface (window/pbuffer/
 * FBO target). The float ones are FBO textures (RGBA16F probes, R16F/R32F
 * shadow maps): the PBE packs them from a fragment shader variant compiled
 * for that class. */
pub const GLES_SURF_RGBA8888: u8 = 0;
pub const GLES_SURF_XRGB8888: u8 = 1;
pub const GLES_SURF_RGB565: u8 = 2;
pub const GLES_SURF_RGBA16F: u8 = 3;
pub const GLES_SURF_R16F: u8 = 4;
pub const GLES_SURF_R32F: u8 = 5;

pub const fn gles_surf_bytes_per_pixel(format: u8) -> u32 {
    if format == GLES_SURF_RGB565 || format == GLES_SURF_R16F {
        2
    } else if format == GLES_SURF_RGBA16F {
        8
    } else {
        4
    }
}

/// The render target class (`SHC_RT_*`) a fragment shader must be compiled for.
pub const fn gles_surf_rt_class(format: u8) -> u32 {
    match format {
        GLES_SURF_RGBA16F => 1,
        GLES_SURF_R16F => 2,
        GLES_SURF_R32F => 3,
        _ => 0,
    }
}

/// The texel format the surface is sampled as (fbload).
pub const fn gles_surf_texfmt(format: u8) -> u8 {
    match format {
        GLES_SURF_RGB565 => GLES_TEXFMT_RGB565,
        GLES_SURF_RGBA16F => GLES_TEXFMT_RGBA16F,
        GLES_SURF_R16F => GLES_TEXFMT_R16F,
        GLES_SURF_R32F => GLES_TEXFMT_R32F,
        _ => GLES_TEXFMT_RGBA8,
    }
}

/// `struct gles_attrib`: a vertex attribute as the VS expects it.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct GlesAttrib {
    pub vi_reg: u8,
    pub components: u8,
    /// [`GlesAttribFormat`].
    pub format: u8,
    /// Integer formats: scale to [0,1] / [-1,1].
    pub normalized: u8,
    pub stride: u32,
    pub offset: u32,
    pub buffer_gpu: u64,
    /// Draw-time data (a converted array, or the constant of a disabled
    /// array): the core's staging is reused by the next draw, so the backend
    /// copies these bytes into its per-frame arena (buffer_gpu/offset unused).
    pub cpu: *const c_void,
    pub cpu_bytes: u32,
    /// ES 3.0 instancing: one element per instance (divisor 1), fetched by
    /// the instance number instead of the vertex index.
    pub per_instance: u8,
    /// With `per_instance`: the array's divisor (one element per `divisor`
    /// instances), at most the backend's `max_instance_divisor`; 0 means 1.
    pub divisor: u16,
}

impl GlesAttrib {
    pub const ZERO: GlesAttrib = GlesAttrib {
        vi_reg: 0,
        components: 0,
        format: 0,
        normalized: 0,
        stride: 0,
        offset: 0,
        buffer_gpu: 0,
        cpu: core::ptr::null(),
        cpu_bytes: 0,
        per_instance: 0,
        divisor: 0,
    };
}

/// Sampler bindings one draw can carry per stage. Qt Quick 3D's material
/// shaders declare a dozen and more, and the descriptor block must hold one
/// slot per sampler the shader declares: the compiler places the shader's
/// uniform block after the descriptors, so a short block would have the shader
/// read image state out of its own uniforms.
pub const GLES_MAX_TEX_BINDS: usize = 16;
/// The same, for the vertex stage: the array's capacity, one layout on
/// every GPU. What a GPU takes is [`GLES_VS_TEX_UNITS`].
pub const GLES_MAX_VS_TEX_BINDS: usize = 16;
/// Texture units the vertex stage samples (GL_MAX_VERTEX_TEXTURE_IMAGE_UNITS,
/// the shader's gl_MaxVertexTextureImageUnits), per GPU.
///
/// V3D: 16. Mesa's v3d gives every stage V3D_MAX_TEXTURE_SAMPLERS
/// (src/broadcom/common/v3d_limits.h:39, 24; v3d_screen.c:169-170
/// max_texture_samplers for each stage, the vertex stage included), and
/// the V3D backend has 16 TMU records a stage (src/v3d/backend.rs
/// MAX_UNITS), so 16. It was 8 for
/// every GPU, and dEQP-GLES2 uniform_api reported 15 cases "N vertex
/// texture units required, 8 supported" (N 9..14) on the Pi 4
/// (records/pi4/record_SW28_U1_2c87c2e4.txt).
#[cfg(feature = "gpu-v3d")]
pub const GLES_VS_TEX_UNITS: usize = 16;
/// PowerVR: 16. Mesa's pvr gives every stage 16 samplers
/// (src/imagination/vulkan/pvr_physical_device.c:634,
/// maxPerStageDescriptorSamplers), and the vertex stage's descriptor block
/// takes one slot per sampler the shader declares (`build_desc_block`).
/// Sixteen vertex samplers put descriptors and uniforms past sh255; a direct
/// read of sh256+k returned sh<k> on the AM62P (gl_test 150,
/// records/am62p/shared-size), so the online compiler reads those through
/// idx0 as pco does (`prepare::index_high_shared`). gl_test 149 vsunits
/// checks it on the board (V samplers, one colour each).
#[cfg(feature = "gpu-powervr")]
pub const GLES_VS_TEX_UNITS: usize = 16;
/// GC7000: 16 (vivante/backend.rs `gc_debug_vs_units16`, which says what
/// Mesa gives this core), measured on the i.MX8MP: gl_test 149 with 9..16
/// vertex samplers and dEQP-GLES2 uniform_api 1,267 / 1,267 (RESULTS). It was
/// 8 until 2026-10-01; `gc_debug_set_vs_units16(0)` still selects 8. What a
/// process answers is [`vs_tex_units`].
#[cfg(feature = "gpu-vivante")]
pub const GLES_VS_TEX_UNITS: usize = 16;
const _: () = assert!(GLES_VS_TEX_UNITS <= GLES_MAX_VS_TEX_BINDS);

/// Whether this build's backend draws into a framebuffer with no colour
/// attachment, one whose only attachments are depth and/or stencil
/// (glmark2's shadow pass, any shadow map). Each backend declares it
/// (`DRAWS_WITHOUT_COLOUR` in its own module); the GL core only asks. Where
/// true, such a framebuffer is complete and a draw into it reaches the
/// backend with `target_gpu` 0 at the depth/stencil size and colour writes
/// off; where false, it is GL_FRAMEBUFFER_UNSUPPORTED (ES 2.0 4.4.5).
pub const DEPTH_ONLY_TARGETS: bool = {
    #[cfg(feature = "gpu-vivante")]
    {
        crate::vivante::backend::DRAWS_WITHOUT_COLOUR
    }
    #[cfg(feature = "gpu-v3d")]
    {
        crate::v3d::backend::DRAWS_WITHOUT_COLOUR
    }
    #[cfg(feature = "gpu-powervr")]
    {
        crate::pvr::backend::DRAWS_WITHOUT_COLOUR
    }
};

/// GL_MAX_VERTEX_TEXTURE_IMAGE_UNITS as this process answers it:
/// [`GLES_VS_TEX_UNITS`], except on the GC7000 with the sixteen-unit switch
/// turned off (`gc_debug_vs_units16` = 0), where it is the earlier 8. The
/// query, the shader's gl_MaxVertexTextureImageUnits, the link's
/// sampler-count rule and the draw's vertex bindings all ask here, so they
/// cannot disagree.
pub fn vs_tex_units() -> usize {
    #[cfg(feature = "gpu-vivante")]
    if !crate::vivante::backend::vs_units16_on() {
        return crate::vivante::backend::VS_TEX_UNITS_SWITCHED_OFF;
    }
    GLES_VS_TEX_UNITS
}
/// Whether the fragment stage has highp floats (GLSL ES 1.00 section 4.5.4).
/// Every GPU here computes every float as IEEE binary32 in every stage, which
/// is what glGetShaderPrecisionFormat reports for GL_HIGH_FLOAT
/// (src/gl/shader.rs). The query and the online preprocessor's predefined
/// GL_FRAGMENT_PRECISION_HIGH both read this one fact, so they cannot
/// disagree; a GPU without fragment highp sets it false and gets neither.
pub const FRAGMENT_HIGHP: bool = true;

/// Colour attachments one draw may write.
pub const GLES_MAX_TARGETS: usize = 4;

/// `struct gles_draw_target`: one colour attachment of a draw.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct GlesDrawTarget {
    pub gpu: u64,
    pub w: u32,
    pub h: u32,
    /// `GLES_SURF_*`.
    pub format: u8,
}

impl GlesDrawTarget {
    pub const ZERO: GlesDrawTarget = GlesDrawTarget { gpu: 0, w: 0, h: 0, format: 0 };
}

/* enum gles_textype: what the shader samples the binding as. A cube map, a 2D
 * array and a 3D texture are one stack of `layers` images of width x height;
 * which of the two ways that stack is read is the backend's business:
 *
 *  - the PowerVR's shader does it. glsl2usc rewrites a samplerCube into 2D
 *    sampling of the six stacked faces and a sampler3D into a grid of slices,
 *    so the image the backend binds is simply the whole stack as one 2D
 *    texture and `type_` never comes up.
 *  - the GC7000's texture unit does it, from a descriptor that names the type,
 *    one layer's size and the stride from one layer to the next
 *    (`YakoglBackend::native_layered_textures`).
 */
pub type GlesTexType = u8;
pub const GLES_TEXTYPE_2D: GlesTexType = 0;
pub const GLES_TEXTYPE_CUBE: GlesTexType = 1;
pub const GLES_TEXTYPE_2D_ARRAY: GlesTexType = 2;
pub const GLES_TEXTYPE_3D: GlesTexType = 3;
/// The PowerVR's native cube (`gl::native_cube`, `pvr_native_cube`, default
/// off): a cube map as a Mesa-layout twiddled copy at `base_gpu`, `width` x
/// `height` a face, `bytes` six 4 KiB-aligned faces, to be described as the
/// TPU's own TEXTYPE_CUBE. The name keeps the probe's it grew from.
pub const GLES_TEXTYPE_CUBE_TWIDDLED_PROBE: GlesTexType = 4;

/// `struct gles_texture_bind`: one texture bound to a sampler binding.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct GlesTextureBind {
    pub unit: u8,
    /// `GLES_TEXFMT_*`.
    pub format: u8,
    /// [`GlesTexType`].
    pub type_: u8,
    /// Images in the stack: 6 for a cube, 1 for plain 2D.
    pub layers: u16,
    pub base_gpu: u64,
    pub width: u32,
    pub height: u32,
    pub stride_texels: u32,
    pub bytes: u32,
    pub linear_filter: u8,
    pub min_linear: u8,
    /// 0 no mipmaps, 1 nearest level, 2 linear between levels.
    pub mip_filter: u8,
    pub mip_levels: u8,
    /// Wrap S ([`GlesWrap`]).
    pub wrap_s: u8,
    /// Wrap T, which GL sets independently of S.
    pub wrap_t: u8,
    pub bgra: u8,
    /// GL's incomplete texture: every sample of it reads (0,0,0,1), whatever
    /// the texels hold. The backend binds its own 1x1 opaque black instead of
    /// this texture (GL ES 2.0 3.7.10; the GL core decides what is complete).
    pub incomplete: u8,
    /// 1: the GL base format has no alpha (GL_RGB, GL_LUMINANCE, GL_RED,
    /// GL_RG, the ETC RGB formats, an XRGB8888 image), so sampling must read
    /// alpha 1; the storage is RGBA8 (or BGRX), and an alpha byte nobody
    /// wrote -- a texture rendered into but never cleared -- is not 1.
    pub no_alpha: u8,
    /// The depth compare of a shadow look-up: GL_TEXTURE_COMPARE_FUNC less
    /// GL_NEVER (bit 0 less, bit 1 equal, bit 2 greater: 0 NEVER .. 7
    /// ALWAYS) under GL_COMPARE_REF_TO_TEXTURE; 16 (bit 4: no compare, the
    /// depth itself, ES 3.0.6 3.8.15.1) under GL_NONE; bit 3 set for a
    /// fixed-point depth, whose reference is clamped to [0, 1]. The sampler
    /// object's when one is bound to the unit, else the texture's. A backend
    /// whose shader reads it at run time puts it where the shader looks (the
    /// PowerVR: sampler meta word 0, descriptor dword 12).
    pub compare_op: u8,
    /// 1: a depth texture sampled as GL_OES_depth_texture says, luminance
    /// (d, d, d, 1), on the ES 2.0 surface; its R32F storage alone reads
    /// (d, 0, 0, 1), which is ES 3.0's answer ("treated as RED", ES 3.0.6
    /// 3.8.8) and is what 0 keeps. A backend repeats the red channel into
    /// green and blue.
    pub depth_luminance: u8,
    /// The texels' write stamp (`src/backend.h`): new at every CPU write the
    /// GL core makes, never 0 for such a texture; 0 is "no stamp" (an
    /// EGLImage-backed texture) and is never cached by a backend.
    pub content_gen: u32,
    /// The kick that last rendered into this texture (0: never), the GL
    /// core's render-into stamp (`framebuffer::texture_target_use`, by name).
    /// A backend's derived copy of the texels compares (content_gen,
    /// target_kick) with its own: a render does not move content_gen.
    pub target_kick: u32,
    /// The storage's CPU view (null without one), for a backend that derives
    /// a copy of the texels at draw time.
    pub cpu: *const core::ffi::c_void,
}

/// GL's texture wrap modes, one per axis. The values are GL's three modes,
/// not any hardware's; each backend maps them (the PowerVR TPU's ADDRMODE and
/// the GC7000's TEX_WRAP happen to use the same three numbers).
pub type GlesWrap = u8;
pub const GLES_WRAP_REPEAT: GlesWrap = 0;
pub const GLES_WRAP_MIRRORED_REPEAT: GlesWrap = 1;
pub const GLES_WRAP_CLAMP_TO_EDGE: GlesWrap = 2;

impl GlesTextureBind {
    pub const ZERO: GlesTextureBind = GlesTextureBind {
        unit: 0,
        format: 0,
        type_: GLES_TEXTYPE_2D,
        layers: 0,
        base_gpu: 0,
        width: 0,
        height: 0,
        stride_texels: 0,
        bytes: 0,
        linear_filter: 0,
        min_linear: 0,
        mip_filter: 0,
        mip_levels: 0,
        wrap_s: GLES_WRAP_REPEAT,
        wrap_t: GLES_WRAP_REPEAT,
        bgra: 0,
        incomplete: 0,
        no_alpha: 0,
        compare_op: 0,
        depth_luminance: 0,
        content_gen: 0,
        target_kick: 0,
        cpu: core::ptr::null(),
    };
}

/// `struct gles_draw`: the fully resolved draw the backend renders. Field
/// documentation lives in `src/backend.h`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct GlesDraw {
    pub target_gpu: u64,
    pub target_w: u32,
    pub target_h: u32,
    pub target_format: u8,
    pub samples: u8,
    pub do_clear: bool,
    /// How many alpha bits the colour target's GL format really has: 8 for
    /// RGBA8 and for the default surface, 4 for RGBA4, 1 for RGB5_A1 and 0
    /// for a format with no alpha at all (GL_RGB, GL_RGB565).
    ///
    /// The storage is RGBA8 whatever the format asked for, so the alpha
    /// channel is there in memory even when the format has none; GL says such
    /// a buffer reads 1 (ES 2.0 4.3.1). 0 therefore means "write 1 into the
    /// alpha channel, do not write the fragment's alpha": the PowerVR back end
    /// does it with the PBE's own swizzle (SWIZ.ONE in the alpha lane), which
    /// costs nothing and covers the clear and the draw alike.
    ///
    /// Attachment 0 only. The other attachments (`targets`) keep 8 bits;
    /// ES 2.0 has no MRT, so nothing in the suite renders an RGB format there.
    pub target_alpha_bits: u8,
    pub clear_rgba: [f32; 4],
    /// The clear is an internal full-screen quad rather than the frame's
    /// background object. The background object covers the whole target and
    /// has no scissor and no write masks, so a clear that is scissored, or
    /// that must leave some stencil bits alone, cannot be one. With this set
    /// the backend draws a quad of its own -- its own geometry, vertex and
    /// fragment shader -- writing `clear_rgba` / `clear_depth` /
    /// `clear_stencil` into the pixels this draw's `scissor`, `color_write`,
    /// `depth_write` and `stencil_write_mask` allow; the `do_clear*` bits say
    /// which buffers. `index_count` is 0 and no application shader is
    /// attached, and the frame's own clear state is left alone.
    ///
    /// Not covered: a per-channel colour mask. glColorMask is lowered into
    /// the fragment shader on the PowerVR (docs/gpu-recipes/blending_recipe.md)
    /// and the backend's clear shader has no masked variant, so the quad
    /// writes every channel; the GL core keeps such a clear whole and says so
    /// once.
    pub clear_quad: bool,
    pub do_clear_depth: bool,
    pub do_clear_stencil: bool,
    pub clear_depth: f32,
    pub ds_persist: bool,
    pub ds_id: u32,
    /// Which depth buffer the draw tests and writes, for a backend that
    /// keeps depth and stencil storage per object (the GC7000's D24S8 per
    /// target, the PowerVR's split depth/stencil). 0: none (the depth test
    /// passes, ES 2.0 4.1.6). `0x8000_0000 | page` for the default
    /// framebuffer's (one per window surface, as `ds_id`). Otherwise the
    /// attached object's serial (`GlRenderbuffer::serial` /
    /// `GlTexture::serial`, below 2^30), with bit 30 set for a texture: it
    /// survives the orphan rule moving the object to another slot.
    pub depth_id: u32,
    /// Changes whenever the depth object's storage is specified again
    /// (`storage_gen`): the old contents are gone. Meaningless with id 0.
    pub depth_gen: u32,
    /// As `depth_id` for the stencil attachment; EQUAL to `depth_id` when
    /// one object provides both (a D24S8 on both points).
    pub stencil_id: u32,
    pub stencil_gen: u32,
    /// The depth/stencil storage's size (depth's, else stencil's). ES 3.0
    /// lets it differ from the colour size; the render area is the
    /// intersection.
    pub ds_w: u32,
    pub ds_h: u32,
    /// glClearBuffer(GL_COLOR, n): bit n = `targets[n]` takes the colour
    /// clear. 0: every target, as glClear.
    pub clear_target_mask: u8,

    pub prim: GlesPrim,
    pub line_width: f32,
    pub index_count: u32,
    /// The index buffer; 0 means a non-indexed sequential draw.
    pub index_buf_gpu: u64,
    /// Bytes per index (1, 2 or 4). The GL core stages u8 and u32 arrays into
    /// u16 today, so this is 2 whenever `index_buf_gpu` is set; a backend
    /// whose vertex fetch takes the other widths reads it rather than
    /// assuming.
    pub index_size: u8,
    pub index_cpu: *const c_void,
    pub index_bytes: u32,
    pub index_count2: u32,
    pub index_buf2_gpu: u64,

    pub vs: *const ShcShader,
    pub fs: *const ShcShader,
    pub vs_usc_gpu: u64,
    pub fs_usc_gpu: u64,

    pub attribs: [GlesAttrib; 16],
    pub num_attribs: u8,
    /// ES 3.0 instanced draw: instances (0 or 1: one).
    pub instance_count: u32,
    pub vs_output_size_dwords: u32,

    pub vs_uniform_gpu: u64,
    pub vs_uniform_dwords: u16,
    pub vs_uniform_cpu: *const c_void,
    pub fs_uniform_gpu: u64,
    pub fs_uniform_dwords: u16,
    pub fs_uniform_cpu: *const c_void,

    /// Colour attachments beyond the first: a G-buffer pass writes three, and
    /// the fragment shader's outputs po0..poN go to them in order. `[0]`
    /// mirrors `target_*`; `num_targets` 0 means one.
    pub targets: [GlesDrawTarget; GLES_MAX_TARGETS],
    pub num_targets: u8,

    pub textures: [GlesTextureBind; GLES_MAX_TEX_BINDS],
    pub num_textures: u8,
    /// The vertex stage's own texture descriptors (Qt Quick 3D reads bone and
    /// morph-target matrices from a texture in the vertex shader).
    pub vs_textures: [GlesTextureBind; GLES_MAX_VS_TEX_BINDS],
    pub num_vs_textures: u8,

    pub viewport: [i32; 4],
    pub depth_range: [f32; 2],
    pub flip_y: bool,
    pub depth_test: bool,
    pub depth_func: GlesCmp,
    pub depth_write: bool,
    pub polygon_offset: bool,
    pub polygon_offset_factor: f32,
    pub polygon_offset_units: f32,
    /// Blending is in the FS on the PowerVR (`fs` is the variant compiled for
    /// the blend state, `fs.uses_fbfetch`) and the flag only documents it;
    /// every object is already submitted TRANSLUCENT (in order, background
    /// readable). A backend with fixed-function blending (the Vivante PE)
    /// programs the state below instead. Factors and equations are
    /// `shc_blend_factor` / `shc_blend_func`, the codes the shader variant
    /// key uses.
    pub blend: bool,
    pub blend_src_rgb: u8,
    pub blend_dst_rgb: u8,
    pub blend_src_alpha: u8,
    pub blend_dst_alpha: u8,
    pub blend_eq_rgb: u8,
    pub blend_eq_alpha: u8,
    /// glBlendColor (the CONSTANT_* factors).
    pub blend_color: [f32; 4],
    /// glColorMask: bit 0 R, 1 G, 2 B, 3 A.
    pub color_mask: u8,
    pub cull: GlesCull,
    pub scissor: bool,
    pub scissor_rect: [i32; 4],
    pub stencil: bool,
    pub front_cw: bool,
    pub stencil_func: GlesCmp,
    pub stencil_ref: u8,
    pub stencil_cmp_mask: u8,
    pub stencil_write_mask: u8,
    pub stencil_sfail: GlesStencilOp,
    pub stencil_dpfail: GlesStencilOp,
    pub stencil_dppass: GlesStencilOp,
    pub stencil_back_func: GlesCmp,
    pub stencil_back_ref: u8,
    pub stencil_back_cmp_mask: u8,
    pub stencil_back_write_mask: u8,
    pub stencil_back_sfail: GlesStencilOp,
    pub stencil_back_dpfail: GlesStencilOp,
    pub stencil_back_dppass: GlesStencilOp,
    pub clear_stencil: u8,
    pub color_write: bool,
    /// The colour channels the pending colour clear may write (glColorMask
    /// at the glClear, bit 0 R .. 3 A), meaningful when `do_clear`. Its own
    /// field because a clear fused into a draw carries its glClear's mask,
    /// not the draw's `color_mask`. 0 is "not given" and means every
    /// channel; the GL core never sends `do_clear` with an empty mask.
    pub clear_color_mask: u8,
    /// The stencil bits the pending stencil clear may change (glStencilMask
    /// at the glClear), meaningful when `do_clear_stencil`. Its own field
    /// for `clear_color_mask`'s reason: fused into a draw, the draw's
    /// `stencil_write_mask` is the draw's, not the clear's. 0 is "not given"
    /// and means `stencil_write_mask`; the GL core never sends a stencil
    /// clear with an empty mask.
    pub clear_stencil_write_mask: u8,
    /// The instance this draw is when the GL core draws an instanced draw
    /// instance by instance (a divisor past the backend's
    /// `max_instance_divisor`): gl_InstanceID's value; 0 for every other
    /// draw. A backend that cannot give it refuses such a draw by name.
    pub instance_id: u32,
    /// The samples a multisampled draw writes (bit n: sample n): the mask
    /// Mesa's state tracker makes of glSampleCoverage (st_atom_msaa.c
    /// st_update_sample_state): the first floor(value x samples) samples,
    /// inverted with GL_SAMPLE_COVERAGE_INVERT, only when GL_SAMPLE_COVERAGE
    /// is on and the target has more than one sample; 0xf otherwise. 0 is
    /// a real value (coverage 0.0: no sample written).
    pub sample_mask: u8,
    /// GL_SAMPLE_ALPHA_TO_COVERAGE is on and the target has more than one
    /// sample.
    pub alpha_to_coverage: bool,
    /// Attachment 0's row pitch in pixels when it is not
    /// `tex_pitch(target_w)`: the default surface of a window that is a view
    /// into a wider scan-out buffer (`GlContext::default_pitch`). 0: the
    /// packed pitch, every FBO and pbuffer. Read through [`draw_pitch`].
    /// Sits in what was tail padding.
    pub target_pitch: u32,
}

/// Attachment 0's row pitch in pixels: [`GlesDraw::target_pitch`], else the
/// pair-padded `tex_pitch(target_w)` every GL-core surface is stored at.
pub fn draw_pitch(d: &GlesDraw) -> u32 {
    if d.target_pitch != 0 {
        d.target_pitch
    } else {
        d.target_w.wrapping_add(1) & !1
    }
}

impl GlesDraw {
    pub const ZERO: GlesDraw = GlesDraw {
        target_gpu: 0,
        target_w: 0,
        target_h: 0,
        target_format: 0,
        target_alpha_bits: 8,
        samples: 0,
        do_clear: false,
        clear_rgba: [0.0; 4],
        clear_quad: false,
        do_clear_depth: false,
        do_clear_stencil: false,
        clear_depth: 0.0,
        ds_persist: false,
        ds_id: 0,
        depth_id: 0,
        depth_gen: 0,
        stencil_id: 0,
        stencil_gen: 0,
        ds_w: 0,
        ds_h: 0,
        clear_target_mask: 0,
        prim: 0,
        line_width: 0.0,
        index_count: 0,
        index_buf_gpu: 0,
        index_size: 0,
        index_cpu: core::ptr::null(),
        index_bytes: 0,
        index_count2: 0,
        index_buf2_gpu: 0,
        vs: core::ptr::null(),
        fs: core::ptr::null(),
        vs_usc_gpu: 0,
        fs_usc_gpu: 0,
        attribs: [GlesAttrib::ZERO; 16],
        num_attribs: 0,
        instance_count: 0,
        vs_output_size_dwords: 0,
        vs_uniform_gpu: 0,
        vs_uniform_dwords: 0,
        vs_uniform_cpu: core::ptr::null(),
        fs_uniform_gpu: 0,
        fs_uniform_dwords: 0,
        fs_uniform_cpu: core::ptr::null(),
        targets: [GlesDrawTarget::ZERO; GLES_MAX_TARGETS],
        num_targets: 0,
        textures: [GlesTextureBind::ZERO; GLES_MAX_TEX_BINDS],
        num_textures: 0,
        vs_textures: [GlesTextureBind::ZERO; GLES_MAX_VS_TEX_BINDS],
        num_vs_textures: 0,
        viewport: [0; 4],
        depth_range: [0.0; 2],
        flip_y: false,
        depth_test: false,
        depth_func: 0,
        depth_write: false,
        polygon_offset: false,
        polygon_offset_factor: 0.0,
        polygon_offset_units: 0.0,
        blend: false,
        blend_src_rgb: 0,
        blend_dst_rgb: 0,
        blend_src_alpha: 0,
        blend_dst_alpha: 0,
        blend_eq_rgb: 0,
        blend_eq_alpha: 0,
        blend_color: [0.0; 4],
        color_mask: 0,
        cull: 0,
        scissor: false,
        scissor_rect: [0; 4],
        stencil: false,
        front_cw: false,
        stencil_func: 0,
        stencil_ref: 0,
        stencil_cmp_mask: 0,
        stencil_write_mask: 0,
        stencil_sfail: 0,
        stencil_dpfail: 0,
        stencil_dppass: 0,
        stencil_back_func: 0,
        stencil_back_ref: 0,
        stencil_back_cmp_mask: 0,
        stencil_back_write_mask: 0,
        stencil_back_sfail: 0,
        stencil_back_dpfail: 0,
        stencil_back_dppass: 0,
        clear_stencil: 0,
        color_write: false,
        clear_color_mask: 0,
        clear_stencil_write_mask: 0,
        instance_id: 0,
        sample_mask: 0xf,
        alpha_to_coverage: false,
        target_pitch: 0,
    };
}

/// `struct yakogl_backend`: the backend's entry points, one table per GPU.
#[repr(C)]
pub struct YakoglBackend {
    pub name: *const c_char,
    /// Vertex attribute source formats the backend fetches without help (a
    /// mask of [`gles_attribf_bit`]); float32 is always included.
    pub native_attrib_formats: u32,
    /// True when this GPU stores an RGBA8888 colour target as B,G,R,A.
    ///
    /// Some pixel engines have no RGBA8888 render format at all -- the
    /// GC7000's 32-bit formats are A8R8G8B8 and X8R8G8B8 -- so what they
    /// write is byte-swapped against what GL calls RGBA8888. Sampling such a
    /// target as a texture is corrected by the descriptor's own format, so
    /// the only path that sees the raw order is the one that hands the bytes
    /// to the application: glReadPixels and what is built on it.
    pub rt_rgba8888_is_bgra: bool,
    /// Non-zero when this GPU reads a mip chain laid out its own way, and the
    /// byte alignment it wants each level's base at. Zero (the default) keeps
    /// the packed layout: every level as wide as it is, one level straight
    /// after the last.
    ///
    /// A GPU that walks the chain itself needs one base address and one
    /// stride and derives the rest -- that is the PowerVR's texture image
    /// word, and the packed layout is exactly what it wants. The GC7000
    /// instead names all sixteen level addresses in its descriptor, but still
    /// carries a single LINEAR_STRIDE for the whole chain, and it fetches a
    /// level from a 64-byte boundary. So a level of its chain has to be
    /// padded to level 0's row pitch *and* start on a multiple of this, or
    /// the hardware reads one level where another was meant.
    ///
    /// Mesa does both for the same part: it pads every level of a mipmapped
    /// texture to one super-tile width, and `etnaviv_resource.c` aligns each
    /// level to ETNA_PE_ALIGNMENT with the comment "align levels to 64 bytes
    /// to be able to render to them".
    ///
    /// It costs some memory in the tail levels, which is the whole price.
    pub mip_level_align: u32,
    /// True when this GPU reads a block-compressed texture as plain raster
    /// rows of 4x4 blocks: ceil(w/4) blocks in a row, one row after the next,
    /// with the row pitch given to the hardware. False (the default) is what
    /// the PowerVR TPU walks instead -- the blocks of a power-of-two block
    /// grid in twiddled order, which needs no pitch.
    ///
    /// Mesa's etnaviv gives a compressed resource ETNA_LAYOUT_LINEAR with both
    /// paddings at 1 ("compressed textures don't use tiling, they have their
    /// own tiles"), so the GC7000 wants the raster rows and no padding at all.
    pub compressed_blocks_in_rows: bool,
    /// True when this GPU's texture unit samples a cube map, a 2D array and a
    /// 3D texture itself, given the type, one layer's size and the stride
    /// between layers. The GL core then stacks the layers plainly -- no grid
    /// of 3D slices -- and fills in `GlesTextureBind::type_`/`layers`.
    ///
    /// False (the default) is the PowerVR's arrangement, where the shader does
    /// the work and the backend only ever sees a 2D image.
    pub native_layered_textures: bool,
    /// True when this GPU's `gl_FrontFacing` has no sense of its own -- the
    /// fragment reads a raw 0.0/1.0 and the comparison against it is compiled
    /// in, so the winding is a shader variant rather than a piece of draw
    /// state. The GL core then asks the shader backend for the
    /// [`SHC_VIV_KEY_FRONT_FLIPPED`](crate::shaderc::SHC_VIV_KEY_FRONT_FLIPPED)
    /// binary whenever the effective winding is reversed, and only for shaders
    /// that read `gl_FrontFacing` at all.
    ///
    /// False (the default) is the PowerVR, where the driver writes the
    /// front-face op into a shared register at draw time
    /// (`ShcShader::fs_front_face_off1`) and one binary serves both.
    pub front_facing_shader_variant: bool,
    /// True when this GPU does not sample ETC1 itself, and the GL core
    /// decodes a `GL_ETC1_RGB8_OES` image to RGBA8 at
    /// `glCompressedTexImage2D`, as Mesa's state tracker does for a driver
    /// with no ETC1 format (`st_format.c`, `st->has_etc1`). The texture is
    /// then an RGB texture like any other, with mip levels and cube faces.
    ///
    /// False (the default) is the PowerVR and the GC7000, whose texture
    /// units read ETC1 blocks.
    pub etc1_decoded_at_upload: bool,
    pub init: Option<unsafe extern "C" fn() -> i32>,
    pub shutdown: Option<unsafe extern "C" fn()>,
    /// Record one draw into the current frame (submits a frame if it must).
    /// Returns 0 or a negative errno; see [`draw`] for the contract.
    pub draw: Option<unsafe extern "C" fn(draw: *const GlesDraw) -> i32>,
    /// Submit the current frame; does not wait for it.
    pub flush: Option<unsafe extern "C" fn() -> i32>,
    /// Wait for every submitted frame.
    pub finish: Option<unsafe extern "C" fn() -> i32>,
    /// Wait for the frame before the last submitted one (pipelined swap).
    pub finish_previous: Option<unsafe extern "C" fn() -> i32>,
    pub kick_id: Option<unsafe extern "C" fn() -> u32>,
    pub kick_pending: Option<unsafe extern "C" fn(kick_id: u32) -> bool>,
    pub heap_base: Option<unsafe extern "C" fn(heap: GlesHeap) -> u64>,
    pub counters: Option<unsafe extern "C" fn(kicks: *mut u32, waits: *mut u32)>,
    /// Texel storage `[base_gpu, base_gpu + bytes)` is about to be released:
    /// called before the OSAL's free is scheduled (`src/backend.h`). `None`
    /// on a backend that keeps nothing for texel memory.
    pub texture_released: Option<unsafe extern "C" fn(base_gpu: u64, bytes: u64)>,
    /// `eglSwapBuffers` is posting the window surface whose colour buffer is
    /// at `surface_gpu`, before the swap's flush (`src/backend.h`): its
    /// depth/stencil are undefined afterwards (EGL 1.5 3.10.1). `None` on a
    /// backend that does nothing with it.
    pub surface_swapped: Option<unsafe extern "C" fn(surface_gpu: u64)>,
    /// `glTexImage2D` / `glTexSubImage2D` stored the rows at `src` (pitch
    /// `stride` bytes: GL_UNPACK_ROW_LENGTH texels, or w, times 4, rounded up
    /// to GL_UNPACK_ALIGNMENT; caller's memory, valid for the call only) into the box
    /// of level `level` of the 2D texture `b` describes, after the GL core's
    /// own store, only when those bytes are the stored texels (RGBA /
    /// UNSIGNED_BYTE, no swap; never a copy, a cube face or a conversion).
    /// `b`'s key fields (base_gpu, content_gen after this write, size, pitch,
    /// bytes, format, bgra) equal a later draw's; its sampling fields do not
    /// have to. The full contract is in `src/backend.h`. `None` on a backend
    /// that keeps nothing.
    #[allow(clippy::type_complexity)]
    pub texture_uploaded: Option<
        unsafe extern "C" fn(
            b: *const GlesTextureBind,
            level: u32,
            x: u32,
            y: u32,
            w: u32,
            h: u32,
            src: *const c_void,
            stride: u32,
        ),
    >,
    /// The largest divisor an instanced draw's arrays may have to be handed
    /// over as one draw (`GlesAttrib::divisor`); 0 or 1: divisor 1 only.
    pub max_instance_divisor: u32,
    /// Wait until kick `kick_id` (already submitted) has completed, submitting
    /// nothing recorded after it: 0, or a negative errno. `None` keeps the
    /// sync waits on `finish`.
    pub wait_kick: Option<unsafe extern "C" fn(kick_id: u32) -> i32>,
    /// True when a draw never reads a texture's storage on the GPU (the
    /// backend samples CPU-made copies): a CPU write into a texture a
    /// pending kick only sampled needs no wait.
    pub samples_texel_copies: bool,
    /// The depth/stencil contents of GL object `id` are gone
    /// ([`attachment_gone`]): a backend that keeps a depth/stencil buffer for
    /// it frees that slot. `None` on a backend that keeps none.
    pub attachment_gone: Option<unsafe extern "C" fn(id: u32)>,
    /// Write the depth plane the backend keeps for GL object `id` into a
    /// depth texture's texels ([`depth_resolve`]; `src/backend.h`). `None`
    /// on a backend that keeps none or cannot read it back.
    pub depth_resolve: Option<unsafe extern "C" fn(id: u32, dst: *mut f32, w: u32, h: u32, pitch: u32) -> i32>,
}

// SAFETY: the table is immutable; `name` points at a static string.
unsafe impl Sync for YakoglBackend {}

/// The GPU backend of this build (`extern const struct yakogl_backend
/// *g_backend;`): the one the GPU feature selected, exactly one per build as
/// in the C (`-DYAKOGL_GPU_VIVANTE`).
#[cfg(all(not(feature = "host-test"), feature = "gpu-powervr"))]
#[no_mangle]
pub static mut g_backend: *const YakoglBackend = &crate::pvr::backend::yakogl_backend_pvr;

/// The GPU backend of this build; see above.
#[cfg(all(not(feature = "host-test"), feature = "gpu-vivante"))]
#[no_mangle]
pub static mut g_backend: *const YakoglBackend = &crate::vivante::backend::yakogl_backend_vivante;

/// The GPU backend of this build; see above.
#[cfg(all(not(feature = "host-test"), feature = "gpu-v3d"))]
#[no_mangle]
pub static mut g_backend: *const YakoglBackend = &crate::v3d::backend::yakogl_backend_v3d;

/// Host test builds (`host-test`) start without a backend: the PowerVR table
/// would pull the driver's C command generators into every test binary. A test
/// installs its own table (`testutil::TestGl` installs a recording mock).
#[cfg(feature = "host-test")]
#[no_mangle]
pub static mut g_backend: *const YakoglBackend = core::ptr::null();

#[inline]
fn table() -> Option<&'static YakoglBackend> {
    // SAFETY: g_backend is null or points at a static table.
    unsafe { g_backend.as_ref() }
}

/* The core's view of the backend (the yakogl_* inlines of backend.h). Every
 * required entry is present in a well-formed table; a missing one reports
 * -ENODEV rather than crashing. */

pub fn init() -> i32 {
    // SAFETY: table entries are valid functions of the documented signature.
    table().and_then(|t| t.init).map_or(-crate::util::ENODEV, |f| unsafe { f() })
}

pub fn shutdown() {
    if let Some(f) = table().and_then(|t| t.shutdown) {
        // SAFETY: as above.
        unsafe { f() }
    }
}

/// `E2BIG`: a draw the backend cannot encode at its size (see [`draw`]).
pub const E2BIG: i32 = 7;

/// Hand one resolved draw to the backend.
///
/// The return contract (every backend implements it; the core maps it):
///
/// | value | meaning | the core |
/// |---|---|---|
/// | `0` | recorded (with `index_count == 0`: the clear was recorded) | nothing |
/// | `-ENOMEM` | not recorded: GPU memory ran out -- the draw does not fit an empty frame (staged arrays or indices larger than the backend's per-frame arenas), or allocating its sequential index buffer failed, or it tests or clears a depth/stencil attachment whose buffer could not be allocated (or whose contents were lost that way and no clear has covered since) | `GL_OUT_OF_MEMORY` |
/// | `-E2BIG` | not recorded: the draw is larger than the backend can encode (PowerVR: a non-indexed draw of more than 65535 vertices as a fan, a line loop, or with a vertex shader that reads gl_VertexID) | `GL_OUT_OF_MEMORY` |
/// | `-EINVAL` | not recorded: a malformed or unsupported table (no target, no linked shaders, 0 or more than 16 attributes, a non-float attribute, staged indices shorter than `index_count`, a program the command generators reject) -- a core bug or an unsupported feature, not an application error | `GL_OUT_OF_MEMORY` (see below) |
/// | `-ENOTSUP` | not recorded: the GL state is legal but the backend cannot render it yet (V3D: alpha-to-coverage); the backend's log names the cause -- a backend gap, not a core bug and not an application error | `GL_INVALID_OPERATION` (see below) |
/// | `-ENODEV` | no backend, or not initialised | `GL_OUT_OF_MEMORY` |
/// | any other negative value (`-EIO`, `-ETIMEDOUT`, a platform submit error) | a frame the draw had to submit first could not be submitted or waited for, or the backend's memory has no CPU mapping; the draw was not recorded (a frame that failed to submit is dropped: the next draw starts a new one) | `GL_OUT_OF_MEMORY` |
///
/// What the value means is fixed above; how the core reacts is the core's
/// choice. `gl::draw` treats every negative value alike (one log line and
/// `GL_OUT_OF_MEMORY`, the only error ES 3.0 offers for "the draw was
/// lost") except `-ENOTSUP`, which it reports as `GL_INVALID_OPERATION`:
/// a harness treats `GL_OUT_OF_MEMORY` as fatal and ends its run (dEQP's
/// ResourceError), while a backend gap should fail one case and let the run
/// go on. Either way a backend must return a negative value only for a draw
/// it did not record.
///
/// A draw the backend splits (a sequential draw of more than 65535 vertices
/// is recorded in pieces) can fail after some pieces were recorded: the
/// error is returned and those pieces stay in the frame. A draw that does
/// not fit the open frame is never an error: the backend submits the frame
/// and records the draw into the next one (that kick loads the colour back).
/// The return value is never positive.
pub fn draw(d: &GlesDraw) -> i32 {
    // SAFETY: `d` is a valid reference for the call.
    table().and_then(|t| t.draw).map_or(-crate::util::ENODEV, |f| unsafe { f(d) })
}

/// How many times the backend reported that submitted GPU work did not
/// complete: a flush that could not submit, or a wait that timed out, hit a
/// bus error, or found its kick thrown away by a GPU reset. Sticky: it only
/// grows, so a sample reads it once at the end and knows whether any picture
/// in the run was never drawn. [`yakogl_gpu_work_lost`] is the C view.
///
/// It exists because the loss used to be silent at the GL layer: the GC7000
/// driver resets the GPU after a fence timeout and says so (-ECANCELED), and
/// glReadPixels ignored that and read a target the lost kick never drew --
/// a plausible wrong picture with no error anywhere (color_clear on the
/// i.MX8MP, 2026-09-24). ES 2.0/3.0 have no error code for it
/// (GL_CONTEXT_LOST is robustness / ES 3.2), so none is invented: the data
/// is withheld, this counts, and the first loss is logged.
static GPU_WORK_LOST: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);

/// Count a backend failure that lost GPU work; log the first one.
fn note_work_lost(what: &str, rc: i32) {
    let before = GPU_WORK_LOST.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    if before == 0 {
        glog!(
            "yakogl: GPU work lost: backend {} returned {}; results that depended on it are \
             withheld (yakogl_gpu_work_lost counts every loss)",
            what,
            rc
        );
    }
}

/// See [`GPU_WORK_LOST`].
pub fn gpu_work_lost() -> u32 {
    GPU_WORK_LOST.load(core::sync::atomic::Ordering::Relaxed)
}

/// C view of [`gpu_work_lost`]: how many times submitted GPU work did not
/// complete in this process. A sample reads it at the end of a run; non-zero
/// means some result it read may be of a picture that was never drawn.
#[no_mangle]
pub extern "C" fn yakogl_gpu_work_lost() -> u32 {
    gpu_work_lost()
}

/// Draws the GL core refused because it could not draw them as asked: an
/// internal program the draw needs (a blend variant, the GC7000's
/// front-face twin) does not exist, build, link or fit. Drawn with the
/// plain program instead they would be a picture of another state (the
/// PowerVR's subtract variants never built and fragment_ops drew them as
/// replaces, 2026-09-24). ES has no error for it, so none is invented:
/// counted, every one, and each cause logged when it first happens.
/// [`yakogl_draws_refused`] is the C view; read it as a difference (it
/// wraps at 2^32).
static DRAWS_REFUSED: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);

/// Count a refused draw; log it the first time a draw is refused.
pub fn note_draw_refused(why: &str, key: u32) {
    let before = DRAWS_REFUSED.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    if before == 0 {
        glog!("yakogl: a draw was refused: {} (key 0x{:x}); yakogl_draws_refused counts every one", why, key);
    }
}

/// See [`DRAWS_REFUSED`].
pub fn draws_refused() -> u32 {
    DRAWS_REFUSED.load(core::sync::atomic::Ordering::Relaxed)
}

/// C view of [`draws_refused`]: how many draws the GL core refused in this
/// process because an internal program they need could not be had.
#[no_mangle]
pub extern "C" fn yakogl_draws_refused() -> u32 {
    draws_refused()
}

/// A backend entry point's result, counted when it lost work. No backend at
/// all (-ENODEV from the wrappers below) submitted nothing, so lost nothing.
fn counted(what: &str, rc: Option<i32>) -> i32 {
    match rc {
        None => -crate::util::ENODEV,
        Some(0) => 0,
        Some(rc) => {
            note_work_lost(what, rc);
            rc
        }
    }
}

pub fn flush() -> i32 {
    // SAFETY: as above.
    counted("flush", table().and_then(|t| t.flush).map(|f| unsafe { f() }))
}

/// Wait for every submitted kick. Non-zero means some of that work did not
/// complete: the caller must not read or hand out anything it produced (see
/// [`GPU_WORK_LOST`]).
pub fn finish() -> i32 {
    // SAFETY: as above.
    counted("finish", table().and_then(|t| t.finish).map(|f| unsafe { f() }))
}

pub fn finish_previous() -> i32 {
    // SAFETY: as above.
    counted("finish_previous", table().and_then(|t| t.finish_previous).map(|f| unsafe { f() }))
}

pub fn kick_id() -> u32 {
    // SAFETY: as above.
    table().and_then(|t| t.kick_id).map_or(0, |f| unsafe { f() })
}

pub fn kick_pending(id: u32) -> bool {
    // SAFETY: as above.
    table().and_then(|t| t.kick_pending).is_some_and(|f| unsafe { f(id) })
}

/// Does this build's backend write only the channels `clear_color_mask`
/// names? The GL core reports a partially masked clear once when it does not.
/// The GC7000 does while `gc_debug_masked_clear` is on; the PowerVR while
/// `pvr_debug_masked_clear` is (its masked clear programs, `CLEAR_MASKED`).
pub fn masked_clear_applied() -> bool {
    #[cfg(feature = "gpu-vivante")]
    {
        crate::vivante::backend::masked_clear_on()
    }
    /* The V3D's TLB clear writes every channel, so src/v3d draws a
     * partially masked colour clear as the clear quad, with the mask as its
     * COLOR_WRITE_MASKS (`v3d::state::colour_needs_quad`). */
    #[cfg(feature = "gpu-v3d")]
    {
        true
    }
    #[cfg(feature = "gpu-powervr")]
    {
        crate::pvr::backend::masked_clear_on()
    }
}

/// Does this build's backend sample GL_LUMINANCE, GL_ALPHA and
/// GL_LUMINANCE_ALPHA textures stored narrow ([`GLES_TEXFMT_L8`] and the
/// others)? The PowerVR while `pvr_narrow_la` is on; the GC7000 while
/// `gc_debug_narrow_la` is (opt-in until the board has measured it); the V3D
/// always, as the TMU's R8 and RG8 with a swizzle (Mesa's L8_UNORM, A8_UNORM
/// and L8A8_UNORM rows; the V3D's `texture::tmu_format`).
pub fn narrow_la_texels() -> bool {
    #[cfg(feature = "gpu-vivante")]
    {
        crate::vivante::backend::narrow_la_on()
    }
    #[cfg(feature = "gpu-v3d")]
    {
        true
    }
    #[cfg(feature = "gpu-powervr")]
    {
        crate::pvr::backend::narrow_la_on()
    }
}

/// The vertex attribute source formats the backend fetches without help
/// (a mask of [`gles_attribf_bit`]); none when there is no backend.
pub fn native_attrib_formats() -> u32 {
    // SAFETY: as above.
    table().map_or(0, |t| t.native_attrib_formats)
}

/// True when the backend's pixel engine stores an RGBA8888 colour target as
/// B,G,R,A (`YakoglBackend::rt_rgba8888_is_bgra`).
pub fn rt_rgba8888_is_bgra() -> bool {
    // SAFETY: as above.
    table().is_some_and(|t| t.rt_rgba8888_is_bgra)
}

/// The byte alignment the backend wants each mip level's base at, and with it
/// one row pitch for the whole chain; 0 for the packed layout
/// (`YakoglBackend::mip_level_align`).
pub fn mip_level_align() -> u32 {
    // SAFETY: as above.
    table().map_or(0, |t| t.mip_level_align)
}

/// True when the backend reads a block-compressed texture as raster rows of
/// blocks (`YakoglBackend::compressed_blocks_in_rows`).
pub fn compressed_blocks_in_rows() -> bool {
    // SAFETY: as above.
    table().is_some_and(|t| t.compressed_blocks_in_rows)
}

/// True when the backend's texture unit walks a layered texture itself
/// (`YakoglBackend::native_layered_textures`).
pub fn native_layered_textures() -> bool {
    // SAFETY: as above.
    table().is_some_and(|t| t.native_layered_textures)
}

/// True when the backend needs a separate binary per front-face winding
/// (`YakoglBackend::front_facing_shader_variant`).
pub fn front_facing_shader_variant() -> bool {
    // SAFETY: as above.
    table().is_some_and(|t| t.front_facing_shader_variant)
}

/// True when the GL core decodes ETC1 images at upload
/// The largest divisor an instanced draw's arrays may have to be handed over
/// as one draw (`YakoglBackend::max_instance_divisor`, 0 read as 1).
pub fn max_instance_divisor() -> u32 {
    table().map_or(1, |t| t.max_instance_divisor.max(1))
}

/// Wait for kick `kick_id` alone (`YakoglBackend::wait_kick`): `None` when
/// the backend has no such wait, and the caller then finishes.
pub fn wait_kick(kick_id: u32) -> Option<i32> {
    // SAFETY: the backend's own entry point.
    table().and_then(|t| t.wait_kick).map(|f| counted("wait_kick", Some(unsafe { f(kick_id) })))
}

/// (`YakoglBackend::samples_texel_copies`).
pub fn samples_texel_copies() -> bool {
    table().is_some_and(|t| t.samples_texel_copies)
}

/// (`YakoglBackend::etc1_decoded_at_upload`).
pub fn etc1_decoded_at_upload() -> bool {
    // SAFETY: as above.
    table().is_some_and(|t| t.etc1_decoded_at_upload)
}

/// Tell the backend that texel storage is about to be released (see
/// [`YakoglBackend::texture_released`]); nothing when it keeps nothing.
pub fn texture_released(base_gpu: u64, bytes: u64) {
    if let Some(f) = table().and_then(|t| t.texture_released) {
        // SAFETY: a static table's entry, called with plain values.
        unsafe { f(base_gpu, bytes) }
    }
}

/// Tell the backend that the depth/stencil contents of the GL object `id`
/// are gone, so the slot it keeps for them is free
/// ([`YakoglBackend::attachment_gone`], `src/ds_retention.rs`). `id` is in
/// the form of [`GlesDraw::depth_id`]: a renderbuffer's serial, a texture's
/// serial with bit 30, or `0x8000_0000 | page` for a window surface's own
/// depth/stencil. The GL core calls it when the object is freed (deleted
/// and held by no framebuffer, or its share group's last context went),
/// when its storage is specified again (its old contents are undefined),
/// and when a surface is destroyed. A backend never learns this any other
/// way, and keeps a buffer that holds contents until it does: ES 2.0 4.4.3
/// keeps a renderbuffer's image across framebuffer switches.
pub fn attachment_gone(id: u32) {
    if let Some(f) = table().and_then(|t| t.attachment_gone) {
        // SAFETY: a static table's entry, called with a plain value.
        unsafe { f(id) }
    }
}

/// Write the depth the backend keeps for GL object `id` (the
/// [`GlesDraw::depth_id`] form: a depth texture's serial with bit 30) into
/// `w` x `h` floats at `dst`, `pitch` floats a row: the depth in [0, 1] as
/// the texture's R32F texels hold it
/// ([`YakoglBackend::depth_resolve`]). The GL core calls it when a draw is
/// about to sample a depth texture, or the CPU is about to write one, after
/// a kick rendered into it; every kick that did has completed. 0: written;
/// 1: the backend holds nothing for it (never written, or undefined), and
/// the texels are left as they are; a negative errno otherwise. A backend
/// without the entry answers 1.
///
/// # Safety
/// `dst` holds `h` rows of `pitch` floats, `w <= pitch`.
pub unsafe fn depth_resolve(id: u32, dst: *mut f32, w: u32, h: u32, pitch: u32) -> i32 {
    match table().and_then(|t| t.depth_resolve) {
        // SAFETY: a static table's entry; the caller's contract covers `dst`.
        Some(f) => unsafe { f(id, dst, w, h, pitch) },
        None => 1,
    }
}

/// Tell the backend that `eglSwapBuffers` is posting the surface at
/// `surface_gpu` (see [`YakoglBackend::surface_swapped`]).
pub fn surface_swapped(surface_gpu: u64) {
    if let Some(f) = table().and_then(|t| t.surface_swapped) {
        // SAFETY: a static table's entry, called with a plain value.
        unsafe { f(surface_gpu) }
    }
}

/// Tell the backend that an upload stored the caller's rows (see
/// [`YakoglBackend::texture_uploaded`]).
///
/// # Safety
/// `src` holds h rows of `stride` bytes, w texels each, for the call.
#[allow(clippy::too_many_arguments)]
pub unsafe fn texture_uploaded(
    b: &GlesTextureBind,
    level: u32,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    src: *const c_void,
    stride: u32,
) {
    if let Some(f) = table().and_then(|t| t.texture_uploaded) {
        // SAFETY: a static table's entry; `b` is live for the call and `src`
        // holds h rows of `stride` bytes (the caller's contract).
        unsafe { f(b, level, x, y, w, h, src, stride) }
    }
}

pub fn heap_base(heap: GlesHeap) -> u64 {
    // SAFETY: as above.
    table().and_then(|t| t.heap_base).map_or(0, |f| unsafe { f(heap) })
}
