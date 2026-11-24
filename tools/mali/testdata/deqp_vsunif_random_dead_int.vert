attribute vec4 dEQP_Position;
const float a = 5.25 + -5.0;
const vec4 b = ((vec4(9.25, 3.5, -1.25, -5.375)));
attribute vec4 c;
attribute float e;
const float f = float(float(1.75)) * -0.5;
uniform mediump int h;
const int i = int(int(4) + -3);
ivec3 j = ((ivec3(-12, i, i)));
varying mediump vec4 v_color;

void main()
{
	gl_Position = dEQP_Position;
	ivec3 g = j;
	j = ivec3(-9 + int(-4), int(i) * int(h), h);
	float d = float(float(11.125)) * float(float(-0.5));
	g = g * ivec3(int(i), i, int(h));
	g = (ivec3(h, -2, h));
	v_color = vec4(float(d), float(a) * float(f), float(e) + float(a), float(d) - d);
	v_color = c + b * vec4(a, -1.0, -1.25, -1.5);
}
