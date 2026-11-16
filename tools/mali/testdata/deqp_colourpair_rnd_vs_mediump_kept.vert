attribute highp vec4 a_position;
attribute highp vec4 a_coords;
varying mediump vec4 v_color;

void main()
{
    gl_Position = a_position;
    mediump float coords = float(a_coords);
    mediump float arr[4];
    arr[0] = float(coords);
    arr[1] = float(coords) * 0.5;
    arr[2] = float(coords) * 0.25;
    arr[3] = float(coords) * 0.125;
    mediump float res = float(0.0);
    res += arr[0];
    res += arr[1];
    res += arr[2];
    res += arr[3];
    v_color = vec4(res, 0.0, 0.0, 1.0);
}
