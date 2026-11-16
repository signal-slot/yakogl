precision mediump float;
varying mediump float c;
float b = vec4(5.5, 7, false, 2.0).a * float(float(0.0));
const bool a = int(int(5)) < 4 * 1;

void main()
{
	gl_FragColor = vec4(c, b, b, a).wxyz.arbg;
}
