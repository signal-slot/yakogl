attribute vec4 dEQP_Position;
attribute vec4 a_d;
attribute float a_j;
varying mediump vec4 d;
varying mediump float j;

void main()
{
	gl_Position = dEQP_Position;
	d = a_d;
	j = a_j;
}
