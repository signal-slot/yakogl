attribute vec4 dEQP_Position;
int b = ivec3(true, true, -2).x;
int a = (-2 * 1);
ivec4 e = ivec4(float(3.0), int(3), 4.75, float(-3)).zxwy;
attribute float f;
attribute float g;
int h = int(int(-7.0)) + (int(0));
const ivec4 j = ivec4(6, 4, -6, -4.0) * ivec4(1.0, 1, 1, true) * ivec4(1.0, 1.0, true, true).wzxy;
const bool k = true;
varying mediump vec4 v_color;

void main()
{
	gl_Position = dEQP_Position;
	v_color = vec4(-8.0, -11.5, -10, -5) + vec4(1.25, g, 6, 8.0) - vec4(-3.75, 6.75, 8.25, -10) * vec4(2.0, -1.25, -0.5, f);
	int d = -7 - -8 * 1;
	{
		if (d != d * int(b))
			vec3(bool(k), g, int(f)).r + 0.001 >= vec4(a, int(k), f, k).x;
		b = a;
	}
	ivec4 i = ivec4(3, d, -4.0, b) * ivec4(true, 1, d, k) - j;
	if (i - ivec4(h, true, g, f) == e)
	{
		int c = d;
		a = int(c) + b;
	}
	v_color = v_color;
}
