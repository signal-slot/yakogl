attribute vec4 dEQP_Position;
float j = vec3(float(-4), int(-3), float(16)).t;
varying mediump float a;
varying mediump float e;

void main()
{
	gl_Position = dEQP_Position;
	a = j;
	e = 0.25;
}
