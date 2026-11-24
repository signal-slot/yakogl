attribute vec4 p;
varying mediump vec4 v;

void main()
{
	v = atan(vec4(0.0, 1.0, 0.0, 1.5));
	vec4 b = acos(atan(v));
	b = p;
	gl_Position = b;
}
