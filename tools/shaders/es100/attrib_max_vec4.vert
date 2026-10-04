#version 100
/* dEQP-GLES2.functional.attribute_location, as VK-GL-CTS
 * modules/glshared/glsAttributeLocationTests.cpp generates it (Apache-2.0);
 * reproduced by v3d-glue (.claude/worktrees/deqp-attrib-repro/bind_max_attributes.vec4.vert). */
varying mediump vec4 v_color;
attribute mediump vec4 a_0;
attribute mediump vec4 a_1;
attribute mediump vec4 a_2;
attribute mediump vec4 a_3;
attribute mediump vec4 a_4;
attribute mediump vec4 a_5;
attribute mediump vec4 a_6;
attribute mediump vec4 a_7;
attribute mediump vec4 a_8;
attribute mediump vec4 a_9;
attribute mediump vec4 a_10;
attribute mediump vec4 a_11;
attribute mediump vec4 a_12;
attribute mediump vec4 a_13;
attribute mediump vec4 a_14;
attribute mediump vec4 a_15;


void main (void)
{
	mediump vec4 color = vec4(0.0);

	color += vec4(a_0);
	color += vec4(a_1);
	color += vec4(a_2);
	color += vec4(a_3);
	color += vec4(a_4);
	color += vec4(a_5);
	color += vec4(a_6);
	color += vec4(a_7);
	color += vec4(a_8);
	color += vec4(a_9);
	color += vec4(a_10);
	color += vec4(a_11);
	color += vec4(a_12);
	color += vec4(a_13);
	color += vec4(a_14);
	color += vec4(a_15);

	v_color = color;
	gl_Position = color;
}
