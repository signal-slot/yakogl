precision mediump float;
varying mediump float d;
float b = 60000.0;

void main()
{
	gl_FragColor = vec4(b + b, d, 0.0, 1.0);
}
