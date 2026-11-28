precision mediump float;
uniform mediump vec4 a;
const bool b = (bool(float(1)));
const bool c = bool(1);
const int d = int(2);
varying mediump vec4 e;
varying mediump vec4 f;
varying mediump vec4 g;
varying mediump vec4 h;
int i = int(int(int(4.0)));
const float k = float(3 + int(-2));
int l = (int(d)) - 3 * int(1.0);
int m = d + d - (int(c));

void main()
{
	float j = 2.25 - float(8.0) + float(i);
	m = l + int(true);
	l = int(float(4.0) * float(b));
	m = int(k - j);
	gl_FragColor = vec4((float(m)), 10 - int(i), l * int(k), j);
	i = i * 1 - 0 * 1;
	gl_FragColor = h - g;
	gl_FragColor = f;
	gl_FragColor = e * vec4(d, 0.25, c, b) * a;
}
