precision mediump float;
uniform sampler2D a;
const int b = int(4) * 1 * int(int(1.0));
varying mediump vec2 c;
ivec4 d = ivec4(2.25, 5.25, true, b) * ivec4(b, 8.25, true, b) + ivec4(-8.25, true, b, b).zxwy;
uniform mediump ivec4 e;
const float f = vec3(b, int(true), bool(true)).s;
const float g = float(int(0.0));
const int h = int(int(-3.0) + 6);
const float i = (vec3(f, g, b)).b;

void main()
{
	d = ivec4(bool(1.0), i, -3.0, h) + (ivec4(g, -3, f, -2.0));
	d = e * d;
	gl_FragColor = (texture2D(a, c, float(b)));
}
