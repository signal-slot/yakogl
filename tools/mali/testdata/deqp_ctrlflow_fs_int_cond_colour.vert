attribute vec4 dEQP_Position;
attribute float a_m;
attribute float a_h;
attribute vec4 a_i;
varying mediump float m;
varying mediump float h;
varying mediump vec4 i;

void main()
{
	gl_Position = dEQP_Position;
	m = a_m;
	h = a_h;
	i = a_i;
}
