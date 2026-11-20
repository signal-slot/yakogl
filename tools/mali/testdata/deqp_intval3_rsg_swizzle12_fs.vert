attribute vec4 dEQP_Position;
attribute float a_h;
attribute float a_k;
varying mediump float h;
varying mediump float k;

void main()
{
	gl_Position = dEQP_Position;
	h = a_h;
	k = a_k;
}
