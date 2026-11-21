attribute highp vec4 a_position;
attribute highp vec4 a_coords;
varying mediump vec3 var[4];

void main()
{
    gl_Position = a_position;
    var[0] = vec3(a_coords);
    var[1] = vec3(a_coords) * 0.5;
    var[2] = vec3(a_coords) * 0.25;
    var[3] = vec3(a_coords) * 0.125;
}
