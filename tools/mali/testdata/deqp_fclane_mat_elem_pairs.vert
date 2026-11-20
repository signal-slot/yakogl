attribute highp vec4 a_position;
attribute highp mat2 a_mat2;
varying highp mat2 v_mat2;
attribute highp float a_coords;
varying highp float v_coords;

void main (void)
{
    gl_Position = a_position;
    v_mat2 = a_mat2;
    v_coords = a_coords;
}
