attribute vec4 dEQP_Position;
attribute vec4 a_a;
varying mediump vec4 a;

void main()
{
	gl_Position = dEQP_Position;
	a = a_a;
}
