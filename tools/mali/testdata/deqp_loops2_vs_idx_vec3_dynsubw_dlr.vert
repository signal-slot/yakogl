attribute highp vec4 a_position;
attribute highp vec4 a_coords;
varying mediump vec3 v_color;
uniform mediump int ui_zero, ui_one, ui_two;
uniform mediump int ui_three;

void main()
{
    gl_Position = a_position;
    mediump vec3 coords = vec3(a_coords);
    mediump vec3 tmp;
    tmp[ui_zero]  = coords.x;
    tmp[ui_one]   = coords.y * 0.5;
    tmp[ui_two]   = coords.z * 0.25;
    mediump float res = 0.0;
    for (int i = 0; i < ui_three; i++)
        res += tmp[i];
    v_color = vec3(res);
}
