/* Vertex shader of gm2_nbshrf_7. */
attribute highp vec4 a_pos;
attribute highp vec4 a_c;
varying vec4 sc;
void main()
{
	gl_Position = a_pos;
	sc = a_pos * 0.5 + a_c;
}
