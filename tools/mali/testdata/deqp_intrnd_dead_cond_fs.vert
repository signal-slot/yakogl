attribute vec4 dEQP_Position;
attribute float a_a;
attribute float a_c;
attribute float a_e;
varying mediump float a;
varying mediump float c;
varying mediump float e;

void main()
{
	gl_Position = dEQP_Position;
	a = a_a;
	c = a_c;
	e = a_e;
}
