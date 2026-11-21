attribute vec4 dEQP_Position;
attribute float a_f;
attribute vec2 a_k;
attribute float a_p;
attribute float a_q;
attribute vec3 a_u;
varying mediump float f;
varying mediump vec2 k;
varying mediump float p;
varying mediump float q;
varying mediump vec3 u;

void main()
{
	gl_Position = dEQP_Position;
	f = a_f;
	k = a_k;
	p = a_p;
	q = a_q;
	u = a_u;
}
