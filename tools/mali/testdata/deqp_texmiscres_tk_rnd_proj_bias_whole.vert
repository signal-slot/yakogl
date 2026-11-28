attribute vec4 dEQP_Position;
attribute float a_b;
attribute float a_c;
attribute vec3 a_d;
attribute float a_f;
varying mediump float b;
varying mediump float c;
varying mediump vec3 d;
varying mediump float f;

void main()
{
	gl_Position = dEQP_Position;
	b = a_b;
	c = a_c;
	d = a_d;
	f = a_f;
}
