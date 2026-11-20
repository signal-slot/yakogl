/* dEQP-GLES2.functional.shaders.indexing.vector_subscript.vec2_direct_write_dynamic_loop_subscript_read_vertex (fragment shader),
 * verbatim from the .qpa of tools/mali/deqp-host-measure.sh at cc89a1cf
 * (VK-GL-CTS 93bca018, Apache-2.0). Shape loop_bcast of
 * docs/notes/mali-deqp-varylink2.md: vec3(res) of a value a run-time loop carries, refused by the vertex shader compiled alone. */
varying mediump vec3 v_color;

void main()
{
    gl_FragColor = vec4(v_color, 1.0);
}
