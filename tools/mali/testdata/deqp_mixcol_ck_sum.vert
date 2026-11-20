attribute vec4 dEQP_Position;
attribute float a_a;
varying mediump float a;

void main()
{
	gl_Position = dEQP_Position;
	a = a_a;
}
