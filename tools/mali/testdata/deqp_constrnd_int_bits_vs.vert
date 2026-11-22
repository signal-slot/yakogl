attribute vec4 dEQP_Position;
attribute float a;
uniform mediump float b;
attribute vec4 d;
int e = -4 - 7 - (int(-6.0));
const int f = int(vec4(7.0, 9.5, 3.75, 8).w);
const int g = (-7 * int(1.0));
vec3 h = vec3(-8, -10, true) * vec3(-0.25, 0.5, 1.5) - vec3(-3, -4.5, 2.25) * vec3(-1.0, true, 1.0);
attribute vec3 i;
int j = int(-4) - 1 - ivec2(-4.0, f).s;
uniform mediump int k;
const bool l = int(1) * 1 == int(int(1.0));
const bool m = bvec4(l, l, 0, 1.0).garb.r;
varying mediump vec4 v_color;

void main()
{
	gl_Position = dEQP_Position;
	v_color = (vec4(-6, 2.5, -3.5, 22) * vec4(-1.0, j, 2, 0.25));
	if (l)
		ivec4(-13.0, -23, -16.0, -5.5) - ivec4(g, e, -5.5, j) + ivec4(2.75, -2, m, -0.5).wzxy - ivec4(-4, -13.25, -19.25, -3.25) * ivec4(l, 1, 1.0, 1) * ivec4(m, float(1.0), m, l);
	if (k - g <= j)
		if (false)
			vec3(float(-3), (j), b) - (i) * h.xzy;
		else 
			e = int(-11) - (g);
	v_color = v_color.xzwy;
	int c = (f) + e;
	v_color = d - v_color - vec4(c, b, -6.5, a).gbar;
}
