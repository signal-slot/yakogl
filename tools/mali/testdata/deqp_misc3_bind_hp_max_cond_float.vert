#version 100
/* dEQP-GLES2.functional.attribute_location.bind_aliasing.max_cond_float: the vertex shader, as dEQP's
 * generateVertexShaderTemplate / BindMaxAliasingAttributeTest write it (glsAttributeLocationTests.cpp,
 * VK-GL-CTS 93bca018, Apache-2.0), with the vertex shader's mediump made highp: its arithmetic is
 * fp32, so the words check the attribute layout at the link's locations alone (#11388); the mediump
 * vertex arithmetic of the case itself is #4536's. Drawn with the bindings of the .bind file: a_0 at 15, a_16 at 15, a_1 at 14, a_17 at 14, a_2 at 13, a_18 at 13, a_3 at 12, a_19 at 12, a_4 at 11, a_20 at 11, a_5 at 10, a_21 at 10, a_6 at 9, a_22 at 9, a_7 at 8, a_23 at 8, a_8 at 7, a_24 at 7, a_9 at 6, a_25 at 6, a_10 at 5, a_26 at 5, a_11 at 4, a_27 at 4, a_12 at 3, a_28 at 3, a_13 at 2, a_29 at 2, a_14 at 1, a_30 at 1, a_15 at 0, a_31 at 0. */
varying highp vec4 v_color;
attribute highp float a_0;
attribute highp float a_16;
attribute highp float a_1;
attribute highp float a_17;
attribute highp float a_2;
attribute highp float a_18;
attribute highp float a_3;
attribute highp float a_19;
attribute highp float a_4;
attribute highp float a_20;
attribute highp float a_5;
attribute highp float a_21;
attribute highp float a_6;
attribute highp float a_22;
attribute highp float a_7;
attribute highp float a_23;
attribute highp float a_8;
attribute highp float a_24;
attribute highp float a_9;
attribute highp float a_25;
attribute highp float a_10;
attribute highp float a_26;
attribute highp float a_11;
attribute highp float a_27;
attribute highp float a_12;
attribute highp float a_28;
attribute highp float a_13;
attribute highp float a_29;
attribute highp float a_14;
attribute highp float a_30;
attribute highp float a_15;
attribute highp float a_31;

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
	if (u_A != 0.0)
	{
		color += vec4(a_4);
	}
	if (u_A == 0.0)
	{
		color += vec4(a_20);
	}
	if (u_A != 0.0)
	{
		color += vec4(a_5);
	}
	if (u_A == 0.0)
	{
		color += vec4(a_21);
	}
	if (u_A != 0.0)
	{
		color += vec4(a_6);
	}
	if (u_A == 0.0)
	{
		color += vec4(a_22);
	}
	if (u_A != 0.0)
	{
		color += vec4(a_7);
	}
	if (u_A == 0.0)
	{
		color += vec4(a_23);
	}
	if (u_A != 0.0)
	{
		color += vec4(a_8);
	}
	if (u_A == 0.0)
	{
		color += vec4(a_24);
	}
	if (u_A != 0.0)
	{
		color += vec4(a_9);
	}
	if (u_A == 0.0)
	{
		color += vec4(a_25);
	}
	if (u_A != 0.0)
	{
		color += vec4(a_10);
	}
	if (u_A == 0.0)
	{
		color += vec4(a_26);
	}
	if (u_A != 0.0)
	{
		color += vec4(a_11);
	}
	if (u_A == 0.0)
	{
		color += vec4(a_27);
	}
	if (u_A != 0.0)
	{
		color += vec4(a_12);
	}
	if (u_A == 0.0)
	{
		color += vec4(a_28);
	}
	if (u_A != 0.0)
	{
		color += vec4(a_13);
	}
	if (u_A == 0.0)
	{
		color += vec4(a_29);
	}
	if (u_A != 0.0)
	{
		color += vec4(a_14);
	}
	if (u_A == 0.0)
	{
		color += vec4(a_30);
	}
	if (u_A != 0.0)
	{
		color += vec4(a_15);
	}
	if (u_A == 0.0)
	{
		color += vec4(a_31);
	}

	v_color = color;
	gl_Position = color;
}
