precision mediump float;
uniform sampler2D a;
varying mediump float b;
const int c = int(float(6.0)) + ivec4(true, -3.75, -2, false).w;
varying mediump float e;
uniform mediump int f;
float h = float(vec2(-4.5, 0.75).t);
varying mediump float i;
const bool j = bvec2(float(0), int(false)).r;
uniform mediump int k;
float l = (float(-1.5) + float(-4));
uniform mediump float m;

void main()
{
	float g = m;
	vec4 d = ((vec4(l, k, k, j)));
	h = i * h;
	d = vec4(h, 4, c, g) * vec4(b, f, e, 1.5) - d;
	gl_FragColor = texture2DProj(a, (vec3(c, true, b)));
}
