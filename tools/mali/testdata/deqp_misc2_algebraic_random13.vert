attribute vec4 dEQP_Position;
attribute float a_j;
attribute vec4 a_k;
attribute float a_l;
attribute float a_n;
varying mediump float j;
varying mediump vec4 k;
varying mediump float l;
varying mediump float n;

void main()
{
	gl_Position = dEQP_Position;
	j = a_j;
	k = a_k;
	l = a_l;
	n = a_n;
}
