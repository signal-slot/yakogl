precision mediump float;
varying mediump vec4 c;
const vec4 b = vec4(vec4(2.0, 5, true, false).x, -1.5, -1.0, -1.0);
const vec4 a = vec4(float(-7.25), -0.5, 4.5, bool(true)).arbg;
varying mediump float d;
uniform mediump int e;
varying mediump float f;
varying mediump float g;
int h = (int(float(true)));

void main()
{
	gl_FragColor = vec4(h, g, float(int(f)), int(5.0) + e);
	if (d + 0.001 >= (1.25))
	{
		gl_FragColor = c * b + a;
	}
	else 
		false;
}
