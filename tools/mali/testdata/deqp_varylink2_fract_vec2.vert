/* dEQP-GLES2.functional.shaders.operator.common_functions.fract.highp_vec2_vertex (vertex shader),
 * verbatim from the .qpa of tools/mali/deqp-host-measure.sh at cc89a1cf
 * (VK-GL-CTS 93bca018, Apache-2.0). Shape fract_vec2 of
 * docs/notes/mali-deqp-varylink2.md: fract of a vec2 local into color.yz, then color * 0.80 + 0.10, refused by the vertex shader compiled alone. */
attribute highp vec4 a_position;
attribute highp vec4 a_in0;
varying mediump vec4 v_color;

void main()
{
    gl_Position = a_position;
	highp vec2 in0 = a_in0.wy;
	highp vec2 res = vec2(0.0);

	res = fract(in0);

	mediump vec4 color = vec4(0.0, 0.0, 0.0, 1.0);
	color.yz = res;
	color = color * 0.80 + 0.10;
    v_color = color;
}
