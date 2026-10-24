// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The GL context: every bound object and all render state, the context
//! pool, the current-context pointer, the error setter and the object-table
//! lookups (`gl_context.h` / `gl_context.c`).
//!
//! Object handles are small integers indexing fixed tables (embedded
//! friendly: no per-object allocation; sizes are compile-time caps). The
//! structs are `#[repr(C)]` and mirror `src/gl_context.h` field
//! for field: the on-silicon test suite inspects them through
//! `gl_lookup_*()`. Every field is valid when all-zero (the pool lives in
//! `.bss` and a context is reset with a zero fill).

use core::ffi::{c_char, c_void, CStr};
use core::sync::atomic::{AtomicBool, AtomicPtr, AtomicU32, Ordering};

use super::consts::*;
use super::types::*;
use crate::backend::{self, GLES_HEAP_DATA};
use crate::osal::{self, OsalGpuMem, OSAL_MEM_DATA};
use crate::shaderc::ShcShader;
use crate::util::{Global, EINVAL};

/// Contexts that can exist at once (EGL contexts). Each one costs a few MB
/// of static state -- the context pool, the program, shader and draw state
/// are all arrays of this length -- so the default is the two an application
/// with a worker context needs, and a build that needs more says so:
/// `-DYAKOGL_MAX_CONTEXTS=<n>` (CMake), which is also `GL_MAX_CONTEXTS` for
/// the C headers. dEQP's `color_clears.multi_context` makes three.
///
/// The C had this knob (`#ifndef GL_MAX_CONTEXTS` in `gl_context.h`); the
/// first translation into Rust fixed the number instead, and a CTS run with
/// three contexts ended in EGL_BAD_ALLOC.
pub const MAX_CONTEXTS: usize = env_usize(option_env!("YAKOGL_MAX_CONTEXTS"), 2);

/// A decimal build-time setting. `const`: the value sizes static arrays.
pub(crate) const fn env_usize(v: Option<&str>, def: usize) -> usize {
    let s = match v {
        Some(s) => s.as_bytes(),
        None => return def,
    };
    let mut n = 0usize;
    let mut i = 0;
    while i < s.len() {
        assert!(s[i] >= b'0' && s[i] <= b'9', "a YAKOGL_MAX_* setting is not a decimal number");
        n = n * 10 + (s[i] - b'0') as usize;
        i += 1;
    }
    assert!(n >= 1, "a YAKOGL_MAX_* setting must be at least 1");
    n
}
/// Qt Quick: a vertex + index buffer per batch, uniform buffers per material.
pub const MAX_BUFFERS: usize = 512;
/// A UI keeps a texture per text item plus FemtoVG gradient look-ups;
/// Qt Quick 3D a few per material. Names an application can hold; the
/// table has [`DEFAULT_TEXTURES`] slots beyond them.
pub const MAX_TEXTURES: usize = 512;
/// Texture name 0 is not "no texture": every target has a default texture
/// object, and glTexParameter* / glTexImage* address it while nothing else
/// is bound (ES 3.0 3.8). The four of them (2D, cube, 2D array, 3D) are the
/// tail of `GlContext::textures`, so that every path here goes on naming a
/// texture by one table index; upstream C keeps them in a second array
/// (`default_textures[4]`), which would mean a second kind of reference
/// through every entry point. Names stop at MAX_TEXTURES, so `texture()`
/// and the draw path still treat 0 as unbound and cannot reach these.
pub const DEFAULT_TEXTURES: usize = 4;
/// Slots of `GlContext::textures`: the names, then the default objects.
pub const TEXTURE_SLOTS: usize = MAX_TEXTURES + DEFAULT_TEXTURES;
/// Qt Quick 3D compiles hundreds of material variants.
pub const MAX_SHADERS: usize = 512;
/// A pipeline each: cascaded shadow maps ran out of 160.
pub const MAX_PROGRAMS: usize = 512;
/// Qt Quick 3D keeps one per render target of every pass (post-processing
/// chains); 32 ran out and the demos with effects presented nothing.
pub const MAX_FRAMEBUFFERS: usize = 256;
pub const MAX_RENDERBUFFERS: usize = 64;
/// The GLES2 minimum (the value GL_MAX_VERTEX_ATTRIBS reports).
pub const MAX_VERTEX_ATTRIBS: usize = 16;
/// glBindAttribLocation names a program keeps (`GlProgram::attrib_binds`).
pub const MAX_ATTRIB_BINDS: usize = crate::shaderc::SHC_MAX_DECLARED_ATTRIBS;
/// 384 vec4 per stage: Qt Quick 3D keeps its full light set when the RHI
/// reports >= 325 vec4, else it regenerates its material shaders at run time.
pub const MAX_UNIFORM_DWORDS: usize = 1536;
/// GL_MAX_VERTEX_UNIFORM_VECTORS and GL_MAX_FRAGMENT_UNIFORM_VECTORS: the
/// uniform block in vec4s. The link packs each stage's uniforms against these
/// (`shaderc::packing`, GLSL ES 1.00 Appendix A), the query answers with them
/// and the shader's gl_MaxVertexUniformVectors / gl_MaxFragmentUniformVectors
/// read them, so the three cannot disagree.
pub const MAX_VERTEX_UNIFORM_VECTORS: usize = MAX_UNIFORM_DWORDS / 4;
pub const MAX_FRAGMENT_UNIFORM_VECTORS: usize = MAX_UNIFORM_DWORDS / 4;
/// GL_MAX_TEXTURE_IMAGE_UNITS and gl_MaxTextureImageUnits: the samplers a
/// fragment shader may use. Every backend's per-draw fragment table holds
/// GLES_MAX_TEX_BINDS (16) bindings, and the link enforces this limit
/// (program.rs `check_sampler_counts`).
pub const CAP_TEXTURE_UNITS: usize = 16;
/// GL_MAX_COMBINED_TEXTURE_IMAGE_UNITS and gl_MaxCombinedTextureImageUnits:
/// the GL texture units (glActiveTexture's range, a sampler uniform's range,
/// the per-unit binding arrays), which the vertex and fragment stages of one
/// program share (ES 2.0 6.2); the link enforces the sum of the two stages'
/// samplers against it. 32, ES 3.0's minimum (Table 6.32), on every GPU: a
/// unit is GL state only. The draw hands each backend compacted per-stage
/// bindings (draw.rs: `GlesTextureBind::unit` is the sampler binding, not the
/// GL unit), each stage under its own limit (CAP_TEXTURE_UNITS,
/// GLES_VS_TEX_UNITS), so no backend table is indexed by the unit. It was
/// 16, the fragment limit, and dEQP-GLES2 uniform_api reported 8 cases
/// "18/20/28 combined texture units required, 16 supported" on the Pi 4.
pub const CAP_COMBINED_TEXTURE_UNITS: usize = 32;
const _: () = assert!(CAP_TEXTURE_UNITS <= crate::backend::GLES_MAX_TEX_BINDS);
const _: () = assert!(CAP_TEXTURE_UNITS <= CAP_COMBINED_TEXTURE_UNITS);
const _: () = assert!(crate::backend::GLES_VS_TEX_UNITS <= CAP_COMBINED_TEXTURE_UNITS);
/* the sampler-unit tables are u8 (GlProgram fs_sampler_unit, vs_sampler_unit) */
const _: () = assert!(CAP_COMBINED_TEXTURE_UNITS <= 256);
/// GL_MAX_VARYING_VECTORS: what the GPU carries AND this driver programs --
/// the ES 3.0 caps are min(hardware, implemented), never a promise
/// (tools/cts/RESULTS.md, "ES 3.0 conformance gaps"). The in-process
/// compiler's `gl_MaxVaryingVectors` reads this too, so the query and the
/// shader constant cannot disagree.
///
/// GC7000: 15. The part has 16 varyings (hwdb `varyings_count`, etnaviv's
/// max_varyings) and the vertex stage's 16 io rows (SHC_VIV_MAX_IO) carry
/// the position beside them, so 15 reach the fragment stage; since 6719382
/// the driver programs all of them. ES 3.0's minimum is 15.
#[cfg(feature = "gpu-vivante")]
pub const MAX_VARYING_VECTORS: usize = 15;
/// PowerVR: 8, as before, until the USC's limit is measured (usc-backend).
#[cfg(feature = "gpu-powervr")]
pub const MAX_VARYING_VECTORS: usize = 8;
/// V3D: 8, the ES 2.0 minimum. Mesa's v3d allows 16 (V3D_MAX_FS_INPUTS, 64
/// components), but nothing here has drawn with more than a few varyings on
/// the board: the M6 first cut claims what it has a chance of having
/// measured, and is raised with the board run that measures more.
#[cfg(feature = "gpu-v3d")]
pub const MAX_VARYING_VECTORS: usize = 8;
/// Mali-G610: 8, the ES 2.0 minimum, as the V3D's first cut: nothing has
/// been compiled for it yet. Mesa's panfrost takes 32 inputs a stage on
/// arch >= 9 (Mesa 26.2.1, src/gallium/drivers/panfrost/pan_screen.c:573).
#[cfg(feature = "gpu-mali")]
pub const MAX_VARYING_VECTORS: usize = 8;
/// Sampler bindings per fragment shader.
pub const CAP_SAMPLERS: usize = 16;
/// Colour attachment points a framebuffer object STORES: the size of its
/// arrays (`GlFramebuffer::color_n`, `draw_buf`), one per slot of the
/// draw's [`GlesDrawTarget`](crate::backend::GlesDrawTarget) array. It is
/// not what the GL reports: [`max_color_attachments`] and
/// [`max_draw_buffers`] are, and they are the back end's
/// ([`crate::backend::draw_targets`]), at most this.
pub const CAP_COLOR_ATTACHMENTS: usize = crate::backend::GLES_MAX_TARGETS;
const _: () = assert!(crate::backend::DRAW_TARGETS <= CAP_COLOR_ATTACHMENTS);
/// GL_MAX_DRAW_BUFFERS: the colour outputs one draw WRITES, which is what
/// the back end draws ([`crate::backend::draw_targets`]; 3 on every GPU
/// until each back end's half of ES 3.0's 4 lands). The query,
/// glDrawBuffers, glClearBuffer and the shader's gl_MaxDrawBuffers all read
/// this one function; arrays are sized by [`CAP_COLOR_ATTACHMENTS`].
pub fn max_draw_buffers() -> usize {
    crate::backend::draw_targets()
}
/// GL_MAX_COLOR_ATTACHMENTS: the attachment points a framebuffer object
/// names, GL_COLOR_ATTACHMENT0 up to this. The same number as
/// [`max_draw_buffers`]: one draw writes every point (a G-buffer pass writes
/// them all), and a point past what the back end draws would be one an
/// application could attach to and never see written. Every attachment
/// check (glFramebufferTexture*, glFramebufferRenderbuffer, the attachment
/// query, glInvalidateFramebuffer, glReadBuffer, glDrawBuffers) reads it.
pub fn max_color_attachments() -> usize {
    max_draw_buffers()
}
/// GL_MAX_VERTEX_OUTPUT_COMPONENTS and GL_MAX_FRAGMENT_INPUT_COMPONENTS:
/// four components per varying vector, per GPU with MAX_VARYING_VECTORS.
/// The shader's gl_MaxVertexOutputVectors / gl_MaxFragmentInputVectors are
/// these divided by four (ES 3.00 section 7.3), so raising one here moves
/// the shader's value with it.
pub const MAX_VERTEX_OUTPUT_COMPONENTS: usize = MAX_VARYING_VECTORS * 4;
pub const MAX_FRAGMENT_INPUT_COMPONENTS: usize = MAX_VARYING_VECTORS * 4;
/// GL_MIN/MAX_PROGRAM_TEXEL_OFFSET, ES 3.0's minimum range: the values the
/// GL core reports, and the shader's gl_Min/MaxProgramTexelOffset.
pub const MIN_PROGRAM_TEXEL_OFFSET: i32 = -8;
pub const MAX_PROGRAM_TEXEL_OFFSET: i32 = 7;
/// GL_MAX_UNIFORM_BUFFER_BINDINGS: indexed binding points a context keeps
/// (ES 3.0's minimum). They are GL-core state only -- the blocks are
/// flattened into each stage's default block and filled from the bound
/// range at draw time -- so nothing on either GPU limits them.
pub const MAX_UNIFORM_BUFFER_BINDINGS: usize = 24;

/// One indexed uniform-buffer binding point: the buffer and the range of it
/// (`size` 0: the whole buffer, as glBindBufferBase binds it).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UboBinding {
    pub buffer: GLuint,
    pub offset: GLintptr,
    pub size: GLsizeiptr,
}

/// 4096 -> 1.
pub const CAP_MIP_LEVELS: usize = 13;
pub const MAX_VAOS: usize = 16;

/// Bytes per texel of the linear RGBA8888 storage.
pub const RGBA8_BPP: u32 = 4;

/// Row pitch of a linear RGBA8888 texture/renderbuffer level in texels. The
/// PBE writes rows at a pitch of whole pixel pairs, so an odd-width render
/// target is padded to the next even texel count, and every row computation
/// (storage, uploads, read-back, the TPU stride) uses the same pitch. Mip
/// chains exist only for power-of-two sizes, where the pitch equals the width.
#[inline]
pub const fn tex_pitch(w: u32) -> u32 {
    w.wrapping_add(1) & !1 /* unsigned wrap, as the C macro */
}

/// `pitch` units of `unit` bytes made the least pitch at or above it whose
/// bytes are a multiple of `align`; `pitch` itself when `align` is 0. A
/// pitch of whole pixel pairs stays one: the step is an even number of
/// units for every unit of up to `align / 2` bytes.
pub const fn row_aligned(pitch: u32, unit: u32, align: u32) -> u32 {
    if align == 0 || unit == 0 {
        return pitch;
    }
    let (mut a, mut b) = (align, unit);
    while b != 0 {
        (a, b) = (b, a % b);
    }
    pitch.next_multiple_of(align / a)
}

/// The row pitch, in pixels of `bpp` bytes, of a linear image `w` wide
/// that the GL core allocates: [`tex_pitch`], made a whole number of the
/// backend's `linear_row_align` bytes (`YakoglBackend::linear_row_align`;
/// unchanged where it is 0). Storage whose pitch its producer chose (an
/// imported image, a native window) keeps that pitch instead.
pub fn linear_pitch(w: u32, bpp: u32) -> u32 {
    row_aligned(tex_pitch(w), bpp, crate::backend::linear_row_align())
}

/// The row pitch of level `lw` wide of a packed (PowerVR) chain whose level 0
/// is `w` wide: `tex_pitch(lw)`, except that a 1-wide level above a wider
/// base is 1 texel a row where the TPU reads it. Level 0 keeps its
/// pair-padded pitch, which the descriptor states explicitly.
#[inline]
pub const fn one_wide_level_pitch(w: u32, lw: u32, chain: bool) -> u32 {
    if chain && lw == 1 && w > 1 {
        1
    } else {
        tex_pitch(lw)
    }
}

impl GlTexture {
    /// `GL_TEX_LEVEL_PITCH()`: the row pitch of one mip level, in texels --
    /// its own pair-padded width, or level 0's pitch when the backend needs
    /// one stride for the whole chain (`YakoglBackend::mip_level_align`).
    ///
    /// A 1-wide level above the base of a PowerVR chain is one texel a row,
    /// not a pair: the descriptor carries level 0's stride only and the TPU
    /// walks level L at its own width (Mesa pvr_image_plane_setup_mip_levels:
    /// pitch = cpp * width), so pair-padded rows put every other row of a
    /// cube's 1x6 tail, or a 32x64 chain's 1x2, in the padding
    /// ([`one_wide_level_pitch`]).
    ///
    /// A PowerVR chain of a non-power-of-two image (`pot_chain`) is laid out
    /// as the chain of its power-of-two extent, so a level's pitch is not a
    /// function of its own width: `lw` names the first level that wide.
    /// Only a 1-wide level past the first one is ambiguous, and its readers
    /// and writers ask [`GlTexture::level_pitch_at`].
    pub const fn level_pitch(&self, lw: u32) -> u32 {
        if self.chain_pitch != 0 {
            self.chain_pitch
        } else if self.pot_chain {
            let mut l = 0;
            while l + 1 < CAP_MIP_LEVELS as u32 && pot_level_side(self.width, l) > lw {
                l += 1;
            }
            self.level_pitch_at(l)
        } else {
            one_wide_level_pitch(self.width, lw, self.chain_levels > 1)
        }
    }

    /// The row pitch of level `level`, in texels: [`GlTexture::level_pitch`]
    /// of that level's width, or, for a `pot_chain`, the pitch of that
    /// level of the power-of-two chain it is laid out in
    /// ([`pot_chain_level_pitch`]).
    pub const fn level_pitch_at(&self, level: u32) -> u32 {
        if self.pot_chain && self.chain_pitch == 0 {
            pot_chain_level_pitch(self.width, level)
        } else {
            self.level_pitch(pot_level_side(self.width, level))
        }
    }
}

/// GL's side of level `l` of a level 0 `side` long: max(side >> l, 1).
#[inline]
pub const fn pot_level_side(side: u32, l: u32) -> u32 {
    let s = if l < 32 { side >> l } else { 0 };
    if s == 0 {
        1
    } else {
        s
    }
}

/// The row pitch, in texels, of level `l` of a PowerVR chain whose level 0
/// is `w` wide and not a power of two (`GlTexture::pot_chain`): the pitch
/// level `l` has in the chain of the power-of-two width (Mesa 26.2.1 pvr,
/// `pvr_image_plane_init_physical_extent` rounds a multi-level image's
/// extent up with `util_next_power_of_two`, and
/// `pvr_image_plane_setup_mip_levels` gives level `l` the pitch
/// `cpp * u_minify(physical_width, l)`), at the pitch a power-of-two chain of
/// that width has here ([`one_wide_level_pitch`]).
#[inline]
pub const fn pot_chain_level_pitch(w: u32, l: u32) -> u32 {
    let p = if w <= 1 { 1 } else { w.next_power_of_two() };
    one_wide_level_pitch(p, pot_level_side(p, l), true)
}

#[repr(C)]
pub struct GlBuffer {
    pub used: bool,
    pub target: GLenum,
    pub usage: GLenum,
    /// GPU-resident storage.
    pub mem: OsalGpuMem,
    pub size: GLsizeiptr,
    /// Kick id of the last draw that read it (0: none).
    pub last_use: u32,
    /// glMapBufferRange: the mapped range (`mapped` null when unmapped).
    pub mapped: *mut c_void,
    pub map_off: GLintptr,
    pub map_len: GLsizeiptr,
    pub map_access: GLbitfield,
}

#[repr(C)]
pub struct GlTexture {
    pub used: bool,
    /// TEXTURE_2D, TEXTURE_CUBE_MAP, TEXTURE_2D_ARRAY or TEXTURE_3D.
    pub target: GLenum,
    /// Level 0, of ONE face/layer.
    pub width: u32,
    pub height: u32,
    /// A cube map (6) or a 2D array (N) keeps its faces/layers stacked
    /// vertically in one linear texture: `layers` rows of `height` at every
    /// level, so the storage is width x (height * layers). The shaders sample
    /// it as a tall 2D texture (the sampler emulation of `yakogl-tools glsl2usc`).
    pub layers: u16,
    /// A 3D texture (target TEXTURE_3D, `layers` = depth) tiles its slices in
    /// a grid of `grid_cols` columns instead (slice k at column k % grid_cols,
    /// row k / grid_cols), which keeps 256^3 inside the TPU's 16384-texel
    /// height; one mip level. grid_cols = ceil(sqrt(depth)), 0 otherwise.
    ///
    /// A backend whose texture unit walks a stack of slices itself
    /// (`YakoglBackend::native_layered_textures`) gets the stacked layout
    /// above for 3D too, and grid_cols stays 0.
    pub grid_cols: u8,
    pub format: GLenum,
    pub type_: GLenum,
    pub min_filter: GLenum,
    pub mag_filter: GLenum,
    pub wrap_s: GLenum,
    pub wrap_t: GLenum,
    /// Linear RGBA8888 texel storage. A power-of-two texture owns room for
    /// its whole mip chain: level i at `level_off[i]`, rows tightly packed at
    /// its own width (the layout the TPU walks with mipmaps_present).
    ///
    /// `chain_pitch` is non-zero when the backend asked for every level at
    /// level 0's pitch instead (`YakoglBackend::mip_level_align`); it holds
    /// that pitch in texels. [`GlTexture::level_pitch`] is the one place the
    /// two layouts are told apart.
    pub mem: OsalGpuMem,
    pub level_off: [u32; CAP_MIP_LEVELS],
    pub chain_pitch: u32,
    /// Levels the storage can hold (1 = no chain).
    pub chain_levels: u8,
    /// Levels with texels (1 ..= chain_levels).
    pub levels: u8,
    /// The last level the application actually put texels in, whether by
    /// upload, by glGenerateMipmap or by rendering into it. Sampling past it
    /// reads memory nobody wrote: Qt prefilters a 512 light probe down to
    /// 16x16 (six levels) in a texture declared with ten.
    pub levels_written: u8,
    /// `GL_TEXTURE_MAX_LEVEL`: the level the application asked the sampler to
    /// stop at (GL's default is "no clamp", `CAP_MIP_LEVELS - 1`).
    pub max_level: u8,
    /// Storage belongs to an EGLImage, which frees it once nothing uses it
    /// (`egl::image_storage_release`); never freed here.
    pub external: bool,
    /// Deleted while a framebuffer that was not bound still held it: the
    /// name is gone (glIsTexture false, glGenTextures never hands it out
    /// again) but the image stays until the last framebuffer lets go
    /// (`framebuffer::attachment_released`).
    pub orphan: bool,
    /// Texels stored B,G,R,X (an XRGB8888 native buffer).
    pub bgra: bool,
    /// `GLES_TEXFMT_*` (0 = RGBA8 texels; the ETC formats are block laid out).
    pub texfmt: u8,
    /// Bytes per texel of a linear format (4 RGBA8, 8 RGBA16F, 2 R16F/RGB565).
    pub bpp: u8,
    /// Compressed: bytes per 4x4 block.
    pub block_bytes: u8,
    /// glTexStorage*: the format and size are fixed (ES 3.0 3.8.4); a
    /// respecification is GL_INVALID_OPERATION.
    pub immutable: bool,
    /// Cube map: one bit per face whose level 0 has been specified at the
    /// current size (bit 0 = POSITIVE_X .. bit 5 = NEGATIVE_Z). Laying the
    /// storage out again clears it, so a face specified at a different size
    /// leaves the cube map incomplete, which is what GL says it is
    /// (ES 2.0 3.7.10). It sits in what was padding, so the layout is
    /// unchanged.
    pub faces_set: u8,
    /// Compressed: the power-of-two block grid held.
    pub block_w: u32,
    pub block_h: u32,
    /// Kick id of the last draw that sampled it (0: none).
    pub last_use: u32,
    /// One bit per level (bit l = level l) for which an image has been
    /// specified since the storage was laid out. Level 0's bit is set by
    /// the layout itself. ES 2.0 3.7.10 / ES 3.0 3.8.13: a mipmap filter
    /// needs every level of the chain, so a hole (missing_level_1) makes the
    /// texture incomplete; `levels` alone (the highest level plus one) could
    /// not say that, and dEQP texture.completeness read a picture where it
    /// required (0,0,0,1).
    pub level_defined: u16,
    /// One bit per level whose image does not fit the chain: a size other
    /// than the chain's for that level, a format other than level 0's, or
    /// level 0 itself with a zero side. Such a level has no storage here and
    /// leaves the texture incomplete while it stands.
    pub level_bad: u16,
    /// The texels' write stamp (`GlesTextureBind::content_gen`): a new value
    /// at every CPU write of the texels (`texture.rs` `stamp_texels`), 0 for
    /// storage an EGLImage shares (a sibling may write it) and before any.
    pub content_gen: u32,
    /// Cube map: bit f of entry l is set when face f (bit 0 = POSITIVE_X)
    /// has an image of the chain's size and format at level l >= 1 (level
    /// 0's record is `faces_set`). `level_defined`/`level_bad` are one bit
    /// per level for all six faces, and cannot see one face missing a level
    /// or holding one of another size (dEQP texture.completeness.cube.
    /// npot_size_level_1_neg_x: under ES 2.0 reporting that face's NPOT
    /// level is refused and never specified).
    pub level_faces: [u8; CAP_MIP_LEVELS],
    /// The ES 3.0 parameters glTexParameter stores and glGetTexParameter
    /// reads back exactly (dEQP-GLES3 state_query.texture.*): the draw does
    /// not apply them yet. `max_level_set` is GL_TEXTURE_MAX_LEVEL as the
    /// application gave it; `max_level` is the clamp the sampler uses.
    pub base_level: i32,
    pub max_level_set: i32,
    pub min_lod: f32,
    pub max_lod: f32,
    pub wrap_r: GLenum,
    pub compare_mode: GLenum,
    pub compare_func: GLenum,
    /// GL_TEXTURE_SWIZZLE_R/G/B/A.
    pub swizzle: [u16; 4],
    /// glTexStorage*'s level count (GL_TEXTURE_IMMUTABLE_LEVELS), which a
    /// 3D texture's one-level store cannot tell.
    pub immutable_levels: u8,
    /// The object's identity for a backend, from its creation (glGen* or a
    /// first bind) to its free: the orphan rule moves the record, serial
    /// and all, to another slot, so a backend keyed on it keeps the image
    /// (`GlesDraw::depth_id`). Never 0.
    pub serial: u32,
    /// Changes whenever the storage is laid out again (a level 0 of another
    /// size, glTexStorage*, an EGLImage attached): the old contents are gone.
    pub storage_gen: u32,
    /// The texels are in the pixel engine's B,G,R,A order: on a backend
    /// whose RGBA8888 target is stored B,G,R,A (`rt_rgba8888_is_bgra`, the
    /// GC7000) a draw rendered into this RGBA8 texture, and the storage was
    /// converted to that order the first time, so every texel is in it;
    /// uploads since write B,G,R,A (`texture_write`), and a re-specification
    /// clears it. Unlike `bgra` (an XRGB8888 image, its fourth byte
    /// meaningless) the alpha is real, so the colour target stays RGBA8888
    /// and the texture keeps its alpha. The binding's `bgra` is the OR of
    /// the two.
    pub pe_order: bool,
    /// The CPU has written texels into the storage since it was last
    /// allocated and cleared (any upload, a mipmap generation, a kept level).
    /// Until then the texels are undefined (glTexImage2D with no data) and
    /// the first render need not convert them (`texture::take_pe_order`).
    /// Conservative: storage reused rather than cleared counts as written.
    pub cpu_wrote: bool,
    /// The chain of a non-power-of-two 2D image laid out for the PowerVR's
    /// TPU: as the chain of the image's power-of-two extent (Mesa pvr's
    /// physical extent), level l's GL image in the top-left of that level.
    /// Level 0's pitch is then the power-of-two width, which the descriptor
    /// states and a render into it uses ([`GlTexture::level_pitch_at`];
    /// `texture.rs` `chain_layout_npot`). It sits in what was padding.
    pub pot_chain: bool,
    /// `es3_formats::FORMATS` index + 1 of the ES 3.0 sized format the
    /// texture holds (0: one of the original tables' formats). Sits in what
    /// was padding.
    pub es3_fmt: u8,
    /// The last glGenerateMipmap was made by the backend in its own image
    /// of the texture (`YakoglBackend::generate_mipmap` answered 1): levels
    /// 1.. of the storage are stale until `backend::return_mipmap_chain`
    /// (`texture.rs` `chain_back`), which every CPU read or write of them
    /// asks for first. Sits in what was padding.
    pub chain_in_backend: bool,
    /// A depth texture laid out with no pixels on a backend that samples
    /// rendered depth from its own buffer (`backend::defers_depth_storage`,
    /// the V3D): the layout is recorded but `mem` is empty, and
    /// `texture.rs` `ensure_storage` allocates it (cleared) when something
    /// first needs the texels. glmark2's shadow and refract never do
    /// (docs/notes/v3d-depth-storage.md). Ask [`GlTexture::has_image`], not
    /// `mem`, whether an image was specified. Sits in what was padding.
    pub storage_deferred: bool,
    /// The sized format of the texture is GL_RGBA32F, which the store holds
    /// as halves (`texture.rs` `sized_format`), so `texfmt` alone cannot
    /// tell it from GL_RGBA16F. A framebuffer attachment reports it as
    /// GL_RGBA32F (`framebuffer.rs` `attachment_storage`), which is not
    /// colour-renderable without GL_EXT_color_buffer_float. Always false in
    /// an ES 2.0 report. Sits in what was padding.
    pub rgba32f: bool,
    /// The texture is a depth (or depth-stencil) texture specified with an
    /// unsized internal format, GL_DEPTH_COMPONENT or GL_DEPTH_STENCIL
    /// (`texture.rs` `is_unsized_depth`). ES 3.0 4.4.4.2 makes only the
    /// sized depth formats depth-renderable and this build claims no
    /// GL_OES_depth_texture there, so such a texture is no framebuffer
    /// attachment (`framebuffer.rs` `attachment_storage`). Always false in
    /// an ES 2.0 report, where the extension makes it one. Sits in what was
    /// padding.
    pub unsized_depth: bool,
    /// The CPU wrote the texels of this depth texture (an upload with data,
    /// a depth glBlitFramebuffer) since the depth plane a backend keeps for
    /// it last matched them: the next draw that has the texture at its depth
    /// attachment hands the texels to the backend with it
    /// (`GlesDraw::depth_texels`, `texture.rs` `depth_seed`), and this goes
    /// false once the backend has recorded that draw, or a draw that clears
    /// the whole depth plane (`texture.rs` `depth_seeded`). The padding
    /// ran out at `unsized_depth`: this grows the struct to 256 bytes.
    pub depth_texels_newer: bool,
}

impl GlTexture {
    /// Was an image specified: storage allocated, or deferred
    /// (`storage_deferred`)? The test the "no image" branches ask; `mem`
    /// alone answers no for a deferred depth texture.
    pub fn has_image(&self) -> bool {
        !self.mem.impl_.is_null() || self.storage_deferred
    }
}

/// A number no object serial or storage generation has had (never 0): one
/// counter for both, process-wide, so a serial never equals a gen by accident
/// either.
pub fn next_object_stamp() -> u32 {
    static NEXT: Global<u32> = Global::new(0);
    // SAFETY: single-threaded by contract; no other reference is live.
    let n = unsafe { NEXT.get() };
    *n = n.wrapping_add(1).max(1);
    *n
}

/// The identity a backend keeps an object's depth/stencil contents under
/// (`GlesDraw::depth_id`): the serial, bit 30 set for a texture.
pub const fn ds_object_id(serial: u32, texture: bool) -> u32 {
    (serial & 0x3fff_ffff) | if texture { 0x4000_0000 } else { 0 }
}

/// A new `storage_gen` for an object whose storage is specified again: the
/// old contents are undefined, so a depth/stencil buffer the backend keeps
/// for them is freed (`backend::attachment_gone`).
pub fn respecified(serial: u32, texture: bool) -> u32 {
    backend::attachment_gone(ds_object_id(serial, texture));
    next_object_stamp()
}

/// How many levels GL's mip chain of a `w x h` image has: 1 + floor(log2 of
/// the larger side) (ES 2.0 3.7.10's q, ES 3.0 3.8.13's), whatever this
/// store can hold. A non-power-of-two image gets one level here
/// (`GlTexture::chain_levels`), and completeness is judged against GL's
/// count, not the store's.
pub fn gl_mip_levels(w: u32, h: u32) -> u8 {
    let m = w.max(h).max(1);
    (32 - m.leading_zeros()) as u8
}

#[repr(C)]
pub struct GlShader {
    pub used: bool,
    pub type_: GLenum,
    pub compiled: bool,
    /// Not in the offline table: reported as compiled so the application goes
    /// on to compile its other stage (both get logged in one run), and refused
    /// at link time.
    pub missing: bool,
    pub compiled_shader: ShcShader,
    /// Owned copy (C heap) of the glShaderSource text.
    pub source: *mut c_char,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct GlAttribBind {
    pub name: [c_char; 32],
    pub index: u8,
}

#[repr(C)]
pub struct GlProgram {
    pub used: bool,
    pub linked: bool,
    pub vs: GLuint,
    pub fs: GLuint,
    /// Resolved wiring after link (coeff/attrib/uniform layout).
    pub rvs: ShcShader,
    pub rfs: ShcShader,
    /// The two stages' push-constant blocks, laid out as the compiler packed
    /// them (`ShcUniform::dword_off`); `draw` uploads them per draw. The
    /// dwords hold float, int or uint bit patterns. C heap, sized at link to
    /// the stages' layouts (`*_uniform_cap` dwords; a fixed 1536-dword pair
    /// made the program pool 15 KB a slot); null before a link.
    ///
    /// The fragment allocation keeps [`BLEND_CONSTS_DWORDS`] dwords past the
    /// block: a blend variant's shader reads `glBlendColor` from four dwords
    /// after the application's uniforms, and the backend snapshots the whole
    /// variant block out of this array.
    pub vs_uniforms: *mut f32,
    pub fs_uniforms: *mut f32,
    pub vs_uniform_cap: u16,
    pub fs_uniform_cap: u16,
    /// Dwords the fragment array really holds: `fs_uniform_cap`, the block
    /// the application may write, plus the [`BLEND_CONSTS_DWORDS`] a
    /// CONSTANT_\* blend variant reads `glBlendColor` from.
    pub fs_uniform_alloc: u16,
    /// Texture unit each FS sampler binding reads (glUniform1i).
    pub fs_sampler_unit: [u8; CAP_SAMPLERS],
    /// The same, for vertex-stage samplers.
    pub vs_sampler_unit: [u8; CAP_SAMPLERS],
    /// Generic attribute index the app uses for each VS input.
    pub attrib_app_index: [u8; 16],
    /// glBindAttribLocation's names: as many as a vertex shader may declare
    /// (`SHC_MAX_DECLARED_ATTRIBS`), since binding two to one location is
    /// how a shader declares more attributes than there are locations.
    pub attrib_binds: [GlAttribBind; MAX_ATTRIB_BINDS],
    pub num_attrib_binds: u8,
}

/// Dwords the fragment uniform allocation keeps past the block, for a blend
/// variant's `glBlendColor` constants (`SHC_BLEND_CONSTS_DWORDS`).
pub const BLEND_CONSTS_DWORDS: usize = crate::shaderc::SHC_BLEND_CONSTS_DWORDS;

impl GlProgram {
    /// The VS uniform block (`vs_uniform_cap` dwords; empty before a link).
    pub fn vs_block(&self) -> &[f32] {
        block(self.vs_uniforms, self.vs_uniform_cap)
    }

    /// The FS uniform block (`fs_uniform_cap` dwords; empty before a link).
    pub fn fs_block(&self) -> &[f32] {
        block(self.fs_uniforms, self.fs_uniform_cap)
    }

    /// The whole FS array, `fs_uniform_alloc` dwords: the block plus the
    /// [`BLEND_CONSTS_DWORDS`] the allocation keeps past it (see the field's
    /// note), writable. The application never reaches the tail: `fs_block`
    /// is what its uniforms are bounded by.
    pub fn fs_block_with_blend_mut(&mut self) -> &mut [f32] {
        block_mut(self.fs_uniforms, self.fs_uniform_alloc)
    }

    /// Both blocks, writable.
    pub fn blocks_mut(&mut self) -> (&mut [f32], &mut [f32]) {
        (block_mut(self.vs_uniforms, self.vs_uniform_cap), block_mut(self.fs_uniforms, self.fs_uniform_cap))
    }

    /// Free the uniform blocks (`gl_program_free_uniforms`).
    pub fn free_uniforms(&mut self) {
        // SAFETY: the blocks are null or C heap allocations this program owns
        // (glLinkProgram); they are forgotten right after.
        unsafe {
            crate::util::free(self.vs_uniforms as *mut c_void);
            crate::util::free(self.fs_uniforms as *mut c_void);
        }
        self.vs_uniforms = core::ptr::null_mut();
        self.fs_uniforms = core::ptr::null_mut();
        self.vs_uniform_cap = 0;
        self.fs_uniform_cap = 0;
        self.fs_uniform_alloc = 0;
    }
}

fn block<'a>(p: *mut f32, cap: u16) -> &'a [f32] {
    if p.is_null() {
        return &[];
    }
    // SAFETY: a non-null block is a live calloc of at least `cap` f32s
    // (glLinkProgram), owned by the program; f32 has no invalid values.
    unsafe { core::slice::from_raw_parts(p, cap as usize) }
}

fn block_mut<'a>(p: *mut f32, cap: u16) -> &'a mut [f32] {
    if p.is_null() {
        return &mut [];
    }
    // SAFETY: as `block`; the two blocks are separate allocations and the
    // caller holds the program mutably.
    unsafe { core::slice::from_raw_parts_mut(p, cap as usize) }
}

/// Free a program's uniform blocks (the C view's `gl_program_free_uniforms`).
///
/// # Safety
/// `p` is null or points at a program of a context.
#[no_mangle]
pub unsafe extern "C" fn gl_program_free_uniforms(p: *mut GlProgram) {
    if let Some(p) = p.as_mut() {
        p.free_uniforms();
    }
}

/// A colour renderbuffer owns linear RGBA8888 storage like a texture;
/// depth/stencil renderbuffers own nothing here (the backend keeps a
/// depth/stencil buffer per renderbuffer pair, keyed by `GlesDraw::ds_id`).
#[repr(C)]
pub struct GlRenderbuffer {
    /// The name is taken (glGenRenderbuffers or a first bind).
    pub used: bool,
    /// A glBindRenderbuffer created the object: glIsRenderbuffer is true
    /// only then (a generated, never bound name is no renderbuffer yet).
    pub bound: bool,
    pub internalformat: GLenum,
    pub width: u32,
    pub height: u32,
    pub mem: OsalGpuMem,
    /// As `GlTexture::external`.
    pub external: bool,
    /// As `GlTexture::orphan`.
    pub orphan: bool,
    /// GL_RENDERBUFFER_SAMPLES: 0, or 1 for storage glRenderbufferStorage-
    /// Multisample made with one sample (GL_MAX_SAMPLES is 1; ES 3.0 4.4.2.1
    /// says the count is at least the one asked for). The pixels are one
    /// sample either way; the count decides completeness (4.4.4.2) and
    /// GL_SAMPLE_BUFFERS. Sits in what was padding.
    pub samples: u8,
    /// As `GlTexture::serial`.
    pub serial: u32,
    /// As `GlTexture::storage_gen`: glRenderbufferStorage*, an EGLImage.
    pub storage_gen: u32,
    /// The colour storage's row pitch in pixels where it is not the
    /// pair-padded `tex_pitch(width)`: the backend's linear row pitch
    /// (`linear_pitch`), or an image's. Sits in what was padding.
    pub pitch: u32,
}

/// One framebuffer attachment point.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct GlAttachment {
    /// GL_NONE / GL_TEXTURE / GL_RENDERBUFFER.
    pub type_: GLenum,
    pub name: GLuint,
    /// Texture mip level.
    pub level: GLint,
    /// Cube face / array layer of the texture.
    pub layer: GLint,
}

#[repr(C)]
pub struct GlFramebuffer {
    /// The name is taken (glGenFramebuffers or a first bind).
    pub used: bool,
    /// A glBindFramebuffer created the object (glIsFramebuffer).
    pub bound: bool,
    /// Attachment 0.
    pub color: GlAttachment,
    /// `color_n[i]` is attachment `i + 1`.
    pub color_n: [GlAttachment; CAP_COLOR_ATTACHMENTS - 1],
    pub depth: GlAttachment,
    pub stencil: GlAttachment,
    /// glDrawBuffers: which attachment the fragment shader's output `i`
    /// writes (-1 = GL_NONE). The default is output `i` to attachment `i`.
    pub draw_buf: [i8; CAP_COLOR_ATTACHMENTS],
    pub num_draw_buf: u8,
    /// glReadBuffer(GL_NONE): nothing to read from.
    pub read_none: bool,
    /// glReadBuffer(GL_COLOR_ATTACHMENTn): the colour attachment glReadPixels
    /// and glCopyTex* read (0 by default).
    pub read_index: u8,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct GlVertexAttrib {
    pub enabled: bool,
    /// Bound ARRAY_BUFFER at pointer-set time.
    pub buffer: GLuint,
    pub size: GLint,
    pub type_: GLenum,
    pub normalized: GLboolean,
    pub stride: GLsizei,
    /// Byte offset into the buffer (or the client pointer when buffer == 0).
    pub offset: usize,
    /// glVertexAttribDivisor: 0 per vertex, N per N instances.
    pub divisor: GLuint,
    /// glVertexAttribIPointer: components fetched as integers.
    pub integer: bool,
}

/// Vertex array object: snapshots the vertex-attribute arrays and the
/// ELEMENT_ARRAY_BUFFER binding (the ARRAY_BUFFER binding is not VAO state).
#[repr(C)]
pub struct GlVao {
    pub used: bool,
    pub attribs: [GlVertexAttrib; MAX_VERTEX_ATTRIBS],
    pub element_buffer: GLuint,
}

/// The objects a share group owns.
///
/// ES 3.0 2.6.1 and EGL 1.5 3.7.1: buffers, textures, renderbuffers, shaders
/// and programs belong to the share group, so every context created with
/// `eglCreateContext(..., share_context, ...)` sees the same ones.
/// Framebuffers and vertex arrays do not -- they stay in [`GlContext`] --
/// and neither do the bindings, which name a shared object per context.
///
/// One record per group in `GROUPS`, and a context reaches its own through
/// [`GlContext::objs`]. There are as many group slots as context slots, so
/// a build where every context is its own group -- which is every build
/// before sharing existed -- uses exactly the memory it used to.
#[repr(C)]
pub struct GlObjects {
    pub buffers: [GlBuffer; MAX_BUFFERS],
    pub textures: [GlTexture; TEXTURE_SLOTS],
    pub shaders: [GlShader; MAX_SHADERS],
    pub programs: [GlProgram; MAX_PROGRAMS],
    pub renderbuffers: [GlRenderbuffer; MAX_RENDERBUFFERS],
}

#[repr(C)]
pub struct GlContext {
    /// The share group whose objects this context uses: an index into
    /// `GROUPS`, reached through [`GlContext::objs`]. Its own unless
    /// eglCreateContext was given a context to share with.
    pub group: u8,
    /* the object tables that are NOT shared (ES 3.0 2.6.1) */
    pub framebuffers: [GlFramebuffer; MAX_FRAMEBUFFERS],

    /* bindings */
    pub bound_array_buffer: GLuint,
    pub bound_element_buffer: GLuint,
    /* OpenGL ES 3.0 adds six more buffer binding points. The copy ones are
     * used by glCopyBufferSubData; the pixel ones make a transfer read or
     * write a buffer instead of client memory; the indexed ones exist so
     * that binding and querying them behaves. */
    pub bound_pixel_pack_buffer: GLuint,
    pub bound_pixel_unpack_buffer: GLuint,
    pub bound_copy_read_buffer: GLuint,
    pub bound_copy_write_buffer: GLuint,
    pub bound_uniform_buffer: GLuint,
    pub bound_transform_feedback_buffer: GLuint,
    /// 0 = the default (window) surface. The DRAW framebuffer: every draw
    /// and clear renders into it.
    pub bound_framebuffer: GLuint,
    /// ES 3.0's READ framebuffer (glReadPixels, glCopyTex*), when
    /// `read_fb_split`; otherwise the read binding is `bound_framebuffer`
    /// (glBindFramebuffer(GL_FRAMEBUFFER) binds both). See
    /// `framebuffer::read_framebuffer`.
    pub bound_read_framebuffer: GLuint,
    pub read_fb_split: bool,
    /// glReadBuffer(GL_NONE) on the default framebuffer.
    pub default_read_none: bool,
    pub bound_renderbuffer: GLuint,
    /// 0 = the default VAO (live state in `attribs`).
    pub bound_vertex_array: GLuint,
    pub vaos: [GlVao; MAX_VAOS],
    pub bound_texture_2d: [GLuint; CAP_COMBINED_TEXTURE_UNITS],
    pub bound_texture_cube: [GLuint; CAP_COMBINED_TEXTURE_UNITS],
    pub bound_texture_2d_array: [GLuint; CAP_COMBINED_TEXTURE_UNITS],
    pub bound_texture_3d: [GLuint; CAP_COMBINED_TEXTURE_UNITS],
    /// Unit index.
    pub active_texture: GLuint,
    pub cur_program: GLuint,

    pub attribs: [GlVertexAttrib; MAX_VERTEX_ATTRIBS],

    /* fixed-function render state */
    pub viewport: [GLint; 4],
    pub depth_range: [GLclampf; 2],
    pub clear_color: [GLclampf; 4],
    pub clear_depth: GLfloat,
    pub clear_stencil: GLint,
    pub depth_test: bool,
    pub depth_func: GLenum,
    pub depth_mask: bool,
    pub blend: bool,
    pub blend_src: GLenum,
    pub blend_dst: GLenum,
    pub blend_eq: GLenum,
    pub blend_src_a: GLenum,
    pub blend_dst_a: GLenum,
    pub blend_eq_a: GLenum,
    pub blend_color: [GLclampf; 4],
    pub cull_face: bool,
    pub cull_mode: GLenum,
    pub front_face: GLenum,
    pub scissor_test: bool,
    pub scissor: [GLint; 4],
    /* stencil: front face ... */
    pub stencil_test: bool,
    pub stencil_func: GLenum,
    pub stencil_ref: GLint,
    pub stencil_mask: GLuint,
    pub stencil_sfail: GLenum,
    pub stencil_dpfail: GLenum,
    pub stencil_dppass: GLenum,
    pub stencil_writemask: GLuint,
    /* ... and the back-face state the *Separate calls record */
    pub stencil_back_func: GLenum,
    pub stencil_back_ref: GLint,
    pub stencil_back_mask: GLuint,
    pub stencil_back_sfail: GLenum,
    pub stencil_back_dpfail: GLenum,
    pub stencil_back_dppass: GLenum,
    pub stencil_back_writemask: GLuint,
    pub color_writemask: [GLclampf; 4],
    pub line_width: GLfloat,
    /// glVertexAttrib*f: the value a VS input reads while its array is disabled.
    pub current_attrib: [[GLfloat; 4]; MAX_VERTEX_ATTRIBS],
    pub unpack_alignment: GLint,
    pub pack_alignment: GLint,
    /// GL_UNPACK_ROW_LENGTH (0: the upload's width).
    pub unpack_row_length: GLint,
    /// The rest of ES 3.0's pixel store state (table 6.29/6.30), honoured by
    /// the uploads (texture.rs `unpack_source`) and glReadPixels.
    pub unpack_image_height: GLint,
    pub unpack_skip_rows: GLint,
    pub unpack_skip_pixels: GLint,
    pub unpack_skip_images: GLint,
    pub pack_row_length: GLint,
    pub pack_skip_rows: GLint,
    pub pack_skip_pixels: GLint,
    /* accepted and queryable, not modelled by the backend */
    pub polygon_offset_fill: bool,
    pub polygon_offset_factor: GLfloat,
    pub polygon_offset_units: GLfloat,
    pub sample_alpha_to_coverage: bool,
    pub sample_coverage: bool,
    pub sample_coverage_value: GLclampf,
    pub sample_coverage_invert: bool,
    pub dither: bool,
    /// ES 3.0 GL_PRIMITIVE_RESTART_FIXED_INDEX: the type's largest index
    /// ends the primitive in glDrawElements* (`draw::restart_to_lists`).
    pub primitive_restart: bool,
    /// ES 3.0 GL_RASTERIZER_DISCARD: draws and clears stop in the GL core
    /// while it is set (not under report-es2, where it is only recorded).
    pub rasterizer_discard: bool,
    pub generate_mipmap_hint: GLenum,

    /// Size of the default (window) surface.
    pub target_w: u32,
    pub target_h: u32,
    /// `GLES_SURF_*` of the default surface.
    pub default_format: u8,
    /// The default surface's samples per pixel (1 or 4).
    pub default_samples: u8,
    /// The default surface's row pitch in pixels when it is not
    /// `tex_pitch(target_w)`: a window that is a view into a wider scan-out
    /// buffer (its native window's stride, egl.rs). 0: `tex_pitch(target_w)`,
    /// every surface the GL core allocates. Sits in what was padding.
    pub default_pitch: u32,
    pub default_surface: OsalGpuMem,
    pub default_surface_valid: bool,
    /// Allocated by `gl_context_create` (else the window system's).
    pub default_surface_owned: bool,
    /// Slot in the context pool (per-context caches key on it).
    pub index: u8,
    /// The default surface is stored bottom-up, as a texture is (an EGL
    /// pbuffer: nothing scans it out). False for a window surface, stored
    /// top-down in scan-out order, and for `gl_context_create`'s own.
    pub default_y_up: bool,

    /// First GL error since the last glGetError.
    pub error: GLenum,
}

/* ---- the pool and the current context ------------------------------------ */

static POOL: Global<[GlContext; MAX_CONTEXTS]> = Global::zeroed();
/// The indexed GL_UNIFORM_BUFFER binding points of each context slot (ES 3.0
/// 2.12.6.2), read at draw time to fill the current program's uniform
/// blocks. Kept here and not in `GlContext`, whose layout the C side shares
/// (src/abi_layout.rs).
static UNIFORM_BUFFERS: Global<[[UboBinding; MAX_UNIFORM_BUFFER_BINDINGS]; MAX_CONTEXTS]> = Global::zeroed();

/// The uniform-buffer binding points of context `c`.
pub fn uniform_buffers(c: &GlContext) -> &'static mut [UboBinding; MAX_UNIFORM_BUFFER_BINDINGS] {
    // SAFETY: single-threaded by contract; a context's slot index is below
    // MAX_CONTEXTS, and callers hold one reference at a time.
    unsafe { &mut UNIFORM_BUFFERS.get()[usize::from(c.index) % MAX_CONTEXTS] }
}
static POOL_USED: Global<[bool; MAX_CONTEXTS]> = Global::zeroed();
/// One share group per context slot: with no sharing every context has its
/// own, which is the most that can be asked for.
static GROUPS: Global<[GlObjects; MAX_CONTEXTS]> = Global::zeroed();
/// Contexts using each group. The group's objects are released when the
/// last of them goes, not when the context that made them does -- that is
/// the whole point of a share group.
static GROUP_REFS: Global<[u8; MAX_CONTEXTS]> = Global::zeroed();
static ERRORS_LOGGED: Global<u32> = Global::zeroed();

/// The GL core's lines of the run-end owner report
/// (`CONFIG_YAKOGL_PVR_POOL_OWNERS`,
/// docs/notes/pvr-render-pool.md): the objects of every share
/// group in use by kind, every context's own default surface, and the draw
/// path's programs and staging. Storage an EGLImage owns (`external`) is not
/// the object's and is left out. Returns their KiB.
#[no_mangle]
pub extern "C" fn yakogl_pool_owners() -> u64 {
    use crate::osal::OwnerTally;
    let (mut bufs, mut texs, mut rbs, mut surfs) =
        (OwnerTally::default(), OwnerTally::default(), OwnerTally::default(), OwnerTally::default());
    // SAFETY: single-threaded by contract (the report runs between frames).
    let (refs, used, pool) = unsafe { (GROUP_REFS.get(), POOL_USED.get(), POOL.get()) };
    for (g, _) in refs.iter().enumerate().filter(|(_, r)| **r != 0) {
        let o = objects(g as u8);
        for b in o.buffers.iter() {
            bufs.add(&b.mem);
        }
        for t in o.textures.iter().filter(|t| !t.external) {
            texs.add(&t.mem);
        }
        for r in o.renderbuffers.iter().filter(|r| !r.external) {
            rbs.add(&r.mem);
        }
    }
    for (c, _) in pool.iter().zip(used.iter()).filter(|(_, u)| **u) {
        if c.default_surface_valid && c.default_surface_owned {
            surfs.add(&c.default_surface);
        }
    }
    let (progs, staging) = super::draw::pool_owners();
    bufs.report("gl buffers")
        + texs.report("gl textures")
        + rbs.report("gl renderbuffers")
        + surfs.report("gl default surfaces")
        + progs.report("gl programs")
        + staging.report("gl draw staging")
}

/// The current context (`extern struct gl_context *gl_ctx;`).
#[no_mangle]
pub static mut gl_ctx: *mut GlContext = core::ptr::null_mut();

/// The objects of share group `group`, independent of any borrow of the
/// context that named it.
///
/// [`GlContext::objs`] is the one to reach for: it ties the objects to the
/// context, so the compiler keeps the references apart. This is for the few
/// places -- the draw path -- that hold a program reference while they walk
/// the buffer or texture table, which the field borrows of the old layout
/// allowed and one `&mut GlContext` does not. The convention is the one
/// [`current`] already runs on: an entry point takes what it needs once and
/// passes it down.
#[inline]
pub fn objects(group: u8) -> &'static mut GlObjects {
    // SAFETY: `group` is a live slot of GROUPS (a context holds it), and
    // the caller holds no other reference to that slot.
    unsafe { &mut GROUPS.get()[group as usize] }
}

/// The current context. Entry points call this once and pass the reference
/// down; nothing below an entry point calls it again.
///
/// None on a thread other than the owner ([`foreign_thread`]): to that
/// thread no context is current, so the call changes nothing.
#[inline]
pub fn current() -> Option<&'static mut GlContext> {
    if foreign_thread() {
        return None;
    }
    // SAFETY: gl_ctx is null or points into POOL; single-threaded by contract,
    // and callers hold at most one reference at a time (module convention).
    unsafe { gl_ctx.as_mut() }
}

/* ---- the owning thread -------------------------------------------------------
 * One thread issues every GL and EGL call of the process (GL_CORE_NOTES.md,
 * "Single-threaded by contract"; include/gles_osal.h, "threads"). The library
 * keeps no per-thread state, so a second thread would read and write the
 * owner's context. Where the OSAL can name threads (`thread_id`), the thread
 * that makes a context current owns the library until it releases it
 * (eglMakeCurrent with EGL_NO_CONTEXT, eglTerminate), and a call from any other
 * thread in that time is refused: it changes nothing and reports
 * GL_INVALID_OPERATION to its own glGetError and EGL_BAD_ACCESS to its own
 * eglGetError. The owner's state and error are untouched.
 *
 * This is detection, not locking: while nothing is current, or on a platform
 * whose `thread_id` is absent or answers 0, nothing is checked, and a second
 * thread is still outside the contract. The online compiler's re-entrancy
 * flag is a separate guard and stays so. */

/// The owner's thread id, 0 while nothing is owned (no context current, or
/// no thread ids on this platform).
static OWNER: AtomicU32 = AtomicU32::new(0);
/// The refusal log line was printed for the present owner.
static REFUSAL_LOGGED: AtomicBool = AtomicBool::new(false);

/// eglMakeCurrent bound a context: the calling thread owns the library.
pub(crate) fn owner_take() {
    REFUSAL_LOGGED.store(false, Ordering::Relaxed);
    OWNER.store(osal::thread_id(), Ordering::Relaxed);
}

/// eglMakeCurrent released the context: nothing is owned.
pub(crate) fn owner_release() {
    OWNER.store(0, Ordering::Relaxed);
}

/// True when the caller is not the thread that owns the library, and so
/// must be refused. One atomic load and a compare while nothing is owned;
/// the OSAL's `thread_id` is asked only while something is.
#[inline]
pub fn foreign_thread() -> bool {
    let owner = OWNER.load(Ordering::Relaxed);
    owner != 0 && is_foreign(owner)
}

/// The slow half of [`foreign_thread`]: asks the OSAL and logs the first
/// refusal for this owner (one line, so a render loop on the wrong thread
/// cannot flood a 115200-baud console).
#[inline(never)]
fn is_foreign(owner: u32) -> bool {
    let me = osal::thread_id();
    if me == 0 || me == owner {
        return false;
    }
    if !REFUSAL_LOGGED.swap(true, Ordering::Relaxed) {
        glog!(
            "yakogl: GL/EGL call from thread {:#x} refused: thread {:#x} has the context current (one thread per process)",
            me,
            owner
        );
    }
    true
}

/// Device-VA base of the general data heap (buffers, textures, uniforms).
#[inline]
pub fn heap_data() -> u64 {
    backend::heap_base(GLES_HEAP_DATA)
}

/// `g_gl_error_log_limit`: how many errors [`log_error`] names before it goes
/// quiet. Sixteen is right for a board whose console is 115200 baud and whose
/// client asks glGetError once a frame; a conformance run wants every one of
/// them, and raises this through the OSAL (src/osal/linux.c reads
/// YAKOGL_ERROR_LOG). Written by C, so it is a plain exported global.
#[no_mangle]
pub static mut g_gl_error_log_limit: core::ffi::c_uint = 16;

/* ---- the GL error counters ---------------------------------------------------
 * How many GL errors the entry points raised, for a run whose only result is
 * a record (the i.MX8MP has no console, so the "GL error" lines below are
 * lost there; issue #13). Two counts, because GL keeps only the first error
 * since the last glGetError (ES 2.0 2.5):
 *
 *   raised    every error an entry point raised, with or without a current
 *             context: what the "GL error 0x.. in gl.." lines count when
 *             every one is logged (YAKOGL_ERROR_LOG). A client that never
 *             calls glGetError (glmark2) sees none of them.
 *   recorded  the errors that became the context's error, that is, the ones
 *             glGetError can return. Without glGetError calls this stops at
 *             one: the first error is kept and masks every later one.
 *
 * Both saturate at u32::MAX instead of wrapping, so a read is a total since
 * the process started, never a value that went round. Process-global like
 * the other instruments (backend::draws_refused). */

/// See "the GL error counters" above: every GL error an entry point raised.
static GL_ERRORS_RAISED: AtomicU32 = AtomicU32::new(0);
/// See "the GL error counters" above: the errors that became the context's
/// error (first-error-wins let them through).
static GL_ERRORS_RECORDED: AtomicU32 = AtomicU32::new(0);

/// Add one to `n`, staying at u32::MAX once there.
pub(crate) fn count_saturating(n: &AtomicU32) {
    /* a compare-exchange loop rather than fetch_update: the newer nightly
     * deprecates fetch_update for try_update, which the older stable lacks,
     * and the gate counts every warning */
    let mut v = n.load(Ordering::Relaxed);
    while v != u32::MAX {
        match n.compare_exchange_weak(v, v + 1, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return,
            Err(now) => v = now,
        }
    }
}

/// Every GL error an entry point raised in this process (saturating).
pub fn gl_errors_raised() -> u32 {
    GL_ERRORS_RAISED.load(Ordering::Relaxed)
}

/// The GL errors that became a context's error in this process (saturating).
pub fn gl_errors_recorded() -> u32 {
    GL_ERRORS_RECORDED.load(Ordering::Relaxed)
}

/// C view of [`gl_errors_raised`]: every GL error an entry point raised in
/// this process, saturating at 0xffffffff. In every build (a product
/// record carries it: driver/samples/zephyr/glmark2).
#[no_mangle]
pub extern "C" fn yakogl_gl_errors_raised() -> u32 {
    gl_errors_raised()
}

/// C view of [`gl_errors_recorded`]: the GL errors that became a context's
/// error (the ones glGetError could return), saturating at 0xffffffff.
#[no_mangle]
pub extern "C" fn yakogl_gl_errors_recorded() -> u32 {
    gl_errors_recorded()
}

/// An entry point raised `e`: count it, keep it (the first
/// [`FIRST_ERRORS`]) and log it (the first few).
fn note_raised(e: GLenum, func: &str) {
    count_saturating(&GL_ERRORS_RAISED);
    keep_first(e, func);
    log_error(e, func);
}

/* ---- the first GL errors -----------------------------------------------------
 * The counters above say how many errors a run raised; these say which: the
 * first FIRST_ERRORS errors of the process, each with its code, the entry
 * point that raised it and the client's tag at that moment
 * (yakogl_gl_error_tag; glmark2 sets the benchmark-list entry it is running).
 * For a run whose console is lost (the i.MX8MP's glmark2, issue #814) the
 * client puts them in its record; a console only shows the first
 * g_gl_error_log_limit lines, without the tag. Process-global like the
 * counters; never cleared. */

/// How many of the first GL errors are kept (see "the first GL errors").
pub const FIRST_ERRORS: usize = 8;
/// The longest entry point name kept, in bytes; a longer one is cut.
const FIRST_ERROR_NAME: usize = 40;

/// One kept GL error.
#[derive(Clone, Copy)]
struct FirstError {
    code: GLenum,
    tag: u32,
    len: u8,
    name: [u8; FIRST_ERROR_NAME],
}

const NO_FIRST_ERROR: FirstError = FirstError { code: 0, tag: 0, len: 0, name: [0; FIRST_ERROR_NAME] };
static FIRST: Global<[FirstError; FIRST_ERRORS]> = Global::new([NO_FIRST_ERROR; FIRST_ERRORS]);
/// How many of FIRST hold an error.
static FIRST_KEPT: AtomicU32 = AtomicU32::new(0);
/// The client's tag, stored with each error kept (yakogl_gl_error_tag).
static ERROR_TAG: AtomicU32 = AtomicU32::new(0);

/// Keep `e` from `func` if fewer than [`FIRST_ERRORS`] are kept.
fn keep_first(e: GLenum, func: &str) {
    let n = FIRST_KEPT.load(Ordering::Relaxed) as usize;
    if n >= FIRST_ERRORS {
        return;
    }
    let len = func.len().min(FIRST_ERROR_NAME);
    let mut f = FirstError { code: e, tag: ERROR_TAG.load(Ordering::Relaxed), len: len as u8, ..NO_FIRST_ERROR };
    f.name[..len].copy_from_slice(&func.as_bytes()[..len]);
    // SAFETY: single-threaded by contract (as ERRORS_LOGGED); n is in range.
    unsafe { FIRST.get()[n] = f };
    FIRST_KEPT.store(n as u32 + 1, Ordering::Relaxed);
}

/// Set the tag stored with every GL error kept from now on (see "the first
/// GL errors"): glmark2's benchmark-list entry, or any value of the
/// client's. 0 until a client sets one. In every build.
#[no_mangle]
pub extern "C" fn yakogl_gl_error_tag(tag: u32) {
    ERROR_TAG.store(tag, Ordering::Relaxed);
}

/// How many of the first GL errors are kept: the errors raised, up to
/// [`FIRST_ERRORS`].
#[no_mangle]
pub extern "C" fn yakogl_gl_errors_kept() -> u32 {
    FIRST_KEPT.load(Ordering::Relaxed)
}

/// Kept GL error `i` (0 is the first the process raised): its code, the tag
/// set when it was raised, and the name of the entry point that raised it,
/// NUL-terminated in `name` and cut to `cap - 1` bytes. Returns 0, or -1 when
/// fewer than `i + 1` are kept (nothing is written then). A null pointer, or
/// a `cap` of 0 for `name`, skips that output.
///
/// # Safety
/// `code` and `tag` are null or valid for a write; `name` is null or valid
/// for `cap` bytes of writes.
#[no_mangle]
pub unsafe extern "C" fn yakogl_gl_error_first(
    i: u32,
    code: *mut u32,
    tag: *mut u32,
    name: *mut c_char,
    cap: usize,
) -> i32 {
    if i >= yakogl_gl_errors_kept() {
        return -1;
    }
    // SAFETY: single-threaded by contract; i is below FIRST_KEPT, at most
    // FIRST_ERRORS.
    let f = unsafe { FIRST.get()[i as usize] };
    // SAFETY: the caller's pointers, as documented.
    unsafe {
        if !code.is_null() {
            *code = f.code;
        }
        if !tag.is_null() {
            *tag = f.tag;
        }
        if !name.is_null() && cap > 0 {
            let n = usize::from(f.len).min(cap - 1);
            core::ptr::copy_nonoverlapping(f.name.as_ptr().cast::<c_char>(), name, n);
            *name.add(n) = 0;
        }
    }
    0
}

/// Log the first few GL errors with the entry point that raised them: a
/// client such as Qt's RHI only ever reports "glGetError 0x500" per frame.
fn log_error(e: GLenum, func: &str) {
    // SAFETY: single-threaded by contract; no other reference is live.
    let logged = unsafe { ERRORS_LOGGED.get() };
    // SAFETY: as above; the OSAL writes it once, from its init.
    let limit = unsafe { core::ptr::read(&raw const g_gl_error_log_limit) };
    if *logged < limit {
        *logged += 1;
        glog!("GL error 0x{:x} in {}", e, func);
    }
}

/// Unit tests: re-arm the "first few errors are logged" cap (process-global).
#[cfg(test)]
pub(crate) fn reset_error_log_cap() {
    // SAFETY: tests are serialised by testutil::TestGl.
    unsafe { *ERRORS_LOGGED.get() = 0 };
}

/* ---- the call tracer ---------------------------------------------------------
 * Where the GL client is: the last traced GL/EGL entry point entered and how
 * many were entered so far, for a keep-alive / watchdog thread to print when
 * the client stops making progress (yakogl_debug_last_call), plus a console
 * line for each of the first `yakogl_trace_log_first` entry points: what a
 * client that draws nothing did instead. The C view is gl_context.h and the
 * symbols are the C library's. The atomics have the layout of the plain C
 * types and may be read from another thread (the C declares them volatile). */

/// The last traced entry point (`const char *volatile yakogl_trace_call`).
#[no_mangle]
pub static yakogl_trace_call: AtomicPtr<c_char> = AtomicPtr::new(c"-".as_ptr() as *mut c_char);
/// Traced entry points entered so far (`volatile uint32_t yakogl_trace_seq`).
#[no_mangle]
pub static yakogl_trace_seq: AtomicU32 = AtomicU32::new(0);
/// Log the first N traced entry points (0: none).
///
/// Off by default (upstream e471a73). On the board's 115200-baud console
/// each line costs about ten milliseconds and the logger prints
/// synchronously, so the 4000 lines this used to emit were forty seconds of
/// startup during which the client could not run: the effects and IBL
/// examples spent their whole first minute in it and never reached a frame.
/// Bring-up sets it explicitly (`YAKOGL_TRACE_FIRST` when the OSAL passes
/// environment).
#[no_mangle]
pub static yakogl_trace_log_first: AtomicU32 = AtomicU32::new(0);

/// The run of one entry point [`yakogl_trace_log`] is collapsing.
struct TraceRun {
    func: *const c_char,
    start: u32,
    n: u32,
}

static TRACE_RUN: Global<TraceRun> = Global::new(TraceRun { func: core::ptr::null(), start: 0, n: 0 });

/// Opens every traced entry point (the C `GL_TRACE()`, through
/// [`gl_trace!`]). Where the C enters another traced entry point through its
/// public function (glBlendFunc -> glBlendFuncSeparate, glTexImage3D ->
/// glTexSubImage3D, eglTerminate -> eglMakeCurrent, ...), the Rust caller
/// traces the inner name as well, so the call counter and the log match.
#[inline]
pub fn trace(name: &'static CStr) {
    yakogl_trace_call.store(name.as_ptr() as *mut c_char, Ordering::Relaxed);
    let seq = yakogl_trace_seq.load(Ordering::Relaxed).wrapping_add(1);
    yakogl_trace_seq.store(seq, Ordering::Relaxed);
    if seq <= yakogl_trace_log_first.load(Ordering::Relaxed) {
        // SAFETY: `name` is a NUL-terminated static string.
        unsafe { yakogl_trace_log(name.as_ptr(), seq) };
    }
}

/// `GL_TRACE()`: `gl_trace!(glFoo)` traces the entry point `glFoo`.
macro_rules! gl_trace {
    ($name:ident) => {
        let __gl_name: &'static core::ffi::CStr = const {
            match core::ffi::CStr::from_bytes_with_nul(concat!(stringify!($name), "\0").as_bytes()) {
                Ok(s) => s,
                Err(_) => panic!("entry point name"),
            }
        };
        /* the entry point's CPU time, to its end (gl::timing) */
        let _gl_timing = $crate::gl::timing::Guard::enter(__gl_name);
        $crate::gl::context::trace(__gl_name)
    };
}
pub(crate) use gl_trace;

/// Runs of the same entry point collapse into one line ("gl#12 glUniform4fv
/// x37"), printed when the next different one arrives (the last run is never
/// printed, as in C). The C tells entry points apart by the address of
/// `__func__`, one per function; the Rust names are string literals that the
/// compiler may or may not merge, so equal text counts as the same entry
/// point too.
///
/// # Safety
/// `func` must be a NUL-terminated string that outlives the program (the
/// tracer keeps the pointer).
#[no_mangle]
pub unsafe extern "C" fn yakogl_trace_log(func: *const c_char, seq: u32) {
    if osal::ops().and_then(|o| o.log).is_none() {
        return;
    }
    // SAFETY: single-threaded by contract (the GL client's thread).
    let run = unsafe { TRACE_RUN.get() };
    // SAFETY: both are NUL-terminated strings that live forever (contract).
    let same = run.func == func
        || (!run.func.is_null() && !func.is_null() && unsafe { CStr::from_ptr(run.func) == CStr::from_ptr(func) });
    if same {
        run.n = run.n.wrapping_add(1);
        return;
    }
    if !run.func.is_null() {
        // SAFETY: a pointer handed to an earlier call (see the contract).
        let name = unsafe { CStr::from_ptr(run.func) };
        glog!("gl#{} {} x{}", run.start, name.to_str().unwrap_or("?"), run.n);
    }
    run.func = func;
    run.start = seq;
    run.n = 1;
}

/// The last traced entry point and (`seq` non-null) the call counter.
///
/// # Safety
/// `seq` must be null or valid for a write of a `u32`.
#[no_mangle]
pub unsafe extern "C" fn yakogl_debug_last_call(seq: *mut u32) -> *const c_char {
    if !seq.is_null() {
        // SAFETY: the caller's pointer (contract above); no alignment assumed.
        unsafe { seq.write_unaligned(yakogl_trace_seq.load(Ordering::Relaxed)) };
    }
    yakogl_trace_call.load(Ordering::Relaxed)
}

/// The GPU address of the current context's default framebuffer target (the
/// window surface or pbuffer it was made current with), or 0 when there is
/// no current context or it has no surface. For a sample to record which
/// address its picture was drawn at: dEQP's color_clear on the i.MX8MP
/// changes result with the image, deterministically, and GPU allocations
/// there are only 64-byte aligned (CONFIG_YAKOGL_GC_ALLOC_PAGE_ALIGN is the
/// A/B). Read-only; it changes nothing.
#[no_mangle]
pub extern "C" fn yakogl_debug_default_target_gpu() -> u64 {
    current().and_then(|c| c.default_surface().map(|m| m.gpu)).unwrap_or(0)
}

/// Unit tests: restart the tracer (process-global).
#[cfg(test)]
pub(crate) fn reset_trace() {
    yakogl_trace_seq.store(0, Ordering::Relaxed);
    yakogl_trace_call.store(c"-".as_ptr() as *mut c_char, Ordering::Relaxed);
    // SAFETY: tests are serialised by testutil::TestGl.
    unsafe { *TRACE_RUN.get() = TraceRun { func: core::ptr::null(), start: 0, n: 0 } };
}

/// Record an error when there may be no current context (GL keeps the first
/// error since the last glGetError). Must not be called while a `&mut
/// GlContext` is held: use [`GlContext::set_error`] there.
pub fn set_error_no_ctx(e: GLenum, func: &str) {
    if foreign_thread() {
        return; /* refused: the error belongs to the owner's context */
    }
    match current() {
        Some(c) => c.set_error(e, func),
        None => note_raised(e, func),
    }
}

/* GL ES 2.0 section 2.10: shader objects and program objects share ONE name
 * space. A name that was never generated is GL_INVALID_VALUE; a name of the
 * other kind is GL_INVALID_OPERATION, and nearly every entry point is asked
 * to tell the two apart. The two kinds live in separate tables here, so a
 * name carries its kind in the low bit and its table index above it: the
 * lookups then reject the wrong kind for free and the tables are untouched.
 * Index 0 stays reserved, which keeps both 0 and 1 out of every name
 * glCreateShader / glCreateProgram can hand out. */

/// The low bit of a program object's name.
pub const OBJ_PROGRAM: GLuint = 0;
/// The low bit of a shader object's name.
pub const OBJ_SHADER: GLuint = 1;

/// The name of table slot `index` of that kind.
#[inline]
pub fn obj_name(index: usize, kind: GLuint) -> GLuint {
    ((index as GLuint) << 1) | kind
}

/// The table slot a shader/program name refers to (whatever its kind).
#[inline]
pub fn obj_index(name: GLuint) -> usize {
    (name >> 1) as usize
}

/// True when the name is a shader object's rather than a program object's.
#[inline]
pub fn obj_is_shader(name: GLuint) -> bool {
    name & 1 == OBJ_SHADER
}

macro_rules! lookup {
    ($name:ident, $name_ref:ident, $field:ident, $ty:ty, $max:expr) => {
        /// `None` for handle 0, an out-of-range handle or a free slot.
        #[inline]
        pub fn $name(&mut self, h: GLuint) -> Option<&mut $ty> {
            if h == 0 || h as usize >= $max {
                return None;
            }
            let o = &mut self.$field[h as usize];
            o.used.then_some(o)
        }

        #[inline]
        pub fn $name_ref(&self, h: GLuint) -> Option<&$ty> {
            if h == 0 || h as usize >= $max {
                return None;
            }
            let o = &self.$field[h as usize];
            o.used.then_some(o)
        }
    };
}

impl GlObjects {
    lookup!(buffer, buffer_ref, buffers, GlBuffer, MAX_BUFFERS);
    lookup!(texture, texture_ref, textures, GlTexture, MAX_TEXTURES);
    /// The shader object a name refers to: `None` for a program name, a name
    /// nothing created, or a freed slot (the one name space above).
    #[inline]
    pub fn shader(&mut self, h: GLuint) -> Option<&mut GlShader> {
        let i = obj_index(h);
        if !obj_is_shader(h) || i == 0 || i >= MAX_SHADERS {
            return None;
        }
        let o = &mut self.shaders[i];
        o.used.then_some(o)
    }

    #[inline]
    pub fn shader_ref(&self, h: GLuint) -> Option<&GlShader> {
        let i = obj_index(h);
        if !obj_is_shader(h) || i == 0 || i >= MAX_SHADERS {
            return None;
        }
        let o = &self.shaders[i];
        o.used.then_some(o)
    }

    /// The program object a name refers to (mirror of [`GlObjects::shader`]).
    #[inline]
    pub fn program(&mut self, h: GLuint) -> Option<&mut GlProgram> {
        let i = obj_index(h);
        if obj_is_shader(h) || i == 0 || i >= MAX_PROGRAMS {
            return None;
        }
        let o = &mut self.programs[i];
        o.used.then_some(o)
    }

    #[inline]
    pub fn program_ref(&self, h: GLuint) -> Option<&GlProgram> {
        let i = obj_index(h);
        if obj_is_shader(h) || i == 0 || i >= MAX_PROGRAMS {
            return None;
        }
        let o = &self.programs[i];
        o.used.then_some(o)
    }
    lookup!(renderbuffer, renderbuffer_ref, renderbuffers, GlRenderbuffer, MAX_RENDERBUFFERS);
}

impl GlContext {
    /// The objects this context shares: buffers, textures, renderbuffers,
    /// shaders and programs (ES 3.0 2.6.1). Every table access goes through
    /// here, so a context that shares a group reads and writes the same
    /// objects as the context it was created with.
    #[inline]
    pub fn objs(&mut self) -> &mut GlObjects {
        // SAFETY: `group` is a slot this context holds a reference to for
        // as long as it lives (alloc_context takes one, destroy_with gives
        // it back), and the caller holds no other reference to that slot --
        // one context is current at a time.
        unsafe { &mut GROUPS.get()[self.group as usize] }
    }

    /// [`GlContext::objs`] without taking the context mutably.
    #[inline]
    pub fn objs_ref(&self) -> &GlObjects {
        // SAFETY: as `objs`.
        unsafe { &GROUPS.get()[self.group as usize] }
    }

    /// Record `e` (the first error wins) and log the entry point.
    pub fn set_error(&mut self, e: GLenum, func: &str) {
        note_raised(e, func);
        if self.error == GL_NO_ERROR {
            self.error = e;
            count_saturating(&GL_ERRORS_RECORDED);
        }
    }

    /* The shared objects are looked up in the group (see [`GlObjects`]);
     * the names stay on the context, which is what every caller has. */
    pub fn buffer(&mut self, h: GLuint) -> Option<&mut GlBuffer> {
        self.objs().buffer(h)
    }

    pub fn buffer_ref(&self, h: GLuint) -> Option<&GlBuffer> {
        self.objs_ref().buffer_ref(h)
    }

    pub fn texture(&mut self, h: GLuint) -> Option<&mut GlTexture> {
        self.objs().texture(h)
    }

    pub fn texture_ref(&self, h: GLuint) -> Option<&GlTexture> {
        self.objs_ref().texture_ref(h)
    }

    pub fn shader(&mut self, h: GLuint) -> Option<&mut GlShader> {
        self.objs().shader(h)
    }

    pub fn shader_ref(&self, h: GLuint) -> Option<&GlShader> {
        self.objs_ref().shader_ref(h)
    }

    pub fn program(&mut self, h: GLuint) -> Option<&mut GlProgram> {
        self.objs().program(h)
    }

    pub fn program_ref(&self, h: GLuint) -> Option<&GlProgram> {
        self.objs_ref().program_ref(h)
    }

    pub fn renderbuffer(&mut self, h: GLuint) -> Option<&mut GlRenderbuffer> {
        self.objs().renderbuffer(h)
    }

    pub fn renderbuffer_ref(&self, h: GLuint) -> Option<&GlRenderbuffer> {
        self.objs_ref().renderbuffer_ref(h)
    }

    /* A name and an object are two different things: glGenTextures /
     * glGenRenderbuffers reserve a name, and the object behind it exists only
     * once the name has been bound (ES 2.0 2.1: "objects are created by
     * binding"). `texture_ref` / `renderbuffer_ref` answer the first question
     * -- is the name taken -- which is what the allocator and the deleters
     * need; the two below answer the second, which is what every caller that
     * must reject a name with no object behind it needs: glIsTexture,
     * glIsRenderbuffer and the framebuffer attachment calls (ES 2.0 4.4.3:
     * attaching a name that is not an object is INVALID_OPERATION). Keeping
     * them apart by name is the point: the attachment calls asked the first
     * question and silently attached a name with nothing behind it. */

    /// The texture object behind `h`, if `h` has been bound at least once.
    pub fn texture_object(&self, h: GLuint) -> Option<&GlTexture> {
        self.texture_ref(h).filter(|t| t.target != 0 && !t.orphan)
    }

    /// The renderbuffer object behind `h`, if `h` has been bound at least once.
    pub fn renderbuffer_object(&self, h: GLuint) -> Option<&GlRenderbuffer> {
        self.renderbuffer_ref(h).filter(|rb| rb.bound && !rb.orphan)
    }

    lookup!(framebuffer, framebuffer_ref, framebuffers, GlFramebuffer, MAX_FRAMEBUFFERS);

    /// The default (window) colour surface, if one is bound.
    pub fn default_surface(&mut self) -> Option<&mut OsalGpuMem> {
        self.default_surface_valid.then_some(&mut self.default_surface)
    }

    fn set_defaults(&mut self, w: u32, h: u32) {
        // SAFETY: every field of GlContext is valid when all-zero.
        unsafe { core::ptr::write_bytes(self as *mut GlContext, 0, 1) };

        self.viewport = [0, 0, w as GLint, h as GLint];
        self.depth_range = [0.0, 1.0];
        self.clear_depth = 1.0;

        self.depth_func = GL_LESS;
        self.depth_mask = true;

        self.blend_src = GL_ONE;
        self.blend_dst = GL_ZERO;
        self.blend_eq = GL_FUNC_ADD;
        self.blend_src_a = GL_ONE;
        self.blend_dst_a = GL_ZERO;
        self.blend_eq_a = GL_FUNC_ADD;

        self.cull_mode = GL_BACK;
        self.front_face = GL_CCW;

        self.scissor = [0, 0, w as GLint, h as GLint];

        self.stencil_func = GL_ALWAYS;
        self.stencil_mask = 0xFFFF_FFFF;
        self.stencil_sfail = GL_KEEP;
        self.stencil_dpfail = GL_KEEP;
        self.stencil_dppass = GL_KEEP;
        self.stencil_writemask = 0xFFFF_FFFF;
        self.stencil_back_func = GL_ALWAYS;
        self.stencil_back_mask = 0xFFFF_FFFF;
        self.stencil_back_sfail = GL_KEEP;
        self.stencil_back_dpfail = GL_KEEP;
        self.stencil_back_dppass = GL_KEEP;
        self.stencil_back_writemask = 0xFFFF_FFFF;

        self.color_writemask = [1.0; 4];
        self.line_width = 1.0;
        for a in self.current_attrib.iter_mut() {
            *a = [0.0, 0.0, 0.0, 1.0];
        }
        self.unpack_alignment = 4;
        self.pack_alignment = 4;
        self.dither = true;
        self.sample_coverage_value = 1.0;
        self.generate_mipmap_hint = GL_DONT_CARE;

        self.target_w = w;
        self.target_h = h;
        self.default_samples = 1;
        self.error = GL_NO_ERROR;
    }
}

impl GlContext {
    /// The default surface's row pitch in pixels ([`GlContext::default_pitch`]).
    pub fn default_pitch_px(&self) -> u32 {
        if self.default_pitch != 0 {
            self.default_pitch
        } else {
            tex_pitch(self.target_w)
        }
    }
}

/// No group: `gl_context_create_for_surface` takes one of its own, and
/// `gl_group_alloc` answers this when none is free.
pub const GROUP_NONE: u8 = 0xff;

/// Take a reference to share group `share`, or to a new empty one when
/// `share` is [`GROUP_NONE`]. `GROUP_NONE` back when every slot is in use.
///
/// The caller holds the reference until it calls [`gl_group_release`]. EGL
/// takes one per EGLContext, because an EGLContext exists (and can be named
/// as another's `share_context`) long before the first eglMakeCurrent gives
/// it a GL context.
#[no_mangle]
pub extern "C" fn gl_group_alloc(share: u8) -> u8 {
    // SAFETY: single-threaded by contract; a free slot has no live reference.
    let refs = unsafe { GROUP_REFS.get() };
    if share != GROUP_NONE {
        /* an existing group: one more holder, the objects stay */
        let Some(r) = refs.get_mut(share as usize).filter(|r| **r > 0) else {
            return GROUP_NONE;
        };
        *r += 1;
        return share;
    }
    let Some(g) = refs.iter().position(|r| *r == 0) else {
        return GROUP_NONE;
    };
    // SAFETY: the slot has no holder and so no live reference; every field
    // of GlObjects is valid when all-zero (the pool hands out a slot as its
    // last group left it).
    unsafe { core::ptr::write_bytes(&mut GROUPS.get()[g] as *mut GlObjects, 0, 1) };
    super::names::reset_group(g);
    refs[g] = 1;
    g as u8
}

/// Give back a reference taken by [`gl_group_alloc`] or by creating a
/// context in the group. The objects go when the last holder does.
#[no_mangle]
pub extern "C" fn gl_group_release(group: u8) {
    group_release_with(group, &mut |m| super::egl::image_storage_release(m));
}

/// [`gl_group_release`] with the caller's own release of EGLImage storage
/// (EGL passes a function over the state it already holds, as it does for
/// [`destroy_with`]).
pub fn group_release_with(group: u8, release_external: &mut dyn FnMut(&OsalGpuMem)) {
    if group as usize >= MAX_CONTEXTS {
        return;
    }
    // SAFETY: single-threaded by contract.
    let refs = unsafe { GROUP_REFS.get() };
    refs[group as usize] = refs[group as usize].saturating_sub(1);
    if refs[group as usize] != 0 {
        return;
    }
    /* the last holder: the objects and their GPU storage go. Storage an
     * EGLImage owns is only unreferenced -- the image, or a sibling in
     * another group, may still use it. */
    super::buffer::release_retired(group, None);
    let o = objects(group);
    for b in o.buffers.iter_mut().filter(|b| b.used && !b.mem.impl_.is_null()) {
        osal::gpu_free(&mut b.mem);
    }
    for t in o.textures.iter_mut().filter(|t| t.used) {
        backend::attachment_gone(ds_object_id(t.serial, true));
        release_storage_with(&mut t.mem, &mut t.external, release_external);
    }
    for r in o.renderbuffers.iter_mut().filter(|r| r.used) {
        backend::attachment_gone(ds_object_id(r.serial, false));
        release_storage_with(&mut r.mem, &mut r.external, release_external);
    }
    for s in o.shaders.iter_mut().filter(|s| s.used) {
        if !s.source.is_null() {
            // SAFETY: the copy glShaderSource made, freed once.
            unsafe { crate::util::free(s.source as *mut c_void) };
            s.source = core::ptr::null_mut();
        }
        crate::shaderc::release_shader(&s.compiled_shader);
        s.compiled_shader = ShcShader::ZERO;
    }
    for i in 0..MAX_PROGRAMS {
        if objects(group).programs[i].used {
            /* the GPU side keys on the group, not on a context: the last
             * context of the group may be gone already */
            super::draw::release_program_in(group, obj_name(i, OBJ_PROGRAM));
            let p = &mut objects(group).programs[i];
            p.free_uniforms();
            crate::shaderc::release_shader(&p.rvs);
            crate::shaderc::release_shader(&p.rfs);
            p.rvs = ShcShader::ZERO;
            p.rfs = ShcShader::ZERO;
        }
    }
}

/// A context of the pool, in share group `group`.
///
/// `GROUP_NONE` takes a fresh group for it. Any other value is a group the
/// caller holds a reference to; the context takes one of its own, so the
/// objects outlive whichever of the two goes first.
fn alloc_context(w: u32, h: u32, group: u8) -> Option<&'static mut GlContext> {
    // SAFETY: single-threaded by contract; a free slot has no live reference.
    let (pool, used) = unsafe { (POOL.get(), POOL_USED.get()) };
    let i = used.iter().position(|u| !*u)?;
    /* the group (and a reference to it) before the slot is taken: a group
     * pool that is full must leave the context pool as it was */
    let g = match gl_group_alloc(group) {
        GROUP_NONE => return None,
        g => g,
    };
    used[i] = true;
    let c = &mut pool[i];
    c.set_defaults(w, h);
    c.index = i as u8;
    super::names::reset_context(i);
    super::tf::reset_context(c.index);
    super::es3::reset_context_queries(c.index);
    /* the slot's binding points start unbound, whatever a context that had
     * the slot before left in them */
    *uniform_buffers(c) = [UboBinding::default(); MAX_UNIFORM_BUFFER_BINDINGS];
    c.group = g;
    /* state kept per context slot outside GlContext (hints) */
    super::state::reset_context_hints(c.index);
    super::framebuffer::reset_loss_seen(c.index);
    super::framebuffer::reset_default_draw_buffer(c.index);
    super::draw::reset_pending_clear(c.index);
    Some(c)
}

/* ---- C ABI (src/gl_context.h, src/gl_internal.h) -----------------------------
 * Used by EGL, the on-silicon test suite and the samples' glue. */

/// Create a context that owns a `w` x `h` RGBA8888 default surface and make
/// it current. Null when the pool is exhausted.
#[no_mangle]
pub unsafe extern "C" fn gl_context_create(w: u32, h: u32) -> *mut GlContext {
    let Some(c) = alloc_context(w, h, GROUP_NONE) else {
        return core::ptr::null_mut();
    };
    /* An EGL surface supplies this buffer normally; for bring-up the core
     * owns it so the draw path always has a valid target. */
    switch_current(c);
    /* rows are pair-padded (tex_pitch) by every reader and writer of the
     * surface: an odd width must be allocated that way too; a backend that
     * wants its rows on `linear_row_align` bytes gets them so, and the
     * surface states the pitch */
    let bytes = linear_surface_bytes(w, h, RGBA8_BPP);
    if bytes == 0 {
        glog!("gl_context_create: {}x{} has no default surface", w, h);
        return c;
    }
    if osal::gpu_alloc(heap_data(), bytes, OSAL_MEM_DATA, &mut c.default_surface) == 0 {
        /* pool memory is not cleared by the platform: a glReadPixels before
         * the first glClear must not hand out an earlier user's pixels */
        clear_storage(&mut c.default_surface);
        c.default_surface_valid = true;
        c.default_surface_owned = true;
        let pitch = linear_pitch(w, RGBA8_BPP);
        c.default_pitch = if pitch == tex_pitch(w) { 0 } else { pitch };
    }
    c
}

/// The largest render target side (the PBE's maximum clip rectangle).
pub const MAX_SURFACE_SIDE: u32 = 16384;

/// Bytes of a `w` x `h` render target of `bpp`-byte pixels at the PBE's
/// pair-padded row pitch; 0 for an empty size or one that does not fit the
/// address space (checked: the shipped profile has no overflow checks, and
/// a wrapped size would allocate a buffer the PBE then writes past).
pub fn surface_bytes(w: u32, h: u32, bpp: u32) -> usize {
    /* MAX_SURFACE_SIDE: the PBE's clip rectangle and the TPU's texture
     * size; a larger surface has no valid size, so nothing is allocated */
    if w == 0 || h == 0 || w > MAX_SURFACE_SIDE || h > MAX_SURFACE_SIDE {
        return 0;
    }
    let pitch = (w as usize).checked_add(1).map_or(0, |p| p & !1);
    pitch.checked_mul(h as usize).and_then(|n| n.checked_mul(bpp as usize)).unwrap_or(0)
}

/// [`surface_bytes`] of a surface the GL core allocates itself, at its
/// [`linear_pitch`] (a renderbuffer, a pbuffer, an owned default surface).
pub fn linear_surface_bytes(w: u32, h: u32, bpp: u32) -> usize {
    if surface_bytes(w, h, bpp) == 0 {
        return 0;
    }
    (linear_pitch(w, bpp) as usize).checked_mul(h as usize).and_then(|n| n.checked_mul(bpp as usize)).unwrap_or(0)
}

/// Zero a fresh allocation (the zephyr / bare-metal pools hand out memory
/// as the previous user left it; GL leaves new storage undefined, but it
/// must not leak another object's contents). Costs one pass over the
/// allocation per (re)allocation: a memset plus a cache flush, or chunked
/// `gpu_write`s without a CPU view. Returns false when the platform
/// reported a failed write (the storage content is then unknown).
pub fn clear_storage(m: &mut OsalGpuMem) -> bool {
    if m.impl_.is_null() || m.size == 0 {
        return true;
    }
    if !m.cpu.is_null() {
        // SAFETY: a live allocation of `size` bytes with a CPU mapping.
        // (the flush covers exactly that mapping)
        unsafe {
            core::ptr::write_bytes(m.cpu as *mut u8, 0, m.size);
            osal::cache_flush(m.cpu, m.size);
        }
        return true;
    }
    let Some(gpu_write) = osal::ops().and_then(|o| o.gpu_write) else {
        return true; /* no way to reach the memory from the CPU at all */
    };
    const CHUNK: usize = 256;
    let zeros = [0u8; CHUNK];
    let mut off = 0;
    while off < m.size {
        let n = (m.size - off).min(CHUNK);
        // SAFETY: `zeros` holds n bytes; the OSAL bounds the write.
        if unsafe { gpu_write(m, off, zeros.as_ptr() as *const c_void, n) } != 0 {
            return false;
        }
        off += n;
    }
    true
}

/// Drop an object's hold on its storage: pool memory the object owns is
/// freed, storage an EGLImage owns (`external`) loses one user (EGL frees
/// it once the image is destroyed and no object uses it any more).
/// `release_external` does the latter, so EGL can pass its own state in.
pub fn release_storage_with(mem: &mut OsalGpuMem, external: &mut bool, release_external: &mut dyn FnMut(&OsalGpuMem)) {
    if !mem.impl_.is_null() {
        /* before the free is scheduled: a backend that keeps something for
         * this memory drops it before the address can be handed out again */
        crate::backend::texture_released(mem.gpu, mem.size as u64);
        /* and the native cube's and native 2D array's copies of it
         * (gl::native_cube, gl::native_array) */
        super::native_cube::released(mem.gpu);
        super::native_array::released(mem.gpu);
        super::native_vol::released(mem.gpu);
        if *external {
            release_external(mem);
        } else {
            osal::gpu_free(mem);
        }
    }
    *mem = OsalGpuMem::ZERO;
    *external = false;
}

/// [`release_storage_with`] outside EGL (a GL entry point).
pub fn release_storage(mem: &mut OsalGpuMem, external: &mut bool) {
    release_storage_with(mem, external, &mut |m| super::egl::image_storage_release(m));
}

/// Create a context whose default surface is memory the caller mapped for
/// the GPU (e.g. the scan-out buffer; `format` is a `GLES_SURF_*`). The
/// caller keeps ownership of `surface`.
///
/// `group`: the share group whose objects the context is to use -- what
/// EGL's `share_context` becomes (ES 3.0 2.6.1: buffers, textures,
/// renderbuffers, shaders and programs). [`GROUP_NONE`] gives it objects of
/// its own. The caller keeps its own reference to a group it passes.
#[no_mangle]
pub unsafe extern "C" fn gl_context_create_for_surface(
    surface: *mut OsalGpuMem,
    w: u32,
    h: u32,
    format: u8,
    samples: u8,
    group: u8,
) -> *mut GlContext {
    let Some(s) = surface.as_ref() else {
        return core::ptr::null_mut();
    };
    if s.gpu == 0 {
        return core::ptr::null_mut();
    }
    /* the GPU writes rows of whole pixel pairs and every reader walks the
     * same pitch: a buffer that does not hold them is refused rather than
     * written past (a caller's `size` of 0 says "unknown", as before) */
    let need = surface_bytes(w, h, crate::backend::gles_surf_bytes_per_pixel(format));
    if need == 0 || (s.size != 0 && s.size < need) {
        glog!("gl_context_create_for_surface: {}x{} format {} needs {} bytes, got {}", w, h, format, need, s.size);
        return core::ptr::null_mut();
    }
    let Some(c) = alloc_context(w, h, group) else {
        return core::ptr::null_mut();
    };
    c.default_format = format;
    c.default_samples = samples.max(1);
    c.default_pitch = 0;
    c.default_surface = *s;
    c.default_surface_valid = true;
    c.default_surface_owned = false;
    switch_current(c);
    c
}

/// Make `c` current (null = none), flushing the outgoing context's pending
/// clear first. EGL 1.5 3.7.3: "If the calling thread already has a current
/// context of the same client API type as ctx, then that context is flushed
/// and marked as no longer current." A clear is a command of the context
/// that issued it: it is carried out for that context, never left for
/// another one's draw, and never lost (review R-04). eglMakeCurrent flushed
/// it already; the C API and gl_context_create* did not.
fn switch_current(c: *mut GlContext) {
    // SAFETY: gl_ctx is null or points into POOL; no other reference to the
    // outgoing context is live across this call.
    unsafe {
        if let Some(prev) = gl_ctx.as_mut() {
            if !core::ptr::eq(prev, c) && super::draw::has_pending_clear(prev) {
                super::draw::flush_pending_clear(prev);
            }
        }
        gl_ctx = c;
    }
}

/// Make a context current (null = none). The outgoing context's pending
/// clear is carried out first, as eglMakeCurrent does ([`switch_current`]).
#[no_mangle]
pub unsafe extern "C" fn gl_context_set_current(c: *mut GlContext) {
    switch_current(c);
}

/// Repoint the current context's default surface without resetting its state.
/// 0 on success, `-EINVAL` when the surface cannot be the render target.
///
/// The return value is the point of the function as much as the rebinding is.
/// A refusal leaves `default_surface`, `target_w` and `target_h` on the
/// PREVIOUS surface, and the caller that ignores it goes on believing the new
/// one is bound: the application then draws into the old surface and reads
/// back the new one, with no error anywhere. That is wrong pixels out of a
/// silent `return`, which is why this reports rather than logs.
///
/// Every surface `egl.rs` creates is large enough by construction today
/// (`surfaces_egl_creates_are_always_large_enough_to_bind` pins that), so the
/// refusal is a guard on the C entry point, which any window-system glue may
/// call with a buffer of its own.
#[no_mangle]
pub unsafe extern "C" fn gl_context_bind_surface(
    surface: *mut OsalGpuMem,
    w: u32,
    h: u32,
    format: u8,
    samples: u8,
) -> i32 {
    let (Some(c), Some(s)) = (current(), surface.as_ref()) else {
        return -EINVAL;
    };
    /* as gl_context_create_for_surface: the GPU writes rows of whole pixel
     * pairs, so a buffer that does not hold them is not a render target */
    let need = surface_bytes(w, h, crate::backend::gles_surf_bytes_per_pixel(format));
    if need == 0 || (s.size != 0 && s.size < need) {
        glog!("gl_context_bind_surface: {}x{} format {} needs {} bytes, got {}", w, h, format, need, s.size);
        return -EINVAL;
    }
    if c.default_surface_valid && c.default_surface_owned {
        osal::gpu_free(&mut c.default_surface);
    }
    c.default_surface = *s;
    c.default_surface_valid = true;
    c.default_surface_owned = false;
    c.target_w = w;
    c.target_h = h;
    c.default_format = format;
    c.default_samples = samples.max(1);
    c.default_pitch = 0;
    0
}

/// The current context's default surface is `pitch` pixels a row instead
/// of `tex_pitch(target_w)`: a window that is a view into a wider scan-out
/// buffer (egl.rs, after binding it). 0 restores the packed pitch. -EINVAL,
/// changing nothing, when there is no current context or default surface,
/// when `pitch` is odd or below `tex_pitch(target_w)` (the GPUs write whole
/// pixel pairs, gl/context.rs `tex_pitch`), above MAX_SURFACE_SIDE, or when
/// the surface's memory does not hold `target_h` rows of it.
pub fn set_default_pitch(pitch: u32) -> i32 {
    let Some(c) = current() else {
        return -EINVAL;
    };
    if pitch == 0 {
        c.default_pitch = 0;
        return 0;
    }
    if !c.default_surface_valid || !pitch.is_multiple_of(2) || pitch < tex_pitch(c.target_w) || pitch > MAX_SURFACE_SIDE
    {
        return -EINVAL;
    }
    let bpp = crate::backend::gles_surf_bytes_per_pixel(c.default_format) as usize;
    let need = pitch as usize * c.target_h as usize * bpp;
    if c.default_surface.size != 0 && c.default_surface.size < need {
        glog!(
            "set_default_pitch: {} pixels x {} rows needs {} bytes, got {}",
            pitch,
            c.target_h,
            need,
            c.default_surface.size
        );
        return -EINVAL;
    }
    c.default_pitch = pitch;
    0
}

#[no_mangle]
pub unsafe extern "C" fn gl_context_destroy(c: *mut GlContext) {
    destroy_with(c, &mut |m| super::egl::image_storage_release(m));
}

/// gl_context_destroy, with the release of EGLImage storage the context's
/// textures and renderbuffers use done by `release_external` (EGL passes a
/// function over the state it already holds).
///
/// # Safety
/// `c` is null or a live context of the pool; no reference to it is held.
pub unsafe fn destroy_with(c: *mut GlContext, release_external: &mut dyn FnMut(&OsalGpuMem)) {
    let prev = gl_ctx;
    let Some(ctx) = c.as_mut() else {
        return;
    };
    gl_ctx = c; /* the per-context caches key on the current context */

    /* Its pending clear is carried out, not dropped: the clear was issued,
     * and EGL destroys a current context only once it is released, which
     * flushes it (EGL 1.5 3.2, 3.7.3). Every other switch flushed the
     * clear of the context it left, so only a current context can still
     * hold one. The frame is finished before the context's own surface is
     * freed below: the GPU must not write it after the free. */
    if super::draw::has_pending_clear(ctx) {
        super::draw::flush_pending_clear(ctx);
        crate::backend::flush();
        crate::backend::finish();
    }
    super::draw::reset_pending_clear(ctx.index);
    /* queries are per context (ES 3.0 Appendix D shares none) */
    super::es3::reset_context_queries(ctx.index);

    /* The objects belong to the share group, so the context only gives its
     * reference back: they go with the last holder, which may be another
     * context of the group or the EGLContext that named it. */
    group_release_with(ctx.group, release_external);

    if ctx.default_surface_valid && ctx.default_surface_owned {
        osal::gpu_free(&mut ctx.default_surface);
    }
    ctx.default_surface_valid = false;
    ctx.default_surface_owned = false;
    POOL_USED.get()[ctx.index as usize] = false;
    gl_ctx = if prev == c { core::ptr::null_mut() } else { prev };
}

/// Record a GL error from C (`gl_set_error()` in gl_context.h).
#[no_mangle]
pub unsafe extern "C" fn gl_set_error_at(e: GLenum, where_: *const c_char) {
    let name = crate::util::cstr_bytes(where_);
    let name = core::str::from_utf8(name).unwrap_or("?");
    set_error_no_ctx(e, if name.is_empty() { "?" } else { name });
}

#[no_mangle]
pub unsafe extern "C" fn gl_default_surface() -> *mut OsalGpuMem {
    current().and_then(|c| c.default_surface()).map_or(core::ptr::null_mut(), |m| m as *mut _)
}

macro_rules! c_lookup {
    ($cname:ident, $method:ident, $ty:ty) => {
        #[no_mangle]
        pub unsafe extern "C" fn $cname(h: GLuint) -> *mut $ty {
            current().and_then(|c| c.$method(h)).map_or(core::ptr::null_mut(), |o| o as *mut _)
        }
    };
    /* the types an application may name without glGen* (gl/names.rs) */
    ($cname:ident, $method:ident, $ty:ty, $kind:ident) => {
        #[no_mangle]
        pub unsafe extern "C" fn $cname(h: GLuint) -> *mut $ty {
            current()
                .and_then(|c| super::names::slot_of(c, super::names::Kind::$kind, h).and_then(|s| c.$method(s)))
                .map_or(core::ptr::null_mut(), |o| o as *mut _)
        }
    };
}

c_lookup!(gl_lookup_buffer, buffer, GlBuffer, Buffer);
c_lookup!(gl_lookup_texture, texture, GlTexture, Texture);
c_lookup!(gl_lookup_shader, shader, GlShader);
c_lookup!(gl_lookup_program, program, GlProgram);
c_lookup!(gl_lookup_framebuffer, framebuffer, GlFramebuffer, Framebuffer);
c_lookup!(gl_lookup_renderbuffer, renderbuffer, GlRenderbuffer, Renderbuffer);

/// Unit tests: an empty context pool, no current context, the error log cap
/// re-armed (process-global; testutil::TestGl calls this when it is created
/// and dropped, so a test that panicked with contexts alive leaks no slot).
#[cfg(test)]
pub(crate) fn reset_for_tests() {
    // SAFETY: tests are serialised by testutil::TestGl; no GL call is running.
    unsafe {
        *ERRORS_LOGGED.get() = 0;
        *POOL_USED.get() = [false; MAX_CONTEXTS];
        /* the share groups go with the contexts that held them; a leaked
         * group would run the pool out one test at a time */
        *GROUP_REFS.get() = [0; MAX_CONTEXTS];
        gl_ctx = core::ptr::null_mut();
    }
    owner_release();
    REFUSAL_LOGGED.store(false, Ordering::Relaxed);
    FIRST_KEPT.store(0, Ordering::Relaxed);
    ERROR_TAG.store(0, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::{GLES_SURF_RGB565, GLES_SURF_RGBA8888, GLES_SURF_XRGB8888};
    use crate::testutil::{MemEvent, TestGl};

    #[test]
    fn the_default_target_address_is_the_current_surface() {
        /* Only while the TestGl holds the test lock: the current context is
         * process-global, so "none after it drops" would race other tests. */
        let gl = crate::testutil::TestGl::new(4, 4);
        let want = gl.ctx().default_surface.gpu;
        assert_ne!(want, 0, "the fixture's surface has an address");
        assert_eq!(yakogl_debug_default_target_gpu(), want);
    }

    #[test]
    fn the_object_pools_hold_what_qt_quick_3d_asks_for() {
        /* upstream 7219228: a post-processing chain keeps a framebuffer per
         * render target of every pass, and 32 slots ran out (the demos with
         * effects presented nothing). The numbers are also in gl_context.h,
         * and src/abi_layout.rs pins the table offsets they produce. */
        assert_eq!((MAX_BUFFERS, MAX_TEXTURES, MAX_SHADERS, MAX_PROGRAMS), (512, 512, 512, 512));
        assert_eq!((MAX_FRAMEBUFFERS, MAX_RENDERBUFFERS), (256, 64));
    }

    /// A context besides the TestGl one. Dropping it destroys it (also when
    /// the test panics, so the pool slot never leaks into other tests) and
    /// makes `restore` current again. Declare it after the TestGl guard.
    struct Extra {
        ctx: *mut GlContext,
        restore: *mut GlContext,
    }

    impl Extra {
        fn owned(gl: &TestGl, w: u32, h: u32) -> Extra {
            let restore = gl.ctx() as *mut GlContext;
            // SAFETY: tests are serialised by the TestGl guard.
            let ctx = unsafe { gl_context_create(w, h) };
            Extra { ctx, restore }
        }

        fn for_surface(gl: &TestGl, s: *mut OsalGpuMem, w: u32, h: u32, format: u8, samples: u8) -> Extra {
            let restore = gl.ctx() as *mut GlContext;
            // SAFETY: tests are serialised by the TestGl guard.
            let ctx = unsafe { gl_context_create_for_surface(s, w, h, format, samples, GROUP_NONE) };
            Extra { ctx, restore }
        }

        #[allow(clippy::mut_from_ref)] /* like TestGl::ctx: the context is not owned by the guard */
        fn get(&self) -> &mut GlContext {
            assert!(!self.ctx.is_null());
            // SAFETY: the context lives until drop; tests are single-threaded.
            unsafe { &mut *self.ctx }
        }
    }

    impl Drop for Extra {
        fn drop(&mut self) {
            // SAFETY: destroy tolerates null; `restore` outlives this guard.
            unsafe {
                gl_context_destroy(self.ctx);
                gl_context_set_current(self.restore);
            }
        }
    }

    /// The current-context pointer, by value.
    fn cur() -> *mut GlContext {
        // SAFETY: a plain read; tests are serialised.
        unsafe { gl_ctx }
    }

    /// Caller-owned "scan-out" memory.
    fn caller_surface(bytes: usize) -> OsalGpuMem {
        let mut m = OsalGpuMem::ZERO;
        assert_eq!(osal::gpu_alloc(heap_data(), bytes, OSAL_MEM_DATA, &mut m), 0);
        m
    }

    /* ---- pure ---------------------------------------------------------------- */

    #[test]
    fn tex_pitch_rounds_up_to_whole_pixel_pairs() {
        assert_eq!(tex_pitch(0), 0);
        assert_eq!(tex_pitch(1), 2);
        assert_eq!(tex_pitch(2), 2);
        assert_eq!(tex_pitch(3), 4);
        assert_eq!(tex_pitch(255), 256);
        assert_eq!(tex_pitch(256), 256);
        assert_eq!(tex_pitch(4095), 4096);
        assert_eq!(tex_pitch(4096), 4096);
    }

    #[test]
    fn mip_level_capacity_covers_the_largest_texture() {
        /* 4096 = 2^12 -> levels 4096, 2048, ..., 1 */
        assert_eq!(CAP_MIP_LEVELS, 12 + 1);
    }

    /* ---- defaults ------------------------------------------------------------ */

    #[test]
    fn new_context_has_the_gl_initial_state() {
        let gl = TestGl::new(48, 20);
        let c = gl.ctx();
        assert_eq!(c.viewport, [0, 0, 48, 20]);
        assert_eq!(c.scissor, [0, 0, 48, 20]);
        assert_eq!(c.depth_range, [0.0, 1.0]);
        assert_eq!(c.clear_color, [0.0; 4]);
        assert_eq!(c.clear_depth, 1.0);
        assert_eq!(c.clear_stencil, 0);
        assert!(!c.depth_test && !c.blend && !c.cull_face && !c.scissor_test && !c.stencil_test);
        assert!(!c.polygon_offset_fill && !c.sample_alpha_to_coverage && !c.sample_coverage);
        assert!(c.dither, "the one capability that starts enabled");
        assert_eq!(c.depth_func, GL_LESS);
        assert!(c.depth_mask);
        assert_eq!((c.blend_src, c.blend_dst, c.blend_eq), (GL_ONE, GL_ZERO, GL_FUNC_ADD));
        assert_eq!((c.blend_src_a, c.blend_dst_a, c.blend_eq_a), (GL_ONE, GL_ZERO, GL_FUNC_ADD));
        assert_eq!(c.blend_color, [0.0; 4]);
        assert_eq!((c.cull_mode, c.front_face), (GL_BACK, GL_CCW));
        assert_eq!(c.color_writemask, [1.0; 4]);
        assert_eq!(c.line_width, 1.0);
        assert_eq!((c.polygon_offset_factor, c.polygon_offset_units), (0.0, 0.0));
        assert_eq!((c.sample_coverage_value, c.sample_coverage_invert), (1.0, false));
        assert_eq!((c.unpack_alignment, c.pack_alignment, c.unpack_row_length), (4, 4, 0));
        assert_eq!(c.generate_mipmap_hint, GL_DONT_CARE);
        assert_eq!(c.error, GL_NO_ERROR);
    }

    #[test]
    fn new_context_stencil_state_is_the_same_for_both_faces() {
        let gl = TestGl::new(8, 8);
        let c = gl.ctx();
        assert_eq!((c.stencil_func, c.stencil_ref, c.stencil_mask, c.stencil_writemask), (GL_ALWAYS, 0, !0, !0));
        assert_eq!((c.stencil_sfail, c.stencil_dpfail, c.stencil_dppass), (GL_KEEP, GL_KEEP, GL_KEEP));
        assert_eq!(
            (c.stencil_back_func, c.stencil_back_ref, c.stencil_back_mask, c.stencil_back_writemask),
            (GL_ALWAYS, 0, !0, !0)
        );
        assert_eq!((c.stencil_back_sfail, c.stencil_back_dpfail, c.stencil_back_dppass), (GL_KEEP, GL_KEEP, GL_KEEP));
    }

    #[test]
    fn new_context_has_no_objects_and_no_bindings() {
        let gl = TestGl::new(8, 8);
        let c = gl.ctx();
        assert!(c.objs().buffers.iter().all(|o| !o.used));
        assert!(c.objs().textures.iter().all(|o| !o.used));
        assert!(c.objs().shaders.iter().all(|o| !o.used));
        assert!(c.objs().programs.iter().all(|o| !o.used));
        assert!(c.framebuffers.iter().all(|o| !o.used));
        assert!(c.objs().renderbuffers.iter().all(|o| !o.used));
        assert!(c.vaos.iter().all(|o| !o.used));
        assert_eq!((c.bound_array_buffer, c.bound_element_buffer), (0, 0));
        assert_eq!((c.bound_framebuffer, c.bound_renderbuffer, c.bound_vertex_array), (0, 0, 0));
        assert_eq!(c.bound_texture_2d, [0; CAP_COMBINED_TEXTURE_UNITS]);
        assert_eq!((c.active_texture, c.cur_program), (0, 0));
        assert!(c.attribs.iter().all(|a| !a.enabled && a.buffer == 0 && a.divisor == 0));
        assert!(c.current_attrib.iter().all(|a| *a == [0.0, 0.0, 0.0, 1.0]));
    }

    #[test]
    fn create_owns_an_rgba8888_default_surface_and_becomes_current() {
        let gl = TestGl::new(10, 6);
        let c = gl.ctx();
        assert_eq!((c.target_w, c.target_h), (10, 6));
        assert_eq!((c.default_format, c.default_samples), (GLES_SURF_RGBA8888, 1));
        assert!(c.default_surface_valid && c.default_surface_owned);
        assert_eq!(c.default_surface.size, 10 * 6 * 4);
        assert_eq!(gl.gpu_bytes(c.default_surface.gpu, 4), [0u8; 4], "the surface is real GPU memory, cleared");
        // SAFETY: reading the current-context pointer; tests are serialised.
        unsafe {
            assert_eq!(cur(), c as *mut GlContext);
            assert_eq!(gl_default_surface(), &mut c.default_surface as *mut OsalGpuMem);
        }
        assert!(current().is_some());
    }

    #[test]
    fn create_survives_a_failed_surface_allocation() {
        let gl = TestGl::new(8, 8);
        gl.fail_alloc_in(1);
        let x = Extra::owned(&gl, 4, 4);
        let c = x.get();
        assert!(!c.default_surface_valid && !c.default_surface_owned);
        assert!(c.default_surface().is_none());
        // SAFETY: plain query of the current context.
        unsafe { assert!(gl_default_surface().is_null()) };
        assert_eq!(c.viewport, [0, 0, 4, 4]);
    }

    /* ---- the pool ------------------------------------------------------------ */

    #[test]
    fn pool_hands_out_max_contexts_then_null_and_reuses_freed_slots() {
        let gl = TestGl::new(8, 8);
        let first = gl.ctx().index;
        {
            /* TestGl holds one slot; these fill the pool, whatever
             * YAKOGL_MAX_CONTEXTS made it */
            let x: Vec<Extra> = (1..MAX_CONTEXTS).map(|_| Extra::owned(&gl, 4, 4)).collect();
            let x = x.last().expect("MAX_CONTEXTS >= 2");
            let second = x.get().index;
            assert_ne!(second, first);
            assert!((second as usize) < MAX_CONTEXTS);
            assert_eq!(cur(), x.ctx, "a new context becomes current");

            let live = gl.live_allocs();
            // SAFETY: plain C-ABI calls.
            unsafe {
                assert!(gl_context_create(4, 4).is_null(), "pool exhausted");
                let mut s = caller_surface(64);
                assert!(gl_context_create_for_surface(&mut s, 4, 4, GLES_SURF_RGBA8888, 1, GROUP_NONE).is_null());
                osal::gpu_free(&mut s);
                assert_eq!(cur(), x.ctx, "a failed create leaves the current context alone");
            }
            assert_eq!(gl.live_allocs(), live, "a failed create allocates nothing");
        }
        /* the slot is free again, and the state is reset rather than inherited */
        let y = Extra::owned(&gl, 5, 7);
        assert_ne!(y.get().index, first);
        assert_eq!(y.get().viewport, [0, 0, 5, 7]);
    }

    #[test]
    fn a_reused_slot_does_not_inherit_the_previous_contexts_state() {
        let gl = TestGl::new(8, 8);
        {
            let x = Extra::owned(&gl, 4, 4);
            let c = x.get();
            c.blend = true;
            c.error = GL_INVALID_ENUM;
            c.viewport = [1, 2, 3, 4];
            c.current_attrib[2] = [9.0; 4];
        }
        let y = Extra::owned(&gl, 4, 4);
        let c = y.get();
        assert!(!c.blend);
        assert_eq!(c.error, GL_NO_ERROR);
        assert_eq!(c.viewport, [0, 0, 4, 4]);
        assert_eq!(c.current_attrib[2], [0.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn contexts_keep_separate_state_and_errors() {
        let gl = TestGl::new(8, 8);
        let x = Extra::owned(&gl, 4, 4);
        // SAFETY: plain GL calls on the extra (current) context.
        unsafe {
            crate::gl::state::glEnable(GL_BLEND);
            crate::gl::state::glDepthFunc(0);
        }
        assert!(x.get().blend);
        assert_eq!(x.get().error, GL_INVALID_ENUM);
        assert!(!gl.ctx().blend);
        assert_eq!(gl.ctx().error, GL_NO_ERROR);
        x.get().error = GL_NO_ERROR;
    }

    /* ---- caller-owned default surfaces --------------------------------------- */

    #[test]
    fn create_for_surface_borrows_the_callers_memory() {
        let gl = TestGl::new(8, 8);
        let mut s = caller_surface(32 * 16 * 2);
        gl.mem_events();
        let live = gl.live_allocs();
        {
            let x = Extra::for_surface(&gl, &mut s, 32, 16, GLES_SURF_RGB565, 4);
            let c = x.get();
            assert!(c.default_surface_valid && !c.default_surface_owned);
            assert_eq!((c.default_surface.gpu, c.default_surface.size), (s.gpu, s.size));
            assert_eq!((c.target_w, c.target_h), (32, 16));
            assert_eq!((c.default_format, c.default_samples), (GLES_SURF_RGB565, 4));
            assert_eq!(c.viewport, [0, 0, 32, 16]);
            assert_eq!(c.scissor, [0, 0, 32, 16]);
            // SAFETY: reading the current-context pointer; tests are serialised.
            assert_eq!(cur(), x.ctx);
            assert_eq!(gl.live_allocs(), live, "no surface of its own");
        }
        assert!(gl.mem_events().is_empty(), "destroy does not free the caller's surface");
        assert_eq!(gl.gpu_bytes(s.gpu, 1), [crate::testutil::FILL], "still allocated");
        osal::gpu_free(&mut s);
    }

    #[test]
    fn create_for_surface_treats_zero_samples_as_single_sampled() {
        let gl = TestGl::new(8, 8);
        let mut s = caller_surface(64);
        {
            let x = Extra::for_surface(&gl, &mut s, 4, 4, GLES_SURF_XRGB8888, 0);
            assert_eq!(x.get().default_samples, 1);
            assert_eq!(x.get().default_format, GLES_SURF_XRGB8888);
        }
        osal::gpu_free(&mut s);
    }

    #[test]
    fn create_for_surface_rejects_null_and_unmapped_surfaces_without_taking_a_slot() {
        let gl = TestGl::new(8, 8);
        let before = gl.ctx() as *mut GlContext;
        let mut unmapped = OsalGpuMem::ZERO;
        // SAFETY: plain C-ABI calls; null is tolerated.
        unsafe {
            assert!(gl_context_create_for_surface(core::ptr::null_mut(), 4, 4, 0, 1, GROUP_NONE).is_null());
            assert!(gl_context_create_for_surface(&mut unmapped, 4, 4, 0, 1, GROUP_NONE).is_null());
            assert_eq!(cur(), before);
        }
        let x = Extra::owned(&gl, 4, 4);
        assert!(!x.ctx.is_null(), "the pool still has its free slot");
    }

    #[test]
    fn bind_surface_releases_the_owned_surface_and_borrows_the_new_one() {
        let gl = TestGl::new(8, 8);
        let owned = gl.ctx().default_surface.gpu;
        let mut s = caller_surface(20 * 10 * 4);
        gl.ctx().viewport = [1, 2, 3, 4];
        gl.ctx().blend = true;
        gl.mem_events();
        // SAFETY: `s` outlives its use as the default surface (rebound below).
        unsafe { gl_context_bind_surface(&mut s, 20, 10, GLES_SURF_XRGB8888, 0) };
        assert_eq!(gl.mem_events(), [MemEvent::Free { gpu: owned }]);
        let c = gl.ctx();
        assert!(c.default_surface_valid && !c.default_surface_owned);
        assert_eq!(c.default_surface.gpu, s.gpu);
        assert_eq!((c.target_w, c.target_h), (20, 10));
        assert_eq!((c.default_format, c.default_samples), (GLES_SURF_XRGB8888, 1));
        assert_eq!(c.viewport, [1, 2, 3, 4], "GL state is not reset");
        assert!(c.blend);

        /* swapping to another caller surface frees neither */
        let mut s2 = caller_surface(64);
        gl.mem_events();
        // SAFETY: as above.
        unsafe { gl_context_bind_surface(&mut s2, 4, 4, GLES_SURF_RGBA8888, 4) };
        assert!(gl.mem_events().is_empty());
        assert_eq!(gl.ctx().default_surface.gpu, s2.gpu);
        assert_eq!(gl.ctx().default_samples, 4);
        gl.ctx().blend = false;
        drop(gl); /* destroy must not free s2 either */
        /* (the mock is gone with the guard; the Vec-backed memory went with it) */
        let _ = (s, s2);
    }

    #[test]
    fn destroy_does_not_free_a_bound_caller_surface() {
        let gl = TestGl::new(8, 8);
        let mut s = caller_surface(64);
        {
            let x = Extra::owned(&gl, 4, 4);
            // SAFETY: `s` outlives the extra context.
            unsafe { gl_context_bind_surface(&mut s, 4, 4, GLES_SURF_RGBA8888, 1) };
            assert!(!x.get().default_surface_owned);
            gl.mem_events();
        }
        assert!(gl.mem_events().is_empty());
        assert_eq!(gl.gpu_bytes(s.gpu, 64).len(), 64, "the caller's surface is still allocated");
        osal::gpu_free(&mut s);
    }

    #[test]
    fn a_surface_too_small_to_render_into_is_refused_and_changes_nothing() {
        /* The silent-failure shape this guards: the bind cannot rebind, so it
         * leaves `default_surface`, `target_w` and `target_h` on the previous
         * surface. A caller that does not read the result goes on believing
         * the new surface is bound, draws into the old one and reads back the
         * new one -- wrong pixels, no error. The result is the point of the
         * call as much as the rebinding is. */
        let gl = TestGl::new(8, 8);
        let before = (gl.ctx().default_surface.gpu, gl.ctx().target_w, gl.ctx().target_h);
        /* 20x10 XRGB8888 needs 20 * 10 * 4; one byte short is short. */
        let mut small = caller_surface(20 * 10 * 4 - 1);
        gl.mem_events();
        // SAFETY: `small` outlives the call.
        let rc = unsafe { gl_context_bind_surface(&mut small, 20, 10, GLES_SURF_XRGB8888, 1) };
        assert_eq!(rc, -EINVAL, "a refusal the caller can see");
        let c = gl.ctx();
        assert_eq!((c.default_surface.gpu, c.target_w, c.target_h), before, "still the old surface");
        assert!(c.default_surface_owned, "and still owned: nothing was released either");
        assert!(gl.mem_events().is_empty(), "a refused bind frees nothing");
        osal::gpu_free(&mut small);
    }

    #[test]
    fn a_surface_of_unknown_size_is_taken_on_trust_and_a_bad_geometry_is_not() {
        let _gl = TestGl::new(8, 8);
        let mut s = caller_surface(64);
        // SAFETY: `s` outlives both calls.
        unsafe {
            /* size 0 is "the caller knows": mapped window pages carry no size */
            let mut unsized_mem = s;
            unsized_mem.size = 0;
            assert_eq!(gl_context_bind_surface(&mut unsized_mem, 20, 10, GLES_SURF_XRGB8888, 1), 0);
            /* a geometry no surface can have is refused whatever the size */
            assert_eq!(gl_context_bind_surface(&mut s, 0, 10, GLES_SURF_XRGB8888, 1), -EINVAL);
            assert_eq!(gl_context_bind_surface(&mut s, 20, 0, GLES_SURF_XRGB8888, 1), -EINVAL);
            assert_eq!(gl_context_bind_surface(&mut s, u32::MAX, 2, GLES_SURF_XRGB8888, 1), -EINVAL);
        }
        osal::gpu_free(&mut s);
    }

    #[test]
    fn bind_surface_ignores_null_and_a_missing_context() {
        let gl = TestGl::new(8, 8);
        let owned = gl.ctx().default_surface.gpu;
        let ctx = gl.ctx() as *mut GlContext;
        let mut s = caller_surface(64);
        // SAFETY: plain C-ABI calls; the context is restored before the guard drops.
        unsafe {
            gl_context_bind_surface(core::ptr::null_mut(), 4, 4, 0, 1);
            gl_context_set_current(core::ptr::null_mut());
            gl_context_bind_surface(&mut s, 4, 4, 0, 1);
            assert!(gl_default_surface().is_null(), "no context, no surface");
            gl_context_set_current(ctx);
        }
        let c = gl.ctx();
        assert!(c.default_surface_owned);
        assert_eq!((c.default_surface.gpu, c.target_w), (owned, 8));
        osal::gpu_free(&mut s);
    }

    /* ---- destroy ------------------------------------------------------------- */

    #[test]
    fn destroy_releases_all_object_storage() {
        let gl = TestGl::new(8, 8);
        let baseline = gl.live_allocs();
        {
            let _x = Extra::owned(&gl, 16, 16);
            let prog = gl.program("passthrough.vert", "uniform_color.frag");
            let verts = [0.0f32; 12];
            let texels = [0x7Fu8; 4 * 4 * 4];
            // SAFETY: plain GL calls; the data pointers are valid for the sizes named.
            unsafe {
                use crate::gl::{buffer::*, draw::*, framebuffer::*, program::*, texture::*, vertex::*};
                let mut bufs = [0; 3];
                glGenBuffers(3, bufs.as_mut_ptr());
                glBindBuffer(GL_ARRAY_BUFFER, bufs[0]);
                glBufferData(GL_ARRAY_BUFFER, 48, verts.as_ptr() as *const c_void, GL_STATIC_DRAW);
                glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, bufs[1]);
                glBufferData(GL_ELEMENT_ARRAY_BUFFER, 6, [0u16, 1, 2].as_ptr() as *const c_void, GL_STATIC_DRAW);

                let mut tex = 0;
                glGenTextures(1, &mut tex);
                glBindTexture(GL_TEXTURE_2D, tex);
                let p = texels.as_ptr() as *const c_void;
                glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA as GLint, 4, 4, 0, GL_RGBA, GL_UNSIGNED_BYTE, p);

                let mut rb = 0;
                glGenRenderbuffers(1, &mut rb);
                glBindRenderbuffer(GL_RENDERBUFFER, rb);
                glRenderbufferStorage(GL_RENDERBUFFER, GL_RGBA4, 8, 8);

                /* a draw makes the core upload the program's USC and uniforms */
                glUseProgram(prog);
                /* both VS inputs come from the VBO: draw.rs stages client arrays and
                 * constant attributes in process-wide scratch, which is not context storage */
                glBindBuffer(GL_ARRAY_BUFFER, bufs[0]);
                for i in 0..2 {
                    glVertexAttribPointer(i, 4, GL_FLOAT, 0, 0, core::ptr::null());
                    glEnableVertexAttribArray(i);
                }
                glDrawArrays(GL_TRIANGLES, 0, 3);
                assert_eq!(crate::gl::state::glGetError(), GL_NO_ERROR);
            }
            assert_eq!(gl.draws().len(), 1);
            /* surface + 2 buffers + texture + renderbuffer at the very least */
            assert!(gl.live_allocs() >= baseline + 5, "{} live", gl.live_allocs());
        }
        assert_eq!(gl.live_allocs(), baseline);
    }

    #[test]
    fn destroy_of_the_current_context_leaves_none_current() {
        let gl = TestGl::new(8, 8);
        let x = Extra::owned(&gl, 4, 4);
        // SAFETY: the Extra guard tolerates the double destroy of a freed slot only
        // because its pointer is nulled here first.
        unsafe {
            gl_context_destroy(x.ctx);
            assert!(cur().is_null());
        }
        assert!(current().is_none());
        let mut x = x;
        x.ctx = core::ptr::null_mut();
    }

    #[test]
    fn destroy_of_another_context_keeps_the_current_one() {
        let gl = TestGl::new(8, 8);
        let first = gl.ctx() as *mut GlContext;
        let mut x = Extra::owned(&gl, 4, 4);
        // SAFETY: plain C-ABI calls; x.ctx is nulled after its destroy.
        unsafe {
            gl_context_set_current(first);
            gl_context_destroy(x.ctx);
            assert_eq!(cur(), first);
            gl_context_destroy(core::ptr::null_mut()); /* no-op */
            assert_eq!(cur(), first);
        }
        x.ctx = core::ptr::null_mut();
    }

    /* ---- errors -------------------------------------------------------------- */

    #[test]
    fn set_error_keeps_the_first_error() {
        let gl = TestGl::new(8, 8);
        let c = gl.ctx();
        c.set_error(GL_INVALID_VALUE, "glFirst");
        c.set_error(GL_INVALID_ENUM, "glSecond");
        c.set_error(GL_OUT_OF_MEMORY, "glThird");
        assert_eq!(c.error, GL_INVALID_VALUE);
        assert_eq!(gl.error(), GL_INVALID_VALUE);
        assert_eq!(gl.error(), GL_NO_ERROR, "reading clears it");
        gl.ctx().set_error(GL_INVALID_ENUM, "glFourth");
        assert_eq!(gl.error(), GL_INVALID_ENUM, "a new error is recorded after the read");
    }

    #[test]
    fn only_the_first_16_errors_are_logged_but_all_are_recorded() {
        let gl = TestGl::new(8, 8);
        reset_error_log_cap();
        gl.logs();
        for i in 0..20 {
            gl.ctx().set_error(GL_INVALID_VALUE, "glBudget");
            assert_eq!(gl.error(), GL_INVALID_VALUE, "error {i} is still recorded");
        }
        let lines = gl.logs();
        assert_eq!(lines.iter().filter(|l| l.contains("glBudget")).count(), 16);
        assert!(lines[0].contains("GL error 0x501 in glBudget"), "{}", lines[0]);
        reset_error_log_cap();
    }

    #[test]
    fn the_error_log_budget_is_settable() {
        /* Sixteen is right for a board at 115200 baud; a conformance run
         * wants every error, and raises this through the OSAL (linux.c reads
         * YAKOGL_ERROR_LOG). */
        let gl = TestGl::new(8, 8);
        // SAFETY: single-threaded; tests are serialised by TestGl.
        let restore = unsafe { core::ptr::read(&raw const g_gl_error_log_limit) };
        reset_error_log_cap();
        gl.logs();
        // SAFETY: as above.
        unsafe { core::ptr::write(&raw mut g_gl_error_log_limit, 3) };
        for _ in 0..8 {
            gl.ctx().set_error(GL_INVALID_VALUE, "glBudget");
        }
        assert_eq!(gl.logs().iter().filter(|l| l.contains("glBudget")).count(), 3);
        // SAFETY: as above.
        unsafe { core::ptr::write(&raw mut g_gl_error_log_limit, restore) };
        reset_error_log_cap();
    }

    /// Issue #814: the first errors are kept by name, with the client's tag
    /// (glmark2's entry), for a record to name them; later ones only count.
    #[test]
    fn the_first_errors_are_kept_with_their_entry_point_and_tag() {
        let gl = TestGl::new(8, 8);
        let raised = gl_errors_raised();
        yakogl_gl_error_tag(22);
        gl.ctx().set_error(GL_OUT_OF_MEMORY, "glTexImage2D");
        yakogl_gl_error_tag(24);
        for _ in 0..FIRST_ERRORS + 3 {
            set_error_no_ctx(GL_INVALID_FRAMEBUFFER_OPERATION, "glDrawElements");
        }
        assert_eq!(yakogl_gl_errors_kept(), FIRST_ERRORS as u32, "only the first are kept");
        assert_eq!(gl_errors_raised() - raised, FIRST_ERRORS as u32 + 4, "every one is counted");
        let (mut code, mut tag) = (0u32, 0u32);
        let mut name = [0 as c_char; 48];
        // SAFETY: valid pointers; `name` holds 48 bytes.
        let rc = unsafe { yakogl_gl_error_first(0, &mut code, &mut tag, name.as_mut_ptr(), name.len()) };
        assert_eq!((rc, code, tag), (0, GL_OUT_OF_MEMORY, 22));
        // SAFETY: yakogl_gl_error_first terminated it.
        assert_eq!(unsafe { CStr::from_ptr(name.as_ptr()) }, c"glTexImage2D");
        // a short buffer gets the name cut and still terminated
        let mut short = [0 as c_char; 6];
        // SAFETY: as above, 6 bytes.
        let rc = unsafe { yakogl_gl_error_first(1, &mut code, &mut tag, short.as_mut_ptr(), short.len()) };
        assert_eq!((rc, code, tag), (0, GL_INVALID_FRAMEBUFFER_OPERATION, 24));
        // SAFETY: as above.
        assert_eq!(unsafe { CStr::from_ptr(short.as_ptr()) }, c"glDra");
        // past the kept ones: -1 and nothing written
        code = 7;
        // SAFETY: valid pointers; null name is allowed.
        let rc = unsafe { yakogl_gl_error_first(FIRST_ERRORS as u32, &mut code, &mut tag, core::ptr::null_mut(), 0) };
        assert_eq!((rc, code), (-1, 7));
        // a name longer than the 40 bytes kept is cut there (a new TestGl
        // starts with none kept and tag 0: reset_for_tests)
        drop(gl);
        let gl = TestGl::new(8, 8);
        gl.ctx().set_error(GL_INVALID_ENUM, "glAnEntryPointWhoseNameIsLongerThanFortyBytes");
        // SAFETY: as above, 48 bytes.
        let rc = unsafe { yakogl_gl_error_first(0, &mut code, &mut tag, name.as_mut_ptr(), name.len()) };
        assert_eq!((rc, code, tag), (0, GL_INVALID_ENUM, 0));
        // SAFETY: as above.
        assert_eq!(unsafe { CStr::from_ptr(name.as_ptr()) }.to_bytes().len(), FIRST_ERROR_NAME);
    }

    #[test]
    fn set_error_no_ctx_records_into_the_current_context() {
        let gl = TestGl::new(8, 8);
        set_error_no_ctx(GL_INVALID_OPERATION, "glTest");
        set_error_no_ctx(GL_INVALID_ENUM, "glTest");
        assert_eq!(gl.error(), GL_INVALID_OPERATION);
    }

    #[test]
    fn set_error_no_ctx_without_a_context_is_harmless() {
        let gl = TestGl::new(8, 8);
        let ctx = gl.ctx() as *mut GlContext;
        // SAFETY: the context is restored before the guard drops.
        unsafe {
            gl_context_set_current(core::ptr::null_mut());
            set_error_no_ctx(GL_INVALID_OPERATION, "glTest");
            gl_set_error_at(GL_INVALID_VALUE, c"glFromC".as_ptr());
            gl_context_set_current(ctx);
        }
        assert_eq!(gl.error(), GL_NO_ERROR, "nothing to record it in");
    }

    #[test]
    fn gl_set_error_at_accepts_any_c_string() {
        let gl = TestGl::new(8, 8);
        // SAFETY: NUL-terminated strings or null.
        unsafe {
            gl_set_error_at(GL_INVALID_VALUE, c"egl_glue".as_ptr());
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            gl_set_error_at(GL_INVALID_ENUM, core::ptr::null());
            assert_eq!(gl.error(), GL_INVALID_ENUM);
            gl_set_error_at(GL_INVALID_OPERATION, c"".as_ptr());
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
            gl_set_error_at(GL_OUT_OF_MEMORY, c"\xff\xfe".as_ptr());
            assert_eq!(gl.error(), GL_OUT_OF_MEMORY);
        }
    }

    /* ---- lookups ------------------------------------------------------------- */

    #[test]
    fn lookups_reject_zero_out_of_range_and_unused_handles() {
        let gl = TestGl::new(8, 8);
        let c = gl.ctx();
        macro_rules! check {
            ($get:ident, $get_ref:ident, $owner:expr, $table:ident, $max:expr) => {
                assert!(c.$get(0).is_none());
                assert!(c.$get(1).is_none(), "unused slot");
                assert!(c.$get($max as GLuint).is_none());
                assert!(c.$get(GLuint::MAX).is_none());
                $owner.$table[0].used = true; /* even a "used" slot 0 is never a valid handle */
                assert!(c.$get(0).is_none() && c.$get_ref(0).is_none());
                $owner.$table[0].used = false;
                $owner.$table[$max - 1].used = true;
                let want = &$owner.$table[$max - 1] as *const _;
                assert_eq!(c.$get_ref($max as GLuint - 1).map(|o| o as *const _), Some(want));
                assert_eq!(c.$get($max as GLuint - 1).map(|o| o as *const _), Some(want));
                assert!(c.$get_ref($max as GLuint - 2).is_none());
                $owner.$table[$max - 1].used = false;
                assert!(c.$get_ref($max as GLuint - 1).is_none());
            };
        }
        check!(buffer, buffer_ref, c.objs(), buffers, MAX_BUFFERS);
        check!(texture, texture_ref, c.objs(), textures, MAX_TEXTURES);
        check!(framebuffer, framebuffer_ref, c, framebuffers, MAX_FRAMEBUFFERS);
        check!(renderbuffer, renderbuffer_ref, c.objs(), renderbuffers, MAX_RENDERBUFFERS);

        /* Shader and program names share one space and carry their kind in the
         * low bit, so the same checks run over encoded names -- and a name of
         * the other kind resolves to nothing at all. */
        macro_rules! check_obj {
            ($get:ident, $get_ref:ident, $owner:expr, $table:ident, $max:expr, $kind:expr) => {
                assert!(c.$get(obj_name(0, $kind)).is_none());
                assert!(c.$get(obj_name(1, $kind)).is_none(), "unused slot");
                assert!(c.$get(obj_name($max, $kind)).is_none());
                assert!(c.$get(GLuint::MAX).is_none());
                $owner.$table[0].used = true; /* even a "used" slot 0 is never a valid handle */
                assert!(c.$get(obj_name(0, $kind)).is_none() && c.$get_ref(obj_name(0, $kind)).is_none());
                $owner.$table[0].used = false;
                $owner.$table[$max - 1].used = true;
                let want = &$owner.$table[$max - 1] as *const _;
                let live = obj_name($max - 1, $kind);
                assert_eq!(c.$get_ref(live).map(|o| o as *const _), Some(want));
                assert_eq!(c.$get(live).map(|o| o as *const _), Some(want));
                assert!(c.$get_ref(live ^ 1).is_none(), "the other kind's name of the same slot");
                assert!(c.$get_ref(obj_name($max - 2, $kind)).is_none());
                $owner.$table[$max - 1].used = false;
                assert!(c.$get_ref(live).is_none());
            };
        }
        check_obj!(shader, shader_ref, c.objs(), shaders, MAX_SHADERS, OBJ_SHADER);
        check_obj!(program, program_ref, c.objs(), programs, MAX_PROGRAMS, OBJ_PROGRAM);
    }

    #[test]
    fn c_lookups_resolve_live_objects_of_the_current_context() {
        let gl = TestGl::new(8, 8);
        let prog = gl.program("passthrough.vert", "uniform_color.frag");
        let mut buf = 0;
        // SAFETY: plain GL / C-ABI calls.
        unsafe {
            crate::gl::buffer::glGenBuffers(1, &mut buf);
            assert_eq!(gl_lookup_buffer(buf), &mut gl.ctx().objs().buffers[buf as usize] as *mut GlBuffer);
            assert!(gl_lookup_buffer(0).is_null());
            assert!(gl_lookup_buffer(buf + 1).is_null());
            assert!(gl_lookup_buffer(MAX_BUFFERS as GLuint).is_null());

            let p = gl_lookup_program(prog);
            assert_eq!(p, &mut gl.ctx().objs().programs[obj_index(prog)] as *mut GlProgram);
            assert!((*p).linked);
            let (vs, fs) = ((*p).vs, (*p).fs);
            assert_eq!((*gl_lookup_shader(vs)).type_, GL_VERTEX_SHADER);
            assert_eq!((*gl_lookup_shader(fs)).type_, GL_FRAGMENT_SHADER);
            assert!((*gl_lookup_shader(fs)).compiled);

            assert!(gl_lookup_texture(1).is_null());
            assert!(gl_lookup_framebuffer(1).is_null());
            assert!(gl_lookup_renderbuffer(1).is_null());
        }
    }

    #[test]
    fn c_lookups_without_a_current_context_are_null() {
        let gl = TestGl::new(8, 8);
        let ctx = gl.ctx() as *mut GlContext;
        let mut buf = 0;
        // SAFETY: the context is restored before the guard drops.
        unsafe {
            crate::gl::buffer::glGenBuffers(1, &mut buf);
            gl_context_set_current(core::ptr::null_mut());
            assert!(gl_lookup_buffer(buf).is_null());
            gl_context_set_current(ctx);
            assert!(!gl_lookup_buffer(buf).is_null());
        }
    }

    #[test]
    fn default_surface_accessor_follows_the_valid_flag() {
        let gl = TestGl::new(8, 8);
        let c = gl.ctx();
        let gpu = c.default_surface.gpu;
        assert_eq!(c.default_surface().map(|m| m.gpu), Some(gpu));
        c.default_surface_valid = false;
        assert!(c.default_surface().is_none());
        c.default_surface_valid = true;
    }

    #[test]
    fn heap_data_is_the_backends_data_heap() {
        let _gl = TestGl::new(8, 8);
        assert_eq!(heap_data(), crate::testutil::HEAP_DATA);
    }
    /* ---- the call tracer ------------------------------------------------------------------------------ */

    /// Tracer on for a scope (restarted, logging the first `first` entry
    /// points); off again afterwards, also when the test panics.
    struct Tracing;

    impl Tracing {
        fn on(first: u32) -> Tracing {
            reset_trace();
            yakogl_trace_log_first.store(first, Ordering::Relaxed);
            Tracing
        }
    }

    impl Drop for Tracing {
        fn drop(&mut self) {
            yakogl_trace_log_first.store(0, Ordering::Relaxed);
            reset_trace();
        }
    }

    fn last_call() -> (std::string::String, u32) {
        let mut seq = 0u32;
        // SAFETY: `seq` is writable; the name is a static C string.
        let name = unsafe { CStr::from_ptr(yakogl_debug_last_call(&mut seq)) };
        (name.to_str().unwrap().into(), seq)
    }

    #[test]
    fn tracer_starts_at_dash_and_counts_entry_points() {
        let gl = TestGl::new(8, 8);
        let _t = Tracing::on(0);
        assert_eq!(last_call(), ("-".into(), 0));
        // SAFETY: plain GL calls.
        unsafe {
            crate::gl::state::glViewport(0, 0, 4, 4);
            crate::gl::state::glGetError();
        }
        assert_eq!(last_call(), ("glGetError".into(), 2));
        assert!(gl.logs().is_empty(), "logging the first 0 entry points logs nothing");
        /* a null counter pointer is allowed */
        // SAFETY: null is documented as accepted.
        let name = unsafe { CStr::from_ptr(yakogl_debug_last_call(core::ptr::null_mut())) };
        assert_eq!(name.to_str().unwrap(), "glGetError");
    }

    #[test]
    fn tracer_collapses_runs_and_prints_a_run_when_the_next_entry_point_arrives() {
        let gl = TestGl::new(8, 8);
        let _t = Tracing::on(4000);
        // SAFETY: plain GL calls.
        unsafe {
            for _ in 0..3 {
                crate::gl::state::glViewport(0, 0, 4, 4);
            }
            crate::gl::state::glScissor(0, 0, 1, 1);
            crate::gl::state::glScissor(0, 0, 1, 1);
            crate::gl::state::glGetError();
        }
        /* the last run (glGetError) is still open: never printed */
        assert_eq!(gl.logs(), ["gl#1 glViewport x3", "gl#4 glScissor x2"]);
        assert_eq!(last_call(), ("glGetError".into(), 6));
    }

    #[test]
    fn tracer_logs_only_the_first_n_entry_points_but_keeps_counting() {
        let gl = TestGl::new(8, 8);
        let _t = Tracing::on(2);
        // SAFETY: plain GL calls.
        unsafe {
            crate::gl::state::glViewport(0, 0, 4, 4);
            crate::gl::state::glScissor(0, 0, 1, 1);
            crate::gl::state::glDepthMask(0);
            crate::gl::state::glCullFace(GL_BACK);
        }
        /* entry point 3 is past the limit: the open glScissor run is never closed */
        assert_eq!(gl.logs(), ["gl#1 glViewport x1"]);
        assert_eq!(last_call(), ("glCullFace".into(), 4));
    }

    #[test]
    fn tracer_counts_the_entry_points_the_c_enters_through_its_public_functions() {
        let _gl = TestGl::new(8, 8);
        let _t = Tracing::on(0);
        let step = |f: &mut dyn FnMut()| {
            let before = last_call().1;
            f();
            last_call().1 - before
        };
        // SAFETY: plain GL calls (the draw fails without a program; the 3D
        // target is rejected before anything is read).
        unsafe {
            assert_eq!(step(&mut || crate::gl::state::glBlendFunc(GL_ONE, GL_ZERO)), 2);
            assert_eq!(last_call().0, "glBlendFuncSeparate");
            assert_eq!(step(&mut || crate::gl::state::glStencilMask(1)), 2);
            assert_eq!(step(&mut || crate::gl::state::glStencilOp(GL_KEEP, GL_KEEP, GL_KEEP)), 2);
            assert_eq!(step(&mut || crate::gl::state::glStencilFunc(GL_ALWAYS, 0, 1)), 2);
            assert_eq!(step(&mut || crate::gl::state::glBlendEquation(GL_FUNC_ADD)), 2);
            assert_eq!(step(&mut || crate::gl::texture::glTexParameterf(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, 0.0)), 2);
            assert_eq!(last_call().0, "glTexParameteri");
            assert_eq!(
                step(&mut || crate::gl::texture::glTexParameteriv(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, core::ptr::null())),
                1
            );
            let mut f = 0.0f32;
            assert_eq!(
                step(&mut || crate::gl::texture::glGetTexParameterfv(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, &mut f)),
                2
            );
            let mut b = 0u8;
            /* GL_BLEND is answered by glIsEnabled inside glGetIntegerv */
            assert_eq!(step(&mut || crate::gl::state::glGetBooleanv(GL_BLEND, &mut b)), 3);
            assert_eq!(last_call().0, "glIsEnabled");
            /* float state needs no glGetIntegerv */
            assert_eq!(step(&mut || crate::gl::state::glGetFloatv(GL_LINE_WIDTH, &mut f)), 1);
            assert_eq!(
                step(&mut || crate::gl::draw::glDrawRangeElements(
                    GL_TRIANGLES,
                    0,
                    1,
                    3,
                    GL_UNSIGNED_SHORT,
                    core::ptr::null()
                )),
                2
            );
            assert_eq!(last_call().0, "glDrawElements");
            assert_eq!(
                step(&mut || crate::gl::draw::glDrawRangeElements(
                    GL_TRIANGLES,
                    1,
                    0,
                    3,
                    GL_UNSIGNED_SHORT,
                    core::ptr::null()
                )),
                1
            );
            assert_eq!(
                step(&mut || crate::gl::framebuffer::glRenderbufferStorageMultisample(
                    GL_RENDERBUFFER,
                    0,
                    GL_RGBA8,
                    1,
                    1
                )),
                /* one: the multisample form validates and lays the storage out
                 * itself since 2026-09-24, without re-entering glRenderbufferStorage */
                1
            );
            assert_eq!(
                step(&mut || crate::gl::framebuffer::glInvalidateSubFramebuffer(
                    GL_FRAMEBUFFER,
                    0,
                    core::ptr::null(),
                    0,
                    0,
                    1,
                    1
                )),
                2
            );
            assert_eq!(
                step(&mut || crate::gl::framebuffer::glInvalidateSubFramebuffer(
                    GL_FRAMEBUFFER,
                    0,
                    core::ptr::null(),
                    0,
                    0,
                    -1,
                    1
                )),
                1
            );
            assert_eq!(
                step(&mut || crate::gl::texture::glTexImage3D(
                    GL_TEXTURE_3D,
                    0,
                    0,
                    1,
                    1,
                    1,
                    0,
                    GL_RGBA,
                    GL_UNSIGNED_BYTE,
                    core::ptr::null()
                )),
                1
            );
            assert_eq!(
                step(&mut || {
                    crate::gl::egl::eglWaitClient();
                }),
                2
            );
            assert_eq!(last_call().0, "eglWaitGL");
        }
    }

    #[test]
    fn trace_log_hook_compares_names_by_text() {
        let gl = TestGl::new(8, 8);
        let _t = Tracing::on(4000);
        let a = std::ffi::CString::new("glFoo").unwrap();
        let b = std::ffi::CString::new("glFoo").unwrap();
        let c = c"glBar";
        // SAFETY: the strings outlive the tracer use in this test (reset on drop).
        unsafe {
            yakogl_trace_log(a.as_ptr(), 1);
            yakogl_trace_log(b.as_ptr(), 2);
            yakogl_trace_log(c.as_ptr(), 3);
        }
        assert_eq!(gl.logs(), ["gl#1 glFoo x2"]);
        drop(_t);
        drop((a, b));
    }

    #[test]
    fn owned_default_surface_of_an_odd_width_is_allocated_at_the_pair_pitch() {
        let gl = TestGl::new(8, 8);
        let e = Extra::owned(&gl, 5, 3);
        let s = e.get().default_surface;
        assert_eq!(s.size, 6 * 3 * 4);
    }

    #[test]
    fn create_with_sizes_that_overflow_gets_no_surface() {
        /* the shipped profile has no overflow checks: tex_pitch(w) * h * 4
         * wrapped to a small (or zero) allocation the PBE then wrote past,
         * and the dev profile aborted */
        let gl = TestGl::new(8, 8);
        {
            let x = Extra::owned(&gl, 0x8000_0000, 0x8000_0000);
            let c = x.get();
            assert!(!c.default_surface_valid, "no surface of that size exists");
            assert_eq!((c.target_w, c.target_h), (0x8000_0000, 0x8000_0000));
        }
        {
            let x = Extra::owned(&gl, u32::MAX, 1);
            assert!(!x.get().default_surface_valid);
        }
        assert_eq!(gl.live_allocs(), 1, "only TestGl's own surface");
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    /// set_default_pitch: a view's pitch on the current context's default
    /// surface, refused (changing nothing) when odd, below tex_pitch(w),
    /// past MAX_SURFACE_SIDE or more than the surface's memory holds.
    #[test]
    fn a_default_surface_pitch_is_checked_against_the_surface() {
        let gl = TestGl::new(10, 6);
        let size = gl.ctx().default_surface.size; /* 10 x 6 x 4 */
        assert_eq!(gl.ctx().default_pitch_px(), 10);
        for bad in [11, 8, MAX_SURFACE_SIDE + 2] {
            assert_eq!(set_default_pitch(bad), -EINVAL, "{bad}");
            assert_eq!(gl.ctx().default_pitch, 0);
        }
        /* 12 pixels x 6 rows x 4 bytes is more than a 10-wide surface holds */
        assert!(size < 12 * 6 * 4);
        assert_eq!(set_default_pitch(12), -EINVAL, "the memory holds only {size} bytes");
        assert_eq!(set_default_pitch(10), 0, "its own pitch");
        assert_eq!(gl.ctx().default_pitch_px(), 10);
        assert_eq!(set_default_pitch(0), 0);
        assert_eq!(gl.ctx().default_pitch, 0);
    }

    #[test]
    fn surface_bytes_is_the_pair_padded_size_or_zero() {
        assert_eq!(surface_bytes(4, 3, 4), 48);
        assert_eq!(surface_bytes(5, 3, 4), 6 * 3 * 4, "odd widths carry the pad pixel");
        assert_eq!(surface_bytes(5, 3, 2), 6 * 3 * 2);
        assert_eq!(surface_bytes(0, 3, 4), 0);
        assert_eq!(surface_bytes(4, 0, 4), 0);
        /* sizes no allocation could hold are 0, not a wrapped small number */
        assert_eq!(surface_bytes(0x8000_0000, 0x8000_0000, 4), 0);
        assert_eq!(surface_bytes(u32::MAX, u32::MAX, 4), 0);
        assert_eq!(surface_bytes(u32::MAX, 1, 4), 0);
        /* the largest one that exists */
        assert_eq!(surface_bytes(16384, 16384, 4), 16384 * 16384 * 4);
        assert_eq!(surface_bytes(16385, 1, 4), 0);
    }

    #[test]
    fn clear_storage_zeroes_a_fresh_allocation_with_and_without_a_cpu_view() {
        let gl = TestGl::new(8, 8);
        let mut m = caller_surface(64);
        assert_eq!(gl.gpu_bytes(m.gpu, 4), [crate::testutil::FILL; 4]);
        assert!(clear_storage(&mut m));
        assert_eq!(gl.gpu_bytes(m.gpu, 64), std::vec![0u8; 64]);
        osal::gpu_free(&mut m);

        gl.set_cpu_mapping(false);
        let mut m = caller_surface(64);
        assert!(m.cpu.is_null());
        assert!(clear_storage(&mut m), "through gpu_write");
        assert_eq!(gl.gpu_bytes(m.gpu, 64), std::vec![0u8; 64]);
        /* a platform that fails the write says so */
        gl.set_gpu_io_fails(true);
        assert!(!clear_storage(&mut m));
        gl.set_gpu_io_fails(false);
        osal::gpu_free(&mut m);
    }

    #[test]
    fn a_surface_smaller_than_the_gpus_rows_is_refused() {
        /* the PBE writes rows of whole pixel pairs: 5 x 3 RGBA8888 needs 6
         * pixels a row, and a caller's buffer that does not hold them would
         * be written past (and read past by glReadPixels) */
        let gl = TestGl::new(8, 8);
        let mut packed = caller_surface(5 * 3 * 4);
        {
            let x = Extra::for_surface(&gl, &mut packed, 5, 3, GLES_SURF_RGBA8888, 1);
            assert!(x.ctx.is_null(), "the context was refused");
        }
        let mut padded = caller_surface(6 * 3 * 4);
        {
            let x = Extra::for_surface(&gl, &mut padded, 5, 3, GLES_SURF_RGBA8888, 1);
            assert!(!x.ctx.is_null());
            /* the same rule when an EGL surface is bound to a live context */
            // SAFETY: both descriptors are live; the context is current.
            unsafe {
                gl_context_bind_surface(&mut packed, 5, 3, GLES_SURF_RGBA8888, 1);
                assert_eq!(x.get().default_surface.gpu, padded.gpu, "the small buffer was refused");
                gl_context_bind_surface(&mut padded, 5, 3, GLES_SURF_XRGB8888, 1);
                assert_eq!(x.get().default_format, GLES_SURF_XRGB8888);
            }
        }
        osal::gpu_free(&mut packed);
        osal::gpu_free(&mut padded);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }
}
