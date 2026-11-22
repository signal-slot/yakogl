attribute vec4 dEQP_Position;
attribute float a_j;
attribute float a_b;
varying mediump float j;
varying mediump float b;

void main()
{
	gl_Position = dEQP_Position;
	j = a_j;
	b = a_b;
}
