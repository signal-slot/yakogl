attribute vec4 dEQP_Position;
attribute float a_a;
attribute float a_b;
attribute float a_d;
attribute float a_j;
attribute float a_l;
attribute float a_m;
varying mediump float a;
varying mediump float b;
varying mediump float d;
varying mediump float j;
varying mediump float l;
varying mediump float m;

void main()
{
	gl_Position = dEQP_Position;
	a = a_a;
	b = a_b;
	d = a_d;
	j = a_j;
	l = a_l;
	m = a_m;
}
