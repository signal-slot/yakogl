/* Vertex shader of gm2_fsarith_7: a colour and a light-space coordinate. */
attribute vec4 a_pos;
attribute vec4 a_c;
varying vec4 col;
varying vec4 sc;
void main()
{
	gl_Position = a_pos;
	col = a_c;
	sc = a_pos * 0.5 + a_c;
}
