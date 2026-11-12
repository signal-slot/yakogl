/* Vertex shader of the fsfold_* references. */
attribute vec4 a_pos;
attribute vec2 a_uv;
varying vec2 uv;
void main()
{
	gl_Position = a_pos;
	uv = a_uv;
}
