attribute highp vec4 a_position;
attribute highp vec4 a_coords;
varying mediump vec4 v_color;

void main()
{
    gl_Position = a_position;
    mediump vec4 coords = vec4(a_coords);
    mediump vec4 arr[4];
    arr[0] = vec4(coords);
    arr[1] = vec4(coords) * 0.5;
    arr[2] = vec4(coords) * 0.25;
    arr[3] = vec4(coords) * 0.125;
    mediump vec4 res = vec4(0.0);
    for (int i = 0; i < 4; i++)
        res += arr[i];
    v_color = vec4(res);
}
