/* dEQP-GLES2.functional.shaders.random.all_features.fragment.14: the fragment shader,
 * verbatim from the .qpa of tools/mali/deqp-host-measure.sh (VK-GL-CTS
 * 93bca018, Apache-2.0; HOST-ONLY run at cc89a1cf). Shape mainbuiltin of
 * docs/notes/mali-deqp-negzero.md. */
precision mediump float;
uniform sampler2D a;
varying mediump float b;
const int c = 0;
uniform mediump float d;
varying mediump float f;
const float g = float((ivec2(-9, true).x)) * float(float(inversesqrt(4.0)));
varying mediump float h;
varying mediump float i;
const int j = (ivec4(-2, 4, true, g) + ivec4(5.0, true, c, c)).s + -4 * (ivec3(true, g, true)).p;
const bool k = (bvec4(0.25, 0.0, c, 1.0).stqp).pqts.b;
varying mediump float l;
const bvec2 m = bvec2((ivec3(-4, k, c).pt).y, float((abs(log2(float(18.625)) - (6.25)) < 0.001) == true));
int n = (ivec2(-4, true) + ivec2(8, g)).r * (ivec3(true, c, 1).sp).t;
varying mediump vec4 o;
varying mediump float p;
vec3 q = vec3(k, float(int(-7)), (int(4.5) > int(3))) * sqrt((vec3(k, 6.5, j) + vec3(12.25, -5.5, 12)));
vec3 r = vec3(-2.5, -2.5, -14) - vec3(6.5, 3, -6.5) - vec3(float(-7.5), -1.0, float(-6.5)) - (vec4(g, -1.0, -7, -3.75)).gab + vec3(sqrt(log2(40.875)), c - -3 - j, int(true) - int(-2.75) > 5 + j);
const bool s = (bvec4(0.5, true, 0, c)).agb.yzx.s;
const vec3 t = vec3(float(float(-8.25)) + log2(64.0) + sqrt(float(0.25)), (ivec4(5, -7.0, -3, 1)) - ivec4(3.5, 5, 3.0, -6.25).ywxz != (ivec4(-3.5, -1, 4, 5.0)).wzxy, sqrt(vec4(c, 2.25, 5.25, 12.75).wzxy).q);
vec3 u = vec3((((3))), (5 + int(-3) * 1), log2(vec4(17, s, j, k).rgba.s));
const vec3 v = (vec3(1.25, 11.25, 26.0) + vec3(-1.75, s, c)).rgb * vec3(cos((float(c))), -1 * 1 * ivec2(1.0, s).s, 4.75 * float(-1) - -4.5);
uniform mediump int x;
const float y = 2.625 - (float(0 + 0));
uniform mediump int z;
uniform mediump int aa;
float ab = (vec4(-8, y, -2, k) - vec4(0.5, y, 4.5, s) + (vec4(2.5, j, s, j))).x;
float ac = log2(float(int(-5.5 + float(7.5))));
float ad = float((int(-8.0)) - int(int(-8)) - int(exp2(float(0.0))));
vec3 ae = (vec4(6.625, 0.25, -8.125, y) - vec4(6, 0.5, -8.0, -4.75)).xyz * tan(asin(vec3(0.375, -0.5, 0.0) * vec3(-1.5, 1.25, s)));
float af = log(float(int(54.25)) + vec3(c, s, 3.75).p) - (sqrt(float(6))) * vec3(true, float(-0.25), g).t;
vec3 ag = vec4(-8.25, k, s, -6).spq - vec4(2.0, j, -5.5, 1.5).zxw + vec3(float(-6.5), float(5), float(5.0)) * (vec3(0.5, -1, 1.5)) + ((vec3(2.5, 0.0, -5.5)) + (vec3(6.0, 6.5, 5.75)));
float ah = float((-5.0) - vec2(7.5, s).s) - float(1);
float ai = ((vec2(g, 11.5).t + (-3.0)));
float aj = float(vec4(4.5, y, k, g).x) * exp2(vec4(-1, s, j, 2.75).x) * vec3(c, int(s), 0.75).z;
int ak = ivec4(g, 4, -6.75, c).xyz.sp.t * int(sqrt((1.0)));
uniform mediump float al;
float am = (tan(vec2(float(-1.125), s)).x);
const float an = 2.0;

void main()
{
	gl_FragColor = vec4(int(c) < int(c), float(b) <= float(s) + 0.001, log2(ai), 4.875).tqps * vec4(s, float(7) * float(-1.0) - -6.0, int(3.0) + j < int(2.0), (int(-8.5) - int(n)));
	if (s)
		if (ivec3(c, ak, -3) * ivec3(1, z, s) * (ivec3(s, z, s)) - ivec4(ad, j, ab, h).bar.brg != ((ivec3(c, s, an))) * (ivec3(2.0, -3, an) + ivec3(-1.0, f, j)))
			ag = (sin(sqrt(vec3(0.25, 1.0, b) * ag)));
		else 
			gl_FragColor = vec4((gl_FragColor.y + (float(af))), ivec3(ivec3(-2.0, 2.0, j) != ivec3(f, n, false), n * c, float(ah) + float(aa)).z, float(bool(z)), vec2(l > 2.75, vec2(p, af) != vec2(-0.25, ai)).y);
	if ((1 < ((int(ac)))))
		r = (vec3(am, an, am)) * (vec3(s, 2, aa)) - vec3(ivec2(ai, al).t, float(int(-7.0)), float(ak)) - (vec3(float(af) + 0.001 >= float(aj), l + ai, ah));
	else 
	{
	}
	float e = ah;
	vec3 w = ag + sqrt(vec3(af, ac, 8) + u) * atan(tan(ae));
	if (z * int(ivec4(ad, -1, ac, ab) != ivec4(aa, 4.75, 2.0, k)) <= j)
		if (k)
			w = exp2((vec3(z, y + float(-1), -10.75 + float(x))));
		else 
			q = v + u * t * vec3(-0.75, s, -1.25) + r;
	else if (tan(tan(sin(d))) + 0.001 >= q.g - h - p)
		e = float(17) * o.yzx.x + float(n);
	gl_FragColor = vec4(m.s, asin(vec2(l, k).x), (ivec3(j, i, h)).t, g) - vec4(f, e, -7.7, d).spqt.sqpt.pstq;
	gl_FragColor = texture2D(a, atan(asin(vec2(0, c))), tan(sqrt(cos(b))));
}
