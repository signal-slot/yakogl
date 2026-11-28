attribute vec4 dEQP_Position;
attribute float a;
attribute float b;
uniform mediump float c;
bvec4 d = ((bvec4(true, false, true, false)));
varying mediump vec4 v_color;

void main()
{
	gl_Position = dEQP_Position;
	d = d;
	v_color = vec4(float(c), (float(c)), float(float(b)), (float(a)));
}
