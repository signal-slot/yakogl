precision mediump float;
uniform samplerCube a;
varying mediump float b;
uniform mediump float c;
uniform mediump int d;
int e = ivec2(10.0, -13).s + -4;
varying mediump float g;
vec4 h = (vec4(true, 4.0, false, 1.25)) + vec4(5.0, -3, false, -13).arbg;
float i = (float(int(true)));
const bool j = bvec3(true, true, 0.5).x;
int k = int(-11) + int(j) - -2 * 1;

void main()
{
	h = vec4(k, b, j, i).argb.qtsp;
	float f = b;
	e = (e + int(-3));
	f = g;
	gl_FragColor = textureCube(a, (vec3(e, d, c)), b + b);
}
