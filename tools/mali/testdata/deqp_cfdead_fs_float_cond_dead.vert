attribute vec4 dEQP_Position;
attribute vec4 a_i;
attribute float a_d;
attribute float a_h;
varying mediump vec4 i;
varying mediump float d;
varying mediump float h;

void main()
{
	gl_Position = dEQP_Position;
	i = a_i;
	d = a_d;
	h = a_h;
}
