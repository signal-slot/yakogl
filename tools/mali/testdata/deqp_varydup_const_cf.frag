precision mediump float;
uniform mediump vec4 b;
float c = (vec4(16, true, -5.75, false)).b;
const float d = float(-9.5) - float(-7.0) * float(1);
const int e = -4 * ivec4(true, d, true, true).p;
const vec4 f = vec4(-13.5, 11.0, 1.75, 3.0) - vec4(0.5, 1.0, 1.5, -2) + (vec4(-2.0, -7.0, 2.5, -1.0));
const bool g = bvec2(bool(true), int(1.0)).g;
const int h = -1 * 1;
const vec4 i = vec4(-0.25, 0.25, 1, 4) * vec4(g, -1, h, 0.25) * (vec4(-1.0, 1.0, g, true));
varying mediump vec4 j;

void main()
{
	gl_FragColor = j;
	vec4 a = i * vec4(-1, h, true, g).brag;
	a = f * a - vec4(e, 0.5, float(d), c);
	a = a * (b);
}
