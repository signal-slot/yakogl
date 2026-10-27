#version 100
/* dEQP shaders.discard, its vertex shader (es2fShaderDiscardTests.cpp,
 * Apache-2.0), for es100/discard_dynloop.frag. */
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
