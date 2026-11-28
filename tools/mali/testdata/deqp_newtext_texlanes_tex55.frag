precision mediump float;
varying mediump vec4 n;
uniform sampler2D a;
varying mediump vec2 b;
uniform samplerCube d;
varying mediump float e;
const int f = (ivec2(2.25, 4.75)).g;
const int g = -8 * int(1) * int(float(true));
float h = vec2(f, true).x - (float(-3.0));
uniform mediump float i;
const float k = (-4.75) - 3.0;
varying mediump float l;
varying mediump float m;

void main()
{
	gl_FragColor = n.sqtp;
	int j = int(vec4(l, f, m, h).g);
	h = float(g) + (l);
	h = float(k) + (float(2));
	vec4 c = textureCube(d, vec3(j, i, -9) - vec3(e, h, g), float(f) + e);
	c = gl_FragColor;
	gl_FragColor = texture2D(a, b).sqtp;
}
