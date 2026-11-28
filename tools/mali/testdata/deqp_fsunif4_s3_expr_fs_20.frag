precision mediump float;
vec4 c = (vec4(float(-1.25), float(-0.5), float(-1.5), 8.0));
uniform mediump float b;
float a = float(-9.0) - float(-8.0) - 6.0;

void main()
{
	gl_FragColor = c + (vec4(1.5, b, b, a));
}
