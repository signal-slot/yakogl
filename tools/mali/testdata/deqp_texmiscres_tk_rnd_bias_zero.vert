attribute vec4 dEQP_Position;
attribute float a_l;
attribute float a_c;
attribute float a_d;
varying mediump float l;
varying mediump float c;
varying mediump float d;

void main()
{
	gl_Position = dEQP_Position;
	l = a_l;
	c = a_c;
	d = a_d;
}
