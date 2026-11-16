attribute vec4 dEQP_Position;
attribute vec4 a_h;
attribute vec3 a_f;
attribute float a_d;
varying mediump vec4 h;
varying mediump vec3 f;
varying mediump float d;

void main()
{
	gl_Position = dEQP_Position;
	h = a_h;
	f = a_f;
	d = a_d;
}
