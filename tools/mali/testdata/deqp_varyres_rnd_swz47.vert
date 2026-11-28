attribute vec4 dEQP_Position;
attribute float a_c;
attribute float a_d;
attribute float a_h;
attribute vec4 a_m;
attribute vec4 a_n;
varying mediump float c;
varying mediump float d;
varying mediump float h;
varying mediump vec4 m;
varying mediump vec4 n;

void main()
{
	gl_Position = dEQP_Position;
	c = a_c;
	d = a_d;
	h = a_h;
	m = a_m;
	n = a_n;
}
