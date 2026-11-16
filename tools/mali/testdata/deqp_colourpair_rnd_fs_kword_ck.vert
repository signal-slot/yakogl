attribute vec4 dEQP_Position;
attribute float a_c;
varying mediump float c;

void main()
{
	gl_Position = dEQP_Position;
	c = a_c;
}
