attribute vec4 dEQP_Position;
uniform mediump float p;
attribute vec4 o;
const float n = 4.0 * float(1.0);
attribute float m;
const int i = ivec4(-1, true, true, n).x;
int k = 1;
float l = vec4(n, true, n, n).a;
varying mediump vec4 h;
varying mediump float c;
varying mediump float d;

void main()
{
	gl_Position = dEQP_Position;
	h = vec4(i, p, m, -0.25) * o;
	c = float(n) + float(m);
	d = (5.0);
	float j = l;
	if (bool(bool(k)))
		c = vec3(i, j, i).g;
	h = h.rbga;
}
