attribute vec4 dEQP_Position;
attribute float a_b;
attribute vec4 a_y;
attribute vec4 a_t;
attribute float a_s;
attribute vec3 a_m;
attribute vec4 a_i;
attribute vec3 a_f;
varying mediump float b;
varying mediump vec4 y;
varying mediump vec4 t;
varying mediump float s;
varying mediump vec3 m;
varying mediump vec4 i;
varying mediump vec3 f;

void main()
{
	gl_Position = dEQP_Position;
	b = a_b;
	y = a_y;
	t = a_t;
	s = a_s;
	m = a_m;
	i = a_i;
	f = a_f;
}
