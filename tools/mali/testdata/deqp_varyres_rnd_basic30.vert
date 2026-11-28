attribute vec4 dEQP_Position;
attribute float a_a;
attribute float a_b;
attribute vec4 a_d;
attribute float a_e;
attribute float a_f;
attribute float a_i;
attribute float a_n;
varying mediump float a;
varying mediump float b;
varying mediump vec4 d;
varying mediump float e;
varying mediump float f;
varying mediump float i;
varying mediump float n;

void main()
{
	gl_Position = dEQP_Position;
	a = a_a;
	b = a_b;
	d = a_d;
	e = a_e;
	f = a_f;
	i = a_i;
	n = a_n;
}
