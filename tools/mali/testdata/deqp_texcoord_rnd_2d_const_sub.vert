attribute vec4 dEQP_Position;
attribute vec2 a_b;
attribute float a_j;
varying mediump vec2 b;
varying mediump float j;

void main()
{
	gl_Position = dEQP_Position;
	b = a_b;
	j = a_j;
}
