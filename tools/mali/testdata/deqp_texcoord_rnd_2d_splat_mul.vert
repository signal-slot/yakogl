attribute vec4 dEQP_Position;
attribute vec4 a_h;
attribute vec4 a_i;
attribute float a_j;
attribute vec2 a_l;
varying mediump vec4 h;
varying mediump vec4 i;
varying mediump float j;
varying mediump vec2 l;

void main()
{
	gl_Position = dEQP_Position;
	h = a_h;
	i = a_i;
	j = a_j;
	l = a_l;
}
