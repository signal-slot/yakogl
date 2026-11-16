attribute vec4 dEQP_Position;
attribute vec4 a_k;
attribute float a_c;
attribute float a_d;
attribute vec4 a_f;
attribute float a_i;
attribute float a_j;
varying mediump vec4 k;
varying mediump float c;
varying mediump float d;
varying mediump vec4 f;
varying mediump float i;
varying mediump float j;

void main()
{
	gl_Position = dEQP_Position;
	k = a_k;
	c = a_c;
	d = a_d;
	f = a_f;
	i = a_i;
	j = a_j;
}
