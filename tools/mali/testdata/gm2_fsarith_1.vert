/* Vertex shader of gm2_fsarith_1 and _2: a normal and a texture coordinate. */
attribute vec4 a_pos;
attribute vec3 a_n;
attribute vec2 a_uv;
varying vec3 n;
varying vec2 uv;
void main()
{
	gl_Position = a_pos;
	n = a_n;
	uv = a_uv;
}
