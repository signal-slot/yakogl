attribute vec4 dEQP_Position;
const float g = -4.0 * 1.0 + 2.0;
float h = float(61.75) * float(0.5) * -0.25;
ivec3 i = ivec3((true), true, int(g) + int(true));
ivec3 j = (ivec3(-9.5, float(-10.25), g));
ivec3 k = ivec3(int(true), -9, float(-5)) * (ivec3(true, 1.0, true));
varying mediump float a;

void main()
{
	gl_Position = dEQP_Position;
	i = k - j;
	a = float(h);
	float f = g;
	f = float(-9) - f;
}
