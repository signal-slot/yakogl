attribute vec4 dEQP_Position;
const vec4 b = (vec4(false, -1.5, -1.5, 4.5)) - vec4(-4.5, float(3.5), 1.5, 5);
const vec4 c = (vec4(-6.625, -4, 3.5, 8)) * (vec4(-1.75, -0.5, 1, -1.0));
attribute vec4 d;
attribute vec2 e;
uniform mediump float f;
const bool g = bool((1));
varying mediump vec4 v_color;

void main()
{
	gl_Position = dEQP_Position;
	v_color = vec4(-14, 6, 0.25, -14.0).tpqs * vec4(-0.5, int(2.0), float(0.5), -0.25);
	vec4 a = vec4(float(-4.375), float(-4.125), 8.125, -3.75) * vec4(1.5, 1.5, -2, -0.75).stqp;
	if (g)
		if (bool(vec4(f, g, -6, g).a))
			a = a;
		else 
			v_color = vec4((float(-3)), 2.0 - f, float(-6) + -1.0, float(float(3.5)));
	else if (bool(vec3(8, 1.0, g).g))
		(f) + v_color.q + 0.001 >= e.s;
	v_color = d - v_color.grba;
	v_color = c - (b);
	v_color = (v_color) + a;
}
