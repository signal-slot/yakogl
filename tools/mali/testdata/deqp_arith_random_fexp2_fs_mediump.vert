attribute vec4 dEQP_Position;
attribute float a_a;
attribute float a_h;
attribute float a_j;
attribute vec3 a_k;
attribute vec4 a_n;
varying mediump float a;
varying mediump float h;
varying mediump float j;
varying mediump vec3 k;
varying mediump vec4 n;

void main()
{
	gl_Position = dEQP_Position;
	a = a_a;
	h = a_h;
	j = a_j;
	k = a_k;
	n = a_n;
}
