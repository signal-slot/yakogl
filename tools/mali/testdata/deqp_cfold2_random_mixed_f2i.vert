attribute vec4 dEQP_Position;
attribute vec3 f;
const int e = int(-5.0) * int(-3.5) * ivec2(10, 14).g;
uniform sampler2D d;
vec2 a = vec2(e, e) * vec2(float(16), bool(false));
uniform mediump int b;
const bool c = false;
varying mediump vec4 v_color;

void main()
{
	gl_Position = dEQP_Position;
	v_color = texture2DProjLod(d, f, float(float(e)));
	a = (vec2(float(c), b));
}
