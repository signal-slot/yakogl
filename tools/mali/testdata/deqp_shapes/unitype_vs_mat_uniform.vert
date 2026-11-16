attribute highp vec4 a_position;
varying mediump vec4 v_color;
attribute highp mat2 a_mat2;
uniform highp mat2 u_in1;

void main (void)
{
    gl_Position = a_position;
    highp mat2 res = a_mat2 + u_in1;
    v_color = vec4(res[0][0], res[1][0], res[0][1]+res[1][1], 1.0);
}
