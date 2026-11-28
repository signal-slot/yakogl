precision mediump float;
uniform sampler2D a;
const bool b = true;
varying mediump vec3 c;
varying mediump float d;
const int e = (int(b) + int(b));

void main()
{
	gl_FragColor = texture2D(a, vec2(e, d).st, d + float(b));
	gl_FragColor = texture2DProj(a, c + vec3(true, b, -5));
}
