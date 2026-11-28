attribute vec4 dEQP_Position;
attribute float a_c;
attribute float a_e;
attribute float a_h;
attribute float a_l;
attribute vec4 a_n;
attribute float a_p;
attribute float a_u;
varying mediump float c;
varying mediump float e;
varying mediump float h;
varying mediump float l;
varying mediump vec4 n;
varying mediump float p;
varying mediump float u;

void main()
{
	gl_Position = dEQP_Position;
	c = a_c;
	e = a_e;
	h = a_h;
	l = a_l;
	n = a_n;
	p = a_p;
	u = a_u;
}
