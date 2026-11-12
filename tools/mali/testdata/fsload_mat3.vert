/* A mat3 varying whose second column is attribute 1 (the vertex shader
 * of fsload_mat3). */
attribute vec4 a_pos;
attribute vec4 a_a;
attribute vec4 a_b;
varying mat3 m;
void main()
{
	gl_Position = a_pos;
	m = mat3(a_b.xyz, a_a.xyz, a_b.yzw);
}
