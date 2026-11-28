precision mediump float;
uniform samplerCube a;
varying mediump float b;
uniform mediump int d;
const ivec2 g = ivec2(int(true) * int(true), (int(-6.0)));
const bool h = (bvec2(1, 1.0)).x;
varying mediump float k;
const bool l = bool(h);
const int m = (ivec4(h, -7.75, true, -11)).a;

void main()
{
	ivec2 i = ivec2(1.0, true).xy.yx;
	ivec3 j = ivec4(m, l, bool(d), bool(1)).bgr;
	float c = float((float(k)));
	i = j.yz * (i);
	ivec3 f = (ivec3(d, 7, h));
	ivec2 e = g;
	e = f.gr;
	gl_FragColor = textureCube(a, vec3(d, c, int(b)));
}
