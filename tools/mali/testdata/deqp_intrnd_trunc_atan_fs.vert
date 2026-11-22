attribute vec4 dEQP_Position;
attribute float a_g;
attribute float a_a;
attribute float a_b;
attribute float a_c;
attribute float a_e;
varying mediump float g;
varying mediump float a;
varying mediump float b;
varying mediump float c;
varying mediump float e;

void main()
{
	gl_Position = dEQP_Position;
	g = a_g;
	a = a_a;
	b = a_b;
	c = a_c;
	e = a_e;
}
