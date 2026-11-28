attribute vec4 dEQP_Position;
attribute vec2 a_l;
attribute vec2 a_k;
attribute float a_h;
attribute float a_g;
attribute float a_m;
varying mediump vec2 l;
varying mediump vec2 k;
varying mediump float h;
varying mediump float g;
varying mediump float m;

void main()
{
	gl_Position = dEQP_Position;
	l = a_l;
	k = a_k;
	h = a_h;
	g = a_g;
	m = a_m;
}
