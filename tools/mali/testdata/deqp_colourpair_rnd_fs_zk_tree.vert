attribute vec4 dEQP_Position;
attribute float a_c;
attribute vec4 a_a;
varying mediump float c;
varying mediump vec4 a;

void main()
{
	gl_Position = dEQP_Position;
	c = a_c;
	a = a_a;
}
