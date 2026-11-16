attribute vec4 dEQP_Position;
attribute float c;
attribute float b;
float a = (3.25) + float(-2.375) * float(1.25);
varying mediump vec4 v_color;

void main()
{
	gl_Position = dEQP_Position;
	v_color = vec4((0.75), c, b, 0.0 * a);
	v_color = v_color;
}
