attribute highp vec4 a_position;
attribute highp vec4 a_coords;
varying mediump vec4 v_color;
uniform mediump int ui_four;

void main()
{
    gl_Position = a_position;
    mediump vec4 coords = a_coords;
    mediump mat4 tmp;
    for (int i = 0; i < ui_four; i++)
    {
        tmp[i] = vec4(coords);
        coords = coords.yzwx * 0.5;
    }
    mediump vec4 res = vec4(0.0);
    for (int i = 0; i < ui_four; i++)
        res += tmp[i];
    v_color = vec4(res);
}
