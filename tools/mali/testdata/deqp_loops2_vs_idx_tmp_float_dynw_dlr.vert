attribute highp vec4 a_position;
attribute highp vec4 a_coords;
varying mediump vec4 v_color;
uniform mediump int ui_zero, ui_one, ui_two, ui_three;
uniform mediump int ui_four;

void main()
{
    gl_Position = a_position;
    mediump float coords = float(a_coords);
    mediump float arr[4];
    arr[ui_zero]  = float(coords);
    arr[ui_one]   = float(coords) * 0.5;
    arr[ui_two]   = float(coords) * 0.25;
    arr[ui_three] = float(coords) * 0.125;
    mediump float res = float(0.0);
    for (int i = 0; i < ui_four; i++)
        res += arr[i];
    v_color = vec4(res, 0.0, 0.0, 1.0);
}
