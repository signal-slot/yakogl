attribute highp vec4 a_position;
attribute highp vec4 a_coords;
varying mediump vec3 v_color;
uniform mediump int ui_zero, ui_one, ui_two, ui_three, ui_four, ui_five, ui_six;

void main()
{
    gl_Position = a_position;
    mediump vec4 coords = a_coords;
    mediump vec4 res = coords;
    for (mediump ivec4 i = ivec4(0, 1, ui_three, 0); i.x < i.z; i.x += i.y) { res = res.yzwx; }
    v_color = res.rgb;
}
