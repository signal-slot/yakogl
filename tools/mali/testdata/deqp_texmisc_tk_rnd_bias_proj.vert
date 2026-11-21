attribute vec4 dEQP_Position;
attribute vec4 a_a;
attribute vec2 a_d;
attribute float a_g;
attribute vec4 a_h;
attribute float a_j;
attribute float a_n;
varying mediump vec4 a;
varying mediump vec2 d;
varying mediump float g;
varying mediump vec4 h;
varying mediump float j;
varying mediump float n;

void main()
{
	gl_Position = dEQP_Position;
	a = a_a;
	d = a_d;
	g = a_g;
	h = a_h;
	j = a_j;
	n = a_n;
}
