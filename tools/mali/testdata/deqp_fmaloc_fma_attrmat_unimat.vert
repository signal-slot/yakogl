attribute highp vec4 a_position;
varying mediump vec4 v_color;
attribute highp mat4 a_mat4;
uniform highp mat4 u_in1;

void main (void)
{
    gl_Position = a_position;
    highp mat4 res = a_mat4 * u_in1;
    v_color = vec4(res[0].xyz+res[1].yzw+res[2].zwx+res[3].wxy, 1.0);
}
