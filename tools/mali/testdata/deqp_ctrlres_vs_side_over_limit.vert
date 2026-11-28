attribute vec4 dEQP_Position;
uniform mediump vec4 o;
const vec4 m = vec4(float(-0.5), (1), 1.0 + -1.5, float(8.5) + float(-7));
uniform mediump float l;
const float k = 2.0;
const int j = 11 + -8 - ivec2(3, -8).r;
varying mediump vec4 c;
varying mediump float g;

void main()
{
	gl_Position = dEQP_Position;
	c = vec4(0.5, 0, 13.5, -4.0) * vec4(-2, -0.25, -0.5, -1.25) - vec4(-3.0, -2.0, 4.75, -7.5).rgab;
	g = float(float(-0.75) + 6.75);
	if (l > float(1.5) * float(true))
	{
		vec4 p = (vec4(-10.0, 4, -0.75, -4.5)) - vec4(1.5, -6.0, -4, 4.25).tsqp;
		vec4 n = (vec4(g, 3, -17.5, -5.75) + vec4(4, l, 8, 7.75));
		p = vec4(bool(j), float(4.5), -4.25, float(0.0)) - p;
		c = o.pqts + n * m;
		c = vec4(float(l) * 0.75, k, ivec2(j, true).g, j * 1);
	}
}
