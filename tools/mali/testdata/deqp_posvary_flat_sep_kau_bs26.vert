attribute vec4 dEQP_Position;
uniform mediump float j;
const int i = ivec3(5.0, true, -2.25).r;
attribute float h;
attribute float g;
const vec4 f = (vec4(-4, 4.0, 1.0, -4));
varying mediump float e;
varying mediump vec4 b;

void main()
{
	gl_Position = dEQP_Position;
	e = j + 4.0;
	b = vec4(i, h, 3.0, g) + f;
}
