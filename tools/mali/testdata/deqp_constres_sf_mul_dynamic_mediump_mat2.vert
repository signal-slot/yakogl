attribute highp vec4 a_position;
attribute mediump mat2 a_mat2;
varying mediump mat2 v_mat2;
attribute mediump float a_coords;
varying mediump float v_coords;

void main (void)
{
    gl_Position = a_position;
    v_mat2 = a_mat2;
    v_coords = a_coords;
}
