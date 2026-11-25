attribute vec4 dEQP_Position;
attribute float a_b;
attribute vec4 a_f;
attribute vec4 a_i;
varying mediump float b;
varying mediump vec4 f;
varying mediump vec4 i;

void main()
{
	gl_Position = dEQP_Position;
	b = a_b;
	f = a_f;
	i = a_i;
}
