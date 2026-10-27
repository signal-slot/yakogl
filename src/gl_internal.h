/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * gl_internal.h - the C view of the GL core's internals.
 *
 * The GL core is the Rust crate (src/gl/); this header, gl_context.h, backend.h
 * and pvr/pvr_backend.h describe the #[repr(C)] structs and the #[no_mangle]
 * helpers it exports, for C code that looks inside the library: the on-silicon
 * test suite, the samples' glue and diagnostics. Keep them in step with
 * src/gl/context.rs, src/backend.rs and src/pvr/backend.rs.
 *
 * NOT a public header: apps only ever see <GLES2/gl2.h>.
 */
#ifndef GL_INTERNAL_H
#define GL_INTERNAL_H

#include <GLES3/gl3.h>   /* the ES 2.0 API plus the ES 3.0 entry points this core provides */

#include "gl_context.h"
#include "backend.h"

/* The Khronos tokens keep their values in every file that includes this one.
 * gl_context.h once redefined GL_MAX_VERTEX_ATTRIBS as its array size (16)
 * and this header #undef'd the token to make room, so gl_test.c's
 * glGetIntegerv(GL_MAX_VERTEX_ATTRIBS) asked for pname 16: GL_INVALID_ENUM on
 * every board, and reset_state left every attribute array enabled. The
 * array size is GL_CAP_VERTEX_ATTRIBS now. */
_Static_assert(GL_MAX_VERTEX_ATTRIBS == 0x8869, "GL_MAX_VERTEX_ATTRIBS is the Khronos query token");

/* ---- GPU device-VA heaps. The backend owns the layout (yakogl_heap_base);
 * the OSAL bump-allocates inside the heap the core names. Buffers, textures and
 * uniform blocks go to the general data heap; shader code to the USC heap. --- */
#define GLES_HEAP_BASE      yakogl_heap_base(GLES_HEAP_DATA)
#define GLES_HEAP_BASE_USC  yakogl_heap_base(GLES_HEAP_SHADER)

/* Bytes-per-texel for the only texel storage format we keep (linear RGBA8888). */
#define GL_RGBA8_BPP    4u

/* ---- object-table helpers (index 0 is the reserved/invalid GL handle) -------
 * Handles are 1-based indices into the fixed tables in gl_context.h. These return
 * NULL for handle 0, out-of-range, or an unused slot. */
struct gl_buffer  *gl_lookup_buffer(GLuint h);
struct gl_texture *gl_lookup_texture(GLuint h);
struct gl_shader  *gl_lookup_shader(GLuint h);
struct gl_program *gl_lookup_program(GLuint h);
struct gl_framebuffer  *gl_lookup_framebuffer(GLuint h);
struct gl_renderbuffer *gl_lookup_renderbuffer(GLuint h);

/* ---- render target (gl/framebuffer.rs) ---------------------------------------
 * The colour buffer a draw renders into: the bound framebuffer's colour
 * attachment, or the default surface when framebuffer 0 is bound. Returns NULL
 * (and sets w/h to 0) when the bound framebuffer is incomplete. */
/* The bound framebuffer's colour target: its storage, size, and the byte
 * offset of the attached level/face/layer inside that storage. */
struct osal_gpu_mem *gl_fb_target(uint32_t *w, uint32_t *h, uint32_t *offset);
uint8_t gl_fb_target_format(void);   /* enum gles_surf_format of the bound target */
/* Colour attachment n (n >= 1) of the bound framebuffer: false when there is
 * none. Attachment 0 is gl_fb_target(). */
bool gl_fb_target_n(uint8_t n, uint64_t *gpu, uint32_t *w, uint32_t *h, uint8_t *format);
/* true when the bound target is stored top-down (the default surface), so GL's
 * bottom-up window y must be flipped on render and on read-back. */
bool gl_fb_flip_y(void);
/* Object deletion hooks: detach the object from every framebuffer (GL 4.4.2). */
void gl_fb_texture_deleted(GLuint texture);
void gl_fb_renderbuffer_deleted(GLuint renderbuffer);
/* Point the bound texture/renderbuffer at an EGLImage's storage (w x h, rows
 * at the pair-padded pitch; surf_format = enum gles_surf_format), which the
 * object uses without owning it: the image frees it once nothing points at it
 * any more.
 *
 * `owner` is the image, for source compatibility with the C library's
 * signature. This port finds the storage record from `mem` itself (every
 * reference is to the memory, whichever image or C caller supplied it), so it
 * ignores the argument; pass the image or NULL. */
int gl_texture_attach_external(const struct osal_gpu_mem *mem, uint32_t w, uint32_t h,
			       uint8_t surf_format, void *owner);
int gl_renderbuffer_attach_external(const struct osal_gpu_mem *mem, uint32_t w, uint32_t h,
				    void *owner);

/* Default (window) color surface allocated by gl_context_create; used as the
 * render target whenever ctx->target is NULL (no FBO bound). */
struct osal_gpu_mem *gl_default_surface(void);

/* ---- pending-clear plumbing (src/gl/draw.rs) -------------------------------
 * glClear records a pending clear (color captured at glClear time, per GL). The
 * next glDraw* fuses it into gles_draw.do_clear (one kick, clear-on-tile-load);
 * gl_flush_pending_clear() emits a clear-only kick for a glClear with no
 * following draw (called from glFlush/glFinish). */
void gl_flush_pending_clear(void);

/* Called by gl_context_destroy to release per-program GPU side-allocations
 * (USC + uniform blocks) that gl/draw.rs caches. */
void gl_draw_release_program(GLuint program);
struct gl_program;
void gl_program_free_uniforms(struct gl_program *p);   /* gl/program.rs */

/* Called by gl/program.rs when a program is (re)linked so the draw layer drops
 * any stale cached USC/uniform GPU uploads for that handle. */
void gl_draw_invalidate_program(GLuint program);

/* Bring-up debug (gl/draw.rs): the GPU allocation holding a program's VS (0) or
 * FS (1) USC, or NULL. */
const struct osal_gpu_mem *gl_draw_debug_usc(GLuint program, int fragment);

/* Row pitch of a linear RGBA8888 texture/renderbuffer level in texels. The
 * PBE writes rows at a pitch of whole pixel pairs (pbe_word[0] linestride),
 * so an odd-width render target (Qt Quick 3D's View3D beside a 305-px
 * drawer: 719 wide) is padded to the next even texel count, and every row
 * computation -- storage, uploads, read-back, the TPU stride -- uses the
 * same pitch. Mip chains exist only for power-of-two sizes, where the pitch
 * equals the width, so the hardware's mip layout is unchanged. */
#define GL_TEX_PITCH(w) (((uint32_t)(w) + 1u) & ~1u)

void gl_debug_bind_hist(int op, unsigned h);
void gl_debug_dump_bind_hist(void);


/* OpenGL ES 3.0 entry points YakoGL provides on top of its ES 2.0 header
 * (gl/program.rs): Qt's RHI calls the unsigned setters for uniforms whose
 * SPIR-V type is uint, whatever the GLSL ES 1.00 text says. */
GL_APICALL void GL_APIENTRY glUniform1ui(GLint location, GLuint v0);
GL_APICALL void GL_APIENTRY glUniform2ui(GLint location, GLuint v0, GLuint v1);
GL_APICALL void GL_APIENTRY glUniform3ui(GLint location, GLuint v0, GLuint v1, GLuint v2);
GL_APICALL void GL_APIENTRY glUniform4ui(GLint location, GLuint v0, GLuint v1, GLuint v2, GLuint v3);
GL_APICALL void GL_APIENTRY glUniform1uiv(GLint location, GLsizei count, const GLuint *value);
GL_APICALL void GL_APIENTRY glUniform2uiv(GLint location, GLsizei count, const GLuint *value);
GL_APICALL void GL_APIENTRY glUniform3uiv(GLint location, GLsizei count, const GLuint *value);
GL_APICALL void GL_APIENTRY glUniform4uiv(GLint location, GLsizei count, const GLuint *value);
GL_APICALL void GL_APIENTRY glTexImage3D(GLenum target, GLint level, GLint internalformat, GLsizei width, GLsizei height, GLsizei depth, GLint border, GLenum format, GLenum type, const void *pixels);
GL_APICALL void GL_APIENTRY glTexSubImage3D(GLenum target, GLint level, GLint xoffset, GLint yoffset, GLint zoffset, GLsizei width, GLsizei height, GLsizei depth, GLenum format, GLenum type, const void *pixels);
GL_APICALL void GL_APIENTRY glCompressedTexImage3D(GLenum target, GLint level, GLenum internalformat, GLsizei width, GLsizei height, GLsizei depth, GLint border, GLsizei imageSize, const void *data);
GL_APICALL void GL_APIENTRY glTexStorage2D(GLenum target, GLsizei levels, GLenum internalformat, GLsizei width, GLsizei height);
GL_APICALL void GL_APIENTRY glTexStorage3D(GLenum target, GLsizei levels, GLenum internalformat, GLsizei width, GLsizei height, GLsizei depth);
GL_APICALL void GL_APIENTRY glFramebufferTextureLayer(GLenum target, GLenum attachment, GLuint texture, GLint level, GLint layer);

#endif /* GL_INTERNAL_H */
