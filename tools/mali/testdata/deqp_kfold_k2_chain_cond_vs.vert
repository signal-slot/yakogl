attribute vec4 dEQP_Position;
const int a = ivec2(int(8), bool(true)).y;
attribute vec3 b;
float c = float(0) * float(-1) * -0.5;
int d = (11 + int(-8));
attribute vec4 i;
uniform mediump vec4 h;
const vec4 g = (vec4(2.25, -1.5, 6.75, -6.0) + vec4(-3.5, 2.5, -6.5, 5.5));
const int f = int(4) * 1;
const int e = (-6);
varying mediump vec4 v_color;

void main()
{
	gl_Position = dEQP_Position;
	v_color = vec4(-3.0, -9.5, a, 3.5) * vec4(-1.0, 0.5, -1.25, -2.0) + vec4(-3, 5.5, 2, int(7.25));
	if (ivec2(2.0, -3) - ivec2(-3.0, c) != ivec2(4.0, c) - ivec2(c, 0.5))
	{
		v_color = (i).wxyz;
		v_color = h * g + vec4(-5.0, f, -1.5, e);
	}
	else if (d < ivec4(6, a, 6, c).b)
		v_color = v_color;
	else 
		v_color = vec4((int(c)), bvec4(true, 0, 0.0, 1).z, b.p, (float(a)));
}
