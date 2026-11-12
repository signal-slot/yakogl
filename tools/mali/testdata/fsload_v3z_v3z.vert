/* Two vec3 varyings sharing a slot (the vertex shader of fsload_v3x_v3z,
 * fsload_v3z_v3z and fsload_v3x_v3x): the link packs p.xyz,q.x in slot 0
 * and q.yz in slot 1. */
attribute vec4 a_pos;
attribute vec4 a_a;
attribute vec4 a_b;
varying vec3 p;
varying vec3 q;
void main()
{
	gl_Position = a_pos;
	p = a_a.xyz;
	q = a_b.yzw;
}
