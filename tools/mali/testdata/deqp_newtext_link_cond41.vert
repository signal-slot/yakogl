attribute vec4 dEQP_Position;
attribute float a_b;
attribute float a_g;
attribute vec2 a_n;
attribute vec4 a_l;
attribute vec4 a_m;
varying mediump float b;
varying mediump float g;
varying mediump vec2 n;
varying mediump vec4 l;
varying mediump vec4 m;

void main()
{
	gl_Position = dEQP_Position;
	b = a_b;
	g = a_g;
	n = a_n;
	l = a_l;
	m = a_m;
}
