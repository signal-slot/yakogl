attribute vec4 dEQP_Position;
attribute float a_c;
attribute float a_j;
attribute vec4 a_k;
attribute vec3 a_l;
attribute float a_m;
varying mediump float c;
varying mediump float j;
varying mediump vec4 k;
varying mediump vec3 l;
varying mediump float m;

void main()
{
	gl_Position = dEQP_Position;
	c = a_c;
	j = a_j;
	k = a_k;
	l = a_l;
	m = a_m;
}
