attribute vec4 dEQP_Position;
attribute float a_g;
varying mediump float g;

void main()
{
	gl_Position = dEQP_Position;
	g = a_g;
}
