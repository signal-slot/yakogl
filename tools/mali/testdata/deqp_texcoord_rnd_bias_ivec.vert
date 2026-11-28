attribute vec4 dEQP_Position;
attribute float a_b;
attribute float a_g;
varying mediump float b;
varying mediump float g;

void main()
{
	gl_Position = dEQP_Position;
	b = a_b;
	g = a_g;
}
