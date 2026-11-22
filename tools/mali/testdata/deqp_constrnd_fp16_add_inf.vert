attribute vec4 dEQP_Position;
attribute float a_d;
varying mediump float d;

void main()
{
	gl_Position = dEQP_Position;
	d = a_d;
}
