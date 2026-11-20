/* Reduced from dEQP-GLES2.functional.shaders.random.all_features.fragment.13
 * (deqp_negzero_constglobal_fs): the constant globals whose initialisers keep
 * the -0.0 refusal at the tip, folded where the refusal stays, and a constant
 * colour. Every initialiser is a GLSL constant expression. */
precision mediump float;
const bool c = vec4(1.0, int(6.0) - 7, true, -1.0).brag == vec4(1, 1, -1, -1);
const bool ab = 7.25 <= -1.1979 - vec2(int(-4), int(c)).s + 0.001;
const bool l = float(-3) <= float(ab) + 0.001;

void main()
{
	gl_FragColor = vec4(1.0);
}
