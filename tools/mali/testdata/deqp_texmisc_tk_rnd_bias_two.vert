attribute vec4 dEQP_Position;
attribute float a_c;
attribute float a_e;
attribute vec2 a_o;
attribute float a_p;
attribute float a_r;
attribute float a_s;
varying mediump float c;
varying mediump float e;
varying mediump vec2 o;
varying mediump float p;
varying mediump float r;
varying mediump float s;

void main()
{
	gl_Position = dEQP_Position;
	c = a_c;
	e = a_e;
	o = a_o;
	p = a_p;
	r = a_r;
	s = a_s;
}
