attribute vec4 dEQP_Position;
attribute float a_s;
attribute float a_n;
attribute vec2 a_b;
attribute float a_h;
varying mediump float s;
varying mediump float n;
varying mediump vec2 b;
varying mediump float h;

void main()
{
	gl_Position = dEQP_Position;
	s = a_s;
	n = a_n;
	b = a_b;
	h = a_h;
}
