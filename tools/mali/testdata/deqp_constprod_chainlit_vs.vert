attribute vec4 dEQP_Position;
const float d = 2.75 * float(-1) * -2.0;
attribute vec4 c;
float a = 10.25 * float(-0.5) * vec2(-1.0, false).r;
attribute float b;
varying mediump vec4 v_color;

void main()
{
	gl_Position = dEQP_Position;
	v_color = vec4(7, d, -2, -5.0).brag - c.yxzw;
	if (float(b) * -0.5 < a)
		v_color = (v_color);
}
