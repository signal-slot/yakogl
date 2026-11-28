precision mediump float;
varying mediump vec4 a;
const bool b = bool(float(1.0));
int c = int(ivec4(0.5, -9, b, 0).a);
float d = float(10.25) - 2.0 + (float(-5.5));
uniform mediump float e;
varying mediump float f;
varying mediump float g;
const int h = (-6) + -2;

void main()
{
	if (ivec3(-5, h, -8.0) - ivec3(g, -6.0, f) == (ivec3(e, -2, c)))
	{
	}
	else if (float(d) < (float(b)))
		gl_FragColor = vec4(c, c, c, b).ptsq;
	gl_FragColor = (a).grab;
}
