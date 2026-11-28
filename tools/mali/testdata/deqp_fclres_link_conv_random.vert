attribute vec4 dEQP_Position;
attribute float b;
attribute float a;
varying mediump vec4 v_color;

void main()
{
	gl_Position = dEQP_Position;
	v_color = (vec4(b, a, float(a), 1.0));
}
