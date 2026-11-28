precision mediump float;
const int c = int(float(-0.75) - -1.0);
const float b = float(bool(float(true)));
uniform mediump float a;

void main()
{
	gl_FragColor = (vec4(bool(c), a, b, a));
}
