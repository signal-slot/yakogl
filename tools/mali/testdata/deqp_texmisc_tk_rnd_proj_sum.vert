attribute vec4 dEQP_Position;
attribute float a_d;
attribute float a_h;
attribute vec2 a_i;
attribute vec2 a_j;
attribute vec3 a_k;
varying mediump float d;
varying mediump float h;
varying mediump vec2 i;
varying mediump vec2 j;
varying mediump vec3 k;

void main()
{
	gl_Position = dEQP_Position;
	d = a_d;
	h = a_h;
	i = a_i;
	j = a_j;
	k = a_k;
}
