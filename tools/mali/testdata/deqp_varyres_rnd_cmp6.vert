attribute vec4 dEQP_Position;
attribute float a_b;
attribute float a_c;
attribute vec4 a_d;
attribute float a_g;
attribute float a_h;
attribute float a_l;
attribute vec4 a_n;
varying mediump float b;
varying mediump float c;
varying mediump vec4 d;
varying mediump float g;
varying mediump float h;
varying mediump float l;
varying mediump vec4 n;

void main()
{
	gl_Position = dEQP_Position;
	b = a_b;
	c = a_c;
	d = a_d;
	g = a_g;
	h = a_h;
	l = a_l;
	n = a_n;
}
