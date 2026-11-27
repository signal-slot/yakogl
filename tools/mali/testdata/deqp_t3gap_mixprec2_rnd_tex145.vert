attribute vec4 dEQP_Position;
attribute float a_h;
attribute float a_g;
attribute float a_c;
varying mediump float h;
varying mediump float g;
varying mediump float c;

void main()
{
	gl_Position = dEQP_Position;
	h = a_h;
	g = a_g;
	c = a_c;
}
