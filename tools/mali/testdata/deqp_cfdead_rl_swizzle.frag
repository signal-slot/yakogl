precision mediump float;
uniform mediump vec4 b;
varying mediump vec4 a;

void main()
{
	gl_FragColor = b * a.zxwy;
}
