attribute highp vec4 a_position;
attribute mediump float a_coords;
varying mediump float v_coords;

void main (void)
{
    gl_Position = a_position;
    v_coords = a_coords;
}
