attribute vec4 dEQP_Position;
attribute vec4 a_i;
attribute float a_b;
attribute float a_c;
attribute float a_h;
varying mediump vec4 i;
varying mediump float b;
varying mediump float c;
varying mediump float h;

void main()
{
	gl_Position = dEQP_Position;
	i = a_i;
	b = a_b;
	c = a_c;
	h = a_h;
}
