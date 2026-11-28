attribute highp vec4 a_position;
attribute highp float a_coords;
varying highp float v_coords;

void main (void)
{
    gl_Position = a_position;
    v_coords = a_coords;
}
