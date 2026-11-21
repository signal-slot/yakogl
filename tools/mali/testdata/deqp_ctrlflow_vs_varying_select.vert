attribute vec4 dEQP_Position;
float a = (-2.5) - (float(0.5));
const int b = -7;
attribute float l;
const bool k = (bvec3(true, 1.0, false)).g;
const float j = float(-18.0) * float(0.75) - float(-4) * 0.25;
const int i = (-8 - int(-7.0));
const float h = float(-0.5) - float(5) + float(-2);
const float g = vec4(-6.5, true, h, true).rb.x;
uniform mediump float e;
const float d = 1.5;
const int c = -2 - 4 * int(k);
attribute float n;
uniform mediump float o;
const float p = (float(3.5)) + float(int(-4));
const float q = -2.0;
varying mediump vec4 v_color;

void main()
{
	gl_Position = dEQP_Position;
	{
		v_color = vec4(float(-3.25), -7.0, float(-6.25), float(-7.25)) - vec4(int(-4), -7, float(-6.5), float(-8.25));
	}
	{
		if (bool(int(4) < int(k)))
			false;
		else 
			a = float(-3.0);
		if (-1.0 + float(2) < float(l))
			v_color = v_color.yzxw * (vec4(p, k, 0, 0));
	}
	bvec4 m = bvec4(q * p, float(int(o)), k, (float(k)));
	if (n > n)
	{
		m = m;
		float f = a + l - float(float(d));
		a = float(ivec4(l, j, k, f).a);
		v_color = vec4(j, 2.5, i, h) - vec4(-7.5, 1, g, f) - (vec4(e, d, 4.5, c));
	}
	else 
		(b - int(b)) < int(int(a)) * (int(1.0));
}
