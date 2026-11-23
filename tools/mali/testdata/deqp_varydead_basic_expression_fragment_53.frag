precision mediump float;
varying mediump vec4 g;
const vec4 f = vec4(-1.0, 13.5, 9.0, 6.25) + vec4(7.5, -5.5, -1.0, -3.5) + vec4(-7.5, -16.0, -7.0, -4.0) * vec4(1.0, 0.5, 1.0, 0.75);
varying mediump vec4 a;
varying mediump vec4 b;
float c = float(float(5.125)) * (float(-0.75));
const float d = (0.3) * -1.0;
float e = float(-30.55) + float(3.75) + float(float(7.0));

void main()
{
	gl_FragColor = g * (f);
	c = c;
	gl_FragColor = vec4(e, 1.15, d, c) - b - gl_FragColor;
	gl_FragColor = a;
	gl_FragColor = gl_FragColor;
}
