precision mediump float;
const bool p = bool(float(-5.5) - -5.5);
int b = (int(10) - 5);
ivec4 e = ivec4(-2.0, 3, -5, 7).yxzw - ivec4(0, -2, -2, 5).zxyw;
varying mediump float f;
varying mediump float g;
int h = ivec2(3.75, -9).t - int(-1.0) * 1;
const ivec4 j = ivec4(4, 4, 7.0, -16) + ivec4(false, -5, -7, 7.0) - ivec4(-2.0, -5, 6.0, -5.0);
ivec4 k = ivec4(7.0, 3, 0.0, -1.0).arbg - ivec4(-5, true, 4.0, 5.0) * ivec4(1.0, 1, 1, true);
const int l = ivec3(int(3), true, float(true)).p;
uniform mediump int m;
const int n = (int(true));
const bool o = (float(l)) <= float(int(5.0)) + 0.001;

void main()
{
	gl_FragColor = (vec4(0.5, 0, -3.75, f) * vec4(1, o, p, 1));
	int a = ivec4(int(g), o, n, float(false)).y;
	int d = ivec2(int(m), int(l)).r;
	ivec4 i = k - j;
	if (i - ivec4(h, true, g, f) == e)
	{
		int c = d;
		a = int(c) + b;
	}
	gl_FragColor = gl_FragColor;
}
