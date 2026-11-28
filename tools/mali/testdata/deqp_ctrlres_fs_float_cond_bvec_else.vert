attribute vec4 dEQP_Position;
attribute float a_c;
attribute float a_n;
attribute float a_m;
attribute float a_f;
attribute float a_r;
varying mediump float c;
varying mediump float n;
varying mediump float m;
varying mediump float f;
varying mediump float r;

void main()
{
	gl_Position = dEQP_Position;
	c = a_c;
	n = a_n;
	m = a_m;
	f = a_f;
	r = a_r;
}
