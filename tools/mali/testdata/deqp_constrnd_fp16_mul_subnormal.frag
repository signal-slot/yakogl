precision mediump float;
varying mediump float d;
float b = 0.001;

void main()
{
	gl_FragColor = vec4(b * b, d, 0.0, 1.0);
}
