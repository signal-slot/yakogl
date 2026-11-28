attribute vec4 dEQP_Position;
attribute vec4 a_b;
attribute vec4 a_c;
attribute float a_d;
attribute float a_e;
attribute float a_g;
attribute float a_i;
attribute float a_j;
varying mediump vec4 b;
varying mediump vec4 c;
varying mediump float d;
varying mediump float e;
varying mediump float g;
varying mediump float i;
varying mediump float j;

void main()
{
	gl_Position = dEQP_Position;
	b = a_b;
	c = a_c;
	d = a_d;
	e = a_e;
	g = a_g;
	i = a_i;
	j = a_j;
}
