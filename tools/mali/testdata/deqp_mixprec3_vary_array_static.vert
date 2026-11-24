attribute highp vec4 a_position;
attribute highp vec4 a_coords;
varying mediump float var[4];

void main()
{
    gl_Position = a_position;
    var[0] = float(a_coords);
    var[1] = float(a_coords) * 0.5;
    var[2] = float(a_coords) * 0.25;
    var[3] = float(a_coords) * 0.125;
}
