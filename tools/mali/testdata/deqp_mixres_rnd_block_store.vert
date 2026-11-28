attribute vec4 dEQP_Position;
const int m = 0 + 7;
const int n = -3;
float o = vec3(m, -7, false).g;
ivec4 p = (ivec4(0.5, true, n, true));
const bool r = float(0.75) + 0.001 >= float(-2);
attribute float s;
uniform mediump int t;
attribute float u;
varying mediump vec4 h;

void main()
{
	gl_Position = dEQP_Position;
	if (int(u) < t)
		p = ivec4(7, t, n, o).arbg;
	ivec4 q = ivec4(float(s), r, 6.0, bool(1));
	{
		p = q;
	}
	h = (vec4(o, n, m, true));
}
