attribute vec4 dEQP_Position;
vec4 c = (vec4(float(-1.25), float(-0.5), float(-1.5), 8.0));
uniform mediump float b;
float a = float(-9.0) - float(-8.0) - 6.0;
varying mediump vec4 v_color;

void main()
{
	gl_Position = dEQP_Position;
	v_color = c + (vec4(1.5, b, b, a));
}
