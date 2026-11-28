attribute vec4 dEQP_Position;
attribute float a_i;
attribute float a_n;
varying mediump float i;
varying mediump float n;

void main()
{
	gl_Position = dEQP_Position;
	i = a_i;
	n = a_n;
}
