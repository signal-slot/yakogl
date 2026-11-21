attribute vec4 dEQP_Position;
attribute vec3 a_d;
varying mediump vec3 d;

void main()
{
	gl_Position = dEQP_Position;
	d = a_d;
}
