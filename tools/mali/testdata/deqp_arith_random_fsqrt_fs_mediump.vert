attribute vec4 dEQP_Position;
attribute float a_e;
attribute vec4 a_h;
attribute float a_i;
varying mediump float e;
varying mediump vec4 h;
varying mediump float i;

void main()
{
	gl_Position = dEQP_Position;
	e = a_e;
	h = a_h;
	i = a_i;
}
