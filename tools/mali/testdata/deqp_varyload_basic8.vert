attribute vec4 dEQP_Position;
attribute float a_d;
attribute float a_g;
attribute float a_i;
attribute float a_l;
varying mediump float d;
varying mediump float g;
varying mediump float i;
varying mediump float l;

void main()
{
	gl_Position = dEQP_Position;
	d = a_d;
	g = a_g;
	i = a_i;
	l = a_l;
}
