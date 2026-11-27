attribute vec4 dEQP_Position;
attribute vec4 b;
vec4 a = ((vec4(-6.0, -2, -8, -5.0) + vec4(-4.5, 5.0, 6.25, 8.0))) - vec4(-10.5 - (float(-1)), 2.25, int(float(4.5) + -5.5), log(float(13) + float(3)));
varying mediump vec4 v_color;

void main()
{
	gl_Position = dEQP_Position;
	v_color = asin(tan(b * a).rgab);
}
