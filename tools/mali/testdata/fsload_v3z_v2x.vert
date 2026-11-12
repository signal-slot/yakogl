/* A vec3 declared before a vec2 (the vertex shader of fsload_v3z_v2x). */
attribute vec4 a_pos;
attribute vec4 a_a;
attribute vec4 a_b;
varying vec3 p;
varying vec2 q;
void main()
{
	gl_Position = a_pos;
	p = a_a.xyz;
	q = a_b.yz;
}
