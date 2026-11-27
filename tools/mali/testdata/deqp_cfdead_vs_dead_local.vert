attribute vec4 dEQP_Position;
attribute vec3 f;
const bool g = bvec3(true, 1.0, 0.75).tps.x;
uniform mediump float h;
uniform mediump int i;
float j = vec4(4, 7.75, -3.25, -13).rgb.b;
const bool k = (float(-1)) != float(-5) + float(8);
attribute vec3 l;
attribute float m;
const bool n = (float(k)) <= float(-2.25) + float(4) + 0.001;
attribute vec4 p;
varying mediump vec4 c;
varying mediump float b;

void main()
{
	gl_Position = dEQP_Position;
	c = vec4(vec2(6.5, n).s, int(-7) - -4, ivec3(j, -5, h).y, 5);
	b = -8.0;
	int e = ivec2(k, 6).gr.s;
	if (e - int(k) < int(bool(1)))
	{
		g;
		float o = float(j) - b - 8.0 * 0.75;
		j = (o + h);
		o = vec2(e, i).s * p.a;
	}
	j = float(vec3(n, j, m) != l);
	if (vec3(k, float(j), i) == vec3(h, 3.5, g) - f)
		b = -8.75;
	int((-5)) - e;
}
