attribute vec4 dEQP_Position;
attribute vec2 a_b;
attribute float a_e;
attribute float a_h;
varying mediump vec2 b;
varying mediump float e;
varying mediump float h;

void main()
{
	gl_Position = dEQP_Position;
	b = a_b;
	e = a_e;
	h = a_h;
}
