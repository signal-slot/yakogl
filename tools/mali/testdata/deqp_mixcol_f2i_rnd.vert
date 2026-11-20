attribute vec4 dEQP_Position;
attribute float a_b;
varying mediump float b;

void main()
{
	gl_Position = dEQP_Position;
	b = a_b;
}
