attribute vec4 dEQP_Position;
attribute float a_e;
attribute vec3 a_b;
attribute vec4 a_c;
varying mediump float e;
varying mediump vec3 b;
varying mediump vec4 c;

void main()
{
	gl_Position = dEQP_Position;
	e = a_e;
	b = a_b;
	c = a_c;
}
