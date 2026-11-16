attribute vec4 dEQP_Position;
attribute float a;
attribute float b;
const float c = (float(-6.75) - -6.5);
varying mediump vec4 v_color;

void main()
{
	gl_Position = dEQP_Position;
	v_color = vec4(0, c * b, a, 0);
}
