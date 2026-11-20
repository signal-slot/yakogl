precision mediump float;
const int d = int(6.0) - -3 + int(-8.0) * int(1.0);
float c = float(2) + 1.75 * float(-1);
varying mediump float b;
const int a = 0;

void main()
{
	gl_FragColor = vec4(bool(d), float(b), c, int(b)) * (vec4(0, 0, 1, a));
}
