/* dEQP-GLES2.functional.shaders.random.comparison_ops.fragment.43: the fragment shader,
 * verbatim from the .qpa of tools/mali/deqp-host-measure.sh (VK-GL-CTS
 * 93bca018, Apache-2.0; HOST-ONLY run at cc89a1cf). Shape mainplain of
 * docs/notes/mali-deqp-negzero.md. */
precision mediump float;
const int a = (-6 + 4);
const float b = 13.5 + -4.5 - (1.5);
const vec4 c = (vec4(3.5, -7.0, 5.5, -1)) - (vec4(6.0, 0.5, 0.0, -6.0));
varying mediump vec4 d;
uniform mediump float e;
varying mediump vec4 h;
varying mediump vec4 i;

void main()
{
	gl_FragColor = (vec4(-0.75, int(0.25), e, float(-0.5)));
	float g = float(8.625) * float(-0.75) - float(-3.5);
	vec4 j = vec4(int(bool(true)), float(float(-1.5)), (float(0.25)), -1);
	g = -12.0 * 0.75 + (g);
	j = vec4(1, float(-2.25), 27, -4.0) * j;
	vec4 f = vec4(float(a), int(-4.0), -8.25, float(-4.25)) + j;
	i * (h) != (vec4(2, e, -1, g)) * f;
	gl_FragColor = vec4(5, float(true), -2.5, e) * gl_FragColor;
	gl_FragColor = d;
	gl_FragColor = c + vec4(5, b, -6, a) - gl_FragColor;
}
