attribute vec4 dEQP_Position;
attribute float a_k;
attribute vec4 a_b;
attribute vec3 a_c;
attribute vec3 a_d;
attribute vec4 a_h;
varying mediump float k;
varying mediump vec4 b;
varying mediump vec3 c;
varying mediump vec3 d;
varying mediump vec4 h;

void main()
{
	gl_Position = dEQP_Position;
	k = a_k;
	b = a_b;
	c = a_c;
	d = a_d;
	h = a_h;
}
