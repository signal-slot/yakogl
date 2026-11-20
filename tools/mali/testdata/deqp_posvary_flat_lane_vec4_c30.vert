attribute vec4 dEQP_Position;
uniform mediump float y;
const float x = 12.0 + float(-7.5) * float(1.0);
const float w = float(-3.5) + (6.0);
uniform mediump float v;
const float u = float(-7.5) + float(7.0) + (0.75);
attribute float t;
const float s = (-5.75) - (float(-4.5));
attribute float r;
attribute float q;
varying mediump float a;
varying mediump float b;
varying mediump vec4 d;
varying mediump float e;
varying mediump float f;
varying mediump float i;
varying mediump float n;

void main()
{
	gl_Position = dEQP_Position;
	a = y;
	b = float((float(u)));
	d = vec4(float(q) * s, x - float(w), v, u);
	e = t * float(s) - r;
	f = float(1.0);
	n = float(1.625 - float(q));
	i = float((float(-2.5)));
}
