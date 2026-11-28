attribute highp vec4 a_position;
attribute highp vec4 a_coords;
varying mediump vec2 var[4];

void main()
{
    gl_Position = a_position;
    var[0] = vec2(a_coords);
    var[1] = vec2(a_coords) * 0.5;
    var[2] = vec2(a_coords) * 0.25;
    var[3] = vec2(a_coords) * 0.125;
}
