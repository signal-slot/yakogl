/* SPDX-License-Identifier: Apache-2.0 */
/*
 * The vertex and pixel shader the bring-up triangle uses, and the reflection
 * the driver needs to program the pipeline around them.
 *
 * Generated on the host with etna_shc (yakogl/tools/vivante/README.md) for
 * model 0x7000 revision 0x6204, from:
 *
 *   VS: layout(location=0) in vec4 aPos; layout(location=1) in vec4 aColor;
 *       layout(location=0) out vec4 vColor;
 *       void main() { gl_Position = aPos; vColor = aColor; }
 *   PS: layout(location=0) in vec4 vColor; layout(location=0) out vec4 oColor;
 *       void main() { oColor = vColor; }
 *
 *       glslangValidator -G tri.vert -o tri.vert.spv
 *       etna_shc vert tri.vert.spv
 *
 * Both compile to a single NOP instruction. That is not a mistake and not a
 * degenerate case: the hardware loads vertex attributes straight into the
 * shader's temporary registers and reads the outputs back out of them, so a
 * pass-through needs no arithmetic at all. The whole program is the register
 * mapping below -- which is exactly what makes this the right shader pair to
 * bring the pipeline up with. If the triangle comes out wrong, the shader
 * cannot be the reason.
 *
 * Gouraud shading needs no change here: per-vertex colours in attribute 1 and
 * smooth interpolation in GL_HALTI5_SHADER_ATTRIBUTES do it.
 */
#ifndef GC_SHADERS_H
#define GC_SHADERS_H

#include <stdint.h>

/* One instruction = four dwords. */
static const uint32_t gc_tri_vs_code[] = {
	0x00000000, 0x00000000, 0x00000000, 0x00000000,
};

static const uint32_t gc_tri_ps_code[] = {
	0x00000000, 0x00000000, 0x00000000, 0x00000000,
};

/* etna_shc's reflection, field by field. */
#define GC_TRI_VS_NUM_TEMPS        2u
#define GC_TRI_VS_POS_OUT_REG      0u   /* gl_Position comes out of r0 */
#define GC_TRI_VS_VARYING0_REG     1u   /* the colour varying out of r1 */
#define GC_TRI_VS_INPUT0_REG       0u   /* aPos lands in r0 */
#define GC_TRI_VS_INPUT1_REG       1u   /* aColor lands in r1 */
#define GC_TRI_VS_INPUT_COUNT_UNK8 1u
#define GC_TRI_VS_LOAD_BALANCING   0x0f3f0241u

#define GC_TRI_PS_NUM_TEMPS        2u
#define GC_TRI_PS_COLOR_OUT_REG    1u   /* the interpolated varying is r1 */
#define GC_TRI_PS_INPUT_COUNT_UNK8 31u

/* Shared shape: two vec4 attributes, one vec4 varying. */
#define GC_TRI_NUM_ATTRIBUTES      2u
#define GC_TRI_NUM_VARYINGS        1u
#define GC_TRI_VARYING0_COMPONENTS 4u

#endif /* GC_SHADERS_H */
