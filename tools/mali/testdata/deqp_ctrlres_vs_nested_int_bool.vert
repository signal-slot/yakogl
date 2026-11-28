attribute vec4 dEQP_Position;
attribute float m;
bool a = 1.5 * float(1) < -12.75 * float(-0.25);
const float b = (vec2(-10, 0.25)).y;
const float c = (vec2(-7.5, b)).r;
uniform mediump int d;
float e = b;
const int f = (-1) * int(true);
float g = -13.0 + float(4.5) + vec2(f, 7.25).y;
attribute float h;
attribute vec4 i;
const float j = 8.0;
uniform mediump float k;
int l = int(-4) * int(1.0) - (-9);
varying mediump vec4 v_color;

void main()
{
	gl_Position = dEQP_Position;
	v_color = (vec4(1, d, float(m), d));
	if ((l <= int(k)))
		if (a)
			e = (c + float(j));
		else 
			g = i.qt.s;
	else if (h > g * float(f))
		v_color = vec4(e, false, d, int(0)).xwzy;
	a = c - c + 0.001 >= b;
}
