attribute vec4 dEQP_Position;
attribute vec4 a_a;
attribute float a_f;
attribute float a_g;
varying mediump vec4 a;
varying mediump float f;
varying mediump float g;

void main()
{
	gl_Position = dEQP_Position;
	a = a_a;
	f = a_f;
	g = a_g;
}
