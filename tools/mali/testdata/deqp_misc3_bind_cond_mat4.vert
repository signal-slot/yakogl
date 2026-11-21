#version 100
/* dEQP-GLES2.functional.attribute_location.bind_aliasing.cond_mat4: the vertex shader, as dEQP's
 * generateVertexShaderTemplate / BindAliasingAttributeTest write it (glsAttributeLocationTests.cpp,
 * VK-GL-CTS 93bca018, Apache-2.0), same form as the verbatim max_cond_float pair of
 * deqp_defects_encode_max_cond_float. Reconstructed from the generator's code, not copied from a .qpa.
 * Drawn with the bindings of the .bind file: a_0 at 1, a_1 at 1. Plan #387 item board-baseline-rows. */
varying mediump vec4 v_color;
attribute mediump mat4 a_0;
attribute mediump vec4 a_1;

uniform mediump float u_A;

void main (void)
{
	mediump vec4 color = vec4(0.0);

	if (u_A != 0.0)
	{
		color += vec4(a_0);
	}
	if (u_A == 0.0)
	{
		color += vec4(a_1);
	}

	v_color = color;
	gl_Position = color;
}
