attribute vec4 dEQP_Position;
attribute float a_b;
attribute float a_e;
attribute float a_i;
varying mediump float b;
varying mediump float e;
varying mediump float i;

void main()
{
	gl_Position = dEQP_Position;
	b = a_b;
	e = a_e;
	i = a_i;
}
