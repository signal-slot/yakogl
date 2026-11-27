#version 100
/* dEQP-GLES2.functional.attribute_location.bind_aliasing.max_cond_vec2: the vertex shader, as dEQP's
 * generateVertexShaderTemplate / BindMaxAliasingAttributeTest write it (glsAttributeLocationTests.cpp,
 * VK-GL-CTS 93bca018, Apache-2.0), with the vertex shader's mediump made highp: its arithmetic is
 * fp32, so the words check the attribute layout at the link's locations alone (#11388); the mediump
 * vertex arithmetic of the case itself is #4536's. Drawn with the bindings of the .bind file: a_0 at 15, a_16 at 15, a_1 at 14, a_17 at 14, a_2 at 13, a_18 at 13, a_3 at 12, a_19 at 12, a_4 at 11, a_20 at 11, a_5 at 10, a_21 at 10, a_6 at 9, a_22 at 9, a_7 at 8, a_23 at 8, a_8 at 7, a_24 at 7, a_9 at 6, a_25 at 6, a_10 at 5, a_26 at 5, a_11 at 4, a_27 at 4, a_12 at 3, a_28 at 3, a_13 at 2, a_29 at 2, a_14 at 1, a_30 at 1, a_15 at 0, a_31 at 0. */
varying highp vec4 v_color;
attribute highp vec2 a_0;
attribute highp vec2 a_16;
attribute highp vec2 a_1;
attribute highp vec2 a_17;
attribute highp vec2 a_2;
attribute highp vec2 a_18;
attribute highp vec2 a_3;
attribute highp vec2 a_19;
attribute highp vec2 a_4;
attribute highp vec2 a_20;
attribute highp vec2 a_5;
attribute highp vec2 a_21;
attribute highp vec2 a_6;
attribute highp vec2 a_22;
attribute highp vec2 a_7;
attribute highp vec2 a_23;
attribute highp vec2 a_8;
attribute highp vec2 a_24;
attribute highp vec2 a_9;
attribute highp vec2 a_25;
attribute highp vec2 a_10;
attribute highp vec2 a_26;
attribute highp vec2 a_11;
attribute highp vec2 a_27;
attribute highp vec2 a_12;
attribute highp vec2 a_28;
attribute highp vec2 a_13;
attribute highp vec2 a_29;
attribute highp vec2 a_14;
attribute highp vec2 a_30;
attribute highp vec2 a_15;
attribute highp vec2 a_31;

uniform highp float u_A;

void main (void)
{
	highp vec4 color = vec4(0.0);

	if (u_A != 0.0)
	{
		color += vec4(a_0.xy, a_0.yx);
	}
	if (u_A == 0.0)
	{
		color += vec4(a_16.xy, a_16.yx);
	}
	if (u_A != 0.0)
	{
		color += vec4(a_1.xy, a_1.yx);
	}
	if (u_A == 0.0)
	{
		color += vec4(a_17.xy, a_17.yx);
	}
	if (u_A != 0.0)
	{
		color += vec4(a_2.xy, a_2.yx);
	}
	if (u_A == 0.0)
	{
		color += vec4(a_18.xy, a_18.yx);
	}
	if (u_A != 0.0)
	{
		color += vec4(a_3.xy, a_3.yx);
	}
	if (u_A == 0.0)
	{
		color += vec4(a_19.xy, a_19.yx);
	}
	if (u_A != 0.0)
	{
		color += vec4(a_4.xy, a_4.yx);
	}
	if (u_A == 0.0)
	{
		color += vec4(a_20.xy, a_20.yx);
	}
	if (u_A != 0.0)
	{
		color += vec4(a_5.xy, a_5.yx);
	}
	if (u_A == 0.0)
	{
		color += vec4(a_21.xy, a_21.yx);
	}
	if (u_A != 0.0)
	{
		color += vec4(a_6.xy, a_6.yx);
	}
	if (u_A == 0.0)
	{
		color += vec4(a_22.xy, a_22.yx);
	}
	if (u_A != 0.0)
	{
		color += vec4(a_7.xy, a_7.yx);
	}
	if (u_A == 0.0)
	{
		color += vec4(a_23.xy, a_23.yx);
	}
	if (u_A != 0.0)
	{
		color += vec4(a_8.xy, a_8.yx);
	}
	if (u_A == 0.0)
	{
		color += vec4(a_24.xy, a_24.yx);
	}
	if (u_A != 0.0)
	{
		color += vec4(a_9.xy, a_9.yx);
	}
	if (u_A == 0.0)
	{
		color += vec4(a_25.xy, a_25.yx);
	}
	if (u_A != 0.0)
	{
		color += vec4(a_10.xy, a_10.yx);
	}
	if (u_A == 0.0)
	{
		color += vec4(a_26.xy, a_26.yx);
	}
	if (u_A != 0.0)
	{
		color += vec4(a_11.xy, a_11.yx);
	}
	if (u_A == 0.0)
	{
		color += vec4(a_27.xy, a_27.yx);
	}
	if (u_A != 0.0)
	{
		color += vec4(a_12.xy, a_12.yx);
	}
	if (u_A == 0.0)
	{
		color += vec4(a_28.xy, a_28.yx);
	}
	if (u_A != 0.0)
	{
		color += vec4(a_13.xy, a_13.yx);
	}
	if (u_A == 0.0)
	{
		color += vec4(a_29.xy, a_29.yx);
	}
	if (u_A != 0.0)
	{
		color += vec4(a_14.xy, a_14.yx);
	}
	if (u_A == 0.0)
	{
		color += vec4(a_30.xy, a_30.yx);
	}
	if (u_A != 0.0)
	{
		color += vec4(a_15.xy, a_15.yx);
	}
	if (u_A == 0.0)
	{
		color += vec4(a_31.xy, a_31.yx);
	}

	v_color = color;
	gl_Position = color;
}
