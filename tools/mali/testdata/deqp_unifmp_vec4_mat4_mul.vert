attribute highp vec4 a_position;
attribute mediump vec4 a_coords;
varying mediump vec4 v_coords;

void main (void)
{
    gl_Position = a_position;
    v_coords = a_coords;
}
