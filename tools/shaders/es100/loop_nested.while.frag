#version 100
/* dEQP shaders.loops.while_uniform_iterations nested, nested_sequence,
 * nested_tricky_dataflow_1 and _2, fragment (es2fShaderLoopTests.cpp
 * createSpecialLoopCase, LOOPTYPE_WHILE, Apache-2.0), in one shader, with
 * dEQP's ui_one = 1, ui_two = 2, ui_three = 3 (ITER_COUNT = 3). */
precision mediump float;
varying mediump vec4 v_coords;
uniform mediump int ui_one, ui_two, ui_three;
void main()
{
    mediump vec4 coords = v_coords;
    mediump vec4 res = coords;
    /* nested */
    mediump int i = 0;
    while (i++ < ui_two)
    {
        mediump int j = 0;
        while (j++ < ui_three)
            res = res.yzwx;
    }
    mediump vec4 a = res;
    /* nested_sequence */
    res = coords;
    mediump int i2 = 0;
    while (i2++ < ui_three)
    {
        mediump int j = 0;
        while (j++ < ui_one)
            res = res.yzwx;
        while (j++ < ui_three)
            res = res.yzwx;
    }
    mediump vec4 b = res;
    /* nested_tricky_dataflow_1 */
    mediump int i3 = 0;
    while (i3++ < ui_three)
    {
        res = coords;
        mediump int j = 0;
        while (j++ < ui_two)
            res = res.yzwx;
    }
    mediump vec4 c = res;
    /* nested_tricky_dataflow_2 */
    mediump int i4 = 0;
    while (i4++ < ui_three)
    {
        res = coords.wxyz;
        mediump int j = 0;
        while (j++ < ui_two)
            res = res.yzwx;
        coords = res;
    }
    gl_FragColor = vec4(a.x * 0.25 + b.y * 0.25 + c.z * 0.25 + res.w * 0.25, a.y - b.x, c.w - res.z, 1.0);
}
