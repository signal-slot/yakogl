attribute vec4 dEQP_Position;
attribute vec2 v;
const int u = -5;
float t = vec4(-9.75, -5.75, false, -5.0).a;
float s = float(int(-4.0));
uniform mediump float r;
varying mediump float q;
varying mediump float d;
varying mediump float e;
varying mediump float j;
varying mediump float m;

void main()
{
	gl_Position = dEQP_Position;
	q = v.x;
	d = float(q) * float(-1);
	e = float(u) - t;
	j = s * -0.5;
	m = r;
}
