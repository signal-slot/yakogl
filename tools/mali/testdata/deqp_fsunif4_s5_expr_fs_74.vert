attribute vec4 dEQP_Position;
attribute float a_a;
attribute float a_b;
attribute float a_f;
attribute float a_g;
attribute vec4 a_h;
attribute float a_l;
attribute float a_p;
varying mediump float a;
varying mediump float b;
varying mediump float f;
varying mediump float g;
varying mediump vec4 h;
varying mediump float l;
varying mediump float p;

void main()
{
	gl_Position = dEQP_Position;
	a = a_a;
	b = a_b;
	f = a_f;
	g = a_g;
	h = a_h;
	l = a_l;
	p = a_p;
}
