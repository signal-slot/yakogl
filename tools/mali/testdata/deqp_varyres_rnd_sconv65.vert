attribute vec4 dEQP_Position;
attribute float a_a;
attribute float a_d;
attribute float a_e;
attribute float a_p;
varying mediump float a;
varying mediump float d;
varying mediump float e;
varying mediump float p;

void main()
{
	gl_Position = dEQP_Position;
	a = a_a;
	d = a_d;
	e = a_e;
	p = a_p;
}
