attribute vec4 dEQP_Position;
attribute vec4 a_k;
attribute float a_b;
attribute float a_c;
attribute float a_d;
attribute float a_j;
varying mediump vec4 k;
varying mediump float b;
varying mediump float c;
varying mediump float d;
varying mediump float j;

void main()
{
	gl_Position = dEQP_Position;
	k = a_k;
	b = a_b;
	c = a_c;
	d = a_d;
	j = a_j;
}
