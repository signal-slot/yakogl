precision mediump float;
const bool e = bool(float(1.0) * 1.0);
int d = (int(true));
const int c = int(-4.0) * int(e) + int(e) * int(e);
int b = ivec3(0.0, true, float(-3.25)).z;
uniform samplerCube a;

void main()
{
	gl_FragColor = textureCube(a, vec4(e, d, c, b).arg);
}
