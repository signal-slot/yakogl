/* Vertex shader of gm2_nbshrf_2. */
attribute highp vec4 a_pos;
attribute highp vec4 a_c;
varying vec4 sc;
varying vec4 col;
void main()
{
	gl_Position = a_pos;
	sc = a_pos * 0.5 + a_c;
	col = a_c;
}
