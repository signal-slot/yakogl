attribute vec4 dEQP_Position;
attribute vec4 a_x;
attribute float a_j;
attribute vec2 a_p;
attribute float a_r;
attribute float a_s;
attribute vec4 a_t;
varying mediump vec4 x;
varying mediump float j;
varying mediump vec2 p;
varying mediump float r;
varying mediump float s;
varying mediump vec4 t;

void main()
{
	gl_Position = dEQP_Position;
	x = a_x;
	j = a_j;
	p = a_p;
	r = a_r;
	s = a_s;
	t = a_t;
}
