/* Load width (second's vertex shader): the varyings it writes, each from
 * an attribute. */
attribute vec4 a_pos;
attribute vec4 a_a;
attribute vec4 a_b;
varying vec4 a;
varying vec4 b;
void main()
{
	gl_Position = a_pos;
	a = a_a;
	b = a_b;
}
