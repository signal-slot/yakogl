attribute vec4 dEQP_Position;
attribute float a_a;
attribute float a_d;
attribute float a_j;
attribute float a_o;
varying mediump float a;
varying mediump float d;
varying mediump float j;
varying mediump float o;

void main()
{
	gl_Position = dEQP_Position;
	a = a_a;
	d = a_d;
	j = a_j;
	o = a_o;
}
