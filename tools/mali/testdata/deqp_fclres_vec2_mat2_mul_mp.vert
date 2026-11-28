attribute highp vec4 a_position;
attribute mediump vec2 a_coords;
varying mediump vec2 v_coords;
attribute mediump mat2 a_mat2;
varying mediump mat2 v_mat2;

void main (void)
{
    gl_Position = a_position;
    v_coords = a_coords;
    v_mat2 = a_mat2;
}
