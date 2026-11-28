attribute vec4 dEQP_Position;
attribute float b;
attribute float c;
uniform mediump float d;
const float e = float(float(-0.5)) - 0.0;
const float g = (-1.0) * (1.0);
const float h = float(8.0) * float(1.0) + -6.0;
const float i = -1.0;
const float j = 7.5;
const float k = float(-13.25) * 1.0 + (5.0);
const float l = 1.0;
attribute float m;
const float n = (float(-5.75) + float(5.0));
varying mediump vec4 v_color;

void main()
{
	gl_Position = dEQP_Position;
	v_color = (vec4(-8.5, 3.5, -4.0, 6.5) + vec4(7.375, -3.5, h, -6.5));
	v_color = (v_color * v_color);
	v_color = (vec4(float(-2.15), d, b, float(j)));
	v_color = (vec4(-11.0, 0.75, -3.25, 70.5)) * vec4(float(l), d, -1.5, float(0.25));
	vec4 f = (v_color) * vec4(float(0.5), -1.5, c, n);
	v_color = vec4(m, l, k, j) * vec4(-0.25, 1.5, g, i) * vec4(float(0.25), h, 0.75, float(g));
	v_color = (f + v_color);
	v_color = (vec4(-1.25, 4.0, -3.75, -6.0)) - (v_color);
	true;
	vec4 a = (vec4(e, float(d), float(c), float(b)));
	v_color = a * v_color;
}
