precision mediump float;
varying mediump vec4 b;
uniform mediump vec4 a;

void main()
{
	gl_FragColor = b * (a);
}
