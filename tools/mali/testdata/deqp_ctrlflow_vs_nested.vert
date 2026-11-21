attribute vec4 dEQP_Position;
attribute vec4 a;
const int b = (-1) * int(1.0);
const bool d = (bvec4(1, 1, true, 1)).a;
int e = int(vec2(0.0, 0) == vec2(0, 0.0));
const ivec3 f = (ivec3(-4.25, 7.5, -7)) + ivec3(float(b), bool(1), float(7));
const bool g = b >= int(float(-4.0));
vec4 h = vec4(-0.75, 11.0, 10.0, -2.5) * vec4(2.0, 0.75, 0.5, 1.0) - vec4(-2, 3.75, 19.75, 1.75) * vec4(d, 2, 0.25, -2);
float i = float((int(12.0)));
varying mediump vec4 v_color;

void main()
{
	gl_Position = dEQP_Position;
	v_color = (h);
	vec4 c = v_color * h * h;
	h = vec4(vec2(b, g) == vec2(0.75, e), float(int(b)), i, float(-19) + i);
	if (int(g) <= (e))
		if (false)
			c = vec4(f.b, 7 + -6, (e), d);
		else 
			v_color = vec4(-1, -1, b, 0).wyxz * (a);
}
