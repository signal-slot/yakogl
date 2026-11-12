/* Load width (two's vertex shader): the varyings it writes, each from
 * an attribute. */
attribute vec4 a_pos;
attribute vec4 a_a;
attribute vec4 a_v;
varying vec4 a;
varying vec2 v;
void main()
{
	gl_Position = a_pos;
	a = a_a;
	v = a_v.zw;
}
