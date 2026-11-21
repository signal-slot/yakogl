attribute vec4 dEQP_Position;
const int d = int(6.0) - -3 + int(-8.0) * int(1.0);
float c = float(2) + 1.75 * float(-1);
attribute float b;
const int a = 0;
varying mediump vec4 v_color;

void main()
{
	gl_Position = dEQP_Position;
	v_color = vec4(bool(d), float(b), c, int(b)) * (vec4(0, 0, 1, a));
}
