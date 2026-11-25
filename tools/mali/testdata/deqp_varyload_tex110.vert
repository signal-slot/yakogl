attribute vec4 dEQP_Position;
attribute float a_f;
attribute float a_i;
attribute float a_l;
attribute float a_n;
varying mediump float f;
varying mediump float i;
varying mediump float l;
varying mediump float n;

void main()
{
	gl_Position = dEQP_Position;
	f = a_f;
	i = a_i;
	l = a_l;
	n = a_n;
}
