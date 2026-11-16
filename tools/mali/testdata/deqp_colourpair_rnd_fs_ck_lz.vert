attribute vec4 dEQP_Position;
attribute float a_c;
attribute float a_b;
varying mediump float c;
varying mediump float b;

void main()
{
	gl_Position = dEQP_Position;
	c = a_c;
	b = a_b;
}
