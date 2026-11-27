attribute vec4 dEQP_Position;
uniform mediump vec4 b;
uniform mediump vec4 c;
const float e = -2.0 * float(float(0.25));
attribute vec4 f;
const int g = int(int(-11) * int(-14));
const float h = -4.0 + (float(2.0));
varying mediump vec4 v_color;

void main()
{
	gl_Position = dEQP_Position;
	v_color = vec4((-0.75), -1, -2.75 - h, asin(-0.5));
	float d = vec2(1.5, g).s * asin(-0.75);
	v_color = f * vec4(e, d, 1.0, 1) * v_color;
	v_color = v_color.psqt;
	v_color = asin((c));
	vec4 a = asin(b.yzxw);
	a = sin(v_color);
}
