attribute vec4 dEQP_Position;
attribute float a_e;
attribute float a_f;
attribute float a_j;
attribute vec3 a_k;
varying mediump float e;
varying mediump float f;
varying mediump float j;
varying mediump vec3 k;

void main()
{
	gl_Position = dEQP_Position;
	e = a_e;
	f = a_f;
	j = a_j;
	k = a_k;
}
