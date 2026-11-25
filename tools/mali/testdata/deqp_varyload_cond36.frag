precision mediump float;
varying mediump float a;
const int b = ivec4(float(-13), int(true), bool(false), float(2)).z;
int c = ivec4(int(true), -6.5, float(-0.5), b).b;
varying mediump float d;
int e = (ivec2(b, 0.5)).x;

void main()
{
	gl_FragColor = vec4(-0.75, int(e), -0.75, float(d)) * (vec4(c, b, 0, a));
}
