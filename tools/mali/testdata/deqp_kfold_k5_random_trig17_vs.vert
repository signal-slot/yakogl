attribute vec4 dEQP_Position;
attribute vec4 g;
attribute float f;
attribute float e;
int a = int(-4 - -4);
const bool b = bvec4(false, 0, false, 0.5).pqst.y;
attribute float c;
vec4 d = (vec4(7.0, -8, 9, 20)) + vec4(2.0, -18, 26, 4.5) * vec4(1, -0.5, -0.25, -1.0);
varying mediump vec4 v_color;

void main()
{
	gl_Position = dEQP_Position;
	v_color = g + vec4(-1, e, f, a) + d * vec4(1.25, e, 0.75, true);
	d = v_color;
	v_color = atan(vec4(bool(c), c, int(b), a));
}
