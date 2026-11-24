attribute vec4 dEQP_Position;
attribute vec4 a_e;
attribute float a_c;
varying mediump vec4 e;
varying mediump float c;

void main()
{
	gl_Position = dEQP_Position;
	e = a_e;
	c = a_c;
}
