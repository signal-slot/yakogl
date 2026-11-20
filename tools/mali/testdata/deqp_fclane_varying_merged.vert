#version 100
varying mediump vec4 v_color;
attribute mediump vec3 a_0;


void main (void)
{
	mediump vec4 color = vec4(0.0);

	color += vec4(a_0.xyz, a_0.x);

	v_color = color;
	gl_Position = color;
}
