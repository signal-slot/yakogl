precision mediump float;
uniform sampler2D a;
const bool b = bvec4(true, true, 1, false).zwy.s;
const bool c = true;
const int d = int(-9.5) * int(1.0) * int(1.0);
uniform samplerCube e;
varying mediump float g;
const ivec3 i = ivec3((b), true, float(0) * float(-1.75));
uniform mediump int j;
const int k = int(-5) - -7 - int(int(8.0));
vec4 l = (vec4(b, d, 20.1, k)).wxzy;
int m = int(float(11)) + int(int(-1.0));
float n = float(k) - float(-1.5) - float(-3.25) * -0.75;

void main()
{
	gl_FragColor = vec4((true), b, n, m);
	gl_FragColor = l + gl_FragColor.wyzx;
	int h = int(k - int(true));
	h = int(-4) - 2 + int(j);
	h = -4;
	int f = i.z;
	gl_FragColor = textureCube(e, (vec3(h, g, f)));
	gl_FragColor = texture2D(a, vec2(d, true) - vec2(c, 6.0), float(false) * float(b));
}
