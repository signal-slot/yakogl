#version 100
/* dEQP shaders.loops.for_uniform_iterations nested and
 * nested_tricky_dataflow_2, vertex (es2fShaderLoopTests.cpp, Apache-2.0),
 * with dEQP's uniform values (ui_two = 2, ui_three = 3). es100/va.frag
 * draws v_color. */
attribute highp vec4 a_position;
attribute highp vec4 a_coords;
varying mediump vec4 v_color;
uniform mediump int ui_two, ui_three;
void main()
{
    gl_Position = a_position;
    mediump vec4 coords = a_coords;
    mediump vec4 res = coords;
    for (mediump int i = 0; i < ui_two; i++)
    {
        for (mediump int j = 0; j < ui_three; j++)
            res = res.yzwx;
    }
    mediump vec4 a = res;
    for (mediump int i = 0; i < ui_three; i++)
    {
        res = coords.wxyz;
        for (mediump int j = 0; j < ui_two; j++)
            res = res.yzwx;
        coords = res;
    }
    v_color = vec4(a.xy, res.zw);
}
