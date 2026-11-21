/* Vertex shader of gm2_fsarith2_1: a position and a varying vec4 made from both attributes. */
attribute highp vec4 a_pos;
attribute highp vec4 a_c;
varying mediump vec4 sc;
void main()
{
	gl_Position = a_pos;
	sc = a_pos * 0.5 + a_c;
}
