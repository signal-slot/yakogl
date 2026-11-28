attribute vec4 dEQP_Position;
attribute vec4 a_g;
attribute float a_f;
attribute float a_e;
attribute float a_c;
varying mediump vec4 g;
varying mediump float f;
varying mediump float e;
varying mediump float c;

void main()
{
	gl_Position = dEQP_Position;
	g = a_g;
	f = a_f;
	e = a_e;
	c = a_c;
}
