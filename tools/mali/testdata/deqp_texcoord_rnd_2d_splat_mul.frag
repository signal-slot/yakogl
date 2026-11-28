precision mediump float;
const vec4 b = (vec4(7.0, 1, 9, -4.5)) - (vec4(2.5, 4.0, 4.5, 0.0));
const vec4 c = vec4(int(12), 11, float(-2.5), float(false)) + vec4(float(-6), float(-6.5), float(-2), float(-7.0));
const float d = vec2(bool(0), float(0.25)).t;
const float e = -2.0;
const float f = -9.0 + float(8);
const int g = (ivec2(f, 2).t);
varying mediump vec4 h;
varying mediump vec4 i;
varying mediump float j;
uniform sampler2D k;
varying mediump vec2 l;

void main()
{
	gl_FragColor = vec4(int(-8), float(-11), false, float(false)).stqp;
	gl_FragColor = texture2D(k, vec2(g, g) * l);
	(vec4(float(true), int(j), int(e), g) * i);
	vec4 a = h * vec4(g, f, e, d) + c;
	a = b - a.stqp;
}
