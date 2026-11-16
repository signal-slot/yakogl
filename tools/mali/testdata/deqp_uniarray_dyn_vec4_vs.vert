attribute highp vec4 a_position;
attribute highp vec4 a_coords;
varying mediump vec4 v_color;
uniform mediump int ui_zero, ui_one, ui_two, ui_three;
uniform mediump vec4 u_arr[4];

void main()
{
    gl_Position = a_position;
    mediump vec4 res = vec4(0.0);
    res += u_arr[ui_zero];
    res += u_arr[ui_one];
    res += u_arr[ui_two];
    res += u_arr[ui_three];
    v_color = vec4(res);
}
