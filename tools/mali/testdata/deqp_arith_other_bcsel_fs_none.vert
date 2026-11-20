attribute highp vec4 a_position;
attribute mediump vec4 a_unitCoords;
varying mediump vec4 v_coords;

void main()
{
    gl_Position = a_position;
    v_coords = a_unitCoords;
}
