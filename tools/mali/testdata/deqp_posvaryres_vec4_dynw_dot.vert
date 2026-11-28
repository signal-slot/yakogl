attribute highp vec4 a_position;
attribute highp vec4 a_coords;
varying mediump vec3 v_color;
uniform mediump int ui_zero, ui_one, ui_two, ui_three;

void main()
{
    gl_Position = a_position;
    mediump vec4 coords = vec4(a_coords);
    mediump vec4 tmp;
    tmp[ui_zero]  = coords.x;
    tmp[ui_one]   = coords.y * 0.5;
    tmp[ui_two]   = coords.z * 0.25;
    tmp[ui_three] = coords.w * 0.125;
    mediump float res = 0.0;
    res = dot(tmp, vec4(1.0));
    v_color = vec3(res);
}
