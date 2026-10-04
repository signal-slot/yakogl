attribute vec4 dEQP_Position;
attribute float b;
const bool c = bvec3(0, bool(0.0), int(true)).r;
const bool d = (true);
attribute float e;
attribute float f;
const int g = ((-1));
uniform mediump int h;
attribute float i;
attribute float j;
attribute float k;
uniform samplerCube l;
uniform mediump vec4 m;
attribute float n;
uniform mediump int o;
const float p = (float(0.5) * 1.0);
varying mediump vec4 v_color;

void main()
{
	gl_Position = dEQP_Position;
	v_color = vec4(e, c, b, d).zwxy;
	v_color = v_color.xzwy;
	int q = (int(int(-1.0)));
	v_color = vec4(5, -1.5, c, -2.75) * vec4(b, 1.5, h, -2.0) * vec4(q, -0.5, q, -1);
	v_color = vec4(-8.1, 12, -5.6, -16) - v_color - vec4(o, p, float(o), d);
	v_color = (vec4(4, e, -3, n)) - m;
	v_color = textureCube(l, v_color.xyw);
	v_color = (vec4(k, j, i, h) * vec4(f, -1.25, g, f));
	int a = int((float(e)));
	a = ivec2(a, d).s - ivec3(8.0, b, true).r;
	v_color = ((vec4(c, c, b, a)));
}
