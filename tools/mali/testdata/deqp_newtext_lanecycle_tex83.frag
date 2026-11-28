precision mediump float;
const int o = int(0);
const float n = float(-6.5 + float(2));
varying mediump float m;
int a = ivec4(n, true, o, true).qspt.x;
const int b = 1;
const float d = (vec4(n, 0.0, false, o)).t;
const bool e = true;
varying mediump float f;
const bool g = bool(int(1.0) - b);
const float h = 0.5;
varying mediump float i;
uniform mediump int j;
const ivec2 k = ivec2(-7, e).rg * (ivec2(e, e));
varying mediump float l;

void main()
{
	gl_FragColor = vec4(d, o - 1, n - m, -2.5 * 0.5);
	gl_FragColor = vec4(int(i), l, l, l) * gl_FragColor.rbag;
	a = (int(-1.0)) * int(1.0);
	ivec2 c = k - ivec2(j, a) + ivec2(i, i).st;
	gl_FragColor = vec4(a, 1, 0.75, -2).qtsp.brga;
	gl_FragColor = vec4(h, g, d, f) * gl_FragColor * vec4(-1, 1, e, bool(d));
	a = c.s * (int(b));
}
