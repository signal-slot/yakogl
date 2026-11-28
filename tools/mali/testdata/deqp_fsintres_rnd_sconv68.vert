attribute vec4 dEQP_Position;
attribute float a_b;
attribute vec4 a_g;
attribute vec4 a_h;
attribute vec4 a_k;
attribute float a_p;
varying mediump float b;
varying mediump vec4 g;
varying mediump vec4 h;
varying mediump vec4 k;
varying mediump float p;

void main()
{
	gl_Position = dEQP_Position;
	b = a_b;
	g = a_g;
	h = a_h;
	k = a_k;
	p = a_p;
}
