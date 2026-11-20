attribute vec4 dEQP_Position;
attribute float a;
attribute float b;
uniform mediump float c;
attribute vec4 d;
varying mediump vec4 v_color;

void main()
{
	gl_Position = dEQP_Position;
	v_color = d;
	v_color = vec4((c), c, b, float(a) - float(5.0));
}
