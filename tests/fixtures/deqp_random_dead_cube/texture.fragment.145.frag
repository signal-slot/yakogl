precision mediump float;
const int k = int(-1) - int(-8) + int(int(-8.0));
uniform mediump int j;
const int i = -5 - int(2.0) - int(-7.0);
varying mediump float h;
varying mediump float g;
float f = float(-0.25);
float e = (6.75) + (-4.5);
vec4 a = (vec4(k, -2, -7.75, -4.25)) + vec4(true, float(true), i, bool(true));
uniform samplerCube b;
varying mediump float c;
vec3 d = vec3(3, int(-1.0), float(i)).pts;

void main()
{
	gl_FragColor = vec4(k, j, i, -0.75) * vec4(h, f, 1.5, g) * vec4(int(0.0), f, -1, e);
	a = textureCube(b, d * vec3(-1, c, -0.75));
}
