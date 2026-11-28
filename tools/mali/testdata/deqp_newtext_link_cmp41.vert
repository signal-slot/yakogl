attribute vec4 dEQP_Position;
attribute float a_e;
attribute float a_f;
attribute float a_g;
attribute float a_i;
attribute float a_j;
attribute float a_k;
attribute vec4 a_n;
varying mediump float e;
varying mediump float f;
varying mediump float g;
varying mediump float i;
varying mediump float j;
varying mediump float k;
varying mediump vec4 n;

void main()
{
	gl_Position = dEQP_Position;
	e = a_e;
	f = a_f;
	g = a_g;
	i = a_i;
	j = a_j;
	k = a_k;
	n = a_n;
}
