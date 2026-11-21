attribute vec4 dEQP_Position;
attribute vec3 a_b;
attribute float a_d;
attribute float a_g;
attribute float a_n;
attribute float a_p;
varying mediump vec3 b;
varying mediump float d;
varying mediump float g;
varying mediump float n;
varying mediump float p;

void main()
{
	gl_Position = dEQP_Position;
	b = a_b;
	d = a_d;
	g = a_g;
	n = a_n;
	p = a_p;
}
