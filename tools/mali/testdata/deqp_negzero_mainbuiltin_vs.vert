/* dEQP-GLES2.functional.shaders.random.all_features.vertex.13: the vertex shader,
 * verbatim from the .qpa of tools/mali/deqp-host-measure.sh (VK-GL-CTS
 * 93bca018, Apache-2.0; HOST-ONLY run at cc89a1cf). Shape mainbuiltin of
 * docs/notes/mali-deqp-negzero.md. */
attribute vec4 dEQP_Position;
const int a = int((float(true) + 7.0)) + ivec4(-7.0, bool(1), float(5.25), int(-8.0)).gar.z;
const bool b = (bvec4(true, 1.0, false, 0.5).tqsp).ptq.x;
ivec2 c = ivec2(-1, -1);
attribute vec2 d;
attribute float e;
attribute float f;
const bool g = bvec3(float(3.25) + 0.001 >= float(true), (int(0.0)), tan(float(0.75))).brg.z;
uniform mediump float z;
ivec2 y = (ivec2(vec4(16, a, g, 9.0).x, float(10))) - (ivec2(8 * 1, log(float(257.0))));
int x = (ivec4(-10.0, 8.0, 2.0, -3.25)).r + ivec4(b, b, 3, -15).xwzy.p - (ivec4(true, a, -14, -8.0).zwx).t;
int w = (ivec3(-10.0, -6.0, true) * ivec3(true, true, a) * (ivec3(a, 1.0, a))).t;
int v = int(-10) * 1 + ivec2(true, a).s - ivec4(-7.5, a, -7, -3.5).p * int(bool(1)) - ivec3(int(-1.0) - -5, acos(float(0.5)), ivec2(-2, 2.0) != ivec2(3.0, 5)).x;
const ivec4 u = ((ivec4(1.0, b, a, -9) + ivec4(0, -1.0, g, 8.0))) * ((ivec4(-4, -2.0, -3, -7.0) + ivec4(5, 3.0, 4.0, 8.0)));
const bool t = bvec3((log2(2.0)), a, asin(atan(float(0.75)))).y;
const int s = int(-1.0);
const float r = -5.0;
const float q = log2(((vec2(32.0, false).x)));
attribute float p;
const int o = int(10.0) * int(1) - (a) - -7 + u.x * a;
const bvec2 n = bvec2((atan(float(3.5) + float(-2))), tan(exp2(vec3(g, -1.0, false).g)));
const ivec2 m = ((ivec3(-2, r, -9.0).br)) - (ivec2(float(-7.0), false)).rg;
const float l = 2.0;
const bool k = bvec2(cos(float(-0.0) * float(-2)), float((1))).r;
int j = (int(int(6) + int(2.0)) * 1);
const int i = int(inversesqrt(float(-4.25) + float(4.5))) - int((q) * -1.0);
const ivec4 ae = ivec4(r, -3, true, 6.0).wzyx * (u) + (ivec4(l, r, 0.0, a).bgra) - ivec4(int(-2.0) + -1 + ivec3(t, l, r).p, acos(exp2(float(s))), (2 - int(-5)), ((a)));
const ivec4 ac = ivec4(float(9.375) * float(-1.75), s == int(s), ivec2(1.0, false) == ivec2(b, false), l * l).barg - ae;
const float ab = ((float(float(-20) + 7.5)));
const float aa = 0.5;
float af = acos(asin(sin(vec4(0, 0, 0, aa)))).r;
float ag = r;
const ivec3 ah = ivec3(ac.zxw.t, int(int(a) < 8), sin((aa))) * (ivec3(i, l, i).rgb).tps;
const int ai = int(exp(float(0)) * vec2(a, -2.0).t * float(vec2(-2, false).s));
varying mediump vec4 v_color;

void main()
{
	gl_Position = dEQP_Position;
	v_color = vec4(w * (int(s)) * -16, int(float(ai)), (log2(inversesqrt(0.625))), ah.pts.rb.y);
	af = 6.0;
	int ad = (ivec3(0.0, k, ai).s * x - -6);
	x = int(vec3(o, f, q).br.rg.x);
	ivec2 h = (ivec3(j, t, w) - ivec3(ai, i, k) + ah.tsp).gb;
	if (sin(log2(exp(float(b)))) + 0.001 >= ag + af)
	{
		x = (ae.x) - (ad) * 1;
		v = ((ac - ivec4(aa, s, s, true)).p);
		v = (int(ab * aa) * a);
	}
	else 
	{
		h = (ivec2(o, -4) - ivec2(j, -2.0) + ivec2(z, 8) - y.xy);
		j = x;
		vec3(ivec4(int(w) - v, bool(float(b)), v_color.t, u == ivec4(1, g, t, s)).xwzy == ivec4(b, int(4), true, int(a)).qpts * (ivec4(4.0, 5, r, p) + ivec4(-3.0, -4, 6.0, q)), (vec4(float(p), float(i), float(o), l).qs).s, (n).r);
		c = c - (m) * ivec2(k, f).yx * ivec2(int(k) <= int(k), bool(bool(1)));
		c = ivec3(l, k, j).zx.rg - ivec2(i, float(2.0)) - h * ivec2(1.0, 1) * ivec2(1.0, true) * (c);
	}
	v_color = atan(vec4(g, exp2(f * e), asin(float(0)) - d.x, c.y <= int(b) * a));
}
