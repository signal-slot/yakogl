attribute vec4 dEQP_Position;
attribute float a_d;
attribute float a_a;
attribute float a_b;
varying mediump float d;
varying mediump float a;
varying mediump float b;

void main()
{
	gl_Position = dEQP_Position;
	d = a_d;
	a = a_a;
	b = a_b;
}
