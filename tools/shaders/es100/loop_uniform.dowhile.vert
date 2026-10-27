#version 100
/* dEQP shaders.loops.do_while_uniform_iterations.basic_mediump_int_vertex
 * (es2fShaderLoopTests.cpp, Apache-2.0): one do-while loop with a uniform
 * bound (ui_three = 3, as dEQP sets it), not inside another loop. */
attribute highp vec4 a_position;
attribute highp vec4 a_coords;
varying mediump vec4 v_color;
uniform mediump int ui_zero, ui_one, ui_two, ui_three, ui_four, ui_five, ui_six;
void main()
{
    gl_Position = a_position;
    mediump vec4 coords = a_coords;
    mediump vec4 res = coords;
    mediump int i = 0;
    do { res = res.yzwx; } while (++i < ui_three);
    v_color = vec4(res.rgb, 1.0);
}
