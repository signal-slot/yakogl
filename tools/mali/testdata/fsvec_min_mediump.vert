/* Vertex shader of fsvec_min_mediump. */
attribute vec4 a_pos;
attribute vec4 a_v;
varying vec4 v;
void main()
{
	gl_Position = a_pos;
	v = a_v;
}
