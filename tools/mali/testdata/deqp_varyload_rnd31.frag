precision mediump float;
varying mediump float a;
varying mediump float b;
uniform mediump int d;

void main()
{
	int c = d;
	c = d * 1;
	gl_FragColor = (vec4(b, float(1.0), a, a));
}
