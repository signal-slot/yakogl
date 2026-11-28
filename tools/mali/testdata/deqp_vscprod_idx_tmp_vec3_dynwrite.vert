attribute highp vec4 a_position;
attribute highp vec4 a_coords;
varying mediump vec4 v_color;
uniform mediump int ui_four;

void main()
{
    gl_Position = a_position;
    mediump vec3 coords = vec3(a_coords);
    mediump vec3 arr[4];
    for (int i = 0; i < ui_four; i++)
    {
        arr[i] = vec3(coords);
        coords = coords * 0.5;
    }
    mediump vec3 res = vec3(0.0);
    res += arr[0];
    res += arr[1];
    res += arr[2];
    res += arr[3];
    v_color = vec4(res, 1.0);
}
