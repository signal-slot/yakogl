attribute vec4 dEQP_Position;
attribute vec4 a_e;
attribute vec4 a_a;
attribute vec4 a_b;
varying mediump vec4 e;
varying mediump vec4 a;
varying mediump vec4 b;

void main()
{
	gl_Position = dEQP_Position;
	e = a_e;
	a = a_a;
	b = a_b;
}
