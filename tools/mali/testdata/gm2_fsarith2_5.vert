/* Vertex shader of gm2_fsarith2_5: a position and two 3D varyings. */
attribute highp vec4 a_pos;
attribute highp vec3 a_n;
attribute highp vec3 a_h;
varying mediump vec3 n;
varying mediump vec3 h;
void main()
{
	gl_Position = a_pos;
	n = a_n;
	h = a_h;
}
