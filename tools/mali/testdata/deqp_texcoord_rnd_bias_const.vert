attribute vec4 dEQP_Position;
attribute vec4 a_k;
attribute vec4 a_j;
attribute float a_b;
attribute float a_c;
attribute vec3 a_d;
attribute vec4 a_e;
varying mediump vec4 k;
varying mediump vec4 j;
varying mediump float b;
varying mediump float c;
varying mediump vec3 d;
varying mediump vec4 e;

void main()
{
	gl_Position = dEQP_Position;
	k = a_k;
	j = a_j;
	b = a_b;
	c = a_c;
	d = a_d;
	e = a_e;
}
