attribute vec4 dEQP_Position;
attribute float a_b;
attribute float a_c;
attribute float a_d;
varying mediump float b;
varying mediump float c;
varying mediump float d;

void main()
{
	gl_Position = dEQP_Position;
	b = a_b;
	c = a_c;
	d = a_d;
}
