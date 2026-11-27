attribute vec4 dEQP_Position;
attribute vec4 a_a;
attribute float a_h;
attribute float a_j;
varying mediump vec4 a;
varying mediump float h;
varying mediump float j;

void main()
{
	gl_Position = dEQP_Position;
	a = a_a;
	h = a_h;
	j = a_j;
}
