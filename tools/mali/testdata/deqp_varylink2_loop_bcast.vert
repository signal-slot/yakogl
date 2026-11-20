/* dEQP-GLES2.functional.shaders.indexing.vector_subscript.vec2_direct_write_dynamic_loop_subscript_read_vertex (vertex shader),
 * verbatim from the .qpa of tools/mali/deqp-host-measure.sh at cc89a1cf
 * (VK-GL-CTS 93bca018, Apache-2.0). Shape loop_bcast of
 * docs/notes/mali-deqp-varylink2.md: vec3(res) of a value a run-time loop carries, refused by the vertex shader compiled alone. */
attribute highp vec4 a_position;
attribute highp vec4 a_coords;
varying mediump vec3 v_color;
uniform mediump int ui_two;

void main()
{
    gl_Position = a_position;
    mediump vec2 coords = vec2(a_coords);
    mediump vec2 tmp;
    tmp = coords.xy * vec4(1.0, 0.5, 0.25, 0.125).xy;
    mediump float res = 0.0;
    for (int i = 0; i < ui_two; i++)
        res += tmp[i];
    v_color = vec3(res);
}
