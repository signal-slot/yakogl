attribute vec4 dEQP_Position;
attribute float a_n;
attribute float a_k;
varying mediump float n;
varying mediump float k;

void main()
{
	gl_Position = dEQP_Position;
	n = a_n;
	k = a_k;
}
