attribute vec4 dEQP_Position;
attribute float a_a;
attribute float a_d;
attribute float a_g;
attribute float a_i;
varying mediump float a;
varying mediump float d;
varying mediump float g;
varying mediump float i;

void main()
{
	gl_Position = dEQP_Position;
	a = a_a;
	d = a_d;
	g = a_g;
	i = a_i;
}
