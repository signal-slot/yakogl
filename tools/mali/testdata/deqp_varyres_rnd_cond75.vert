attribute vec4 dEQP_Position;
attribute float a_g;
attribute vec3 a_f;
attribute float a_a;
attribute float a_m;
varying mediump float g;
varying mediump vec3 f;
varying mediump float a;
varying mediump float m;

void main()
{
	gl_Position = dEQP_Position;
	g = a_g;
	f = a_f;
	a = a_a;
	m = a_m;
}
