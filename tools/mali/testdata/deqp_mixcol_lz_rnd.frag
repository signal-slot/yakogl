precision mediump float;
varying mediump float a;
varying mediump float b;
const bool c = (ivec4(-1.0, 3.0, -5.0, 0) == ivec4(2.25, 9.0, -3.0, 8.75));
const bool d = bool(int(-1) < int(c));
const bool f = ivec3(8.25, true, 0).b >= 0;
varying mediump float g;
const bool h = false;
int i = 1;
const float j = (0.0) + (4.5);
varying mediump float k;

void main()
{
	vec4 e = vec4(k, float(-4) + float(j), bvec4(0, 0.0, i, g).y, vec4(h, g, a, f).x);
	gl_FragColor = e.sptq * (vec4(d, c, b, a));
}
