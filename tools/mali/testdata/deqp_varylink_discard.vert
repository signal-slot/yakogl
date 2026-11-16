/* dEQP-GLES2.functional.shaders.discard.basic_dynamic, rebuilt from
 * VK-GL-CTS 93bca018 (Apache-2.0): modules/gles2/functional/es2fShaderDiscardTests.cpp:83-92,
 * the vertex shader of every discard case. Shape E1 of
 * docs/notes/mali-deqp-varylink.md: v_color.xyz equals v_coords.xyz. */
attribute highp   vec4 a_position;
attribute highp   vec4 a_coords;
varying   mediump vec4 v_color;
varying   mediump vec4 v_coords;

void main (void)
{
    gl_Position = a_position;
    v_color = vec4(a_coords.xyz, 1.0);
    v_coords = a_coords;
}
