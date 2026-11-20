/* dEQP-GLES2.functional.fragment_ops.interaction.basic_shader.91: the vertex shader,
 * verbatim from the .qpa of tools/mali/deqp-host-measure.sh (VK-GL-CTS
 * 93bca018, Apache-2.0; HOST-ONLY run at cc89a1cf). Shape mainplain of
 * docs/notes/mali-deqp-negzero.md. */
attribute vec4 dEQP_Position;
const int ag = -6 - 1;
attribute float af;
float ae = vec4(9.25, ag, -4.25, true).z;
const bool ad = (bool(true));
attribute vec4 ac;
float r = vec2(4, ag).s;
vec2 s = vec2(false, ad) - vec2(7.75, ad);
const bool t = vec3(1, -1.0, 0.5) == vec3(ad, -1, 0.5);
const vec4 u = (vec4(-1, 5.5, -8, 6.5));
attribute vec4 v;
uniform mediump int w;
const ivec2 x = ivec2(6.0, 2.0) + ivec2(-5, -1);
const bool y = int(-3.0) < int(0.75);
uniform mediump ivec4 z;
attribute vec2 aa;
uniform mediump vec3 ab;
varying mediump vec4 p;
varying mediump float b;
varying mediump float c;

void main()
{
	gl_Position = dEQP_Position;
	p = vec4(6.25, ag, -5.25, 3) - vec4(6.0, af, ae, ad);
	b = ac.p;
	c = 3.5;
	if (ab == vec3(b, c, t))
		s = aa;
	else 
		z.br != ivec2(c, y) * x;
	if (int(true) > w)
		p = v - u;
	s = vec2(t, int(t));
	c = r - c;
}
