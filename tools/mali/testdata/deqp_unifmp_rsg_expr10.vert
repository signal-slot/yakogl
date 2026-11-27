attribute vec4 dEQP_Position;
attribute float a_p;
attribute float a_e;
attribute float a_i;
attribute float a_j;
attribute float a_l;
attribute float a_m;
varying mediump float p;
varying mediump float e;
varying mediump float i;
varying mediump float j;
varying mediump float l;
varying mediump float m;

void main()
{
	gl_Position = dEQP_Position;
	p = a_p;
	e = a_e;
	i = a_i;
	j = a_j;
	l = a_l;
	m = a_m;
}
