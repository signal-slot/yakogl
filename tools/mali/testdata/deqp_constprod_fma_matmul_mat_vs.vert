attribute highp vec4 a_position;
varying mediump vec4 v_color;
attribute lowp mat3 a_mat3;
uniform lowp mat3 u_in1;

void main (void)
{
    gl_Position = a_position;
    lowp mat3 res = a_mat3 * u_in1;
    v_color = vec4(res[0]+res[1]+res[2], 1.0);
}
