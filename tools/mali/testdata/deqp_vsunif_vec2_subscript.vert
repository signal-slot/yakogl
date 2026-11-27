attribute highp vec4 a_position;
attribute highp vec4 a_coords;
varying mediump vec3 v_color;
uniform mediump int ui_zero, ui_one;

void main()
{
    gl_Position = a_position;
    mediump vec2 coords = vec2(a_coords);
    mediump vec2 tmp;
    tmp.x = coords.x;
    tmp.y = coords.y * 0.5;
    mediump float res = 0.0;
    res += tmp[ui_zero];
    res += tmp[ui_one];
    v_color = vec3(res);
}
