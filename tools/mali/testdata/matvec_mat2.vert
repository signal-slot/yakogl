/* A mat2 varying built from ONE vector: the four components in
 * column-major order, as mat2(a_a.x, a_a.y, a_a.z, a_a.w). */
attribute vec4 a_pos;
attribute vec4 a_a;
varying mat2 m;
void main()
{
	gl_Position = a_pos;
	m = mat2(a_a);
}
