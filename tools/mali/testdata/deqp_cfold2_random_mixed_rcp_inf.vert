attribute vec4 dEQP_Position;
uniform mediump vec4 a;
int b = ivec2(int(5.25), int(6)).t;
const float d = float(2.875) * -1.0 * float(-1.0);
const bool e = bool(atan(float(false)));
attribute float f;
attribute float g;
const int h = ivec2(11, d).r - (int(5.0));
const bool i = bool(int(8.0) - int(7.0));
varying mediump vec4 v_color;

void main()
{
	gl_Position = dEQP_Position;
	v_color = vec4(i, 6, h, g).sqtp + vec4(-2.0, -4.5, -7.0, f).xwzy;
	int c = int(float(f));
	v_color = vec4(e, d, c, b).ptsq * v_color;
	v_color = atan(a.ragb);
}
