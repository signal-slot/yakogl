attribute vec4 dEQP_Position;
attribute float a_b;
attribute float a_d;
attribute vec3 a_e;
attribute vec4 a_f;
attribute float a_g;
attribute float a_l;
varying mediump float b;
varying mediump float d;
varying mediump vec3 e;
varying mediump vec4 f;
varying mediump float g;
varying mediump float l;

void main()
{
	gl_Position = dEQP_Position;
	b = a_b;
	d = a_d;
	e = a_e;
	f = a_f;
	g = a_g;
	l = a_l;
}
