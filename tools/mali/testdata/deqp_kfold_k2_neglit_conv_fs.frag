precision mediump float;
const int a = -1 - -2 + int(-3);
const float b = (float(5.5)) - -5.0 * -1.0;
const float c = 1.0;
varying mediump float d;
const float e = (float(float(-7)));
const int f = -6 + 7 - int(float(-5.0));
const float g = ((-2.5));
const int h = -6 + 0 + int(int(3));
const bool i = true;
uniform mediump float j;
int l = -4 - (int(1));

void main()
{
	int m = a;
	int k = m - l;
	gl_FragColor = vec4(true, k * 1, j, i);
	gl_FragColor = vec4(11.0, h, -8.0, g) - vec4(f, e, d, -1.5) - vec4(2.375, 6.25, 4.125, c) * vec4(2, b, -1.25, a);
}
