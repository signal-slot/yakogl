attribute highp vec4 a_position;
attribute mediump mat4 a_mat4;
varying mediump mat4 v_mat4;
attribute mediump float a_coords;
varying mediump float v_coords;

void main (void)
{
    gl_Position = a_position;
    v_mat4 = a_mat4;
    v_coords = a_coords;
}
