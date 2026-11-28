attribute vec4 dEQP_Position;
attribute float a_a;
attribute vec3 a_e;
attribute vec3 a_f;
attribute float a_j;
attribute vec4 a_k;
attribute vec4 a_l;
varying mediump float a;
varying mediump vec3 e;
varying mediump vec3 f;
varying mediump float j;
varying mediump vec4 k;
varying mediump vec4 l;

void main()
{
	gl_Position = dEQP_Position;
	a = a_a;
	e = a_e;
	f = a_f;
	j = a_j;
	k = a_k;
	l = a_l;
}
