attribute highp vec4 a_position;
attribute highp vec4 a_coords;
varying mediump vec4 v_color;
uniform mediump int ui_zero, ui_one, ui_two, ui_three;

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
    res += arr[0];
    res += arr[1];
    res += arr[2];
    res += arr[3];
    v_color = vec4(res, 0.0, 0.0, 1.0);
}
