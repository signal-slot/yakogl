attribute highp vec4 a_position;
attribute highp vec4 a_coords;
varying mediump vec4 v_color;
uniform mediump int ui_zero, ui_one, ui_two, ui_three;

void main()
{
    gl_Position = a_position;
    mediump vec4 coords = a_coords;
    mediump mat4 tmp;
    tmp[ui_zero]  = vec4(coords);
    tmp[ui_one]   = vec4(coords.yzwx) * 0.5;
    tmp[ui_two]   = vec4(coords.zwxy) * 0.25;
    tmp[ui_three] = vec4(coords.wxyz) * 0.125;
    mediump vec4 res = vec4(0.0);
    res += tmp[0];
    res += tmp[1];
    res += tmp[2];
    res += tmp[3];
    v_color = vec4(res);
}
