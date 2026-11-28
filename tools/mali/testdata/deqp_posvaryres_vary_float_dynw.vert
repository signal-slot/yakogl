attribute highp vec4 a_position;
attribute highp vec4 a_coords;
uniform mediump int ui_zero, ui_one, ui_two, ui_three;
varying mediump float var[4];

void main()
{
    gl_Position = a_position;
    var[ui_zero]  = float(a_coords);
    var[ui_one]   = float(a_coords) * 0.5;
    var[ui_two]   = float(a_coords) * 0.25;
    var[ui_three] = float(a_coords) * 0.125;
}
