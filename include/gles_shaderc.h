/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * gles_shaderc.h - shader-backend interface: GLSL ES -> PowerVR USC + metadata.
 *
 * glShaderSource/glCompileShader/glLinkProgram call through this. Two backends:
 *   - offline (Zephyr/bare-metal): glShaderSource text is hashed and looked up in
 *     a build-time table of precompiled {USC + metadata} blobs (tools/glsl2usc,
 *     which wraps the pco_shc GLSL->SPIR-V->NIR->PCO toolchain).
 *   - online (Linux/PX5/dev): the PCO compiler runs on-target for arbitrary source.
 *
 * Both produce the same shader ABI the PowerVR backend consumes. The ABI matches
 * what the toolchain already emits (see scratchpad/shader_toolchain.md):
 *   VS: attributes in vi0.. (ascending location, 1 dword/attrib dword); outputs
 *       emitted via UVSW at vtxout offsets; vs_output_size dwords.
 *   FS: input varyings iterated from coefficient regs cf.. via a DOUTI program
 *       (w iterator at cf0..3 for perspective/smooth, then VAR0, VAR1, ..);
 *       uniforms (push-const) in shared regs sh0.. via a DOUTD; output to po0...
 */
#ifndef GLES_SHADERC_H
#define GLES_SHADERC_H

#include <stdint.h>
#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

enum shc_stage { SHC_STAGE_VERTEX = 0, SHC_STAGE_FRAGMENT = 1 };

/* Blending on this GPU is not fixed-function: the compiler lowers it into the
 * fragment shader (an on-chip read-modify-write of the pixel output). So a
 * blended draw needs a *variant* of its FS compiled for that blend state. The
 * state is summarised in a key (0 = no blending / replace). Factor and
 * equation codes follow Mesa's pipe_blendfactor / pipe_blend_func so the
 * generator and the runtime agree. */
enum shc_blend_factor {
	SHC_BF_ONE = 1, SHC_BF_SRC_COLOR, SHC_BF_SRC_ALPHA, SHC_BF_DST_ALPHA,
	SHC_BF_DST_COLOR, SHC_BF_SRC_ALPHA_SATURATE, SHC_BF_CONST_COLOR, SHC_BF_CONST_ALPHA,
	SHC_BF_INVERT = 0x10,
	SHC_BF_ZERO = SHC_BF_INVERT | SHC_BF_ONE,
	SHC_BF_INV_SRC_COLOR = SHC_BF_INVERT | SHC_BF_SRC_COLOR,
	SHC_BF_INV_SRC_ALPHA = SHC_BF_INVERT | SHC_BF_SRC_ALPHA,
	SHC_BF_INV_DST_ALPHA = SHC_BF_INVERT | SHC_BF_DST_ALPHA,
	SHC_BF_INV_DST_COLOR = SHC_BF_INVERT | SHC_BF_DST_COLOR,
	SHC_BF_INV_CONST_COLOR = SHC_BF_INVERT | SHC_BF_CONST_COLOR,
	SHC_BF_INV_CONST_ALPHA = SHC_BF_INVERT | SHC_BF_CONST_ALPHA,
};
enum shc_blend_func { SHC_BLEND_ADD = 0, SHC_BLEND_SUBTRACT, SHC_BLEND_REVERSE_SUBTRACT,
		      SHC_BLEND_MIN, SHC_BLEND_MAX };
/* Variant key layout: [0:4] src_rgb, [5:9] dst_rgb, [10:14] src_a, [15:19]
 * dst_a, [20:22] eq_rgb, [23:25] eq_a, [26:29] disabled colour channels
 * (glColorMask, bit 0 = R ... bit 3 = A; the mask is lowered into the FS on
 * this GPU, like blending). */
#define SHC_BLEND_KEY(src_rgb, dst_rgb, src_a, dst_a, eq_rgb, eq_a) \
	((uint32_t)(src_rgb) | ((uint32_t)(dst_rgb) << 5) | ((uint32_t)(src_a) << 10) | \
	 ((uint32_t)(dst_a) << 15) | ((uint32_t)(eq_rgb) << 20) | ((uint32_t)(eq_a) << 23))
#define SHC_BLEND_KEY_MASK_OFF(disabled_rgba) ((uint32_t)((disabled_rgba) & 0xfu) << 26)
/* GC7000 only. Blending and colour masks are pixel-engine state on that part,
 * so the blend key above is always 0 there and the whole space is free; this
 * borrows the value 1 to mean "the same fragment shader, compiled for the
 * opposite front-face winding". See shc_shader.reads_front_facing. */
#define SHC_VIV_KEY_FRONT_FLIPPED 1u
/* Bits 30..31: the render target class the colour output is packed for
 * (glsl2usc '@rt' variants, pco_shc PCO_SHC_OUT): 0 RGBA8 unorm (the base
 * shader), 1 RGBA16F (four halves in two dwords), 2 R16F, 3 R32F. */
#define SHC_BLEND_KEY_RT_SHIFT 30
#define SHC_BLEND_KEY_RT(cls)  ((uint32_t)((cls) & 3u) << SHC_BLEND_KEY_RT_SHIFT)
#define SHC_RT_RGBA8   0u
#define SHC_RT_F16X4   1u
#define SHC_RT_F16     2u
#define SHC_RT_F32     3u
/* ONE/ZERO ADD on both channels with every channel written is a replace: no
 * variant needed. */
#define SHC_BLEND_KEY_REPLACE \
	SHC_BLEND_KEY(SHC_BF_ONE, SHC_BF_ZERO, SHC_BF_ONE, SHC_BF_ZERO, SHC_BLEND_ADD, SHC_BLEND_ADD)

/* One interpolated fragment-input varying (for building the DOUTI coeff program). */
struct shc_varying {
	uint8_t location;     /* VARYING_SLOT_VAR0 + n */
	uint8_t components;   /* 1..4 f32 */
	uint8_t interp;       /* 0 smooth(persp), 1 noperspective(npc), 2 flat */
	uint8_t vtxout_off;   /* dword offset in the VS output vector */
	/* GLSL name ("" when the source gave explicit locations): a fragment
	 * shader's inputs are linked to the vertex shader's outputs by name
	 * (GLSL ES 1.00 `varying`), which patches vtxout_off at link time. */
	char    name[32];
};

/* One vertex attribute (for building vertex-fetch). */
struct shc_attrib {
	uint8_t location;     /* GLSL layout(location) */
	uint8_t components;   /* 1..4 */
	uint8_t vi_reg;       /* first vi register */
	uint8_t format;       /* 0 = float32; packed formats later */
	char    name[32];     /* GLSL variable name (glGetAttribLocation) */
	uint16_t gl_type;     /* GL_FLOAT_VEC4 ... (glGetActiveAttrib) */
};

/* Link-time attribute layout (src/shaderc/SHADERC_NOTES.md): one declared
 * vertex attribute and the generic index of its first column the link
 * decided; columns 1, or 2..4 for a matrix. Two entries whose ranges overlap
 * alias. No register numbers: the back end places what it reads. */
#define SHC_MAX_DECLARED_ATTRIBS 32
#define SHC_ATTRIB_NO_LOCATION   0xff
struct shc_attrib_binding {
	char    name[32];     /* as shc_attrib.name */
	uint8_t location;
	uint8_t columns;
	uint16_t gl_type;     /* as declared: what glGetActiveAttrib reports */
	uint8_t active;       /* 1: statically read (an active attribute) */
};
struct shc_attrib_map {
	uint8_t count;
	struct shc_attrib_binding b[SHC_MAX_DECLARED_ATTRIBS];
};

/* One uniform the app can address by name (glGetUniformLocation). Block
 * members live in the stage's push-constant block (dword_off/dwords, std430
 * layout as the compiler packed them); samplers are not in the block
 * (dword_off = SHC_UNIFORM_NO_BLOCK) and carry their descriptor binding, which
 * is the texture unit the backend wires the sampler to. */
#define SHC_NO_PSIZE 0xffu
#define SHC_NO_BLEND_CONSTS 0xffu
#define SHC_BLEND_CONSTS_DWORDS 4u   /* glBlendColor rgba, appended to a CONSTANT_* variant's block */
#define SHC_UNIFORM_NAME_MAX 64   /* Qt Quick 3D: "_71.qt_material_clearcoat_fresnel_power" is 39 */
#define SHC_UNIFORM_NO_BLOCK 0xffffu
/* A fragment shader that uses discard is compiled in FULL mode, where the USC
 * reads a per-draw ISP-feedback control word (colour-write enables, sample
 * mask, alpha-to-coverage / alpha-to-one) from a shared register. SHC_NO_META
 * marks a shader that needs none. See shc_shader.meta_off. */
#define SHC_NO_META 0xffffu
/* The benign default the backend supplies for the full-mode meta word on this
 * non-MSAA path: colour writes enabled on all 8 RTs (COLOR_WRITE_ENABLE=0xff),
 * every sample-mask bit set (SAMPLE_MASK=0xffff), no alpha-to-coverage and no
 * alpha-to-one. Bit layout from Mesa src/imagination/common/pvr_iface.h
 * (PVR_FS_META_ALPHA_TO_ONE off 0, COLOR_WRITE_ENABLE off 1 len 8,
 * SAMPLE_MASK off 9 len 16, ALPHA_TO_COVERAGE off 25); packed the way
 * vulkan/pvr_arch_cmd_buffer.c builds PVR_BUFFER_TYPE_FS_META. */
#define SHC_FS_META_DEFAULT 0x01fffffeu
struct shc_uniform {
	char     name[SHC_UNIFORM_NAME_MAX];
	uint16_t dword_off;   /* offset in the stage's uniform block, or NO_BLOCK */
	uint16_t dwords;      /* size (arrays: whole array) */
	uint16_t gl_type;     /* GL_FLOAT_VEC4, GL_FLOAT_MAT4, GL_SAMPLER_2D, ... */
	uint8_t  array_len;   /* 1 for non-arrays */
	uint8_t  binding;     /* samplers: descriptor binding (texture unit) */
	uint8_t  inner;       /* elements per outer index of a flattened array-of-structs member
	                       * ("A.B[0].C": B[k].C is element k*inner; 1 for a plain array, 0 = 1) */
};

/* ---- Vivante GC7000 (i.MX8MP) shader ABI ----------------------------------
 *
 * On a YAKOGL_GPU_VIVANTE build the same `struct shc_shader` carries the
 * reflection (attributes, varyings, uniforms), `usc`/`usc_dw` below carry the
 * GC7000 instruction words instead of PowerVR USC (the GL core uploads the
 * binary the same way for both GPUs, so the field keeps its name; the backend
 * calls it the shader code), and `viv` carries everything else the backend has
 * to program: the register numbers the shader reads and writes, and the
 * constant register file. src/vivante/VIVANTE_NOTES.md documents it in full.
 */
#if defined(YAKOGL_GPU_VIVANTE)

/* Where one dword of the constant register file comes from. The codes are
 * Mesa's enum etna_uniform_contents, which the offline tool reports. */
enum shc_viv_const {
	SHC_VIV_CONST_UNUSED = 0,
	SHC_VIV_CONST_LITERAL,          /* data is the value itself */
	SHC_VIV_CONST_UNIFORM,          /* data is a dword index into the uniform block */
	SHC_VIV_CONST_TEXRECT_SCALE_X,  /* data is a sampler binding: 1.0 / width */
	SHC_VIV_CONST_TEXRECT_SCALE_Y,  /* data is a sampler binding: 1.0 / height */
	SHC_VIV_CONST_TEXTURE_WIDTH,    /* data is a sampler binding (textureSize) */
	SHC_VIV_CONST_TEXTURE_HEIGHT,
	SHC_VIV_CONST_TEXTURE_DEPTH,
	SHC_VIV_CONST_SAMPLER_LOD_MIN,  /* data is a sampler binding */
	SHC_VIV_CONST_SAMPLER_LOD_MAX,
	SHC_VIV_CONST_SAMPLER_LOD_BIAS,
	SHC_VIV_CONST_UBO_ADDR,         /* uniform blocks: not produced by this path */
	SHC_VIV_CONST_FRAGCOORD_Y_SCALE,  /* ours: -1 on a mirrored target, else 1 */
	SHC_VIV_CONST_FRAGCOORD_Y_OFFSET, /* ours: the target height when mirrored, else 0 */
	SHC_VIV_CONST_PNTC_Y_SCALE,       /* ours: 1 on a mirrored target, else -1 */
	SHC_VIV_CONST_PNTC_Y_OFFSET,      /* ours: 0 on a mirrored target, else 1 */
};

/* One shader input or output register. */
struct shc_viv_io {
	uint8_t reg;         /* GC7000 register number */
	uint8_t slot;        /* the compiler's gl_varying_slot / gl_vert_attrib */
	uint8_t components;  /* 1..4 */
	uint8_t interp;      /* fragment inputs: 0 smooth, 1 noperspective, 2 flat */
	char    name[32];    /* GLSL name: how link() pairs VS outputs with FS inputs */
};

#define SHC_VIV_NO_REG 0xffu   /* an int8_t -1 from the compiler, as a register */
#define SHC_VIV_MAX_IO 16

/* The GC7000-specific half of a compiled shader. */
struct shc_viv {
	/* inputs: VS attributes, FS varyings (the compiler places FS varyings in
	 * registers 1..N, so N is also the varying count) */
	struct shc_viv_io in[SHC_VIV_MAX_IO];
	uint8_t num_in;
	/* outputs: VS varyings and position, FS colour */
	struct shc_viv_io out[SHC_VIV_MAX_IO];
	uint8_t num_out;
	/* fragment, filled by link(): the VS output register that feeds in[i],
	 * or SHC_VIV_NO_REG when the vertex shader writes no such output */
	uint8_t vs_reg_of_in[SHC_VIV_MAX_IO];

	/* special registers (SHC_VIV_NO_REG when the shader has none) */
	uint8_t vs_pos_out_reg, vs_pointsize_out_reg, vs_id_in_reg;
	uint8_t ps_color_out_reg, ps_depth_out_reg;

	uint32_t vs_load_balancing;  /* VS_LOAD_BALANCING word the compiler chose */
	uint16_t input_count_unk8;   /* {VS,PS}_INPUT_COUNT field UNK8 */

	/* constant register file: one entry per dword, in register order */
	const uint8_t  *const_contents;  /* enum shc_viv_const */
	const uint32_t *const_data;
	uint16_t const_count;
};
#endif /* YAKOGL_GPU_VIVANTE */

/* ---- Broadcom V3D 4.2 (Raspberry Pi 4) shader ABI -------------------------
 *
 * The V3D compiles a program, not a stage. On a YAKOGL_GPU_V3D build a
 * compiled stage carries only its reflection and no code; link() replaces the
 * program's fragment shader with the linked program, whose `usc`/`usc_dw` hold
 * the three QPU programs (FS, VS, CS) back to back and whose `v3d` says where
 * each starts and what the shader state record needs for it
 * (docs/online-shaderc/v3d-backend.md, "M6"). The structs are declared in
 * every build, like their Rust mirrors, so the layout pins cover them
 * everywhere; only the `v3d` member of struct shc_shader is V3D-only.
 */
#define SHC_V3D_NO_RECORD      0xffu  /* an attribute location the program does not read */
#define SHC_V3D_PRIM_TRIANGLES 0u

/* One uniform stream word: Mesa's enum quniform_contents and its datum. For
 * QUNIFORM_UNIFORM (1) the datum is a dword index into the stage's uniform block. */
struct shc_v3d_uniform {
	uint32_t contents;
	uint32_t data;
};

/* The fields of the driver's struct v3d_fs_prog that are not addresses. */
struct shc_v3d_fs {
	uint32_t code_off;      /* bytes into usc, a multiple of 8 */
	uint32_t code_bytes;
	const struct shc_v3d_uniform *uniforms;
	uint32_t num_uniforms;
	uint32_t num_inputs;
	uint32_t flat_shade_flags, noperspective_flags, centroid_flags;
	uint8_t  threads, single_seg, writes_z, disable_ez, uses_center_w;
	uint8_t  lock_scoreboard_on_first_thrsw, uses_implicit_point_line_varyings, uses_pid;
	uint8_t  rt_mask;       /* bit n: the program writes colour to render target n (0..3) */
};

/* The fields of the driver's struct v3d_vs_prog that are not addresses. */
struct shc_v3d_vs {
	uint32_t code_off;
	uint32_t code_bytes;
	const struct shc_v3d_uniform *uniforms;
	uint32_t num_uniforms;
	uint32_t vpm_input_size, vpm_output_size, vcm_cache_size;
	uint8_t  threads, single_seg, separate_segments, uses_vid, uses_iid, uses_biid, writes_psiz;
	uint8_t  vattr_sizes[16];  /* per attribute record */
};

/* The V3D half of a compiled shader: all zero on a stage, filled by link(). */
struct shc_v3d {
	uint8_t linked;                   /* 1 on a linked program */
	uint8_t prim;                     /* SHC_V3D_PRIM_* */
	uint8_t num_records;              /* attribute records fetched, 0..16 */
	uint8_t record_of_location[16];   /* per location: its record, or SHC_V3D_NO_RECORD */
	struct shc_v3d_fs fs;
	struct shc_v3d_vs vs, cs;
};

/* Compiled-shader ABI the backend needs to wire the shader up. */
struct shc_shader {
	/* the compiled binary, in GPU instruction dwords: PowerVR USC, or the
	 * GC7000 shader code on a YAKOGL_GPU_VIVANTE build */
	const uint32_t *usc;
	uint32_t        usc_dw;   /* count */
	uint16_t        temps;    /* fragment/vertex temp register count */

	/* fragment: the inputs; vertex: the outputs (name -> vtxout_off) */
	struct shc_varying varyings[16];
	uint8_t  num_varyings;
	uint8_t  uses_w;          /* perspective => a w iterator at cf0..3 */
	uint8_t  uses_discard;    /* needs full-mode meta control word */
	/* fragment, GC7000 only: the shader reads gl_FrontFacing, whose value
	 * the hardware hands over as a raw 0.0/1.0 with no sense of its own.
	 * There the sense is compiled in, so such a shader has a second binary
	 * under the same sha256 with blend_key SHC_VIV_KEY_FRONT_FLIPPED, and
	 * the GL core asks for it when the effective winding is reversed. The
	 * PowerVR needs none of this: fs_front_face_off1 below names a shared
	 * register the driver writes the sense into at draw time. */
	uint8_t  reads_front_facing;
	/* full-mode (discard) shaders: shared-reg offset of the driver-supplied
	 * ISP-feedback meta control word (SHC_FS_META_DEFAULT); SHC_NO_META = none */
	uint16_t meta_off;
	/* gl_FrontFacing: the shared register the driver writes the front-face
	 * op into (0 keep the ISP's facing, 1 swap it, 2 always front), PLUS
	 * ONE (0 = the shader does not read gl_FrontFacing). */
	uint16_t fs_front_face_off1;
	uint8_t  num_outputs;     /* po0..po(N-1) */
	/* fragment: the outputs that did not fit the pixel's four output
	 * registers (three RGBA16F attachments want six) are stored to tile
	 * buffers instead, in output order after the ones that fit. The driver
	 * writes each buffer's address into two shared registers starting at
	 * tile_buffer_sh, and the end-of-tile program loads the data back
	 * before emitting that attachment. 0 = every output is a register. */
	uint8_t  num_tile_buffers;
	uint16_t tile_buffer_sh;

	/* vertex */
	struct shc_attrib attribs[16];
	uint8_t  num_attribs;
	uint8_t  vs_output_size;  /* dwords emitted (position + varyings [+ point size]) */
	uint8_t  vs_psize_off;    /* vtxout dword of gl_PointSize, SHC_NO_PSIZE if not written */
	/* OpenGL ES 3.0 gl_VertexID / gl_InstanceID: the vtxin register the
	 * compiler reads them from, PLUS ONE (0 = the shader reads none; a table
	 * entry from before these fields existed is all zeros). The vertex fetch
	 * has to write the IDs there (not provided yet: such a shader reads
	 * whatever the register holds). */
	uint8_t  vs_vertex_id_reg1;
	uint8_t  vs_instance_id_reg1;

	/* uniforms (push-constant block) -> shared regs sh0.. via DOUTD */
	uint16_t uniform_dwords;  /* size of the uniform block in dwords (0 = none) */
	/* The prefix of that block the compiled shader actually reads (a
	 * contiguous [0, N) DMA). The application writes the whole block, but
	 * only this much has to reach the shared registers -- a Quick 3D
	 * material writes a hundred dwords and reads a dozen, and shared
	 * registers are what limits how many tasks the USC runs at once.
	 * 0 = the whole block. */
	uint16_t uniform_sh_dwords;
	const struct shc_uniform *uniforms;   /* name reflection (block members + samplers) */
	uint8_t  num_uniforms;

	/* fragment: the blend state this binary was compiled for (0 = none) and
	 * whether it reads the pixel output (needs the TRANSLUCENT pass type) */
	uint32_t blend_key;
	uint8_t  uses_fbfetch;
	/* fragment: dword offset in the uniform block where the variant reads
	 * the blend colour (glBlendColor rgba, CONSTANT_* factors); the block
	 * (uniform_dwords) includes those 4 dwords. SHC_NO_BLEND_CONSTS if the
	 * variant reads none. */
	uint8_t  blend_consts_off;
	/* fragment, PowerVR: the shader reads gl_PointCoord. pco iterates it
	 * with no divide in the eight coefficients straight after W's four
	 * (from 0 when nothing iterates W) and places every varying after it;
	 * the driver's coefficient program must feed them with a point-sprite
	 * iterator (Mesa pvr_arch_pipeline.c: DOUTI GOURAUD, 2D, pointsprite).
	 * In what was padding: no field moved. */
	uint8_t  uses_pntc;
	/* 1: the fragment shader reads gl_FragCoord.z, which pco iterates
	 * non-perspective from its own coefficient set at cf0..3, BEFORE W
	 * (which then sits at cf4..7) and every other input (Mesa pco_shc.c
	 * alloc_fs_varyings: POS z/w first; pco_trans_nir.c
	 * trans_load_input_fs, VARYING_SLOT_POS). The driver's coefficient
	 * program must set up that Z plane first. In what was padding: no
	 * field moved. */
	uint8_t  uses_z;
	/* fragment, PowerVR, online compiler: the shader reads gl_FragCoord and
	 * computes y' = y * scale + offset from a pair of floats the driver
	 * writes per draw at shared register fragcoord_ytf_off1 - 1: (-1,
	 * height) on a target stored top-down (the draw's flip_y), (1, 0) on
	 * any other. ONE-BASED: 0 = the shader reads no pair (pco's table
	 * programs, and anything zero-initialised). In what was padding: no
	 * field moved. */
	uint16_t fragcoord_ytf_off1;
	/* PowerVR, online compiler: 1 when the program samples a samplerCube
	 * as the TPU's own cube (smp 3d), so the draw must bind a twiddled
	 * TEXTYPE_CUBE copy (gl::native_cube); 0 for every stacked-cube
	 * program (the table's, and the online compiler's with pvr_native_cube
	 * off). In what was padding: no field moved. */
	uint8_t  native_cube;
	/* Vertex: the back end needs the program's bindings to lay the inputs
	 * out and compiled for the reflection only; COMPILE_STATUS is true and
	 * glLinkProgram compiles again with the layout (compile_vs_linked). */
	uint8_t  layout_at_link;
	/* With layout_at_link: every declared attribute, location
	 * SHC_ATTRIB_NO_LOCATION; NULL otherwise. */
	const struct shc_attrib_binding *declared;
	uint8_t  num_declared;

#if defined(YAKOGL_GPU_VIVANTE)
	/* the GC7000 register wiring and constant file (struct shc_viv above);
	 * varyings[]/vs_output_size and the blend fields stay unused there */
	struct shc_viv viv;
#endif
#if defined(YAKOGL_GPU_V3D)
	/* the linked V3D program (struct shc_v3d above) */
	struct shc_v3d v3d;
#endif

	const void *priv;         /* backend handle (offline: the table entry) */
	/* Debug only: the name of the source this shader was compiled from
	   ("" when the backend does not track one). Reading it costs nothing
	   and a per-draw log that names the shader turns "which variant is
	   this?" from an afternoon into a grep. */
};

struct shc_backend {
	/* Compile one stage. `glsl` is the app's shader source (NUL-terminated). On
	 * success fills *out and returns 0; on failure returns <0 and, if `log` is
	 * non-NULL, writes a compile log. The offline backend matches by source hash. */
	int (*compile)(enum shc_stage stage, const char *glsl,
		       struct shc_shader *out, char *log, size_t log_sz);
	/* Link/validate a VS+FS pair: matches the FS varying inputs to the VS
	 * outputs (by name when the shaders carry names, else by location) and
	 * patches the FS copy's vtxout_off wiring. */
	int (*link)(const struct shc_shader *vs, struct shc_shader *fs,
		    char *log, size_t log_sz);
	/* The variant of a compiled fragment shader for one blend state
	 * (SHC_BLEND_KEY). Returns 0 and fills *out; -ENOMEM when the compiler
	 * had no room for it now (it may succeed once something is freed); any
	 * other value < 0 when the variant cannot be had (-ENOENT: the offline
	 * table has no such variant). On failure, if `log` is non-NULL, writes
	 * why. */
	int (*blend_variant)(const struct shc_shader *fs, uint32_t blend_key,
			     struct shc_shader *out, char *log, size_t log_sz);
	/* Compile a vertex shader with the program's attribute layout, at
	 * glLinkProgram (when the compile set layout_at_link, or the bindings
	 * alias). NULL: no such path -- the offline table cannot recompile;
	 * with layout_at_link the link fails. */
	int (*compile_vs_linked)(const char *glsl, const struct shc_attrib_map *map,
				 struct shc_shader *out, char *log, size_t log_sz);
	/* Sampler-state variants (src/shaderc/mod.rs LodKey): which texture
	 * bindings' filter state the shader's code depends on, one bit per
	 * binding -- low 32 bits "no mipmap filter" (PowerVR exact gradients not
	 * needed), high 32 bits "mag LINEAR with min NEAREST_MIPMAP_*" (the
	 * c = 0.5 switch remapped). 0: none. NULL: no variants (offline). */
	uint64_t (*lod_sensitivity)(const struct shc_shader *sh);
	/* The shader compiled again for one such key; 0 and *out filled, or <0
	 * with the meaning blend_variant gives it (-ENOMEM: no room now) and,
	 * if `log` is non-NULL, why. Key 0 copies the input. */
	int (*sampler_variant)(const struct shc_shader *sh, uint64_t key,
			       struct shc_shader *out, char *log, size_t log_sz);
};

extern const struct shc_backend *g_shaderc;

/* Select a backend explicitly, before the first GL call: the offline one
 * (the table compiled into the image), or the in-process compiler (only in a
 * build with the cargo feature `shaderc-online`, which selects it on its
 * own). Each returns the backend it selected. */
const struct shc_backend *shc_offline_register(void);
const struct shc_backend *shc_online_register(void);

/* Debug: the source basename behind a compiled shader ("" if unknown). */
const char *shc_shader_source_name(const struct shc_shader *sh);

/* DIAGNOSTIC entry, not API: the in-process compiler's keep region, bytes
 * held now and the most ever held (either pointer may be NULL); returns 0.
 * The name is stable and there is no struct behind it, so it adds nothing to
 * the pinned layouts. It exists in the static archive (libyakogl.a, what the
 * Zephyr and gl_test_user builds link) and only in a build with the online
 * compiler (cargo feature shaderc-online), so a harness that may run on
 * either declares it weak. It is deliberately NOT exported from
 * libGLESv2.so: tools/yakogl.ver keeps everything but the Khronos entry
 * points local, and nothing that loads the shared library needs it. */
int shc_online_keep_stats(size_t *in_use, size_t *peak);

/* How many compiles the in-process compiler has run (a cache hit is not one),
 * wrapping: read around a frame, it says whether the frame compiled. Static
 * archive only, like shc_online_keep_stats. */
uint32_t shc_online_compiles(void);

/* The in-process compiler's keep region, lent by the image before the first
 * compile (on Zephyr, zephyr_shaderc_scratch.c, sized by
 * CONFIG_YAKOGL_SHADERC_KEEP_BYTES). buf 8-byte aligned, len at least 64 KiB.
 * Returns 0, -EINVAL, or -EBUSY when the region in use already keeps a shader.
 * Static archive only, like shc_online_keep_stats. */
int shc_online_lend_keep(unsigned char *buf, size_t len);

#ifdef __cplusplus
}
#endif

#endif /* GLES_SHADERC_H */
