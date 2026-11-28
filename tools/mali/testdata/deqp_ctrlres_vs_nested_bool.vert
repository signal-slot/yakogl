attribute vec4 dEQP_Position;
attribute float a;
const int b = (ivec2(2.0, true)).r;
const float c = vec4(b, 2.75, -1.5, b).rgba.z;
const vec4 d = vec4(9.0, 8, 8.5, 1) + vec4(-1.5, -5.5, -6.5, -2.5) + (vec4(-1, -0.75, -3.0, 5.0));
attribute float e;
const bool f = bool(int(0) - int(-1));
vec4 g = vec4(0.75, 4.0, -1.75, 11.0) * vec4(-1.5, 1.5, 1.0, -0.5) + (vec4(2, -6.0, 2.5, 5.75));
const int h = -5 * 1;
uniform mediump float i;
varying mediump vec4 v_color;

void main()
{
	gl_Position = dEQP_Position;
	v_color = (g).rbag;
	{
	}
	if ((bvec3(e, i, i).t))
		if (0 <= h - int(-5.0))
			v_color = v_color;
		else 
			v_color = g.yzwx;
	else if ((bvec3(f, e, e)).g)
		v_color = (d + vec4(-6, c, b, a));
}
