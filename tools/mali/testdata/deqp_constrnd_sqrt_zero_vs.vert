attribute vec4 dEQP_Position;
int a = ivec4(14, true, -0.25, true).p * int(1);
int b = (ivec2(-6.75, -16).y);
attribute float c;
const bool d = bvec2(int(true), 0).g;
const float e = exp2(sqrt(0.0));
const ivec4 f = ivec4(bvec4(0, 1, 1, 1.0).w, log2(35.625), (bool(1)), -6.5 - float(-6.25));
varying mediump vec4 v_color;

void main()
{
	gl_Position = dEQP_Position;
	b = f.a * int(e);
	v_color = sqrt(exp(vec4(d, c, b, a)));
}
