precision mediump float;
vec4 g = (vec4(0, -8, 7, -7.0));
const vec4 f = (vec4(-0.5, -8, 6.5, -8));
vec4 a = vec4(16, false, 4.5, false) * vec4(false, false, -10, -2);
varying mediump vec4 b;
const bool c = int(2) >= int(3);
uniform mediump int d;
varying mediump float e;

void main()
{
	gl_FragColor = g - f;
	if (bool(int(false)))
		gl_FragColor = gl_FragColor;
	else 
		a = vec4(int(e), d, true, c);
	a = b;
}
