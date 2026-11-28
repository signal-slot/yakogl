attribute vec4 dEQP_Position;
attribute float a_b;
attribute float a_k;
varying mediump float b;
varying mediump float k;

void main()
{
	gl_Position = dEQP_Position;
	b = a_b;
	k = a_k;
}
