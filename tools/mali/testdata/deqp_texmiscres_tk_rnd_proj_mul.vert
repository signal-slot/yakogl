attribute vec4 dEQP_Position;
attribute vec4 a_n;
attribute vec4 a_m;
attribute vec3 a_b;
attribute float a_d;
attribute float a_l;
varying mediump vec4 n;
varying mediump vec4 m;
varying mediump vec3 b;
varying mediump float d;
varying mediump float l;

void main()
{
	gl_Position = dEQP_Position;
	n = a_n;
	m = a_m;
	b = a_b;
	d = a_d;
	l = a_l;
}
