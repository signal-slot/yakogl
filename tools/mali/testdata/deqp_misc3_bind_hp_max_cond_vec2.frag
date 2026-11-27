#version 100
/* dEQP-GLES2.functional.attribute_location.bind_aliasing.max_cond_vec2: the fragment shader, as dEQP's
 * generateVertexShaderTemplate / BindMaxAliasingAttributeTest write it (glsAttributeLocationTests.cpp,
 * VK-GL-CTS 93bca018, Apache-2.0), with the vertex shader's mediump made highp: its arithmetic is
 * fp32, so the words check the attribute layout at the link's locations alone (#11388); the mediump
 * vertex arithmetic of the case itself is #4536's. Drawn with the bindings of the .bind file: a_0 at 15, a_16 at 15, a_1 at 14, a_17 at 14, a_2 at 13, a_18 at 13, a_3 at 12, a_19 at 12, a_4 at 11, a_20 at 11, a_5 at 10, a_21 at 10, a_6 at 9, a_22 at 9, a_7 at 8, a_23 at 8, a_8 at 7, a_24 at 7, a_9 at 6, a_25 at 6, a_10 at 5, a_26 at 5, a_11 at 4, a_27 at 4, a_12 at 3, a_28 at 3, a_13 at 2, a_29 at 2, a_14 at 1, a_30 at 1, a_15 at 0, a_31 at 0. */

varying mediump vec4 v_color;
void main (void)
{
	gl_FragColor = v_color;
}
