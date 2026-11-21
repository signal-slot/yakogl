attribute vec4 dEQP_Position;
const float j = 2.375;
const float i = (-1.25 + 1.0);
float h = float(0.75 * i);
attribute float g;
attribute vec4 b;
attribute vec4 c;
attribute float d;
attribute float e;
attribute float f;
varying mediump vec4 v_color;

void main()
{
	gl_Position = dEQP_Position;
	v_color = vec4(j, 4.0, -1.0, -1.5) * vec4(e, d, 1.0, i) * vec4(float(-0.0), float(f), h, float(g));
	vec4 a = vec4(e, f, e, f) - vec4(e, d, e, d) - c;
	a = b;
	a = b;
}
