attribute vec4 dEQP_Position;
uniform mediump float v;
attribute float u;
float t = float(-2);
vec4 s = vec4(4.0, -0.0, 0.125, -0.125) * vec4(0.5, -0.75, -2, 2.0);
attribute float r;
const float q = vec4(4.75, -3.5, false, -5.5).q;
bool k = 4 <= int(4);
vec4 m = (vec4(true, 0, 0.0, 0));
attribute float n;
uniform mediump int o;
ivec3 p = ivec3(-1.25, 0.75, 1) * ivec3(1.0, 1, 1);
varying mediump float j;
varying mediump vec4 c;
varying mediump vec4 d;

void main()
{
	gl_Position = dEQP_Position;
	j = (v);
	c = vec4(u, 1.0, t, -0.25) * s;
	d = vec4(-6, 7.5, r, float(q));
	if (p != ivec3(n, -2, k))
		j;
	int l = ivec3(o, true, n).x;
	if (3 >= l)
		c = m.yxwz;
	if ((true))
		k = 2 <= l;
}
