attribute vec4 dEQP_Position;
attribute float a_b;
attribute float a_a;
varying mediump float b;
varying mediump float a;

void main()
{
	gl_Position = dEQP_Position;
	b = a_b;
	a = a_a;
}
