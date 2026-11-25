attribute vec4 dEQP_Position;
attribute float a_a;
attribute float a_d;
varying mediump float a;
varying mediump float d;

void main()
{
	gl_Position = dEQP_Position;
	a = a_a;
	d = a_d;
}
