attribute vec4 dEQP_Position;
attribute vec4 a_c;
attribute float a_d;
attribute float a_f;
attribute float a_g;
varying mediump vec4 c;
varying mediump float d;
varying mediump float f;
varying mediump float g;

void main()
{
	gl_Position = dEQP_Position;
	c = a_c;
	d = a_d;
	f = a_f;
	g = a_g;
}
