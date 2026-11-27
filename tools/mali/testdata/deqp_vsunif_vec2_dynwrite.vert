attribute highp vec4 a_position;
attribute highp vec4 a_coords;
varying mediump vec3 v_color;
uniform mediump int ui_zero, ui_one;

void main()
{
    gl_Position = a_position;
    mediump vec2 coords = vec2(a_coords);
    mediump vec2 tmp;
    tmp[ui_zero]  = coords.x;
    tmp[ui_one]   = coords.y * 0.5;
    mediump float res = 0.0;
    res += tmp.x;
    res += tmp.y;
    v_color = vec3(res);
}
