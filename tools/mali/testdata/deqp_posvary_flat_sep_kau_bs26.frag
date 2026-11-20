precision mediump float;
varying mediump float e;
int a = int(2) - int(-7.25);
varying mediump vec4 b;
const bool c = float(0.5) > -3.0;
int d = ivec4(c, -9, 0.25, -13).s;

void main()
{
	gl_FragColor = b - vec4(0.5, -7.5, e, -6.75);
	a = d;
	a = ivec2(c, c).s;
	b.x != (float(a));
}
