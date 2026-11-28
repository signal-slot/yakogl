attribute vec4 dEQP_Position;
attribute float a_e;
attribute vec3 a_g;
attribute float a_k;
attribute float a_l;
attribute vec4 a_p;
attribute vec4 a_q;
attribute vec4 a_u;
varying mediump float e;
varying mediump vec3 g;
varying mediump float k;
varying mediump float l;
varying mediump vec4 p;
varying mediump vec4 q;
varying mediump vec4 u;

void main()
{
	gl_Position = dEQP_Position;
	e = a_e;
	g = a_g;
	k = a_k;
	l = a_l;
	p = a_p;
	q = a_q;
	u = a_u;
}
