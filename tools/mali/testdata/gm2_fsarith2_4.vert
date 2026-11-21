/* Vertex shader of gm2_fsarith2_4: a position and a highp 3D varying. */
attribute highp vec4 a_pos;
attribute highp vec3 a_v;
varying highp vec3 lv;
void main()
{
	gl_Position = a_pos;
	lv = a_v;
}
