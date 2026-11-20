attribute vec4 dEQP_Position;
vec4 v = vec4(5.5, -5, 2, 2) * vec4(0.5, 1.0, 2, -1.5);
uniform mediump vec4 u;
attribute float t;
const int s = int(-3) * 1;
ivec4 i = (ivec4(-7, -3.0, 5, 3.75));
const int k = (4);
const float l = vec2(4, false).x;
ivec4 n = ivec4(8, 11.5, -15, -12.5) * ivec4(true, 1, 1, true);
const int o = (int(1.0));
const bool p = l + 0.001 >= float(0);
int q = ivec2(p, o).t;
attribute float r;
varying mediump vec4 a;
varying mediump float b;
varying mediump float d;

void main()
{
	gl_Position = dEQP_Position;
	a = v - u;
	b = t;
	d = float(float(s));
	if (int(r) <= q)
		b = float(p) * float(o);
	ivec4 m = (n);
	ivec4 j = (m);
	if ((abs(l - (float(k))) < 0.001) == true)
		j.barg - i;
	else 
		b;
}
