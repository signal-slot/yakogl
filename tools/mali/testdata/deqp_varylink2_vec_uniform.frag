/* dEQP-GLES2.functional.shaders.random.basic_expression.vertex.27 (fragment shader),
 * verbatim from the .qpa of tools/mali/deqp-host-measure.sh at cc89a1cf
 * (VK-GL-CTS 93bca018, Apache-2.0). Shape vec_uniform of
 * docs/notes/mali-deqp-varylink2.md: a uniform scalar lane in a vec4 constructor beside attribute lanes, refused by the vertex shader compiled alone. */
precision mediump float;
varying mediump vec4 v_color;

void main()
{
	gl_FragColor = v_color;
}
