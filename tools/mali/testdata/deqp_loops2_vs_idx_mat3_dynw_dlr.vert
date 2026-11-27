attribute highp vec4 a_position;
attribute highp vec4 a_coords;
varying mediump vec4 v_color;
uniform mediump int ui_zero, ui_one, ui_two;
uniform mediump int ui_three;

void main()
{
    gl_Position = a_position;
    mediump vec4 coords = a_coords;
    mediump mat3 tmp;
    tmp[ui_zero]  = vec3(coords);
    tmp[ui_one]   = vec3(coords.yzwx) * 0.5;
    tmp[ui_two]   = vec3(coords.zwxy) * 0.25;
    mediump vec3 res = vec3(0.0);
    for (int i = 0; i < ui_three; i++)
        res += tmp[i];
    v_color = vec4(res, 1.0);
}
