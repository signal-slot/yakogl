precision mediump float;
uniform sampler2D a;
varying mediump vec2 b;
int d = int(ivec4(true, 7, 7, -8.5).z);
varying mediump float e;
const bool f = bvec4(float(false), 0, false, float(1.0)).x;
const bool g = (bool(float(0.0)));
varying mediump float h;
int i = (int(int(-18)));
const float j = vec2(-8.25, -3.7).y + float(-5.5) * float(-1);

void main()
{
	vec4 c = (vec4(h, j, i, h) + vec4(g, f, e, d));
	gl_FragColor = c;
	gl_FragColor = texture2D(a, b * b, gl_FragColor.y);
}
