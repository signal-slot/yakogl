attribute vec4 dEQP_Position;
attribute vec4 h;
attribute vec4 g;
const vec4 a = vec4(float(-8) - float(-7.5), int(-6.0) - int(-6.0), float(float(0)), 0);
const int b = int(-1) * int(true);
attribute float c;
attribute float d;
uniform mediump int e;
float f = (float(-7) + float(2));
varying mediump vec4 v_color;

void main()
{
	gl_Position = dEQP_Position;
	v_color = (h - g);
	v_color = (vec4(f, e, d, b)) + v_color;
	v_color = vec4(-1.75, bool(c), b, -1.25) * a;
}
