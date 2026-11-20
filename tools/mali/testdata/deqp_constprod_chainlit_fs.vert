attribute vec4 dEQP_Position;
attribute vec4 a_c;
attribute float a_b;
varying mediump vec4 c;
varying mediump float b;

void main()
{
	gl_Position = dEQP_Position;
	c = a_c;
	b = a_b;
}
