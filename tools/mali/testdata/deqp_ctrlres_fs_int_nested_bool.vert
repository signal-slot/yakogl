attribute vec4 dEQP_Position;
attribute float a_d;
attribute float a_n;
attribute vec2 a_k;
varying mediump float d;
varying mediump float n;
varying mediump vec2 k;

void main()
{
	gl_Position = dEQP_Position;
	d = a_d;
	n = a_n;
	k = a_k;
}
