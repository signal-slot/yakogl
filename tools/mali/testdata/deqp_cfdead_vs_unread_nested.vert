attribute vec4 dEQP_Position;
const float n = 12.0 + float(-6) - (6.5);
const bool o = (int(n)) >= int(n);
const vec4 p = vec4(-8.5, -6.0, 7, n).wzxy + vec4(float(-6.0), float(-1.5), float(5.5), false);
const int q = (9) + ivec2(13, -4).g;
const float r = -2.0;
const float s = (vec3(3.5, q, r)).r;
const vec4 t = vec4(-19, 9, -3, -8.0) - vec4(-6.5, 1.5, s, -1.5) + vec4(3.5, 1, -2, 5.0).stpq;
attribute float u;
uniform mediump float v;
varying mediump vec4 f;
varying mediump vec2 i;
varying mediump vec2 j;

void main()
{
	gl_Position = dEQP_Position;
	i = (vec2(2, s)) * vec2(v, 1.0);
	j = vec3(bool(true), int(7.0), -5).zy;
	float w = float(-2 + int(true));
	f = vec4(-0.5, float(-0.5), float(1), 0).abgr;
	if (bool(w + 0.001 >= v))
		if (bool(u - float(1.5)))
			i = vec2(float(2), int(r)).ts;
		else 
			f = t + vec4(s, r, q, -5.0) - p;
	else 
		f = vec4((o), 1.0, 0.0 + n, n);
}
