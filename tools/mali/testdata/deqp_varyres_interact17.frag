precision mediump float;
varying mediump float c;
const float b = -4.0 + 4.5;
varying mediump float a;
float d = b - float(7);
const int e = (int(-2));
const bool f = false;
varying mediump vec4 g;
varying mediump vec4 h;
const bool i = float(5) < float(6);
uniform mediump int j;

void main()
{
	gl_FragColor = vec4(j, float(true), bool(c), e);
	if ((i))
		gl_FragColor = h - g;
	if (f)
		gl_FragColor = vec4(e, d, d, c) + gl_FragColor;
	{
		gl_FragColor = vec4(true, c, b, a).zwxy;
	}
}
