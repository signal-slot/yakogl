#version 100
/* dEQP-GLES2.functional.attribute_location.bind_aliasing.max_cond_float: the vertex shader, verbatim from the .qpa of
 * tools/mali/deqp-host-measure.sh (VK-GL-CTS 93bca018, Apache-2.0;
 * HOST-ONLY run at f9b37651). Drawn with the bindings of the .bind file.
 * Defect `encode` of docs/notes/mali-deqp-defects.md. */
varying mediump vec4 v_color;
attribute mediump float a_0;
attribute mediump float a_16;
attribute mediump float a_1;
attribute mediump float a_17;
attribute mediump float a_2;
attribute mediump float a_18;
attribute mediump float a_3;
attribute mediump float a_19;
attribute mediump float a_4;
attribute mediump float a_20;
attribute mediump float a_5;
attribute mediump float a_21;
attribute mediump float a_6;
attribute mediump float a_22;
attribute mediump float a_7;
attribute mediump float a_23;
attribute mediump float a_8;
attribute mediump float a_24;
attribute mediump float a_9;
attribute mediump float a_25;
attribute mediump float a_10;
attribute mediump float a_26;
attribute mediump float a_11;
attribute mediump float a_27;
attribute mediump float a_12;
attribute mediump float a_28;
attribute mediump float a_13;
attribute mediump float a_29;
attribute mediump float a_14;
attribute mediump float a_30;
attribute mediump float a_15;
attribute mediump float a_31;

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
