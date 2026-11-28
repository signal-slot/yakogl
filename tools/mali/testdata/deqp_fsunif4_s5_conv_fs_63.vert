attribute vec4 dEQP_Position;
attribute vec4 a_k;
attribute float a_i;
attribute float a_a;
attribute float a_b;
attribute float a_g;
varying mediump vec4 k;
varying mediump float i;
varying mediump float a;
varying mediump float b;
varying mediump float g;

void main()
{
	gl_Position = dEQP_Position;
	k = a_k;
	i = a_i;
	a = a_a;
	b = a_b;
	g = a_g;
}
