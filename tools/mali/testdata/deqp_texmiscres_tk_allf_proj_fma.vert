attribute vec4 dEQP_Position;
attribute vec3 a_n;
attribute vec3 a_m;
attribute vec3 a_l;
attribute float a_f;
attribute float a_g;
varying mediump vec3 n;
varying mediump vec3 m;
varying mediump vec3 l;
varying mediump float f;
varying mediump float g;

void main()
{
	gl_Position = dEQP_Position;
	n = a_n;
	m = a_m;
	l = a_l;
	f = a_f;
	g = a_g;
}
