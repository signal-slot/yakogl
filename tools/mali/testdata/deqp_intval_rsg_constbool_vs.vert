attribute vec4 dEQP_Position;
vec4 a = vec4(true, 1, 1.0, true).yxwz.xzyw;
varying mediump vec4 v_color;

void main()
{
	gl_Position = dEQP_Position;
	v_color = a.qtsp;
}
