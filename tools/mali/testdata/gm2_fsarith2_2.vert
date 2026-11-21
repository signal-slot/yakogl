/* Vertex shader of gm2_fsarith2_2: a position and a 2D varying. */
attribute highp vec4 a_pos;
attribute highp vec2 a_uv;
varying mediump vec2 uv;
void main()
{
	gl_Position = a_pos;
	uv = a_uv;
}
