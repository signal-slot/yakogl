attribute highp vec4 a_position;
attribute mediump mat3 a_mat3;
varying mediump mat3 v_mat3;

void main (void)
{
    gl_Position = a_position;
    v_mat3 = a_mat3;
}
