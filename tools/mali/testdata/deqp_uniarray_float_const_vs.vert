attribute highp vec4 a_position;
attribute highp vec4 a_coords;
varying mediump vec4 v_color;
uniform mediump float u_arr[4];

void main()
{
    gl_Position = a_position;
    mediump float res = float(0.0);
    res += u_arr[0];
    res += u_arr[1];
    res += u_arr[2];
    res += u_arr[3];
    v_color = vec4(res, 0.0, 0.0, 1.0);
}
