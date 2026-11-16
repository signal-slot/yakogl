/* Load width (v2_one's vertex shader): the varyings it writes, each from
 * an attribute. */
attribute vec4 a_pos;
attribute vec4 a_v;
varying vec2 v;
void main()
{
	gl_Position = a_pos;
	v = a_v.zw;
}
