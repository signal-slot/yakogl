attribute vec4 dEQP_Position;
attribute vec4 a_g;
attribute vec4 a_a;
attribute vec4 a_b;
varying mediump vec4 g;
varying mediump vec4 a;
varying mediump vec4 b;

void main()
{
	gl_Position = dEQP_Position;
	g = a_g;
	a = a_a;
	b = a_b;
}
