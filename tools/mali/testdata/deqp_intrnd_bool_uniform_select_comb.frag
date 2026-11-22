precision mediump float;
varying mediump float a;
const int b = (ivec2(2.0, true)).r;
const float c = vec4(b, 2.75, -1.5, b).rgba.z;
const vec4 d = vec4(9.0, 8, 8.5, 1) + vec4(-1.5, -5.5, -6.5, -2.5) + (vec4(-1, -0.75, -3.0, 5.0));
varying mediump float e;
const bool f = bool(int(0) - int(-1));
vec4 g = vec4(0.75, 4.0, -1.75, 11.0) * vec4(-1.5, 1.5, 1.0, -0.5) + (vec4(2, -6.0, 2.5, 5.75));
const int h = -5 * 1;
uniform mediump float i;

void main()
{
	gl_FragColor = (g).rbag;
	{
	}
	if ((bvec3(e, i, i).t))
		if (0 <= h - int(-5.0))
			gl_FragColor = gl_FragColor;
		else 
			gl_FragColor = g.yzwx;
	else if ((bvec3(f, e, e)).g)
		gl_FragColor = (d + vec4(-6, c, b, a));
}
