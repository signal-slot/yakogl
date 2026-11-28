precision mediump float;
varying mediump vec4 e;
int a = int(int(bool(1)));
const bool b = bool(0.75 - float(0));
varying mediump float c;
vec4 d = (vec4(-7.0, 6, -6.75, -2.25)) - (vec4(-8, 6.5, -6.5, -2));

void main()
{
	gl_FragColor = e * d * vec4(0, 2, -0.5, 0.25);
	d = gl_FragColor;
	gl_FragColor = (vec4(bool(c), c, int(b), a));
}
