/* A mat2 varying (the vertex shader of fsload_mat2 and fsload_hp_mat2). */
attribute vec4 a_pos;
attribute vec4 a_a;
attribute vec4 a_b;
varying mat2 m;
void main()
{
	gl_Position = a_pos;
	m = mat2(a_a.xy, a_b.zw);
}
