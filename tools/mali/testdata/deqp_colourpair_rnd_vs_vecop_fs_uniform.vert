attribute vec4 dEQP_Position;
const float h = (float(4.0) - 0.5);
const float g = (float(5.0)) + (float(-2.5));
attribute float f;
const vec4 e = (vec4(-2.0, -7.0, -1.5, -0.5) - vec4(0.5, -6.0, -7.5, 5.0));
const float d = (float(4.625) * -0.75);
attribute float c;
varying mediump vec4 b;

void main()
{
	gl_Position = dEQP_Position;
	b = vec4(5.25, h, g, f) - e - vec4(float(7.75), 5.5, float(d), float(c));
}
