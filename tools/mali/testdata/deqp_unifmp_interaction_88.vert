attribute vec4 dEQP_Position;
attribute vec4 m;
uniform mediump vec4 l;
const bool k = float(-0.25) != float(0);
attribute float j;
attribute vec4 h;
uniform mediump int i;
varying mediump vec4 g;
varying mediump float b;

void main()
{
	gl_Position = dEQP_Position;
	g = m * l;
	b = vec2(k, j).r;
	g = g;
	if (i > int(-0.75))
		b = b;
	g = h.stqp;
}
