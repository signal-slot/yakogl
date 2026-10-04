/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * gl_context.h - the OpenGL ES 2.0 context/state machine.
 *
 * One GL context bundles all bound objects and render state. gl* entry points
 * record into it; glDraw* resolves it into a PowerVR command stream (gl/draw.rs).
 * Object handles are small integers indexing fixed tables (embedded-friendly: no
 * per-object malloc storms; sizes are compile-time caps).
 */
#ifndef GL_CONTEXT_H
#define GL_CONTEXT_H

#include <GLES3/gl3.h>
#include <stdint.h>
#include <stdbool.h>
#include "gles_osal.h"
#include "gles_shaderc.h"

/* Contexts that can exist at once (EGL contexts; ~400 KB of static state each). */
#ifndef GL_MAX_CONTEXTS
#define GL_MAX_CONTEXTS       2
#endif
#define GL_MAX_BUFFERS        512   /* Qt Quick: a vertex + index buffer per batch, uniform buffers per material; 64 ran out on the Coffee demo's second screen */
#define GL_MAX_TEXTURES       512   /* a UI keeps a texture per text item plus femtovg gradient look-ups; Qt Quick 3D a few per material */
/* Texture name 0 is not "no texture": every target has a default texture
 * object, and glTexParameter* / glTexImage* address it while nothing else is
 * bound (ES 3.0 3.8). The four of them (2D, cube, 2D array, 3D) are the tail
 * of the texture table, past the names an application can hold. */
#define GL_DEFAULT_TEXTURES   4
#define GL_TEXTURE_SLOTS      (GL_MAX_TEXTURES + GL_DEFAULT_TEXTURES)
#define GL_MAX_SHADERS        512   /* Qt Quick 3D compiles hundreds of material variants */
#define GL_MAX_PROGRAMS       512   /* a pipeline each: cascaded shadow maps ran out of 160 */
#define GL_MAX_FRAMEBUFFERS   256   /* Qt Quick 3D keeps one per render target of every pass (post-processing chains) */
#define GL_MAX_RENDERBUFFERS  64
/* The attribute arrays' size, the GLES2 minimum. NOT GL_MAX_VERTEX_ATTRIBS:
 * that is the Khronos query token (0x8869), and a header that redefined it
 * made gl_test.c's glGetIntegerv(GL_MAX_VERTEX_ATTRIBS) ask for pname 16
 * (GL_INVALID_ENUM, and reset_state disabled no attribute array). */
#define GL_CAP_VERTEX_ATTRIBS 16
#define GL_MAX_UNIFORM_DWORDS 1536   /* 384 vec4 per stage: Qt Quick 3D keeps its full light set when the RHI reports >= 325 vec4 (5200 bytes), else it flips a shader feature and regenerates its material shaders at run time */
#define GL_CAP_COMBINED_TEXTURE_UNITS 32 /* the GL texture units, GL_MAX_COMBINED_TEXTURE_IMAGE_UNITS (ES 3.0 asks for 32); context.rs CAP_COMBINED_TEXTURE_UNITS. GL_MAX_TEXTURE_IMAGE_UNITS is 16 (context.rs CAP_TEXTURE_UNITS) */
#define GL_CAP_SAMPLERS       16   /* sampler bindings per shader stage (shc_shader.uniforms) */

struct gl_buffer {
	bool     used;
	GLenum   target;         /* ARRAY_BUFFER / ELEMENT_ARRAY_BUFFER */
	GLenum   usage;          /* glBufferData usage hint (query only) */
	struct osal_gpu_mem mem; /* GPU-resident storage */
	GLsizeiptr size;
	uint32_t last_use;       /* kick id of the last draw that read it (0: none) */
	/* glMapBufferRange: the mapped range (mapped == NULL when unmapped) */
	void      *mapped;
	GLintptr   map_off;
	GLsizeiptr map_len;
	GLbitfield map_access;
};

#define GL_CAP_MIP_LEVELS 13      /* 4096 -> 1 */

struct gl_texture {
	bool     used;
	GLenum   target;         /* TEXTURE_2D, TEXTURE_CUBE_MAP or TEXTURE_2D_ARRAY */
	uint32_t width, height;  /* level 0, of ONE face/layer */
	/* A cube map (6) or a 2D array (N) keeps its faces/layers stacked
	 * vertically in one linear texture: `layers` rows of `height` at every
	 * level, so the storage is width x (height * layers). The shaders sample
	 * it as a tall 2D texture (tools/glsl2usc.py emulation). */
	uint16_t layers;
	/* A 3D texture (target TEXTURE_3D, `layers` = depth) tiles its slices in
	 * a grid of grid_cols columns instead (slice k at column k % grid_cols,
	 * row k / grid_cols), which keeps 256^3 inside the TPU's 16384-texel
	 * height; one mip level. grid_cols = ceil(sqrt(depth)), 0 otherwise. */
	uint8_t  grid_cols;
	GLenum   format, type;
	GLenum   min_filter, mag_filter, wrap_s, wrap_t;
	/* Linear RGBA8888 texel storage. A power-of-two texture owns room for
	 * its whole mip chain: level i at level_off[i], rows tightly packed at
	 * its own width (the layout the TPU walks with mipmaps_present).
	 *
	 * chain_pitch is non-zero when the backend asked for every level at
	 * level 0's pitch instead (yakogl_backend.mip_level_align); it holds
	 * that pitch in texels. GlTexture::level_pitch() is the one place the
	 * two layouts are told apart. */
	struct osal_gpu_mem mem;
	uint32_t level_off[GL_CAP_MIP_LEVELS];
	uint32_t chain_pitch;
	uint8_t  chain_levels;   /* levels the storage can hold (1 = no chain) */
	uint8_t  levels;         /* levels with texels (1 .. chain_levels) */
	/* The last level the application actually put texels in, whether by
	 * upload, by glGenerateMipmap or by rendering into it, and the
	 * GL_TEXTURE_MAX_LEVEL it asked the sampler to stop at. Sampling past
	 * either reads memory nobody wrote: Qt prefilters a 512 light probe
	 * down to 16x16 (six levels) in a texture declared with ten. */
	uint8_t  levels_written;
	uint8_t  max_level;
	bool     external;       /* storage belongs to an EGLImage: the image frees it once unused */
	bool     orphan;         /* deleted while an unbound framebuffer holds it: image kept, name gone */
	bool     bgra;           /* texels stored B,G,R,X (an XRGB8888 native buffer) */
	uint8_t  texfmt;         /* enum gles_texfmt (0 = RGBA8 texels; the ETC formats are block laid out) */
	uint8_t  bpp;            /* bytes per texel of a linear format (4 RGBA8, 8 RGBA16F, 2 R16F/RGB565) */
	uint8_t  block_bytes;    /* compressed: bytes per 4x4 block */
	bool     immutable;      /* glTexStorage*: TEXTURE_IMMUTABLE_FORMAT (no respecification) */
	/* Cube map: one bit per face whose level 0 has been specified at the
	 * current size (bit 0 = POSITIVE_X .. bit 5 = NEGATIVE_Z). Laying the
	 * storage out again clears it, so a face specified at a different size
	 * leaves the cube map incomplete, which is what GL says it is. */
	uint8_t  faces_set;
	uint32_t block_w, block_h;   /* compressed: the power-of-two block grid held */
	uint32_t last_use;       /* kick id of the last draw that sampled it (0: none) */
	uint16_t level_defined;  /* bit l: an image was specified for level l since the layout */
	uint16_t level_bad;      /* bit l: that image does not fit the chain (size, format, or a zero side of level 0) */
	uint32_t content_gen;    /* the texels' write stamp (gles_texture_bind.content_gen); 0: none */
	uint8_t  level_faces[GL_CAP_MIP_LEVELS]; /* cube map: bit f of [l], face f has a fitting image at level l >= 1 */
	/* ES 3.0 parameters stored and read back, not applied by the draw */
	int32_t  base_level;
	int32_t  max_level_set;      /* GL_TEXTURE_MAX_LEVEL as given; max_level is the sampler's clamp */
	float    min_lod, max_lod;
	uint32_t wrap_r, compare_mode, compare_func;
	uint16_t swizzle[4];
	uint8_t  immutable_levels;   /* glTexStorage*'s level count */
	uint32_t serial;             /* the object's identity for a backend, carried by the orphan move */
	uint32_t storage_gen;        /* changes whenever the storage is laid out again */
	bool     pe_order;           /* texels in the pixel engine's B,G,R,A order (rendered into; alpha real) */
	bool     cpu_wrote;          /* the CPU wrote texels since the storage was allocated and cleared */
};

struct gl_shader {
	bool   used;
	GLenum type;             /* VERTEX_SHADER / FRAGMENT_SHADER */
	bool   compiled;
	/* not in the offline table: reported as compiled so the application
	 * goes on to compile its other stage (both get logged in one run), and
	 * refused at link time */
	bool   missing;
	struct shc_shader compiled_shader;
	char  *source;           /* owned copy for glShaderSource */
};

struct gl_program {
	bool   used;
	bool   linked;
	GLuint vs, fs;           /* attached shader handles */
	/* resolved wiring after link (coeff/attrib/uniform layout) */
	struct shc_shader rvs, rfs;
	/* the two stages' push-constant blocks, laid out as the compiler packed
	 * them (shc_uniform.dword_off); gl/draw.rs uploads them per draw */
	float  *vs_uniforms, *fs_uniforms;   /* heap, sized at link (uniform_cap dwords) */
	uint16_t vs_uniform_cap, fs_uniform_cap;
	/* Dwords the fragment array really holds: the block the application may
	 * write (fs_uniform_cap) plus the four a CONSTANT_* blend variant reads
	 * glBlendColor from, which sit right after it. */
	uint16_t fs_uniform_alloc;
	/* texture unit each fragment-shader sampler binding reads (glUniform1i) */
	uint8_t fs_sampler_unit[GL_CAP_SAMPLERS];
	uint8_t vs_sampler_unit[GL_CAP_SAMPLERS];   /* same, for vertex-stage samplers */
	/* generic attribute index the app uses for each VS input (GLSL location
	 * unless glBindAttribLocation said otherwise before the link) */
	uint8_t attrib_app_index[16];
	/* as many as a vertex shader may declare (SHC_MAX_DECLARED_ATTRIBS):
	 * aliasing binds two names to one location */
	struct { char name[32]; uint8_t index; } attrib_binds[32];
	uint8_t num_attrib_binds;
};

/* Renderbuffer: a colour renderbuffer owns linear RGBA8888 storage like a
 * texture; depth/stencil renderbuffers own nothing here (the backend keeps a
 * depth/stencil buffer per renderbuffer pair, keyed by gles_draw.ds_id). */
struct gl_renderbuffer {
	bool     used;
	bool     bound;           /* created by a glBindRenderbuffer (glIsRenderbuffer) */
	GLenum   internalformat;  /* RGBA4 / RGB5_A1 / RGB565 / DEPTH_COMPONENT16 / STENCIL_INDEX8 */
	uint32_t width, height;
	struct osal_gpu_mem mem;  /* colour formats only */
	bool     external;        /* storage belongs to an EGLImage: the image frees it once unused */
	bool     orphan;          /* as gl_texture.orphan */
	uint32_t serial;      /* as gl_texture.serial */
	uint32_t storage_gen; /* as gl_texture.storage_gen */
};

/* One framebuffer attachment point. */
struct gl_attachment {
	GLenum type;    /* GL_NONE / GL_TEXTURE / GL_RENDERBUFFER */
	GLuint name;    /* texture or renderbuffer handle */
	GLint  level;   /* texture mip level */
	GLint  layer;   /* cube face / array layer of the texture */
};

/* Colour attachments a framebuffer may carry. One render can write them all
 * (a G-buffer pass writes three), which the backend and the driver's
 * end-of-tile programs support up to GL_CAP_COLOR_ATTACHMENTS. */
#define GL_CAP_COLOR_ATTACHMENTS 3

struct gl_framebuffer {
	bool used;
	bool bound;     /* created by a glBindFramebuffer (glIsFramebuffer) */
	/* color is attachment 0; color_n[i] is attachment i + 1 */
	struct gl_attachment color, color_n[GL_CAP_COLOR_ATTACHMENTS - 1], depth, stencil;
	/* glDrawBuffers: which attachment the fragment shader's output i writes
	 * (-1 = GL_NONE). The default is output i to attachment i. */
	int8_t draw_buf[GL_CAP_COLOR_ATTACHMENTS];
	uint8_t num_draw_buf;
	bool read_none;  /* glReadBuffer(GL_NONE) */
	uint8_t read_index; /* glReadBuffer(GL_COLOR_ATTACHMENTn): the attachment read */
};

struct gl_vertex_attrib {
	bool     enabled;
	GLuint   buffer;         /* bound ARRAY_BUFFER at pointer-set time */
	GLint    size;           /* 1..4 */
	GLenum   type;           /* FLOAT (others later) */
	GLboolean normalized;
	GLsizei  stride;
	uintptr_t offset;        /* byte offset into the buffer */
	GLuint   divisor;        /* glVertexAttribDivisor: 0 per vertex, N per N instances */
	bool     integer;        /* glVertexAttribIPointer: components fetched as integers */
};

/* Vertex array object (GL_OES_vertex_array_object; core in ES 3.0). Snapshots
 * the vertex-attribute arrays and the ELEMENT_ARRAY_BUFFER binding; the
 * ARRAY_BUFFER binding is NOT VAO state (captured per attribute at
 * glVertexAttribPointer time). */
#define GL_MAX_VAOS 16
struct gl_vao {
	bool     used;
	struct gl_vertex_attrib attribs[GL_CAP_VERTEX_ATTRIBS];
	GLuint   element_buffer;
};

/* The objects a share group owns. ES 3.0 2.6.1 and EGL 1.5 3.7.1: buffers,
 * textures, renderbuffers, shaders and programs belong to the group, so every
 * context made with eglCreateContext(..., share_context, ...) sees the same
 * ones. Framebuffers and vertex arrays do not, and neither do the bindings --
 * those name a shared object per context. */
struct gl_objects {
	struct gl_buffer  buffers[GL_MAX_BUFFERS];
	struct gl_texture textures[GL_TEXTURE_SLOTS];
	struct gl_shader  shaders[GL_MAX_SHADERS];
	struct gl_program programs[GL_MAX_PROGRAMS];
	struct gl_renderbuffer renderbuffers[GL_MAX_RENDERBUFFERS];
};

struct gl_context {
	/* the share group whose objects this context uses */
	uint8_t group;
	/* the object tables that are NOT shared */
	struct gl_framebuffer  framebuffers[GL_MAX_FRAMEBUFFERS];

	/* bindings */
	GLuint bound_array_buffer;
	GLuint bound_element_buffer;
	/* OpenGL ES 3.0 adds six more buffer binding points. The copy ones are
	 * used by glCopyBufferSubData; the pixel ones make a transfer read or
	 * write a buffer instead of client memory; the indexed ones exist so
	 * that binding and querying them behaves. */
	GLuint bound_pixel_pack_buffer;
	GLuint bound_pixel_unpack_buffer;
	GLuint bound_copy_read_buffer;
	GLuint bound_copy_write_buffer;
	GLuint bound_uniform_buffer;
	GLuint bound_transform_feedback_buffer;
	GLuint bound_framebuffer;   /* 0 = the default (window) surface; the DRAW binding */
	GLuint bound_read_framebuffer; /* ES 3.0 READ binding, when read_fb_split */
	bool   read_fb_split;
	bool   default_read_none;   /* glReadBuffer(GL_NONE) on the default framebuffer */
	GLuint bound_renderbuffer;
	GLuint bound_vertex_array;   /* 0 = the default VAO (live state in attribs[]) */
	struct gl_vao vaos[GL_MAX_VAOS];
	GLuint bound_texture_2d[GL_CAP_COMBINED_TEXTURE_UNITS];   /* per texture unit */
	GLuint bound_texture_cube[GL_CAP_COMBINED_TEXTURE_UNITS];
	GLuint bound_texture_2d_array[GL_CAP_COMBINED_TEXTURE_UNITS];
	GLuint bound_texture_3d[GL_CAP_COMBINED_TEXTURE_UNITS];
	GLuint active_texture;    /* unit index */
	GLuint cur_program;

	struct gl_vertex_attrib attribs[GL_CAP_VERTEX_ATTRIBS];

	/* fixed-function render state (maps onto the PVR backend paths) */
	GLint    viewport[4];         /* x,y,w,h */
	GLclampf depth_range[2];      /* glDepthRangef near, far */
	GLclampf clear_color[4];
	GLfloat  clear_depth;
	GLint    clear_stencil;
	bool     depth_test;    GLenum depth_func;   bool depth_mask;
	bool     blend;         GLenum blend_src, blend_dst, blend_eq;       /* rgb */
	GLenum   blend_src_a, blend_dst_a, blend_eq_a;                    /* alpha */
	GLclampf blend_color[4];
	bool     cull_face;     GLenum cull_mode;    GLenum front_face;
	bool     scissor_test;  GLint  scissor[4];
	/* stencil: front face (the backend is single-faced and uses these) ... */
	bool     stencil_test;  GLenum stencil_func; GLint stencil_ref; GLuint stencil_mask;
	GLenum   stencil_sfail, stencil_dpfail, stencil_dppass; GLuint stencil_writemask;
	/* ... and the back-face state the *Separate calls record */
	GLenum   stencil_back_func; GLint stencil_back_ref; GLuint stencil_back_mask;
	GLenum   stencil_back_sfail, stencil_back_dpfail, stencil_back_dppass;
	GLuint   stencil_back_writemask;
	GLclampf color_writemask[4];
	GLfloat  line_width;
	/* glVertexAttrib*f: the value a VS input reads while its array is disabled */
	GLfloat  current_attrib[GL_CAP_VERTEX_ATTRIBS][4];
	GLint    unpack_alignment, pack_alignment;   /* glPixelStorei */
	GLint    unpack_row_length;                  /* GL_UNPACK_ROW_LENGTH (0: the upload's width) */
	GLint    unpack_image_height, unpack_skip_rows, unpack_skip_pixels, unpack_skip_images;
	GLint    pack_row_length, pack_skip_rows, pack_skip_pixels;
	/* accepted and queryable, not modelled by the backend */
	bool     polygon_offset_fill; GLfloat polygon_offset_factor, polygon_offset_units;
	bool     sample_alpha_to_coverage, sample_coverage;
	GLclampf sample_coverage_value; bool sample_coverage_invert;
	bool     dither;
	bool     primitive_restart; /* ES 3.0 GL_PRIMITIVE_RESTART_FIXED_INDEX */
	bool     rasterizer_discard; /* ES 3.0 GL_RASTERIZER_DISCARD, recorded only */
	GLenum   generate_mipmap_hint;

	/* size of the default (window) surface; the render target of a draw is
	 * resolved from bound_framebuffer by gl_fb_target() (gl/framebuffer.rs) */
	uint32_t target_w, target_h;
	uint8_t  default_format;      /* enum gles_surf_format of the default (window) surface */
	uint8_t  default_samples;     /* the default surface's samples per pixel (1 or 4) */
	/* the default surface's row pitch in pixels when it is not
	 * tex_pitch(target_w) (a view into a wider scan-out buffer); 0: that */
	uint32_t default_pitch;
	/* the default (window) colour surface; owned = allocated by gl_context_create */
	struct osal_gpu_mem default_surface;
	bool     default_surface_valid, default_surface_owned;
	uint8_t  index;               /* slot in the context pool (per-context caches key on it) */
	bool     default_y_up;        /* the default surface is stored bottom-up (a pbuffer) */

	GLenum   error;               /* first GL error since last glGetError */
};

/* the current context (single-context bring-up; EGL makeCurrent later). */
extern struct gl_context *gl_ctx;

/* helpers */
void gl_set_error_at(GLenum e, const char *where);
#define gl_set_error(e) gl_set_error_at((e), __func__)
/* Where the GL client is: the last traced entry point and a call counter,
 * for a watchdog thread to print when the client stops making progress
 * (yakogl_debug_last_call). GL_TRACE() opens the traced entry points. */
extern const char *volatile yakogl_trace_call;
extern volatile uint32_t yakogl_trace_seq;
const char *yakogl_debug_last_call(uint32_t *seq);
void yakogl_trace_log(const char *fn, uint32_t seq);
#define GL_TRACE() do { yakogl_trace_call = __func__; if (++yakogl_trace_seq <= yakogl_trace_log_first) \
			 yakogl_trace_log(__func__, yakogl_trace_seq); } while (0)
extern uint32_t yakogl_trace_log_first;   /* log the first N entry points (0: none) */              /* records first error only */
struct gl_context *gl_context_create(uint32_t w, uint32_t h);
/* Window-system hook: the default surface is memory the caller mapped for the
 * GPU (e.g. the scan-out buffer, w x h pixels, tightly packed rows). format
 * is its enum gles_surf_format (XRGB8888 or RGB565 scan-out, RGBA8888); FBO
 * textures stay RGBA. The caller keeps ownership of `surface`. */
/* Share groups (struct gl_objects). gl_group_alloc(share) takes a reference to
 * an existing group, or to a new empty one when share is GL_GROUP_NONE, and
 * answers GL_GROUP_NONE when every slot is in use; gl_group_release()
 * gives a reference back and the objects go with the last holder. EGL takes
 * one per EGLContext, because an EGLContext exists -- and can be named as
 * another's share_context -- before the first eglMakeCurrent gives it a GL
 * context. `group` below is such a group, or GL_GROUP_NONE for objects of the
 * context's own; the caller keeps the reference it passes. */
#define GL_GROUP_NONE 0xff
uint8_t gl_group_alloc(uint8_t share);
void    gl_group_release(uint8_t group);
struct gl_context *gl_context_create_for_surface(struct osal_gpu_mem *surface,
						 uint32_t w, uint32_t h, uint8_t format,
						 uint8_t samples, uint8_t group);
void gl_context_destroy(struct gl_context *c);
/* EGL hooks: make a context current (NULL = none) and repoint the current
 * context's default surface without resetting its state.
 *
 * gl_context_bind_surface() returns 0, or -EINVAL when `surface` cannot be the
 * render target for w x h in that format -- the context is then left bound to
 * the PREVIOUS surface, so a caller that discards the result draws into the
 * old surface and reads back the new one with no error raised anywhere. */
void gl_context_set_current(struct gl_context *c);
int gl_context_bind_surface(struct osal_gpu_mem *surface, uint32_t w, uint32_t h, uint8_t format,
			    uint8_t samples);

#endif /* GL_CONTEXT_H */
