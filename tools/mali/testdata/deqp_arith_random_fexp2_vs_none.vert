attribute vec4 dEQP_Position;
attribute vec4 a;
varying mediump vec4 v_color;

void main()
{
	gl_Position = dEQP_Position;
	v_color = exp2(a);
}
