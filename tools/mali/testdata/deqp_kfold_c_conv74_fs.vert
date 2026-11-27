attribute vec4 dEQP_Position;
attribute float a_k;
attribute float a_c;
attribute float a_h;
varying mediump float k;
varying mediump float c;
varying mediump float h;

void main()
{
	gl_Position = dEQP_Position;
	k = a_k;
	c = a_c;
	h = a_h;
}
