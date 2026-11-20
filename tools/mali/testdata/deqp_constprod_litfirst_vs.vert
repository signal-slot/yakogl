attribute vec4 dEQP_Position;
attribute float a;
attribute float b;
varying mediump vec4 v_color;

void main()
{
	gl_Position = dEQP_Position;
	v_color = vec4(a, a, -0.0 * b, a);
}
