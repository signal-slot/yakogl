#version 100
/* dEQP-GLES2.functional.attribute_location.bind_aliasing.max_cond_vec3: the vertex shader, as dEQP's
 * generateVertexShaderTemplate / BindMaxAliasingAttributeTest write it (glsAttributeLocationTests.cpp,
 * VK-GL-CTS 93bca018, Apache-2.0), with the vertex shader's mediump made highp: its arithmetic is
 * fp32, so the words check the attribute layout at the link's locations alone (#11388); the mediump
 * vertex arithmetic of the case itself is #4536's. Drawn with the bindings of the .bind file: a_0 at 15, a_16 at 15, a_1 at 14, a_17 at 14, a_2 at 13, a_18 at 13, a_3 at 12, a_19 at 12, a_4 at 11, a_20 at 11, a_5 at 10, a_21 at 10, a_6 at 9, a_22 at 9, a_7 at 8, a_23 at 8, a_8 at 7, a_24 at 7, a_9 at 6, a_25 at 6, a_10 at 5, a_26 at 5, a_11 at 4, a_27 at 4, a_12 at 3, a_28 at 3, a_13 at 2, a_29 at 2, a_14 at 1, a_30 at 1, a_15 at 0, a_31 at 0. */
varying highp vec4 v_color;
attribute highp vec3 a_0;
attribute highp vec3 a_16;
attribute highp vec3 a_1;
attribute highp vec3 a_17;
attribute highp vec3 a_2;
attribute highp vec3 a_18;
attribute highp vec3 a_3;
attribute highp vec3 a_19;
attribute highp vec3 a_4;
attribute highp vec3 a_20;
attribute highp vec3 a_5;
attribute highp vec3 a_21;
attribute highp vec3 a_6;
attribute highp vec3 a_22;
attribute highp vec3 a_7;
attribute highp vec3 a_23;
attribute highp vec3 a_8;
attribute highp vec3 a_24;
attribute highp vec3 a_9;
attribute highp vec3 a_25;
attribute highp vec3 a_10;
attribute highp vec3 a_26;
attribute highp vec3 a_11;
attribute highp vec3 a_27;
attribute highp vec3 a_12;
attribute highp vec3 a_28;
attribute highp vec3 a_13;
attribute highp vec3 a_29;
attribute highp vec3 a_14;
attribute highp vec3 a_30;
attribute highp vec3 a_15;
attribute highp vec3 a_31;

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
		color += vec4(a_16.xyz, a_16.x);
	}
	if (u_A != 0.0)
	{
		color += vec4(a_1.xyz, a_1.x);
	}
	if (u_A == 0.0)
	{
		color += vec4(a_17.xyz, a_17.x);
	}
	if (u_A != 0.0)
	{
		color += vec4(a_2.xyz, a_2.x);
	}
	if (u_A == 0.0)
	{
		color += vec4(a_18.xyz, a_18.x);
	}
	if (u_A != 0.0)
	{
		color += vec4(a_3.xyz, a_3.x);
	}
	if (u_A == 0.0)
	{
		color += vec4(a_19.xyz, a_19.x);
	}
	if (u_A != 0.0)
	{
		color += vec4(a_4.xyz, a_4.x);
	}
	if (u_A == 0.0)
	{
		color += vec4(a_20.xyz, a_20.x);
	}
	if (u_A != 0.0)
	{
		color += vec4(a_5.xyz, a_5.x);
	}
	if (u_A == 0.0)
	{
		color += vec4(a_21.xyz, a_21.x);
	}
	if (u_A != 0.0)
	{
		color += vec4(a_6.xyz, a_6.x);
	}
	if (u_A == 0.0)
	{
		color += vec4(a_22.xyz, a_22.x);
	}
	if (u_A != 0.0)
	{
		color += vec4(a_7.xyz, a_7.x);
	}
	if (u_A == 0.0)
	{
		color += vec4(a_23.xyz, a_23.x);
	}
	if (u_A != 0.0)
	{
		color += vec4(a_8.xyz, a_8.x);
	}
	if (u_A == 0.0)
	{
		color += vec4(a_24.xyz, a_24.x);
	}
	if (u_A != 0.0)
	{
		color += vec4(a_9.xyz, a_9.x);
	}
	if (u_A == 0.0)
	{
		color += vec4(a_25.xyz, a_25.x);
	}
	if (u_A != 0.0)
	{
		color += vec4(a_10.xyz, a_10.x);
	}
	if (u_A == 0.0)
	{
		color += vec4(a_26.xyz, a_26.x);
	}
	if (u_A != 0.0)
	{
		color += vec4(a_11.xyz, a_11.x);
	}
	if (u_A == 0.0)
	{
		color += vec4(a_27.xyz, a_27.x);
	}
	if (u_A != 0.0)
	{
		color += vec4(a_12.xyz, a_12.x);
	}
	if (u_A == 0.0)
	{
		color += vec4(a_28.xyz, a_28.x);
	}
	if (u_A != 0.0)
	{
		color += vec4(a_13.xyz, a_13.x);
	}
	if (u_A == 0.0)
	{
		color += vec4(a_29.xyz, a_29.x);
	}
	if (u_A != 0.0)
	{
		color += vec4(a_14.xyz, a_14.x);
	}
	if (u_A == 0.0)
	{
		color += vec4(a_30.xyz, a_30.x);
	}
	if (u_A != 0.0)
	{
		color += vec4(a_15.xyz, a_15.x);
	}
	if (u_A == 0.0)
	{
		color += vec4(a_31.xyz, a_31.x);
	}

	v_color = color;
	gl_Position = color;
}
