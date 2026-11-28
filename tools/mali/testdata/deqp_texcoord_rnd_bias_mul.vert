attribute vec4 dEQP_Position;
attribute vec3 a_d;
attribute float a_b;
varying mediump vec3 d;
varying mediump float b;

void main()
{
	gl_Position = dEQP_Position;
	d = a_d;
	b = a_b;
}
