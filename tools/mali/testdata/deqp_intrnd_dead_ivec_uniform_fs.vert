attribute vec4 dEQP_Position;
attribute float a_a;
attribute float a_b;
attribute float a_o;
varying mediump float a;
varying mediump float b;
varying mediump float o;

void main()
{
	gl_Position = dEQP_Position;
	a = a_a;
	b = a_b;
	o = a_o;
}
