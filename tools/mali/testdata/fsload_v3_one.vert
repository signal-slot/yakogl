/* Load width (v3_one's vertex shader): the varyings it writes, each from
 * an attribute. */
attribute vec4 a_pos;
attribute vec4 a_t;
varying vec3 t;
void main()
{
	gl_Position = a_pos;
	t = a_t.xyz;
}
