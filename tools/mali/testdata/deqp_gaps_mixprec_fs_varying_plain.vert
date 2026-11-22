attribute vec4 dEQP_Position;
const float a = float(6.0) * float(0.25) - (float(0.5));
const float b = float(0.25);
const float c = (3.5 - -0.5);
attribute float d;
const float e = -3.0 - (-6.0);
attribute float g;
const float h = (5.0) * (0.5);
attribute float i;
const vec4 j = vec4(float(16.0), 10.0, -17.0, float(7.75)) - vec4(-12.0, -3.5, 2.25, -7.0) * vec4(-0.25, -2.0, 2.0, -1.0);
uniform mediump vec4 k;
varying mediump vec4 v_color;

void main()
{
	gl_Position = dEQP_Position;
	v_color = vec4(1.0 * float(1.5), 1.0 * float(-1.5), (float(1.5)), -1.0 * a);
	v_color = vec4(-6.375, c, 1.5, 9.9) * v_color - vec4(float(h), float(-8.75), float(b), float(d));
	v_color = (vec4(i, a, -3.15, -10.25) + vec4(4.75, b, b, d));
	v_color = vec4(float(-10.7), -9.0, e, c) - k;
	vec4 f = j + vec4(i, 6.0, h, 3.5) + vec4(float(g), -2.5, float(g), -7.5);
	v_color = f;
	v_color = vec4(11.5, -10.0, 10.5, e) + vec4(d, c, -6.0, a) + v_color * vec4(-0.5, a, b, a);
}
