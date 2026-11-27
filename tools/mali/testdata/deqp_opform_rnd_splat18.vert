attribute vec4 dEQP_Position;
attribute float a_b;
attribute float a_h;
attribute float a_j;
attribute float a_n;
attribute float a_q;
attribute float a_r;
attribute float a_s;
varying mediump float b;
varying mediump float h;
varying mediump float j;
varying mediump float n;
varying mediump float q;
varying mediump float r;
varying mediump float s;

void main()
{
	gl_Position = dEQP_Position;
	b = a_b;
	h = a_h;
	j = a_j;
	n = a_n;
	q = a_q;
	r = a_r;
	s = a_s;
}
