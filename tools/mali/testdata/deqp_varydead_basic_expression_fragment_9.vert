attribute vec4 dEQP_Position;
attribute float a_r;
attribute float a_f;
attribute vec4 a_g;
attribute vec4 a_h;
attribute float a_i;
attribute float a_j;
attribute float a_n;
varying mediump float r;
varying mediump float f;
varying mediump vec4 g;
varying mediump vec4 h;
varying mediump float i;
varying mediump float j;
varying mediump float n;

void main()
{
	gl_Position = dEQP_Position;
	r = a_r;
	f = a_f;
	g = a_g;
	h = a_h;
	i = a_i;
	j = a_j;
	n = a_n;
}
