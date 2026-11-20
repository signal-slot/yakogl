/* dEQP-GLES2.functional.shaders.random.all_features.fragment.14: the vertex shader,
 * verbatim from the .qpa of tools/mali/deqp-host-measure.sh (VK-GL-CTS
 * 93bca018, Apache-2.0; HOST-ONLY run at cc89a1cf). Shape mainbuiltin of
 * docs/notes/mali-deqp-negzero.md. */
attribute vec4 dEQP_Position;
attribute float a_b;
attribute float a_f;
attribute float a_h;
attribute float a_i;
attribute float a_l;
attribute vec4 a_o;
attribute float a_p;
varying mediump float b;
varying mediump float f;
varying mediump float h;
varying mediump float i;
varying mediump float l;
varying mediump vec4 o;
varying mediump float p;

void main()
{
	gl_Position = dEQP_Position;
	b = a_b;
	f = a_f;
	h = a_h;
	i = a_i;
	l = a_l;
	o = a_o;
	p = a_p;
}
