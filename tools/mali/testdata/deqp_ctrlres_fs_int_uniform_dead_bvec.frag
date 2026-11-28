precision mediump float;
varying mediump float i;
uniform mediump vec4 a;
varying mediump float c;
const bool d = vec4(2, 1.0, 1.75, 2.25) == vec4(6, 3, 3.25, 10.0);
varying mediump vec4 e;
const float f = -10.0 * float(-0.75);
uniform mediump int g;
float h = float(6) + float(-5);

void main()
{
	gl_FragColor = (vec4(0.5, -2, -0.25, i));
	bvec4 b = bvec4(0, 1, h, c).stqp;
	if (int(true) < g)
		gl_FragColor = vec4(0.75, -1, -8.0, f) + e;
	else 
		b = bvec4(0, d, 1.0, c).rbga;
	gl_FragColor = gl_FragColor * a;
}
