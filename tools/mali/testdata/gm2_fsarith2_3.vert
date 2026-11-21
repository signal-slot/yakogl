/* Vertex shader of gm2_fsarith2_3: a position and a 3D varying. */
attribute highp vec4 a_pos;
attribute highp vec3 a_d;
varying mediump vec3 d;
void main()
{
	gl_Position = a_pos;
	d = a_d;
}
