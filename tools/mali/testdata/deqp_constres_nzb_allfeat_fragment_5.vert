attribute vec4 dEQP_Position;
attribute float a_d;
attribute vec2 a_h;
attribute vec4 a_n;
attribute vec3 a_s;
attribute float a_y;
attribute vec2 a_x;
attribute vec4 a_v;
varying mediump float d;
varying mediump vec2 h;
varying mediump vec4 n;
varying mediump vec3 s;
varying mediump float y;
varying mediump vec2 x;
varying mediump vec4 v;

void main()
{
	gl_Position = dEQP_Position;
	d = a_d;
	h = a_h;
	n = a_n;
	s = a_s;
	y = a_y;
	x = a_x;
	v = a_v;
}
