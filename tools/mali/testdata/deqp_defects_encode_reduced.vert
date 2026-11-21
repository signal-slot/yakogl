/* max_cond_float of dEQP-GLES2.functional.attribute_location.bind_aliasing
 * (glsAttributeLocationTests.cpp, VK-GL-CTS 93bca018) cut to 18 attributes
 * on 9 locations: 18 read attributes are past LD_ATTR_IMM's 16 records
 * when the compile numbers them without the link's bindings. Drawn with
 * the bindings of the .bind file. Defect `encode` of
 * docs/notes/mali-deqp-defects.md. */
varying mediump vec4 v_color;
attribute mediump float a_0;
attribute mediump float a_9;
attribute mediump float a_1;
attribute mediump float a_10;
attribute mediump float a_2;
attribute mediump float a_11;
attribute mediump float a_3;
attribute mediump float a_12;
attribute mediump float a_4;
attribute mediump float a_13;
attribute mediump float a_5;
attribute mediump float a_14;
attribute mediump float a_6;
attribute mediump float a_15;
attribute mediump float a_7;
attribute mediump float a_16;
attribute mediump float a_8;
attribute mediump float a_17;

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
		color += vec4(a_9);
	}
	if (u_A != 0.0)
	{
		color += vec4(a_1);
	}
	if (u_A == 0.0)
	{
		color += vec4(a_10);
	}
	if (u_A != 0.0)
	{
		color += vec4(a_2);
	}
	if (u_A == 0.0)
	{
		color += vec4(a_11);
	}
	if (u_A != 0.0)
	{
		color += vec4(a_3);
	}
	if (u_A == 0.0)
	{
		color += vec4(a_12);
	}
	if (u_A != 0.0)
	{
		color += vec4(a_4);
	}
	if (u_A == 0.0)
	{
		color += vec4(a_13);
	}
	if (u_A != 0.0)
	{
		color += vec4(a_5);
	}
	if (u_A == 0.0)
	{
		color += vec4(a_14);
	}
	if (u_A != 0.0)
	{
		color += vec4(a_6);
	}
	if (u_A == 0.0)
	{
		color += vec4(a_15);
	}
	if (u_A != 0.0)
	{
		color += vec4(a_7);
	}
	if (u_A == 0.0)
	{
		color += vec4(a_16);
	}
	if (u_A != 0.0)
	{
		color += vec4(a_8);
	}
	if (u_A == 0.0)
	{
		color += vec4(a_17);
	}

	v_color = color;
	gl_Position = color;
}
