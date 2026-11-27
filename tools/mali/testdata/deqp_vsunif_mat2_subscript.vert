attribute highp vec4 a_position;
attribute highp vec4 a_coords;
varying mediump vec4 v_color;
uniform mediump int ui_zero, ui_one;

void main()
{
    gl_Position = a_position;
    mediump vec4 coords = a_coords;
    mediump mat2 tmp;
    tmp[0] = vec2(coords);
    tmp[1] = vec2(coords.yzwx) * 0.5;
    mediump vec2 res = vec2(0.0);
    res += tmp[ui_zero];
    res += tmp[ui_one];
    v_color = vec4(res, 0.0, 1.0);
}
