attribute highp vec4 a_position;
attribute highp vec4 a_coords;
varying mediump vec4 v_color;
uniform mediump float u_arr[4];

void main()
{
    gl_Position = a_position;
    mediump float res = float(0.0);
    for (int i = 0; i < 4; i++)
        res += u_arr[i];
    v_color = vec4(res, 0.0, 0.0, 1.0);
}
