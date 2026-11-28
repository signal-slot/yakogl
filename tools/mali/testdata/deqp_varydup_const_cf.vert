attribute vec4 dEQP_Position;
const bool k = vec4(-0.5, float(0.5), float(-0.5), -0.5) != vec4(-8.5, 6, -7, -1.5) - vec4(-8, 5.5, -6.5, -1);
const int l = 0;
float m = -2.5 + (3.0);
const float n = vec2(9.5, -0.25).g - vec4(true, l, 0.0, l).p;
const int o = int(-5.0 - -6.0);
uniform mediump float p;
attribute vec4 q;
attribute float r;
const float s = vec3(n, n, -5.5).rbg.y;
attribute float t;
const int u = 0;
const float aa = float(float(-10.0)) + (float(7.5));
const float z = float(-2.5) * float(1) - (float(-3.0));
attribute float y;
const float x = vec2(true, 8).st.t;
float w = aa * 1.25 - float(float(-3.5));
const float v = vec3(u, 7.5, n).y;
varying mediump vec4 j;

void main()
{
	gl_Position = dEQP_Position;
	j = (vec4(1.0, int(z), o, float(k)));
	{
		if (float(int(u)) + 0.001 >= (t))
			w = vec2(3.0, w).r - (2.5);
		else 
			m = -4.5 - float(1.5) - -6.5;
		if (k)
			j = vec4(aa, false, s, -4) * vec4(-2, z, z, 1.75) + vec4(y, x, r, w).sqpt;
		else 
			m = (8.0) - v;
	}
	if (bvec3(m, true, u).t)
		m = m;
	else if (t + s < -0.375 * r)
		j = (q);
	else 
		j = vec4(m, 1.25, 0, p) * vec4(o, n, false, 0.25) * vec4(m, l, 1, k).barg;
}
