precision mediump float;
const float d = 0.5;
varying mediump float c;
const int b = (ivec2(d, -1.0)).g;
varying mediump vec4 a;

void main()
{
	gl_FragColor = vec4(float(d), c, 0, b) * (a);
}
