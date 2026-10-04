#version 100
/* dEQP shaders.loops.for_uniform_iterations nested, nested_sequence,
 * nested_tricky_dataflow_1 and _2, fragment (es2fShaderLoopTests.cpp
 * createSpecialLoopCase, Apache-2.0), in one shader: loops with uniform
 * bounds (ui_one = 1, ui_two = 2, ui_three = 3, as dEQP sets them) inside
 * loops with uniform bounds, each a counted loop with a data exit. */
precision mediump float;
varying mediump vec4 v_coords;
uniform mediump int ui_one, ui_two, ui_three;
void main()
{
    mediump vec4 coords = v_coords;
    mediump vec4 res = coords;
    /* nested */
    for (mediump int i = 0; i < ui_two; i++)
    {
        for (mediump int j = 0; j < ui_three; j++)
            res = res.yzwx;
    }
    mediump vec4 a = res;
    /* nested_sequence */
    res = coords;
    for (mediump int i = 0; i < ui_three; i++)
    {
        for (mediump int j = 0; j < ui_two; j++)
            res = res.yzwx;
        for (mediump int j = 0; j < ui_one; j++)
            res = res.yzwx;
    }
    mediump vec4 b = res;
    /* nested_tricky_dataflow_1 */
    for (mediump int i = 0; i < ui_three; i++)
    {
        res = coords;
        for (mediump int j = 0; j < ui_two; j++)
            res = res.yzwx;
    }
    mediump vec4 c = res;
    /* nested_tricky_dataflow_2 */
    for (mediump int i = 0; i < ui_three; i++)
    {
        res = coords.wxyz;
        for (mediump int j = 0; j < ui_two; j++)
            res = res.yzwx;
        coords = res;
    }
    gl_FragColor = vec4(a.x * 0.25 + b.y * 0.25 + c.z * 0.25 + res.w * 0.25, a.y - b.x, c.w - res.z, 1.0);
}
