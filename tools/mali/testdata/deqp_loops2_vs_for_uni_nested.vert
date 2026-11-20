attribute highp vec4 a_position;
attribute highp vec4 a_coords;
varying mediump vec3 v_color;
uniform mediump int ui_zero, ui_one, ui_two, ui_three, ui_four, ui_five, ui_six;

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
    v_color = res.rgb;
}
