/* Vertex shader of gm2_fsarith_3 to _5: an eye-space normal and position. */
attribute vec4 a_pos;
attribute vec3 a_n;
varying vec3 vn;
varying vec4 vp;
void main()
{
	gl_Position = a_pos;
	vn = a_n;
	vp = a_pos;
}
