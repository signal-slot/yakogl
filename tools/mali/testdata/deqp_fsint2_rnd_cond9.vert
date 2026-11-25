attribute vec4 dEQP_Position;
attribute float a_r;
attribute float a_a;
attribute vec2 a_m;
attribute float a_o;
varying mediump float r;
varying mediump float a;
varying mediump vec2 m;
varying mediump float o;

void main()
{
	gl_Position = dEQP_Position;
	r = a_r;
	a = a_a;
	m = a_m;
	o = a_o;
}
