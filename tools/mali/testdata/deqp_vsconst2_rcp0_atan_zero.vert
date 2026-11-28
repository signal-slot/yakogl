attribute vec4 dEQP_Position;
attribute float a_f;
attribute float a_g;
varying mediump float f;
varying mediump float g;

void main()
{
	gl_Position = dEQP_Position;
	f = a_f;
	g = a_g;
}
