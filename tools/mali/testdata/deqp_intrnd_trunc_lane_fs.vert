attribute vec4 dEQP_Position;
attribute float a_u;
attribute float a_a;
attribute float a_b;
attribute float a_c;
attribute vec3 a_d;
attribute float a_q;
attribute float a_n;
varying mediump float u;
varying mediump float a;
varying mediump float b;
varying mediump float c;
varying mediump vec3 d;
varying mediump float q;
varying mediump float n;

void main()
{
	gl_Position = dEQP_Position;
	u = a_u;
	a = a_a;
	b = a_b;
	c = a_c;
	d = a_d;
	q = a_q;
	n = a_n;
}
