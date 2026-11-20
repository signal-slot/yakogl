/* dEQP-GLES2.functional.shaders.random.comparison_ops.fragment.43: the vertex shader,
 * verbatim from the .qpa of tools/mali/deqp-host-measure.sh (VK-GL-CTS
 * 93bca018, Apache-2.0; HOST-ONLY run at cc89a1cf). Shape mainplain of
 * docs/notes/mali-deqp-negzero.md. */
attribute vec4 dEQP_Position;
attribute vec4 a_d;
attribute vec4 a_h;
attribute vec4 a_i;
varying mediump vec4 d;
varying mediump vec4 h;
varying mediump vec4 i;

void main()
{
	gl_Position = dEQP_Position;
	d = a_d;
	h = a_h;
	i = a_i;
}
