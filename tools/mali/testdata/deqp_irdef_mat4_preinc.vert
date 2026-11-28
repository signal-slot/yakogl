attribute highp vec4 a_position;
attribute mediump mat4 a_mat4;
varying mediump mat4 v_mat4;

void main (void)
{
    gl_Position = a_position;
    v_mat4 = a_mat4;
}
