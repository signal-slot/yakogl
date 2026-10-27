#version 100
/* dEQP shaders.loops.for_uniform_iterations.basic_mediump_int_fragment
 * (es2fShaderLoopTests.cpp, Apache-2.0): one loop with a uniform bound
 * (ui_three = 3, as dEQP sets it), not inside another loop. */
precision mediump float;
varying mediump vec4 v_coords;
uniform mediump int ui_zero, ui_one, ui_two, ui_three, ui_four, ui_five, ui_six;
void main()
{
    mediump vec4 coords = v_coords;
    mediump vec4 res = coords;
    for (mediump int i = 0; i < ui_three; i++)
    {
        res = res.yzwx;
    }
    gl_FragColor = vec4(res.rgb, 1.0);
}
