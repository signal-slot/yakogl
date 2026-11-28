attribute vec4 dEQP_Position;
attribute float a_n;
attribute float a_c;
attribute float a_e;
attribute float a_h;
attribute float a_j;
attribute float a_m;
varying mediump float n;
varying mediump float c;
varying mediump float e;
varying mediump float h;
varying mediump float j;
varying mediump float m;

void main()
{
	gl_Position = dEQP_Position;
	n = a_n;
	c = a_c;
	e = a_e;
	h = a_h;
	j = a_j;
	m = a_m;
}
