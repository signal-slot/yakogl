attribute highp vec4 a_position;
attribute highp vec4 a_coords;
varying mediump vec4 v_color;
uniform mediump int ui_four;

void main()
{
    gl_Position = a_position;
    mediump float coords = float(a_coords);
    mediump float arr[4];
    for (int i = 0; i < ui_four; i++)
    {
        arr[i] = float(coords);
        coords = coords * 0.5;
    }
    mediump float res = float(0.0);
    res += arr[0];
    res += arr[1];
    res += arr[2];
    res += arr[3];
    v_color = vec4(res, 0.0, 0.0, 1.0);
}
