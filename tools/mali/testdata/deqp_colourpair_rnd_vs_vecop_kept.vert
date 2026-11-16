attribute vec4 dEQP_Position;
const float d = 0.5;
attribute float c;
const int b = (ivec2(d, -1.0)).g;
attribute vec4 a;
varying mediump vec4 v_color;

void main()
{
	gl_Position = dEQP_Position;
	v_color = vec4(float(d), c, 0, b) * (a);
}
