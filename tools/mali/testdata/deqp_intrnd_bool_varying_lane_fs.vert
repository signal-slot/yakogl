attribute vec4 dEQP_Position;
attribute float a_o;
attribute float a_m;
attribute float a_g;
attribute float a_f;
attribute float a_c;
varying mediump float o;
varying mediump float m;
varying mediump float g;
varying mediump float f;
varying mediump float c;

void main()
{
	gl_Position = dEQP_Position;
	o = a_o;
	m = a_m;
	g = a_g;
	f = a_f;
	c = a_c;
}
