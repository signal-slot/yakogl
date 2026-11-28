precision mediump float;
varying mediump float p;
varying mediump float a;
const float b = (float(7));
const int c = (int(-2.0));
varying mediump float d;
const float e = -3.5;
vec4 f = vec4(5.0, 3.5, -8.0, 12.0) - vec4(-0.5, -4.0, -0.5, 7.5);
const float g = (float(-1.0));
const int h = -3 + 2;
float i = float(bool(1));
float j = -6.0 + 6.5;
const bool k = b > float(9.0);
varying mediump float l;
varying mediump float m;
const bool n = k;
varying mediump float o;

void main()
{
	gl_FragColor = vec4(int(-6), p, float(6.5), float(-5.5));
	if (ivec4(o, true, n, 1) == ivec4(m, l, k, 1))
		f = vec4(j, i, h, g) - f;
	else 
		gl_FragColor = vec4(e, d, 7.5, -2.0) + vec4(c, 4.5, -1, -3.5);
	gl_FragColor = gl_FragColor + vec4(6, b, -6.25, a);
}
