attribute vec4 dEQP_Position;
attribute vec4 a_b;
attribute vec4 a_c;
attribute float a_d;
attribute vec4 a_g;
varying mediump vec4 b;
varying mediump vec4 c;
varying mediump float d;
varying mediump vec4 g;

void main()
{
	gl_Position = dEQP_Position;
	b = a_b;
	c = a_c;
	d = a_d;
	g = a_g;
}
