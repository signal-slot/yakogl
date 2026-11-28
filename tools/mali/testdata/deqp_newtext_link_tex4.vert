attribute vec4 dEQP_Position;
attribute float a_b;
attribute vec3 a_l;
attribute vec4 a_m;
attribute float a_q;
varying mediump float b;
varying mediump vec3 l;
varying mediump vec4 m;
varying mediump float q;

void main()
{
	gl_Position = dEQP_Position;
	b = a_b;
	l = a_l;
	m = a_m;
	q = a_q;
}
