attribute vec4 dEQP_Position;
attribute float a_b;
attribute float a_f;
attribute vec4 a_k;
varying mediump float b;
varying mediump float f;
varying mediump vec4 k;

void main()
{
	gl_Position = dEQP_Position;
	b = a_b;
	f = a_f;
	k = a_k;
}
