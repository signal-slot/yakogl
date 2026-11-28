attribute vec4 dEQP_Position;
attribute vec4 a_n;
attribute vec2 a_b;
attribute float a_e;
attribute float a_l;
attribute float a_m;
varying mediump vec4 n;
varying mediump vec2 b;
varying mediump float e;
varying mediump float l;
varying mediump float m;

void main()
{
	gl_Position = dEQP_Position;
	n = a_n;
	b = a_b;
	e = a_e;
	l = a_l;
	m = a_m;
}
