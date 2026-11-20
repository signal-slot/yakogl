/* dEQP-GLES2.functional.shaders.operator.common_functions.fract.highp_vec2_vertex (fragment shader),
 * verbatim from the .qpa of tools/mali/deqp-host-measure.sh at cc89a1cf
 * (VK-GL-CTS 93bca018, Apache-2.0). Shape fract_vec2 of
 * docs/notes/mali-deqp-varylink2.md: fract of a vec2 local into color.yz, then color * 0.80 + 0.10, refused by the vertex shader compiled alone. */
varying mediump vec4 v_color;

void main()
{
    gl_FragColor = v_color;
}
