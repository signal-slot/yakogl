/* Vertex shader of gm2_nbshrf_6. */
attribute highp vec4 a_pos;
attribute highp vec3 a_n;
varying vec3 vn;
varying vec4 vp;
void main()
{
	gl_Position = a_pos;
	vp = a_pos * 0.5;
	vn = a_n;
}
