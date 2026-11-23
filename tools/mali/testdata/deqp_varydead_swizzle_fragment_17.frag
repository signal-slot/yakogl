precision mediump float;
varying mediump vec4 g;
varying mediump float f;
varying mediump float e;
int a = int(-4 - -4);
const bool b = bvec4(false, 0, false, 0.5).pqst.y;
varying mediump float c;
vec4 d = (vec4(7.0, -8, 9, 20)) + vec4(2.0, -18, 26, 4.5) * vec4(1, -0.5, -0.25, -1.0);

void main()
{
	gl_FragColor = g + vec4(-1, e, f, a) + d * vec4(1.25, e, 0.75, true);
	d = gl_FragColor;
	gl_FragColor = (vec4(bool(c), c, int(b), a));
}
