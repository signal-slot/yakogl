attribute vec4 dEQP_Position;
attribute vec4 a_b;
attribute float a_j;
attribute float a_m;
varying mediump vec4 b;
varying mediump float j;
varying mediump float m;

void main()
{
	gl_Position = dEQP_Position;
	b = a_b;
	j = a_j;
	m = a_m;
}
