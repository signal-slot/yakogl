attribute vec4 dEQP_Position;
attribute float a_b;
attribute float a_d;
attribute vec2 a_j;
attribute vec4 a_l;
varying mediump float b;
varying mediump float d;
varying mediump vec2 j;
varying mediump vec4 l;

void main()
{
	gl_Position = dEQP_Position;
	b = a_b;
	d = a_d;
	j = a_j;
	l = a_l;
}
