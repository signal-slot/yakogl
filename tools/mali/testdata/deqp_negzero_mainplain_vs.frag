/* dEQP-GLES2.functional.fragment_ops.interaction.basic_shader.91: the fragment shader,
 * verbatim from the .qpa of tools/mali/deqp-host-measure.sh (VK-GL-CTS
 * 93bca018, Apache-2.0; HOST-ONLY run at cc89a1cf). Shape mainplain of
 * docs/notes/mali-deqp-negzero.md. */
precision mediump float;
const vec4 q = vec4(3.25, -2.375, 2.375, -8.75) - vec4(3, -3.0, 2, -8);
varying mediump vec4 p;
uniform mediump int a;
varying mediump float b;
varying mediump float c;
float e = float(-0.0);
const int f = -1;
ivec2 g = ivec2(2.25, true) * ivec2(true, 0.75);
ivec2 h = ivec2(f, -0.5) * ivec2(true, true);
uniform mediump vec4 i;
const vec4 j = vec4(false, -2, -0.25, true) * vec4(-0.75, 0.5, -2, -1.25);
float k = float(2.25) * 2.5;
int l = int(0.5) * int(1.0);
float m = float(bool(0));
int n = 7 * 1;
int o = int(5) + int(f);

void main()
{
	gl_FragColor = q * p;
	if (vec4(c, o, c, n) == vec4(m, l, a, k))
		gl_FragColor = j * i;
	else 
		g = h;
	vec4 d = vec4(b, -0.5, f, e);
	gl_FragColor = d * gl_FragColor;
	if (ivec2(b, b) == ivec2(true, c))
		gl_FragColor = vec4(b, 0, a, true).xywz;
}
