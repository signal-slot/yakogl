attribute vec4 dEQP_Position;
const int a = 7 + (-6);
uniform mediump float b;
const float c = -5.875 + (float(2));
const float d = float(0.75) * (float(-2));
const bool f = true;
const bvec3 g = (bvec3(float(0), float(f), 0));
const vec4 h = vec4(-9.75, -4.75, -12.25, 0.75) - vec4(-6.5, 2.5, -3, -2.0) + vec4(4.5, 7.0, int(8.0), -2.0);
attribute float i;
const float j = float(-7) + 7.5;
const float l = (-1.75 * float(f));
const int m = 6;
varying mediump vec4 v_color;

void main()
{
	gl_Position = dEQP_Position;
	v_color = vec4(3.0, float(-2.75), -2.75, i) + vec4(1.625, 11.0, -2.0, -6.5) * vec4(d, 0.25, -0.75, 1);
	v_color = vec4(-5.4, 5.0, -14, 3.6) + vec4(-2.5, b, 8, b) - vec4(4.875, a, a, -5) * v_color;
	int k = (-1) - m;
	v_color = (vec4(l, k, j, i) * h);
	bvec3 e = g;
	e = bvec3(1.0, float(bool(f)), 1);
	v_color = v_color;
	v_color = v_color * vec4(2, -0.5, d, -0.75) + vec4(c, b, 0.375, 5.0) * vec4(-1.25, -0.5, -1, a);
}
