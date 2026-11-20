attribute highp vec4 a_position;
attribute highp vec4 a_coords;
varying mediump vec4 v_coords;

void main()
{
    gl_Position = a_position;
    v_coords = a_coords;
}
