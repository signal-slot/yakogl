attribute highp vec4 a_position;
attribute highp vec2 a_coords;
varying highp vec2 v_coords;
attribute highp mat2 a_mat2;
varying highp mat2 v_mat2;

void main (void)
{
    gl_Position = a_position;
    v_coords = a_coords;
    v_mat2 = a_mat2;
}
