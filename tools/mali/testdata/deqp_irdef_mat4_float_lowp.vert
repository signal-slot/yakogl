attribute highp vec4 a_position;
attribute lowp mat4 a_mat4;
varying lowp mat4 v_mat4;
attribute lowp float a_coords;
varying lowp float v_coords;

void main (void)
{
    gl_Position = a_position;
    v_mat4 = a_mat4;
    v_coords = a_coords;
}
