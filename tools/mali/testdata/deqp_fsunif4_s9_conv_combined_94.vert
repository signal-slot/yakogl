attribute vec4 dEQP_Position;
ivec4 d = (ivec4(9, 3, -8, -2.0)) * (ivec4(6, true, -7.0, -14));
float f = float(float(2.5) + float(-6.5));
const int i = 2;
uniform mediump float j;
attribute float k;
attribute float l;
uniform mediump int m;
int n = -7;
int o = (-10 - int(true));
ivec2 q = ivec2(9.0, 0) - ivec2(7, -5.75) + (ivec2(i, 4.0));
const ivec2 r = ivec2(5.0, 10.0) - ivec2(-3, 6.0) + (ivec2(-7.0, -3.0));
float s = float(8) * (-2.0);
int t = int(1) * int(1.0) * int(1);
const int u = (i) * (int(8.25));

void main()
{
	gl_Position = dEQP_Position;
	ivec2 p = ivec2(u, int(i)) * ivec2(float(n), bool(t));
	p = ivec2(t, s) * r + q;
	int h = (int(o) - n);
	int g = (int(float(m)));
	float e = (f * float(-1.25));
	d = ivec4(8.5 + -7.5, int(int(l)), (k), -8 + g);
	d = ivec4(k, j, true, int(i)) + d;
	d = ivec4((true), h + int(-2), float(g) + f, (e));
}
