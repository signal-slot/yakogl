attribute vec4 dEQP_Position;
attribute float a_d;
attribute vec3 a_e;
attribute float a_i;
varying mediump float d;
varying mediump vec3 e;
varying mediump float i;

void main()
{
	gl_Position = dEQP_Position;
	d = a_d;
	e = a_e;
	i = a_i;
}
