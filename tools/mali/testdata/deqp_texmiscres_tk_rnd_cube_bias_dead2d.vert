attribute vec4 dEQP_Position;
attribute float a_c;
attribute vec4 a_f;
attribute vec3 a_k;
varying mediump float c;
varying mediump vec4 f;
varying mediump vec3 k;

void main()
{
	gl_Position = dEQP_Position;
	c = a_c;
	f = a_f;
	k = a_k;
}
