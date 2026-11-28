attribute vec4 dEQP_Position;
attribute float a_g;
attribute float a_h;
attribute float a_k;
attribute vec4 a_n;
varying mediump float g;
varying mediump float h;
varying mediump float k;
varying mediump vec4 n;

void main()
{
	gl_Position = dEQP_Position;
	g = a_g;
	h = a_h;
	k = a_k;
	n = a_n;
}
