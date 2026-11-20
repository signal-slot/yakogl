precision mediump float;
const vec4 m = vec4(float(19.25), -40.0, 1.375, -8) * vec4(float(-1), float(0.25), 2.0, 1.0);
const bool b = true;
uniform mediump int c;
const vec4 d = (vec4(-1.0, 0.625, -1.75, -0.25)) * (vec4(-1.0, 2.0, 1, true));
const int e = -33 - int(int(2));
const float f = -3.5;
const int g = 6;
const float h = 5.0;
const int i = -2;
const float j = float(-5.75) + 2.0 - (-4.0);
const bvec4 l = ((bvec4(b, j, b, j)));

void main()
{
	gl_FragColor = m;
	vec4 k = gl_FragColor;
	bvec4 a = l;
	gl_FragColor = k;
	gl_FragColor = gl_FragColor * vec4(j, 0.75, i, c) + vec4(h, g, f, e) * d;
	a = bvec4(1.0, int(c), float(bool(b)), int(0));
	gl_FragColor = gl_FragColor;
}
