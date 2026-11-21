attribute highp vec4 a_position;
attribute highp vec4 a_coords;
varying mediump vec3 v_color;
uniform mediump int ui_three;

void main()
{
    gl_Position = a_position;
    mediump vec3 coords = vec3(a_coords);
    mediump vec3 tmp;
    for (int i = 0; i < ui_three; i++)
    {
        tmp[i] = coords.x;
        coords = coords.yzx * 0.5;
    }
    mediump float res = 0.0;
    res += tmp.x;
    res += tmp.y;
    res += tmp.z;
    v_color = vec3(res);
}
