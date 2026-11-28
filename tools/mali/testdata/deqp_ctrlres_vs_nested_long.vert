attribute vec4 dEQP_Position;
const float a = float(vec4(7, 4.25, -1, -9.0).p);
const bool b = (int(true) <= int(a));
uniform mediump int c;
uniform mediump float d;
attribute vec2 l;
attribute vec2 k;
vec2 j = (vec2(2.75, 6.75)) + (vec2(-2.5, -8));
const float i = (vec3(b, -2.5, b)).y;
attribute float h;
attribute float g;
const float f = vec2(-8, 10).r * vec2(b, -1).g;
const int e = 4 * 1 * int(1);
attribute float m;
varying mediump vec4 v_color;

void main()
{
	gl_Position = dEQP_Position;
	v_color = vec4(9, 5, -3, -3.75) + vec4(-4.0, -5.0, 6, true) + vec4(-4.5, d, float(-3), h);
	j = vec2(18, 14) * j - vec3(-6, g, 1.5).gr;
	v_color = v_color;
	vec4 n = vec4(-4, float(-9.25), float(-9.5), h) - vec4(2.5, -3, -8.5, 4.0).agbr;
	if (bvec3(0, bool(c), bool(false)).t)
		if (h + 0.001 >= float(float(h)))
			j = ((j));
		else 
			v_color = vec4(8.5, -5.0, g, 11.5) + vec4(-1, 6.5, d, -8.0) - vec4(7, 2.5, 0.75, g).rbag;
	else 
		n = vec4(-0.5, -5.0, -2, -15.0) + vec4(-5, 7.0, 1.5, m) - n.tqps;
	if (float(1.25) + m <= vec3(h, g, b).s + 0.001)
	{
		j = (l + k);
		v_color = (v_color);
		v_color = vec4(5, -1.0, i, 1) + vec4(h, g, 0.5, -4) - (vec4(f, e, -2.0, -3));
		v_color = vec4(1, 1, c, d).wyzx.argb;
	}
	else 
		v_color = vec4(d, c, b, int(-0.75) - int(a));
}
