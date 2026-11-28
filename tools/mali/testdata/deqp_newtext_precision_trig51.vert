attribute vec4 dEQP_Position;
attribute float a_d;
attribute float a_f;
varying mediump float d;
varying mediump float f;

void main()
{
	gl_Position = dEQP_Position;
	d = a_d;
	f = a_f;
}
