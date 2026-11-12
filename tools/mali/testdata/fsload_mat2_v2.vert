/* A vec2 declared before a mat2 (the vertex shader of fsload_mat2_v2). */
attribute vec4 a_pos;
attribute vec4 a_a;
attribute vec4 a_b;
varying vec2 t;
varying mat2 m;
void main()
{
	gl_Position = a_pos;
	t = a_b.yz;
	m = mat2(a_a.xy, a_a.zw);
}
