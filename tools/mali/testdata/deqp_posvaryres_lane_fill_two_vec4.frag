precision mediump float;
varying mediump vec4 a;
const vec4 b = vec4(int(-5), 6.0, float(0.5), -2.0) * (vec4(0.5, 0.75, -1.0, -1.0));
const float c = -2.5;
const float d = ((float(-4)));
vec4 e = vec4(-0.5, true, -12.0, -1.0) + vec4(5.0, -0.5, 7, -4.5) - vec4(2.5, -4, 1, float(-1.0));
vec4 f = vec4(float(-4.375), -8.0, float(-8.5), -20) * vec4(float(1.5), true, -1.0, -0.5);
varying mediump vec4 g;

void main()
{
	gl_FragColor = g;
	gl_FragColor = f + e;
	true;
	gl_FragColor = gl_FragColor + vec4(float(d), 4, float(c), -2.0);
	gl_FragColor = gl_FragColor - b - a;
}
