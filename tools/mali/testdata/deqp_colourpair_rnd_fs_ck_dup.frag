precision mediump float;
const float b = float(-3.5) - float(-3.0) + float(1.0);
varying mediump float a;

void main()
{
	gl_FragColor = (vec4(b, float(a), float(a), a));
}
