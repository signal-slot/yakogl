/* dEQP-GLES2.functional.shaders.indexing.tmp_array.float_const_write_dynamic_loop_read_vertex (fragment shader),
 * verbatim from the .qpa of tools/mali/deqp-host-measure.sh at cc89a1cf
 * (VK-GL-CTS 93bca018, Apache-2.0). Shape loop_uniform of
 * docs/notes/mali-deqp-varylink2.md: a lane a run-time loop computes from constants alone, refused by the vertex shader compiled alone. */
varying mediump vec4 v_color;

void main()
{
    gl_FragColor = v_color;
}
