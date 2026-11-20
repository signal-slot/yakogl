attribute vec4 dEQP_Position;
attribute vec3 a_c;
attribute vec3 a_b;
varying mediump vec3 c;
varying mediump vec3 b;

void main()
{
	gl_Position = dEQP_Position;
	c = a_c;
	b = a_b;
}
