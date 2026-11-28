attribute vec4 dEQP_Position;
attribute float a_m;
attribute float a_f;
attribute float a_i;
attribute float a_l;
varying mediump float m;
varying mediump float f;
varying mediump float i;
varying mediump float l;

void main()
{
	gl_Position = dEQP_Position;
	m = a_m;
	f = a_f;
	i = a_i;
	l = a_l;
}
