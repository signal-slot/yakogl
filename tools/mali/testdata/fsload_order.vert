/* One vec4 varying from an attribute (the vertex shader of fsload_order
 * and fsload_disc_col). */
attribute vec4 a_pos;
attribute vec4 a_a;
varying vec4 a;
void main()
{
	gl_Position = a_pos;
	a = a_a;
}
