attribute vec4 dEQP_Position;
attribute float a_d;
attribute float a_p;
attribute float a_r;
varying mediump float d;
varying mediump float p;
varying mediump float r;

void main()
{
	gl_Position = dEQP_Position;
	d = a_d;
	p = a_p;
	r = a_r;
}
