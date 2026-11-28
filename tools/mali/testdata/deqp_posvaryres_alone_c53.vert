attribute vec4 dEQP_Position;
const float m = 8.0 * float(0.25);
attribute vec4 l;
uniform mediump float h;
uniform mediump float i;
attribute float j;
vec4 k = vec4(float(-11.5), -16.0, float(17.25), 11.5) + vec4(float(-3.25), float(5.0), -7.5, -7.25);
varying mediump vec4 g;
varying mediump vec4 a;
varying mediump vec4 b;

void main()
{
	gl_Position = dEQP_Position;
	g = vec4(11.25, 10.75, 6.5, 5.875) * vec4(-0.5, 0.5, h, h) * (vec4(-1.25, -1.25, -1.0, m));
	a = vec4(float(-1.5), -1.5, float(-4.5), float(j)) - l;
	b = k + vec4(h, float(j), i, float(h));
}
