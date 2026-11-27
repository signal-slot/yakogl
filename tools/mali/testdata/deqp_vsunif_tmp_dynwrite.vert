attribute highp vec4 a_position;
attribute highp vec4 a_coords;
varying mediump vec4 v_color;
uniform mediump int ui_zero, ui_one, ui_two, ui_three;

void main()
{
    gl_Position = a_position;
    mediump vec4 coords = vec4(a_coords);
    mediump vec4 arr[4];
    arr[ui_zero]  = vec4(coords);
    arr[ui_one]   = vec4(coords) * 0.5;
    arr[ui_two]   = vec4(coords) * 0.25;
    arr[ui_three] = vec4(coords) * 0.125;
    mediump vec4 res = vec4(0.0);
    res += arr[0];
    res += arr[1];
    res += arr[2];
    res += arr[3];
    v_color = vec4(res);
}
