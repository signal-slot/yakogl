attribute vec4 dEQP_Position;
attribute vec4 a_d;
attribute float a_g;
attribute float a_h;
varying mediump vec4 d;
varying mediump float g;
varying mediump float h;

void main()
{
	gl_Position = dEQP_Position;
	d = a_d;
	g = a_g;
	h = a_h;
}
