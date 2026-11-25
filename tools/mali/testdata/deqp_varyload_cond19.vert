attribute vec4 dEQP_Position;
attribute vec3 a_g;
attribute float a_l;
attribute float a_k;
varying mediump vec3 g;
varying mediump float l;
varying mediump float k;

void main()
{
	gl_Position = dEQP_Position;
	g = a_g;
	l = a_l;
	k = a_k;
}
