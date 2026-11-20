attribute vec4 dEQP_Position;
attribute float a_a;
attribute float a_b;
attribute float a_g;
attribute float a_k;
varying mediump float a;
varying mediump float b;
varying mediump float g;
varying mediump float k;

void main()
{
	gl_Position = dEQP_Position;
	a = a_a;
	b = a_b;
	g = a_g;
	k = a_k;
}
