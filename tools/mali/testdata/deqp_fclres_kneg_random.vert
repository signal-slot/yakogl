attribute vec4 dEQP_Position;
const float k = float(float(-1.0) * -1.0);
attribute float l;
varying mediump float g;

void main()
{
	gl_Position = dEQP_Position;
	g = k;
	g = l;
	g = float(4.0) * float(0.25);
	g = k * g;
	g = float(3.0 - g);
	g = float(-0.75) - g - float(-2.25);
	g = (-2.375 * g);
}
