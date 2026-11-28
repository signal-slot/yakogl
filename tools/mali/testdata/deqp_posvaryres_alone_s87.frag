precision mediump float;
varying mediump vec4 b;
varying mediump vec4 c;
const float d = -1.5;
const float e = -3.125;
varying mediump vec4 f;
uniform mediump vec4 g;
varying mediump float h;
const int i = -8 * (1);
const vec4 j = vec4(5.0, -1, i, 2) - vec4(-6, 3.5, -1, -4.5) + vec4(true, float(6.0), 4.5, -6.5);

void main()
{
	gl_FragColor = vec4(-1.75, 7.25, 4.25, -5.25) + vec4(-3, -2, -6, 8) - vec4(float(-4.5), 6.5, float(-2.5), int(2));
	gl_FragColor = (vec4(i, 12, -19, 4) * gl_FragColor);
	gl_FragColor = vec4(float(15) * float(0.5), 7.5, (float(-6.5)), 4.5);
	gl_FragColor = (vec4(-15.5, -6.5, 11.5, -12.0) + gl_FragColor);
	gl_FragColor = gl_FragColor - (gl_FragColor);
	vec4 a = j + vec4(-4.5, i, h, h) - (g);
	a = f + a - vec4(-7, e, 4.5, 1.375) * vec4(1.25, 1.5, d, -1.75);
	a = c - a * b;
}
