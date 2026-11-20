attribute vec4 dEQP_Position;
attribute vec4 o;
uniform mediump float n;
varying mediump vec4 c;
varying mediump float e;

void main()
{
	gl_Position = dEQP_Position;
	c = o;
	true;
	e = (float(n));
}
