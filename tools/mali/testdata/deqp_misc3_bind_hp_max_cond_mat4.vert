#version 100
/* dEQP-GLES2.functional.attribute_location.bind_aliasing.max_cond_mat4: the vertex shader, as dEQP's
 * generateVertexShaderTemplate / BindMaxAliasingAttributeTest write it (glsAttributeLocationTests.cpp,
 * VK-GL-CTS 93bca018, Apache-2.0), with the vertex shader's mediump made highp: its arithmetic is
 * fp32, so the words check the attribute layout at the link's locations alone (#11388); the mediump
 * vertex arithmetic of the case itself is #4536's. Drawn with the bindings of the .bind file: a_0 at 12, a_16 at 12, a_1 at 8, a_17 at 8, a_2 at 4, a_18 at 4, a_3 at 0, a_19 at 0. */
varying highp vec4 v_color;
attribute highp mat4 a_0;
attribute highp mat4 a_16;
attribute highp mat4 a_1;
attribute highp mat4 a_17;
attribute highp mat4 a_2;
attribute highp mat4 a_18;
attribute highp mat4 a_3;
attribute highp mat4 a_19;

uniform highp float u_A;

void main (void)
{
	highp vec4 color = vec4(0.0);

	if (u_A != 0.0)
	{
		color += vec4(a_0);
	}
	if (u_A == 0.0)
	{
		color += vec4(a_16);
	}
	if (u_A != 0.0)
	{
		color += vec4(a_1);
	}
	if (u_A == 0.0)
	{
		color += vec4(a_17);
	}
	if (u_A != 0.0)
	{
		color += vec4(a_2);
	}
	if (u_A == 0.0)
	{
		color += vec4(a_18);
	}
	if (u_A != 0.0)
	{
		color += vec4(a_3);
	}
	if (u_A == 0.0)
	{
		color += vec4(a_19);
	}

	v_color = color;
	gl_Position = color;
}
