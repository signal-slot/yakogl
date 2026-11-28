attribute vec4 dEQP_Position;
const float w = (2.5 + 0.5);
attribute float r;
attribute float s;
uniform mediump int t;
attribute vec4 u;
float v = 1.0 * float(2.0) + w;
varying mediump float a;
varying mediump float c;
varying mediump float e;

void main()
{
	gl_Position = dEQP_Position;
	a = float(int(w) - 3);
	e = float(int(float(v)));
	c = u.sqpt.b;
	if (float(t) + s > r - -2.0)
		a = 0.25;
}
