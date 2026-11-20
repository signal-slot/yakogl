attribute vec4 dEQP_Position;
attribute vec2 a_c;
varying mediump vec2 c;

void main()
{
	gl_Position = dEQP_Position;
	c = a_c;
}
