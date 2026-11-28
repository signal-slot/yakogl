precision mediump float;
const float k = ((float(1.5)));
uniform mediump float j;
const vec4 i = vec4(int(2.0), 2, 0.5, int(7.0)) * vec4(0.25, -1, true, -1.0).zxyw;
uniform sampler2D a;
varying mediump float b;
varying mediump float c;
varying mediump vec3 d;
const bool e = false;
varying mediump float f;
uniform mediump vec4 g;
uniform mediump vec4 h;

void main()
{
	gl_FragColor = vec4(k, float(b), float(j), float(-7.25)) - i;
	gl_FragColor = h.garb * (gl_FragColor);
	gl_FragColor = g.rbga * gl_FragColor;
	gl_FragColor = (gl_FragColor - vec4(e, f, 0, e));
	gl_FragColor = texture2DProj(a, (d), c + b);
}
