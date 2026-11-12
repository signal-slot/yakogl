/* Load width (discard's vertex shader): the varyings it writes, each from
 * an attribute. */
attribute vec4 a_pos;
attribute vec4 a_a;
varying vec4 a;
void main()
{
	gl_Position = a_pos;
	a = a_a;
}
