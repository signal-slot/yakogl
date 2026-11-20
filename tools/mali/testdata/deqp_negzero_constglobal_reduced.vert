/* The vertex shader of dEQP-GLES2.functional.shaders.random.all_features.fragment.13
 * (VK-GL-CTS 93bca018, Apache-2.0), verbatim; see deqp_negzero_constglobal_fs. */
attribute vec4 dEQP_Position;
attribute float a_b;
attribute float a_e;
attribute float a_t;
attribute float a_g;
attribute float a_ad;
attribute float a_af;
attribute vec2 a_aq;
varying mediump float b;
varying mediump float e;
varying mediump float t;
varying mediump float g;
varying mediump float ad;
varying mediump float af;
varying mediump vec2 aq;

void main()
{
	gl_Position = dEQP_Position;
	b = a_b;
	e = a_e;
	t = a_t;
	g = a_g;
	ad = a_ad;
	af = a_af;
	aq = a_aq;
}
