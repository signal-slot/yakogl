attribute vec4 dEQP_Position;
attribute vec4 a_a;
attribute vec4 a_s;
varying mediump vec4 a;
varying mediump vec4 s;

void main()
{
	gl_Position = dEQP_Position;
	a = a_a;
	s = a_s;
}
