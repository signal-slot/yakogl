#version 100
/* dEQP-GLES2.functional.attribute_location.bind_aliasing.cond_vec3: the vertex shader, as dEQP's
 * generateVertexShaderTemplate / BindAliasingAttributeTest write it (glsAttributeLocationTests.cpp,
 * VK-GL-CTS 93bca018, Apache-2.0), with the vertex shader's mediump made highp: its arithmetic is
 * fp32, so the words check the attribute layout at the link's locations alone (#11388); the mediump
 * vertex arithmetic of the case itself is #4536's. Drawn with the bindings of the .bind file: a_0 at 1, a_1 at 1. */
varying highp vec4 v_color;
attribute highp vec3 a_0;
attribute highp vec4 a_1;

uniform highp float u_A;

void main (void)
{
	highp vec4 color = vec4(0.0);

	if (u_A != 0.0)
	{
		color += vec4(a_0.xyz, a_0.x);
	}
	if (u_A == 0.0)
	{
		color += vec4(a_1);
	}

	v_color = color;
	gl_Position = color;
}
