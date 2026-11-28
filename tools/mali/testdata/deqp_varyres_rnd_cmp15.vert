attribute vec4 dEQP_Position;
attribute vec3 a_a;
attribute vec3 a_d;
attribute float a_g;
attribute float a_i;
attribute vec4 a_l;
attribute float a_n;
varying mediump vec3 a;
varying mediump vec3 d;
varying mediump float g;
varying mediump float i;
varying mediump vec4 l;
varying mediump float n;

void main()
{
	gl_Position = dEQP_Position;
	a = a_a;
	d = a_d;
	g = a_g;
	i = a_i;
	l = a_l;
	n = a_n;
}
