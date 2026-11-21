precision mediump float;
const bool a = (true);
varying mediump float b;
const int c = int(1 + int(-5.0));
const bool d = vec3(1, -1.0, false) == (vec3(1, -1.0, 0.0));
const int e = ivec4(true, 1.0, 9.75, a).xywz.y;
const float f = vec2(-4.5, a).r;
varying mediump float g;
ivec3 p = ivec3(f, f, c) - ivec3(f, 1.0, e) + ivec3(int(d), f, a);
const bool o = float(5.75) * 1.0 > float(4.75) - float(1.5);
varying mediump vec2 n;
const int i = ivec2(-8.0, c).r;
uniform mediump int j;
const bool k = o;
varying mediump vec4 l;
varying mediump vec4 m;

void main()
{
	gl_FragColor = vec4(2.0, 7.25, 10.25, 0) - vec4(g, 1.5, 3, 6) + vec4(float(5.5), int(c), float(-6.5), 5.5);
	{
		p = (ivec4(b, true, a, 9.75).ptq);
		gl_FragColor = gl_FragColor;
		vec4 h = vec4(vec2(i, o).t, j - -9, (bool(e)), n.g);
		{
			h = m;
			h = (l.zxwy);
			h = vec4(k, j, j, g) * gl_FragColor - vec4(float(true), i, float(b), float(c));
		}
	}
	gl_FragColor = vec4(g + f, 1 > e, (int(d)), vec3(c, b, a).z);
}
