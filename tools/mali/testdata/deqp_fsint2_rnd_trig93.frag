precision mediump float;
varying mediump vec4 i;
const int a = 1 * int(true) + int(int(-1.0));
varying mediump float b;
varying mediump float c;
const int d = ivec4(a, a, true, true).zxy.r;
int e = int(true) * d - int(float(7));
int g = 3;
varying mediump float h;

void main()
{
	gl_FragColor = i;
	int f = ivec2(true, h).y - int(g);
	gl_FragColor = vec4(f - d, (e), gl_FragColor.g, -4 * d);
	gl_FragColor = asin(vec4(c, c, b, -0.0)) * tan(vec4(a, 1.0, 0, -0.375));
}
