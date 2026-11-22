precision mediump float;
varying mediump float g;
int f = (int(-19.0) + int(8));
varying mediump float a;
varying mediump float b;
varying mediump float c;
int d = int(bvec2(1.0, 1).g);
varying mediump float e;

void main()
{
	gl_FragColor = vec4(float(c), g, bool(d), float(f)).wyzx;
	gl_FragColor = (gl_FragColor).sqpt;
	gl_FragColor = vec4(int(c) * 1, int(e) - int(d), acos(b), c);
	gl_FragColor = atan(vec4(b, b, false, int(a)));
}
