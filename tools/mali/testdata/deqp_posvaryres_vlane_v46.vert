attribute vec4 dEQP_Position;
uniform mediump float a;
attribute float b;
const float c = float(6.0) * float(1.0) * float(float(0.5));
attribute vec4 d;
vec4 e = vec4((float(-4.5)), float(c) + 4.7, float(float(10.0)), 4.75 - float(-0.950001));
attribute vec4 f;
attribute float g;
attribute float h;
uniform mediump float i;
uniform mediump float j;
uniform mediump float k;
varying mediump vec4 v_color;

void main()
{
	gl_Position = dEQP_Position;
	v_color = (vec4(c, a, k, 3.1) + e);
	e = vec4(h, j, i, h) * vec4(g, -0.25, g, g) + (f);
	v_color = e;
	v_color = v_color;
	v_color = d;
	v_color = vec4(float(a), c - float(b), a, 0.5 * float(a));
}
