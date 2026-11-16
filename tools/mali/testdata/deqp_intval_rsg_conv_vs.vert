attribute vec4 dEQP_Position;
const vec4 a = vec4(float(6) + float(-6.5), int(-1.0) + int(4), bool(int(true)), int(-6));
attribute vec4 b;
const float c = 9.25;
const int d = int(-2);
float e = -1.0;
const float f = -4.5 * 1.0 + 3.5 * float(true);
bool g = true;
const int h = -6;
int i = 1 * (int(1.0));
attribute vec4 j;
varying mediump vec4 v_color;

void main()
{
	gl_Position = dEQP_Position;
	i = int(bool(1)) * i;
	e = float(float(-1.25) - e);
	v_color = j * vec4(int(e), e, i, bool(true));
	e = 10.375 + float(h) - (2.75);
	g = g;
	v_color = vec4(f, e, d, c) * b + (a);
}
