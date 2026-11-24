attribute highp vec4 a_position;
attribute highp vec4 a_coords;
uniform mediump int ui_zero, ui_one, ui_two, ui_three;
varying mediump vec4 var[4];

void main()
{
    gl_Position = a_position;
    var[ui_zero]  = vec4(a_coords);
    var[ui_one]   = vec4(a_coords) * 0.5;
    var[ui_two]   = vec4(a_coords) * 0.25;
    var[ui_three] = vec4(a_coords) * 0.125;
}
