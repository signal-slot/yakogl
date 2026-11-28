attribute vec4 dEQP_Position;
attribute float a_b;
attribute float a_c;
attribute float a_l;
attribute vec4 a_n;
attribute float a_o;
varying mediump float b;
varying mediump float c;
varying mediump float l;
varying mediump vec4 n;
varying mediump float o;

void main()
{
	gl_Position = dEQP_Position;
	b = a_b;
	c = a_c;
	l = a_l;
	n = a_n;
	o = a_o;
}
