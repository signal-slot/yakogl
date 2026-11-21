#version 100
/* dEQP-GLES2.functional.attribute_location.bind_aliasing.max_cond_mat4: the fragment shader, as dEQP's
 * generateVertexShaderTemplate / BindAliasingAttributeTest write it (glsAttributeLocationTests.cpp,
 * VK-GL-CTS 93bca018, Apache-2.0), same form as the verbatim max_cond_float pair of
 * deqp_defects_encode_max_cond_float. Reconstructed from the generator's code, not copied from a .qpa.
 * Drawn with the bindings of the .bind file: a_0 at 12, a_16 at 12, a_1 at 8, a_17 at 8, a_2 at 4, a_18 at 4, a_3 at 0, a_19 at 0. Plan #387 item board-baseline-rows. */

varying mediump vec4 v_color;
void main (void)
{
	gl_FragColor = v_color;
}
