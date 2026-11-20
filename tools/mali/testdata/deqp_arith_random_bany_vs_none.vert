attribute vec4 dEQP_Position;
const ivec4 a = ((ivec4(int(true), bool(1.0), -5.0, int(true)).wxzy));
const float b = (float(-4.0) - float(0.5) - (-6.0)) + (3.0);
attribute float c;
const vec4 d = (vec4(-35, -0.25, 0, 5) * vec4(0.25, -2.0, 0.75, -1.0)).abgr - ((vec4(-1.5, -7.0, -1.5, -4))).wzxy;
const int e = int(ivec4(ivec4(-1.0, -4.5, 2.75, -1) != ivec4(true, -3.0, b, true), int(-6) - int(b), (true), sqrt(float(9.0))).q);
const float f = float(8 - -5 + -1) * 0.5;
attribute vec4 g;
const ivec2 i = (ivec2(int(float(-2.0)) < int(-2.0) - int(-9), float(b - float(-1))));
const float j = sqrt(vec4(b, 12.25, f, false).g * log2(2.0)) - ((12.0)) * float(vec4(b, -11, true, -0.5).w);
const bool k = ((bvec3(false, 0, 1)).b);
const bool l = ivec3(1.0, k, k).yxz * ivec3(1.0, 1, k) * ivec4(e, 1.0, float(1.0), 1).gba == ((ivec3(7.0, e, 7))) - ivec3(6, 6.0, 2).xzy * (ivec3(1, true, 1));
attribute vec3 m;
const int n = int((-2)) * (int(log2(2.0)));
attribute vec2 o;
const bool p = bvec2(1, i.g != int(b)).s;
attribute float q;
uniform mediump int r;
uniform mediump float s;
const bool t = p;
const float u = (vec2(-7.75, b) * vec2(true, n) - (vec2(-6.0, t))).x;
attribute vec2 v;
int w = (ivec3(e, float(u), -16).gbr.t);
int x = -3 * 1 + int(-4) * int(1) + (10 - e) - -1;
uniform mediump int y;
attribute vec4 z;
const float aa = (-23.7);
int ac = ivec3(-14, k, l).st.r + int(float(int(7))) * 1;
const ivec3 ad = ivec3(int(log(d.g)), (vec4(7, u, l, k)).r * -1.0, acos(vec2(-0.5, n).x) + (f));
varying mediump vec4 v_color;

void main()
{
	gl_Position = dEQP_Position;
	int ab = ad.y - 7 * (int(x)) - int(vec2(ac, s).s) * ivec4(true, f, e, -5.5).wxzy.g;
	v_color = ((vec4(s, -10, ab, aa) - z + vec4(int(y), int(-5), x, 6)));
	if (t)
		v_color = vec4(w - ((int(-6))), (v * vec2(u, t)).r, log(inversesqrt(float(true)) - float(s)), c);
	if (int(cos(-2.625 - float(r))) >= ivec3(e, (q), (int(p))).y)
		v_color = vec4((n), (float(a.w)), float(o.t) * float(n), ((m).r));
	float h = vec4(ivec4(l, e, k, true).wzyx.t, log(f), j * (float(0.25)), i.yx.x).z;
	exp2(asin((log(h)))) + exp((g).z + f);
	v_color = (vec4(2.5, b, 2.75, e)).ywzx * (d) - vec4(c + float(6), (b), a.z, int(-5.0) * int(true)).bgra;
}
