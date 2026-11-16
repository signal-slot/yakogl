attribute vec4 dEQP_Position;
attribute vec4 a_c;
attribute float a_d;
varying mediump vec4 c;
varying mediump float d;

void main()
{
	gl_Position = dEQP_Position;
	c = a_c;
	d = a_d;
}
