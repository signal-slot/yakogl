precision mediump float;
uniform sampler2D a;
varying mediump float b;
const bool c = (bvec2(1.0, 0).x);
uniform mediump vec4 d;
bool e = bool(c);
bvec4 f = bvec4(bool(float(0.5)), float(0.0) * float(1), float(-5) - float(-6.0), c);
const bool g = (bvec4(1.0, 0, 0.75, 0)).w;
const int h = -4 - -5;
bvec4 i = bvec4((c), bvec3(g, false, h).t, bool(1), true);

void main()
{
	i = f.yxwz;
	f = bvec4(1, h, 0, g).sqpt.stqp;
	e = false;
	gl_FragColor = d;
	gl_FragColor = texture2D(a, vec2(c, b));
}
