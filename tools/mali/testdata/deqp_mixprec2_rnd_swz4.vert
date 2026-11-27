attribute vec4 dEQP_Position;
attribute float a_b;
attribute float a_e;
attribute float a_f;
attribute float a_g;
attribute vec4 a_i;
varying mediump float b;
varying mediump float e;
varying mediump float f;
varying mediump float g;
varying mediump vec4 i;

void main()
{
	gl_Position = dEQP_Position;
	b = a_b;
	e = a_e;
	f = a_f;
	g = a_g;
	i = a_i;
}
