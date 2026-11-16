attribute highp vec4 a_position;
attribute highp vec4 a_coords;
varying mediump vec4 v_coords;

void main (void)
{
    v_coords = a_coords;
    gl_Position = a_position;
}
