attribute highp vec4 a_position;
attribute highp mat3 a_mat3;
varying highp mat3 v_mat3;
attribute highp float a_coords;
varying highp float v_coords;

void main (void)
{
    gl_Position = a_position;
    v_mat3 = a_mat3;
    v_coords = a_coords;
}
