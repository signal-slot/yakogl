/* dEQP-GLES2.functional.shaders.random.all_features.vertex.13: the fragment shader,
 * verbatim from the .qpa of tools/mali/deqp-host-measure.sh (VK-GL-CTS
 * 93bca018, Apache-2.0; HOST-ONLY run at cc89a1cf). Shape mainbuiltin of
 * docs/notes/mali-deqp-negzero.md. */
precision mediump float;
varying mediump vec4 v_color;

void main()
{
	gl_FragColor = v_color;
}
