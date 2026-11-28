attribute vec4 dEQP_Position;
attribute float a_g;
attribute float a_r;
varying mediump float g;
varying mediump float r;

void main()
{
	gl_Position = dEQP_Position;
	g = a_g;
	r = a_r;
}
