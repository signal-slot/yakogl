precision mediump float;
const bool a = true;
const float b = float(1.5) - -0.25 * -2.0;
uniform mediump float c;
uniform mediump ivec4 f;
uniform sampler2D g;
uniform mediump int h;
const int i = ivec3(b, -15, b).r * (int(a));
const int j = i;

void main()
{
	gl_FragColor = texture2DProj(g, (vec3(j, i, h)), float(c) - float(a));
	int e = (f.z);
	int d = (e - int(1.0));
	(ivec4(b, d, c, c) * ivec4(1.0, b, 1.0, 1.0)) * (ivec4(1.0, float(1), int(a), int(a)));
}
