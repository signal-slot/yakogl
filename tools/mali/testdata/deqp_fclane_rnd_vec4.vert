attribute vec4 dEQP_Position;
attribute float a_f;
attribute float a_e;
attribute float a_d;
varying mediump float f;
varying mediump float e;
varying mediump float d;

void main()
{
	gl_Position = dEQP_Position;
	f = a_f;
	e = a_e;
	d = a_d;
}
