precision mediump float;
const bool k = (ivec4(-1, -6.25, -4, true) != ivec4(true, -3, 1, 2));
const ivec2 j = ivec4(-13, 9.0, 5.25, k).tp + ivec2(9, -5.0) * ivec2(-2, 1);
int i = ivec2(4.0, k).s * 1;
const int h = (-2) * ivec2(-0.25, 1.0).g;
float g = vec4(3.5, h, -8.75, k).x + float(0);
const int f = ivec4(3, k, true, 15).r - ivec3(5.0, 5.0, h).r;
varying mediump vec4 a;
bvec4 b = bvec4(int(-6) - -7, bool(1), int(bool(1.0)), 7 + int(-6.0));
bvec2 c = bvec2(vec4(-6.75, h, k, 1.0).w, int(k));
const bvec2 d = bvec4(k, 0.0, 0.0, k).wyzx.rb;
varying mediump vec3 e;

void main()
{
	gl_FragColor = vec4(k, j.g, -7.5 + 8.25, vec3(i, g, h) == vec3(g, g, f));
	if (bool(e == e))
		if (-1 >= (-1))
			c = d;
		else 
			b = b;
	else 
		gl_FragColor = a.bgra.arbg;
}
