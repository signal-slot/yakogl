attribute vec4 dEQP_Position;
attribute float a_c;
attribute float a_d;
attribute float a_k;
attribute float a_l;
attribute float a_o;
varying mediump float c;
varying mediump float d;
varying mediump float k;
varying mediump float l;
varying mediump float o;

void main()
{
	gl_Position = dEQP_Position;
	c = a_c;
	d = a_d;
	k = a_k;
	l = a_l;
	o = a_o;
}
