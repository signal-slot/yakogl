precision mediump float;
varying mediump vec4 e;
int a = int(float(4.0)) - 3;
const bool b = float(8.0) - float(5.0) <= float(2.0) - float(-0.5) + 0.001;
varying mediump float c;
vec4 d = (vec4(-7.0, 6, -6.75, -2.25)) - (vec4(-8, 6.5, -6.5, -2));

void main()
{
	gl_FragColor = e * d * vec4(0, 2, -0.5, 0.25);
	d = gl_FragColor;
	gl_FragColor = (vec4(bool(c), c, int(b), a));
}
