/* Vertex shader of fsvec_rcp_mediump. */
attribute vec4 a_pos;
attribute vec2 a_v;
varying vec2 v;
void main()
{
	gl_Position = a_pos;
	v = a_v;
}
