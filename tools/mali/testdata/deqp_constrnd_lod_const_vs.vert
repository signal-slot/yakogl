attribute vec4 dEQP_Position;
int j = ivec3(int(8.0), int(9), int(-6.5)).x;
uniform samplerCube a;
attribute float b;
attribute float c;
attribute vec3 d;
attribute vec4 e;
const bool h = (bool(bool(0.5)));
const bvec4 i = (bvec4(h, int(h), true, int(1)));
varying mediump vec4 v_color;

void main()
{
	gl_Position = dEQP_Position;
	v_color = vec4(-15.65, -19.3, j, -14).zwxy + (vec4(-3, 2.3, j, 10));
	int g = int((1));
	bvec3 f = i.sptq.brg;
	f = bvec3(float(h), float(g), true).brg;
	f = (f);
	v_color = (e) - v_color;
	v_color = textureCubeLod(a, d - vec3(0, c, b), 6.5);
}
