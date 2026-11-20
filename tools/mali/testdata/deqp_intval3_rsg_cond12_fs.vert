attribute vec4 dEQP_Position;
attribute vec3 a_b;
attribute vec4 a_i;
varying mediump vec3 b;
varying mediump vec4 i;

void main()
{
	gl_Position = dEQP_Position;
	b = a_b;
	i = a_i;
}
