/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * backend.h - the boundary between the GL core and a GPU command backend.
 *
 * gl/draw.rs resolves the current GL context into one `gles_draw` (the whole
 * state a draw needs: target, clears, depth/stencil, blend, textures, vertex
 * streams, shaders) and hands it to the backend; the backend records draws
 * into the current frame and submits the frame when the target changes, on
 * glFlush/glFinish or when its own limits are hit. The backend owns the GPU
 * memory layout (heaps) and the GPU's command encoding; the GL core never sees
 * a register. Everything below is GPU-agnostic; `pvr/` implements it for the
 * PowerVR BXS (`yakogl_backend_pvr`) and `vivante/` for the i.MX8MP's GC7000
 * (`yakogl_backend_vivante`). One backend is linked per build (g_backend).
 */
#ifndef YAKOGL_BACKEND_H
#define YAKOGL_BACKEND_H
#include <stdint.h>
#include <stdbool.h>
#include "gles_osal.h"
#include "gles_shaderc.h"
#ifdef __cplusplus
extern "C" {
#endif

enum gles_prim { GLES_PRIM_TRIANGLES = 0, GLES_PRIM_TRI_STRIP, GLES_PRIM_TRI_FAN,
		GLES_PRIM_LINES, GLES_PRIM_LINE_STRIP, GLES_PRIM_LINE_LOOP, GLES_PRIM_POINTS };
enum gles_cmp  { GLES_CMP_NEVER=0, GLES_CMP_LESS, GLES_CMP_EQUAL, GLES_CMP_LEQUAL,
		GLES_CMP_GREATER, GLES_CMP_NOTEQUAL, GLES_CMP_GEQUAL, GLES_CMP_ALWAYS };
enum gles_cull { GLES_CULL_NONE=0, GLES_CULL_CW, GLES_CULL_CCW };
/* Source formats of a vertex attribute array (glVertexAttribPointer type).
 * A backend advertises the ones its vertex fetch reads natively in
 * yakogl_backend.native_attrib_formats; the GL core converts every other
 * array to float32 into a staging buffer at draw time. */
enum gles_attrib_format { GLES_ATTRIBF_FLOAT32 = 0, GLES_ATTRIBF_BYTE, GLES_ATTRIBF_UBYTE,
			 GLES_ATTRIBF_SHORT, GLES_ATTRIBF_USHORT, GLES_ATTRIBF_FIXED,
			 GLES_ATTRIBF_COUNT };
#define GLES_ATTRIBF_BIT(f) (1u << (unsigned)(f))
/* Stencil operations, in the ISPB encoding order (= Vulkan's). */
enum gles_stencil_op { GLES_SOP_KEEP=0, GLES_SOP_ZERO, GLES_SOP_REPLACE, GLES_SOP_INCR_SAT,
		      GLES_SOP_DECR_SAT, GLES_SOP_INVERT, GLES_SOP_INCR_WRAP, GLES_SOP_DECR_WRAP };

/* Device-VA heaps the GL core allocates in (the backend owns the layout; the
 * OSAL places allocations inside the heap the core names). */
enum gles_heap { GLES_HEAP_DATA = 0, GLES_HEAP_SHADER, GLES_HEAP_PROGRAM };

/* A vertex attribute as the VS expects it (already resolved to a vi register). */
struct gles_attrib {
	uint8_t  vi_reg;         /* first vi register */
	uint8_t  components;     /* 1..4 */
	uint8_t  format;         /* enum gles_attrib_format */
	uint8_t  normalized;     /* integer formats: scale to [0,1] / [-1,1] */
	uint32_t stride;         /* bytes between vertices in the source buffer */
	uint32_t offset;         /* byte offset of this attrib within a vertex */
	uint64_t buffer_gpu;     /* GPU VA of the source vertex buffer */
	/* Draw-time data (an array the GL core converted to float32, or the
	 * constant of a disabled array): the GL core's staging buffer is reused
	 * by the next draw, so the backend copies these bytes into its
	 * per-frame arena and fetches from there (buffer_gpu/offset unused). */
	const void *cpu;
	uint32_t cpu_bytes;
	/* ES 3.0 instancing: one element per instance (divisor 1), fetched by
	 * the instance number instead of the vertex index */
	uint8_t  per_instance;
	/* With per_instance: the array's glVertexAttribDivisor (one element per
	 * `divisor` instances), at most the backend's max_instance_divisor; 0
	 * (a producer older than the field) means 1. Sits in what was tail
	 * padding. */
	uint16_t divisor;
};

/* One texture bound to a fragment-shader sampler binding. */
/* Texel formats the TPU decodes (gles_texture_bind.format). RGBA8 is linear
 * (rows of stride_texels); the ETC formats are 4x4 blocks, laid out by
 * gl/texture.rs in whichever of the two orders the backend asked for with
 * yakogl_backend.compressed_blocks_in_rows. */
enum gles_texfmt { GLES_TEXFMT_RGBA8 = 0, GLES_TEXFMT_ETC2_RGB, GLES_TEXFMT_ETC2_RGBA, GLES_TEXFMT_ETC2_RGB_PT,
		  GLES_TEXFMT_RGB565,     /* linear 16-bit R5G6B5 (a native buffer or surface) */
		  /* OpenGL ES 3.0 float texels, linear rows like RGBA8: 8 bytes
		   * (four halves), 2 bytes (one half, reads (r,0,0,1)), 4 bytes (one
		   * float). HDR light probes and Qt Quick 3D's shadow maps. */
		  GLES_TEXFMT_RGBA16F, GLES_TEXFMT_R16F, GLES_TEXFMT_R32F };

/* Bytes per texel of a linear (non-block) texel format. */
static inline uint32_t gles_texfmt_bytes(uint8_t f)
{
	return f == GLES_TEXFMT_RGBA16F ? 8u : f == GLES_TEXFMT_R16F || f == GLES_TEXFMT_RGB565 ? 2u : 4u;
}

/* Memory formats of a colour surface (window/pbuffer/FBO target). The float
 * ones are FBO textures (RGBA16F probes, R16F/R32F shadow maps): the PBE
 * packs them from a fragment shader variant compiled for that class. */
enum gles_surf_format { GLES_SURF_RGBA8888 = 0, GLES_SURF_XRGB8888, GLES_SURF_RGB565,
			GLES_SURF_RGBA16F, GLES_SURF_R16F, GLES_SURF_R32F };

static inline uint32_t gles_surf_bytes_per_pixel(uint8_t format)
{
	return format == GLES_SURF_RGB565 || format == GLES_SURF_R16F ? 2u :
	       format == GLES_SURF_RGBA16F ? 8u : 4u;
}

/* The render target class (SHC_RT_*) a fragment shader must be compiled for. */
static inline uint32_t gles_surf_rt_class(uint8_t format)
{
	return format == GLES_SURF_RGBA16F ? 1u : format == GLES_SURF_R16F ? 2u : format == GLES_SURF_R32F ? 3u : 0u;
}

/* The texel format the surface is sampled as (fbload). */
static inline uint8_t gles_surf_texfmt(uint8_t format)
{
	return format == GLES_SURF_RGB565 ? GLES_TEXFMT_RGB565 : format == GLES_SURF_RGBA16F ? GLES_TEXFMT_RGBA16F :
	       format == GLES_SURF_R16F ? GLES_TEXFMT_R16F : format == GLES_SURF_R32F ? GLES_TEXFMT_R32F : GLES_TEXFMT_RGBA8;
}

/* Sampler bindings one draw can carry per stage. Qt Quick 3D's material
 * shaders declare a dozen and more (base colour, normal, roughness, occlusion,
 * emissive, the light probe, shadow maps, the G-buffer of a deferred pass), and
 * the descriptor block must hold one slot per sampler the shader declares:
 * the compiler placed the shader's uniform block after them, so a short block
 * would have the shader read image state out of its uniforms. */
#define GLES_MAX_TEX_BINDS    16
#define GLES_MAX_VS_TEX_BINDS 16 /* the capacity; the per-GPU limit is backend.rs GLES_VS_TEX_UNITS */

/* Colour attachments one draw may write (attachment 0 plus GL_CAP_COLOR_ATTACHMENTS - 1); a back
 * end writes its own count of them (src/backend.rs DRAW_TARGETS), at most this. */
#define GLES_MAX_TARGETS      4
/* occlusion query handles are 1 .. GLES_QUERY_HANDLES - 1 (gles_draw.occlusion_query) */
#define GLES_QUERY_HANDLES    64

/* What the shader samples the binding as. A cube map, a 2D array and a 3D
 * texture are one stack of `layers` images of width x height; which of the
 * two ways that stack is read is the backend's business:
 *
 *  - the PowerVR's shader does it. glsl2usc.py rewrites a samplerCube into
 *    2D sampling of the six stacked faces and a sampler3D into a grid of
 *    slices, so the image the backend binds is simply the whole stack as one
 *    2D texture and `type` never comes up.
 *  - the GC7000's texture unit does it, from a descriptor that names the
 *    type, one layer's size and the stride from one layer to the next
 *    (yakogl_backend.native_layered_textures).
 */
enum gles_textype { GLES_TEXTYPE_2D = 0, GLES_TEXTYPE_CUBE, GLES_TEXTYPE_2D_ARRAY,
		   GLES_TEXTYPE_3D };

/* GL's texture wrap modes. The TPU has all three: ADDRMODE REPEAT = 0,
 * FLIP = 1 (GL's mirrored repeat) and CLAMP_TO_EDGE = 2 in SAMPLER_WORD0
 * (docs/gpu-recipes/mesa_texture_layout.md, "Sampler-state words"), and the
 * GC7000's TEX_WRAP uses the same three numbers. The values here are GL's
 * modes, not a hardware's; each backend maps them. */
enum gles_wrap { GLES_WRAP_REPEAT = 0, GLES_WRAP_MIRRORED_REPEAT, GLES_WRAP_CLAMP_TO_EDGE };

struct gles_texture_bind {
	uint8_t  unit;           /* sampler binding = descriptor slot (sh[20 unit..]) */
	uint8_t  format;         /* enum gles_texfmt */
	uint8_t  type;           /* enum gles_textype */
	uint16_t layers;         /* images in the stack: 6 for a cube, 1 for plain 2D */
	uint64_t base_gpu;       /* GPU VA of the texels (level 0) */
	uint32_t width, height, stride_texels;
	uint32_t bytes;          /* size of the level-0 storage (compressed: the block grid) */
	uint8_t  linear_filter;  /* magnification: 0 nearest, 1 linear */
	uint8_t  min_linear;     /* minification: 0 nearest, 1 linear */
	uint8_t  mip_filter;     /* 0 no mipmaps, 1 nearest level, 2 linear between levels */
	uint8_t  mip_levels;     /* levels present in memory (consecutive, tightly packed) */
	uint8_t  wrap_s;         /* enum gles_wrap */
	uint8_t  wrap_t;         /* wrap T, which GL sets independently of S */
	uint8_t  bgra;           /* texels stored B,G,R,A (a scan-out surface being re-read) */
	/* GL's incomplete texture: every sample of it reads (0,0,0,1), whatever
	 * the texels hold. The backend binds its own 1x1 opaque black instead
	 * of this texture (GL ES 2.0 section 3.7.10; the GL core decides). */
	uint8_t  incomplete;
	/* 1: the GL base format has no alpha (RGB, LUMINANCE, RED, RG, ETC RGB,
	 * an XRGB image): sampling reads alpha 1, whatever the stored byte. */
	uint8_t  no_alpha;
	/* The depth compare of a shadow look-up: GL_TEXTURE_COMPARE_FUNC less
	 * GL_NEVER (bit 0 less, 1 equal, 2 greater: 0 NEVER .. 7 ALWAYS) under
	 * GL_COMPARE_REF_TO_TEXTURE; 16 (no compare: the depth itself,
	 * ES 3.0.6 3.8.15.1) under GL_NONE; bit 3 set for a
	 * fixed-point depth (the reference is clamped to [0, 1]); the bound
	 * sampler object's, else the texture's. */
	uint8_t  compare_op;
	/* 1: a depth texture sampled as GL_OES_depth_texture says, (d, d, d, 1)
	 * (the ES 2.0 surface); 0: its R32F storage's own (d, 0, 0, 1), ES 3.0's
	 * "treated as RED". */
	uint8_t  depth_luminance;
	/* The texels' write stamp: a new value from one global counter every
	 * time the GL core writes the texels on the CPU (settle_texture, and a
	 * fresh glTexImage), never 0 for such a texture. 0 is "no stamp" (an
	 * EGLImage-backed texture, whose memory a sibling may write): a backend
	 * that keeps a copy of the texels keyed on (base_gpu, content_gen) must
	 * never cache a bind with 0. The same storage with the same stamp holds
	 * the same texels, except where the backend itself rendered into it. */
	uint32_t content_gen;
	/* The kick that last rendered into this texture (0: never), from the GL
	 * core's render-into stamp. A backend keeping a derived copy of the
	 * texels compares (content_gen, target_kick) with the copy's: a render
	 * does not move content_gen. */
	uint32_t target_kick;
	/* The storage's CPU view (NULL without one), for a backend that derives
	 * a copy of the texels at draw time. */
	const void *cpu;
};

/* The fully-resolved draw the backend renders. */
struct gles_draw {
	/* target framebuffer: linear rows, enum gles_surf_format (FBO textures
	 * are RGBA8888; a window may be XRGB8888 or RGB565 scan-out memory) */
	uint64_t target_gpu; uint32_t target_w, target_h;
	uint8_t  target_format;
	/* samples per pixel of the target (1 or 4): a multisampled target is
	 * rasterised with 4 samples on chip and resolved by the PBE into the
	 * single-sampled memory (the EGL surface's EGL_SAMPLES); FBOs are 1 */
	uint8_t  samples;
	bool     do_clear;
	/* How many alpha bits the colour target's GL format really has: 8 for
	 * RGBA8 and for the default surface, 4 for RGBA4, 1 for RGB5_A1, 0 for
	 * a format with no alpha at all (GL_RGB, GL_RGB565). The storage is
	 * RGBA8 whatever was asked for, so the alpha channel is there in memory
	 * even when the format has none, and GL says such a buffer reads 1
	 * (ES 2.0 4.3.1): 0 means "write 1 into alpha, not the fragment's
	 * alpha". Attachment 0 only; the rest keep 8 bits (ES 2.0 has no MRT). */
	uint8_t  target_alpha_bits;
	float clear_rgba[4];
	/* The clear is an internal full-screen quad instead of the frame's
	 * background object. The ISP background object covers the whole target
	 * and has no scissor and no write masks, so a clear that is scissored,
	 * or that must leave some stencil bits alone, cannot be one. With this
	 * set, the backend draws a quad of its own (its own geometry, vertex
	 * and fragment shader) that writes clear_rgba / clear_depth /
	 * clear_stencil into the pixels this draw's scissor, color_write,
	 * depth_write and stencil_write_mask allow; the do_clear* bits say
	 * which buffers. index_count is 0 and no app shader is attached, and
	 * the frame's own clear state is left alone.
	 *
	 * A per-channel colour mask is carried in clear_color_mask. The PowerVR
	 * does not honour it yet: glColorMask is lowered into the fragment
	 * shader there (docs/gpu-recipes/blending_recipe.md) and the backend's
	 * clear shader has no masked variant, so its quad writes every channel
	 * and the GL core says so once. */
	bool     clear_quad;
	/* Depth and stencil clears pending for this draw (glClear bits); the
	 * values are what the target's depth/stencil start from after the clear
	 * (window-space depth 0..1; clear_depth is 1.0 when no clear is pending). */
	bool     do_clear_depth, do_clear_stencil;
	float    clear_depth;
	/* Depth/stencil are kept in memory across kicks (ISP Z load/store) for
	 * targets that own a depth/stencil buffer: the window surface, and FBOs
	 * with a depth or stencil renderbuffer. ds_id names that buffer (the GL
	 * objects behind it), so two FBOs sharing renderbuffers share it. Without
	 * ds_persist the depth/stencil are on-chip only (they start from the clear
	 * values at every kick). */
	bool     ds_persist; uint32_t ds_id;
	/* The depth and stencil objects by identity: 0 none, 0x80000000 | page
	 * for a window surface's, else the object's serial (bit 30: a
	 * texture), carried by the orphan move. *_gen changes when that
	 * storage is specified again. ds_w/ds_h: its size. */
	uint32_t depth_id, depth_gen, stencil_id, stencil_gen, ds_w, ds_h;
	/* glClearBuffer(GL_COLOR, n): bit n = targets[n] takes the clear; 0 all */
	uint8_t  clear_target_mask;

	/* geometry */
	enum gles_prim prim;
	float    line_width;     /* pixels (lines); points are 1 px */
	uint32_t index_count;
	uint64_t index_buf_gpu;  /* index buffer; 0 => non-indexed sequential */
	/* bytes per index (1, 2 or 4). The GL core stages u8 and u32 arrays into
	 * u16 today, so this is 2 whenever index_buf_gpu is set; a backend whose
	 * vertex fetch takes the other widths reads it rather than assuming. */
	uint8_t  index_size;
	/* u16 indices the GL core widened/copied at draw time (staging reused
	 * by the next draw): the backend snapshots them like `cpu` attributes. */
	const void *index_cpu; uint32_t index_bytes;
	/* optional 2nd/Nth draw for multi-draw batching */
	uint32_t index_count2; uint64_t index_buf2_gpu;

	/* shaders (USC + PDS wiring metadata) */
	const struct shc_shader *vs, *fs;
	uint64_t vs_usc_gpu, fs_usc_gpu;   /* USC uploaded by the caller (USC heap) */

	/* attributes + vertex output */
	struct gles_attrib attribs[16]; uint8_t num_attribs;
	uint32_t instance_count;   /* ES 3.0 instanced draw: instances (0 or 1: one) */
	uint32_t vs_output_size_dwords;

	/* uniforms (push-const block, already packed) for VS and/or FS.
	 * The *_gpu buffers are per-program and are overwritten by the next draw
	 * that uses the same program, so when several draws accumulate into one
	 * kick the backend must snapshot each draw's block into its per-frame
	 * arena (using the *_cpu pointers) or every such draw would read the last
	 * draw's uniforms. *_cpu may be NULL (no snapshot; single-draw kicks). */
	uint64_t vs_uniform_gpu; uint16_t vs_uniform_dwords; const void *vs_uniform_cpu;
	uint64_t fs_uniform_gpu; uint16_t fs_uniform_dwords; const void *fs_uniform_cpu;

	/* Colour attachments beyond the first: a G-buffer pass writes three, and
	 * the fragment shader's outputs po0..poN go to them in order. [0] is
	 * target_gpu/target_w/target_h/target_format; num_targets 0 means one.
	 * (Upstream declares the element type inline; it is named here so that
	 * `yakogl-tools abi-layout` pins its layout too.) */
	struct gles_draw_target { uint64_t gpu; uint32_t w, h; uint8_t format; } targets[GLES_MAX_TARGETS];
	uint8_t num_targets;

	/* textures: the fragment stage's, and the vertex stage's (Qt Quick 3D
	 * reads bone and morph-target matrices from a texture in the vertex
	 * shader, so the vertex USC gets descriptors of its own) */
	struct gles_texture_bind textures[GLES_MAX_TEX_BINDS]; uint8_t num_textures;
	struct gles_texture_bind vs_textures[GLES_MAX_VS_TEX_BINDS]; uint8_t num_vs_textures;

	/* fixed-function render state */
	int32_t  viewport[4];
	float    depth_range[2]; /* glDepthRangef near, far: window z = ndc z * (f-n)/2 + (n+f)/2 */
	/* 1: the target is stored top-down (window surface): flip GL's bottom-up
	 * window y in the viewport transform, the scissor and the face winding.
	 * 0: the target is a texture (rows in t order): no flip. */
	bool     flip_y;
	bool     depth_test; enum gles_cmp depth_func; bool depth_write;
	/* glPolygonOffset (triangles only): window z += factor * slope + units * r */
	bool     polygon_offset; float polygon_offset_factor, polygon_offset_units;
	/* Blending is in the FS on the PowerVR (fs is the variant compiled for the
	 * blend state, fs->uses_fbfetch) and the flag only documents it; every
	 * object is already submitted TRANSLUCENT (in order, background readable).
	 * A backend with fixed-function blending (the Vivante PE) programs the
	 * state below instead. Factors and equations are enum shc_blend_factor /
	 * enum shc_blend_func, the codes the shader variant key uses. */
	bool     blend;
	uint8_t  blend_src_rgb, blend_dst_rgb, blend_src_alpha, blend_dst_alpha;
	uint8_t  blend_eq_rgb, blend_eq_alpha;
	float    blend_color[4];       /* glBlendColor (the CONSTANT_* factors) */
	uint8_t  color_mask;           /* glColorMask: bit 0 R, 1 G, 2 B, 3 A */
	enum gles_cull cull;
	bool     scissor; int32_t scissor_rect[4];
	/* stencil, front face then back face (glStencil*Separate). Triangles
	 * whose winding in GL window space is front_cw (glFrontFace) use the
	 * front state; lines and points always use the front state. */
	bool     stencil; bool front_cw;
	enum gles_cmp stencil_func; uint8_t stencil_ref;
	uint8_t  stencil_cmp_mask, stencil_write_mask;
	enum gles_stencil_op stencil_sfail, stencil_dpfail, stencil_dppass;
	enum gles_cmp stencil_back_func; uint8_t stencil_back_ref;
	uint8_t  stencil_back_cmp_mask, stencil_back_write_mask;
	enum gles_stencil_op stencil_back_sfail, stencil_back_dpfail, stencil_back_dppass;
	/* Stencil the target starts from after a stencil clear (glClearStencil
	 * value when do_clear_stencil, else 0). */
	uint8_t  clear_stencil;
	/* false: the draw writes no colour (glColorMask all off); per-channel
	 * masks are not supported (they would need a fragment-shader variant). */
	bool     color_write;
	/* The colour channels the pending colour clear may write (glColorMask
	 * at the glClear: bit 0 R, 1 G, 2 B, 3 A), meaningful when do_clear.
	 * Separate from color_mask because a clear fused into a draw carries
	 * the mask of its glClear, not the draw's. 0 is "not given" (a producer
	 * older than the field) and means every channel; the GL core never
	 * sends do_clear with an empty mask. Sits in what was tail padding. */
	uint8_t  clear_color_mask;
	/* The stencil bits the pending stencil clear may change (glStencilMask
	 * at the glClear), meaningful when do_clear_stencil. Its own field for
	 * the reason clear_color_mask is: a clear fused into a draw carries its
	 * glClear's mask, and stencil_write_mask is the draw's. 0 is "not
	 * given" (a producer older than the field) and means stencil_write_mask;
	 * the GL core never sends do_clear_stencil with an empty mask. */
	uint8_t  clear_stencil_write_mask;
	/* The instance this draw is, when the GL core draws an instanced draw
	 * instance by instance (an array's divisor past the backend's
	 * max_instance_divisor): gl_InstanceID's value. 0 for every other
	 * draw. A backend that cannot give a program this value refuses such a
	 * draw (instance_id != 0 and the VS reads gl_InstanceID) by name
	 * rather than draw it with 0. Sits in what was tail padding. */
	uint32_t instance_id;
	/* The samples a multisampled draw writes (bit n: sample n), the mask
	 * Mesa's state tracker makes of glSampleCoverage (st_atom_msaa.c):
	 * the first floor(value x samples) samples, inverted with
	 * GL_SAMPLE_COVERAGE_INVERT, only when GL_SAMPLE_COVERAGE is on and
	 * the target has more than one sample; 0xf otherwise. 0 is a real
	 * value (coverage 0.0). */
	uint8_t  sample_mask;
	/* GL_SAMPLE_ALPHA_TO_COVERAGE is on and the target is multisampled. */
	bool     alpha_to_coverage;
	/* Attachment 0's row pitch in pixels when it is not tex_pitch(target_w):
	 * the default surface of a window that is a view into a wider scan-out
	 * buffer. 0: the packed pitch (every FBO and pbuffer). Sits in what was
	 * tail padding. */
	uint32_t target_pitch;
	/* The ES 3.0 occlusion query (GL_ANY_SAMPLES_PASSED[_CONSERVATIVE])
	 * that counts this draw's samples: a backend query handle (1 ..
	 * GLES_QUERY_HANDLES, started by query_begin), 0 for none. The GL core
	 * sends 0 for every clear-only draw; a clear fused into a counted draw
	 * (do_clear, clear_quad) is never counted (ES 3.0 4.2.3: a clear is
	 * not a draw), whatever path the backend clears with. Sits in what was
	 * tail padding. */
	uint32_t occlusion_query;
};

/* One-time backend init (after OSAL init: allocates persistent GPU objects,
 * builds static PDS, etc.). Returns 0 on success. */

/* The backend's entry points, one table per GPU. */
struct yakogl_backend {
	const char *name;
	/* vertex attribute source formats the backend fetches without help
	 * (a mask of GLES_ATTRIBF_BIT); float32 is always included */
	uint32_t native_attrib_formats;
	/* True when this GPU stores an RGBA8888 colour target as B,G,R,A.
	 *
	 * Some pixel engines have no RGBA8888 render format at all -- the
	 * GC7000's 32-bit formats are A8R8G8B8 and X8R8G8B8 -- so what they
	 * write is byte-swapped against what GL calls RGBA8888. Sampling such
	 * a target as a texture is corrected by the descriptor's own format,
	 * so the only path that sees the raw order is the one that hands the
	 * bytes to the application: glReadPixels and what is built on it. */
	bool rt_rgba8888_is_bgra;
	/* Non-zero when this GPU reads a mip chain laid out its own way, and
	 * the byte alignment it wants each level's base at. Zero (the default)
	 * keeps the packed layout: every level as wide as it is, one level
	 * straight after the last.
	 *
	 * A GPU that walks the chain itself needs one base address and one
	 * stride and derives the rest -- that is the PowerVR's texture image
	 * word, and the packed layout is exactly what it wants. The GC7000
	 * instead names all sixteen level addresses in its descriptor, but
	 * still carries a single LINEAR_STRIDE for the whole chain, and it
	 * fetches a level from a 64-byte boundary. So a level of its chain has
	 * to be padded to level 0's row pitch *and* start on a multiple of
	 * this, or the hardware reads one level where another was meant.
	 *
	 * Mesa does both for the same part: it pads every level of a mipmapped
	 * texture to one super-tile width, and etnaviv_resource.c aligns each
	 * level to ETNA_PE_ALIGNMENT with the comment "align levels to 64
	 * bytes to be able to render to them".
	 *
	 * It costs some memory in the tail levels, which is the whole price. */
	uint32_t mip_level_align;
	/* True when this GPU reads a block-compressed texture as plain raster
	 * rows of 4x4 blocks: ceil(w/4) blocks in a row, one row after the
	 * next, with the row pitch given to the hardware. False (the default)
	 * is what the PowerVR TPU walks instead -- the blocks of a
	 * power-of-two block grid in twiddled order, which needs no pitch.
	 *
	 * Mesa's etnaviv gives a compressed resource ETNA_LAYOUT_LINEAR with
	 * both paddings at 1 ("compressed textures don't use tiling, they
	 * have their own tiles"), so the GC7000 wants the raster rows and
	 * no padding at all.
	 */
	bool compressed_blocks_in_rows;
	/* True when this GPU's texture unit samples a cube map, a 2D array and
	 * a 3D texture itself, given the type, one layer's size and the stride
	 * between layers. The GL core then stacks the layers plainly -- no
	 * grid of 3D slices -- and fills in gles_texture_bind.type/layers.
	 *
	 * False (the default) is the PowerVR's arrangement, where the shader
	 * does the work and the backend only ever sees a 2D image. */
	bool native_layered_textures;
	/* True when this GPU's gl_FrontFacing has no sense of its own -- the
	 * fragment reads a raw 0.0/1.0 and the comparison against it is
	 * compiled in, so the winding is a shader variant rather than a piece
	 * of draw state. The GL core then asks the shader backend for the
	 * SHC_VIV_KEY_FRONT_FLIPPED binary whenever the effective winding is
	 * reversed, and only for shaders that read gl_FrontFacing at all.
	 *
	 * False (the default) is the PowerVR, where the driver writes the
	 * front-face op into a shared register at draw time
	 * (shc_shader.fs_front_face_off1) and one binary serves both. */
	bool front_facing_shader_variant;
	/* True when this GPU does not sample ETC1 itself, and the GL core
	 * decodes a GL_ETC1_RGB8_OES image to RGBA8 at glCompressedTexImage2D,
	 * as Mesa's state tracker does for a driver with no ETC1 format
	 * (st_format.c, st->has_etc1). The texture is then an RGB texture like
	 * any other, with mip levels and cube faces.
	 *
	 * False (the default) is the PowerVR and the GC7000, whose texture
	 * units read ETC1 blocks. */
	bool etc1_decoded_at_upload;
	int  (*init)(void);
	void (*shutdown)(void);
	/* Record one draw into the current frame (submits a frame if it must).
	 * 0: recorded. A negative errno means the draw was NOT recorded (a
	 * split draw may have recorded some of its pieces):
	 *   -ENOMEM  GPU memory ran out: the draw does not fit an empty frame,
	 *            or an allocation it needed failed (a depth/stencil buffer
	 *            too: a draw that tests or clears one the backend could not
	 *            allocate, or lost that way, is refused) -> GL_OUT_OF_MEMORY
	 *   -E2BIG   larger than the backend can encode (PowerVR: a non-indexed
	 *            draw past 65535 vertices that it cannot split) ->
	 *            GL_OUT_OF_MEMORY
	 *   -EINVAL  a malformed or unsupported gles_draw (a core bug or an
	 *            unsupported feature)
	 *   -ENOTSUP legal GL state the backend cannot render yet (V3D:
	 *            alpha-to-coverage), named in the backend's log
	 *   other   the platform failed (submit, wait, no CPU mapping)
	 * Never positive. gl/draw.rs turns every negative value into one log
	 * line and GL_OUT_OF_MEMORY, so return a negative value only for a
	 * draw that was not recorded. See src/backend.rs `draw`. */
	int  (*draw)(const struct gles_draw *draw);
	/* submit the current frame; does not wait for it */
	int  (*flush)(void);
	/* wait for every submitted frame */
	int  (*finish)(void);
	/* wait for the frame before the last submitted one (pipelined swap) */
	int  (*finish_previous)(void);
	/* The kick being recorded (the open frame's id, or the next frame's).
	 * After a draw that returned 0 it names the kick holding that draw (its
	 * last part, when the backend split it): the GL core moves the draw's
	 * stamps there (gl::draw::restamp_draw). kick_pending(id): whether kick
	 * id has still to complete (the kick being recorded is, from its first
	 * draw). */
	uint32_t (*kick_id)(void);
	bool (*kick_pending)(uint32_t kick_id);
	/* device-VA base of a heap the core allocates in */
	uint64_t (*heap_base)(enum gles_heap heap);
	/* frames submitted / blocking waits so far (optional, diagnostics) */
	void (*counters)(uint32_t *kicks, uint32_t *waits);
	/* Texel storage at [base_gpu, base_gpu + bytes) is about to be released
	 * (a texture or renderbuffer deleted or re-specified, a context
	 * destroyed): called before the OSAL's free is scheduled, so a backend
	 * drops whatever it keeps for that memory before the address can come
	 * back. Optional: NULL on a backend that keeps nothing. */
	void (*texture_released)(uint64_t base_gpu, uint64_t bytes);
	/* eglSwapBuffers is posting the window surface whose colour buffer is
	 * at surface_gpu: called before the swap's flush. EGL 1.5 3.10.1: "The
	 * contents of ancillary buffers are always undefined after calling
	 * eglSwapBuffers", so a backend may drop the surface's depth/stencil
	 * (not store them, and start the next frame from nothing). Optional:
	 * NULL on a backend that does nothing with it. */
	void (*surface_swapped)(uint64_t surface_gpu);
	/* glTexImage2D / glTexSubImage2D stored the rows at `src` (row pitch
	 * `stride` bytes: GL_UNPACK_ROW_LENGTH texels, or w when it is 0, times
	 * 4, rounded up to GL_UNPACK_ALIGNMENT; in the caller's memory, valid
	 * for this call only) into
	 * [x, x+w) x [y, y+h) of level `level` of the 2D texture `b` describes.
	 * Called after the GL core's own store succeeded, and only when those
	 * bytes ARE the stored texels: GL_RGBA / GL_UNSIGNED_BYTE into storage
	 * with no red/blue swap. Never for a conversion, a NULL upload, a cube
	 * face, a 3D texture or a glCopyTex* copy; those reach the backend only
	 * through content_gen, as before.
	 * `b` is the binding a draw of the texture would carry now: its
	 * base_gpu, content_gen (the stamp AFTER this write; 0 for an EGLImage,
	 * never cached), width, height, stride_texels, bytes, format and bgra
	 * equal a later draw's until the next write. Its sampling fields
	 * (filters, mip_levels, wrap) are as of the upload and may change
	 * without a write, so they are not a key.
	 * A backend that keeps a copy of the texels may build it from `src`
	 * (cacheable memory) instead of from the storage. Optional: NULL on a
	 * backend that keeps nothing. */
	void (*texture_uploaded)(const struct gles_texture_bind *b, uint32_t level, uint32_t x,
				 uint32_t y, uint32_t w, uint32_t h, const void *src, uint32_t stride);
	/* The largest glVertexAttribDivisor an instanced draw's arrays may have
	 * for the GL core to hand it over as one draw (gles_attrib.divisor);
	 * past it the GL core draws instance by instance (gles_draw.instance_id).
	 * 0 or 1: divisor 1 only (the PowerVR: the PDS fetches by the instance
	 * number). The V3D takes 0xffff (the attribute record's 16-bit Instance
	 * Divisor). */
	uint32_t max_instance_divisor;
	/* Wait until kick `kick_id` (a kick already submitted: kick_id() - 1
	 * at a fence's creation) has completed, submitting nothing recorded
	 * after it; 0, or a negative errno when that kick failed. The sync
	 * waits (eglClientWaitSync, glClientWaitSync) use it: EGL 1.5 3.8.1.3
	 * and ES 3.0 5.2.1 flush only with the FLUSH_COMMANDS bit, and the
	 * fence was flushed at its creation. Optional: NULL keeps finish(),
	 * which also submits the open frame (the PowerVR and the GC7000 until
	 * their owners implement it). */
	int (*wait_kick)(uint32_t kick_id);
	/* True when a draw never reads a texture's storage on the GPU: the
	 * backend samples copies it makes on the CPU when the draw is recorded
	 * (the V3D's tiled copies, kept or in the job's arena). A CPU write into
	 * a texture a pending kick sampled then needs no wait: that kick reads
	 * its copy. A pending kick that RENDERS into the texture, and storage
	 * an EGLImage shares, still wait. False (the default): the PowerVR and
	 * the GC7000, whose texture units read the storage. */
	bool samples_texel_copies;
	/* The depth/stencil contents of GL object `id` are gone (its form is
	 * gles_draw.depth_id's: a renderbuffer's serial, a texture's with bit
	 * 30, 0x80000000 | page for a window surface): the object was freed,
	 * its storage specified again, or its surface destroyed. A backend that
	 * keeps a depth/stencil buffer for it frees that slot; it never gives
	 * away a buffer that holds contents otherwise (src/ds_retention.rs).
	 * Optional: NULL on a backend that keeps none. */
	void (*attachment_gone)(uint32_t id);
	/* Write the depth plane kept for GL object `id` (gles_draw.depth_id's
	 * form: a depth texture's serial with bit 30) into w x h floats at
	 * `dst`, `pitch` floats a row, as the texture's R32F texels hold depth
	 * (0..1). The GL core calls it before a draw samples a depth texture,
	 * or the CPU writes one, that a kick rendered into; every such kick has
	 * completed. 0: written; 1: nothing kept for it (never written, or
	 * undefined), the texels are left as they are; a negative errno
	 * otherwise. Optional: NULL answers 1. */
	int (*depth_resolve)(uint32_t id, float *dst, uint32_t w, uint32_t h, uint32_t pitch);
	/* ES 3.0 occlusion queries. query_begin: a query starts counting on
	 * `handle` (1 .. GLES_QUERY_HANDLES - 1); reset its counter. Every kick
	 * that counted on the handle before has completed (the GL core waits).
	 * 0, or a negative errno (no counter memory). NULL: the backend counts
	 * no samples and glBeginQuery refuses the occlusion targets.
	 * query_end (optional): the query ended; every draw it counts was
	 * handed over (gles_draw.occlusion_query); the backend may submit the
	 * open frame (a tile-based GPU whose visibility test sees later
	 * occluders of the same kick must). query_result: the samples (or any
	 * non-zero value when some passed) counted on `handle` since its
	 * query_begin, called once every kick holding a counted draw has
	 * completed; 0, or a negative errno. */
	int (*query_begin)(uint32_t handle);
	void (*query_end)(uint32_t handle);
	int (*query_result)(uint32_t handle, uint64_t *samples);
	/* glGenerateMipmap on the 2D texture `b` describes, whose storage holds
	 * a chain of `levels` levels. A backend that keeps the texture's level 0
	 * as its jobs rendered it (the V3D's render image) may make the chain
	 * there itself, on the GPU, and answer 1: the GL core then filters
	 * nothing, leaves levels 1.. of its storage as they are, and calls
	 * return_mipmap_chain before it reads or writes those levels on the CPU.
	 * 0: the GL core box-filters on the CPU, as without the entry. Called
	 * after every kick rendering into the texture was handed over (the
	 * backend waits for it) and before the texels' stamp would move.
	 * Optional: NULL answers 0. */
	int (*generate_mipmap)(const struct gles_texture_bind *b, uint32_t levels);
	/* The GL core is about to read or write levels 1.. of the texture `b`
	 * describes on the CPU, and the last generate_mipmap on it answered 1:
	 * write the chain the backend made back into the storage (levels 1 ..
	 * levels - 1, each at the GL core's level offset and pitch for this
	 * backend's chain layout) before returning; nothing when the backend
	 * no longer holds it. Optional: NULL on a backend that never answers 1. */
	void (*return_mipmap_chain)(const struct gles_texture_bind *b, uint32_t levels);
};

extern const struct yakogl_backend *g_backend;   /* the backend of this build (src/backend.rs) */

static inline int yakogl_backend_init(void) { return g_backend->init(); }
static inline void yakogl_backend_shutdown(void) { if (g_backend->shutdown) g_backend->shutdown(); }
static inline int yakogl_draw(const struct gles_draw *d) { return g_backend->draw(d); }
static inline int yakogl_flush(void) { return g_backend->flush(); }
static inline int yakogl_finish(void) { return g_backend->finish(); }
static inline int yakogl_finish_previous(void) { return g_backend->finish_previous(); }
static inline uint32_t yakogl_kick_id(void) { return g_backend->kick_id(); }
static inline bool yakogl_kick_pending(uint32_t id) { return g_backend->kick_pending(id); }
static inline uint64_t yakogl_heap_base(enum gles_heap h) { return g_backend->heap_base(h); }

#ifdef __cplusplus
}
#endif
#endif /* YAKOGL_BACKEND_H */
