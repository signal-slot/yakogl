attribute vec4 dEQP_Position;
attribute vec4 a_a;
attribute vec3 a_e;
varying mediump vec4 a;
varying mediump vec3 e;

void main()
{
	gl_Position = dEQP_Position;
	a = a_a;
	e = a_e;
}
