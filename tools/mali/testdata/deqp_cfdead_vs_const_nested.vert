attribute vec4 dEQP_Position;
const int u = int(-3 + 5);
attribute vec4 v;
const vec4 w = vec4(float(-5.0), 0.5, 0, float(3.5)) - (vec4(-4, -3.5, 0.5, 3));
varying mediump vec4 a;
varying mediump float f;
varying mediump vec3 h;
varying mediump float i;
varying mediump vec4 j;
varying mediump float n;
varying mediump float m;

void main()
{
	gl_Position = dEQP_Position;
	a = vec4(-0.25, 0.625, 0.625, -0.25) * vec4(-1.0, 1.25, 2.0, u) * vec4(true, -1, -1, -0.75).rbag;
	f = 7.0;
	h = (vec3(4, -7.0, 6.0)) + vec4(-5.0, u, true, f).zwx;
	i = vec3(int(f), float(-4.5), int(true)).g;
	j = ((vec4(false, false, 0.25, 0.25)));
	n = (6.5) * float(int(1));
	m = (float(-1.125)) * 1.25;
	if (-2 + 5 > int(9) - 7)
		if (float(1.375) * float(1.25) <= n * float(false) + 0.001)
			j = vec4(-0.5, 1.5, m, 9) - w + v;
	m = (float(u)) - n;
}
