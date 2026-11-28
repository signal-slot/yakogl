precision mediump float;
varying mediump float a;
const int b = ivec4(7.75, -15, 0, true).tpsq.y;
varying mediump float d;
const ivec4 f = (ivec4(0.75, -6.0, true, true)) + (ivec4(true, 7, b, 0));
varying mediump float g;
const bool h = bool(float(6.0) + -5.0);
varying mediump float i;
const float j = (vec2(4.75, -2.5)).s;
const bvec3 l = bvec4(float(b), int(b), true, bool(b)).tpq;
uniform mediump vec4 m;

void main()
{
	gl_FragColor = vec4(11.25, g, -3.5, -5.25).arbg * vec4(1, -1.0, -2.0, h).ywzx;
	gl_FragColor = vec4(-17.7, -20, i, -15.0) + gl_FragColor + vec4(int(9), int(4), float(8.5), 6.25);
	gl_FragColor = vec4((d), true, h, (int(15.5)));
	vec4 n = vec4(10, -24.1, -17.45, -10.5) + vec4(-1, 1.5, 7, 6) + (vec4(2.5, a, 1, -3));
	gl_FragColor = n - m + gl_FragColor;
	vec4 k = vec4(bool(int(0)), h, l.x, i - float(h));
	gl_FragColor = k;
	gl_FragColor = vec4(j, 11.0, i, g) * vec4(0, -1, h, g).brga;
	int c = -1;
	int e = b * f.y;
	gl_FragColor = vec4(-0.75, int(e), -0.75, float(d)) * (vec4(c, b, 0, a));
}
