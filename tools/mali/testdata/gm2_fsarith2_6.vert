/* Vertex shader of gm2_fsarith2_6: a position and a 3D varying. */
attribute highp vec4 a_pos;
attribute highp vec3 a_v;
varying mediump vec3 v;
void main()
{
	gl_Position = a_pos;
	v = a_v;
}
