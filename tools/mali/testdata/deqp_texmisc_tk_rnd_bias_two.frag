precision mediump float;
uniform mediump int a;
const int b = int(13.0) + int(-8) - int(5);
varying mediump float c;
uniform mediump int d;
varying mediump float e;
const int f = -1;
const float g = float(1 * 1);
const float h = (vec2(-6.0, true).x);
const float j = -1.75 + 6.5 + vec2(-3.5, true).r;
const float k = -1.0;
const float l = float(float(6.5)) * (k);
const vec4 m = (vec4(8.5, 8.125, -4, -5.625)) - (vec4(7.0, l, 1.5, 4));
uniform sampler2D n;
varying mediump vec2 o;
varying mediump float p;
const int q = (ivec3(true, j, -10)).b;
varying mediump float r;
varying mediump float s;
const bool t = bool(bvec4(b, true, false, g).q);
uniform sampler2D u;

void main()
{
	gl_FragColor = texture2D(u, vec2(false, t) + vec2(a, b), (float(p)));
	gl_FragColor = vec4((bool(true)), int(p) * int(t), float(s) * float(r), float(q));
	gl_FragColor = (gl_FragColor);
	vec4 i = texture2D(n, vec2(q, p) + o, float(d) - k);
	i = (m) + vec4(-6.5, l, 1, -5.5) * vec4(k, 1.0, j, -1.0);
	gl_FragColor = i.qpst + vec4(-4, 4.5, h, 4) * vec4(-1.0, 1.0, g, -2.0);
	gl_FragColor = gl_FragColor * vec4(1, f, -1.5, e) * (vec4(d, c, b, a));
}
