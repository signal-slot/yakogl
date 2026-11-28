attribute vec4 dEQP_Position;
attribute float a_l;
attribute float a_n;
attribute float a_x;
varying mediump float l;
varying mediump float n;
varying mediump float x;

void main()
{
	gl_Position = dEQP_Position;
	l = a_l;
	n = a_n;
	x = a_x;
}
