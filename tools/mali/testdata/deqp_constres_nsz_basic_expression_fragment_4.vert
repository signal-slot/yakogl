attribute vec4 dEQP_Position;
attribute float a_b;
attribute float a_e;
attribute vec4 a_i;
attribute float a_m;
attribute vec4 a_s;
attribute float a_u;
varying mediump float b;
varying mediump float e;
varying mediump vec4 i;
varying mediump float m;
varying mediump vec4 s;
varying mediump float u;

void main()
{
	gl_Position = dEQP_Position;
	b = a_b;
	e = a_e;
	i = a_i;
	m = a_m;
	s = a_s;
	u = a_u;
}
