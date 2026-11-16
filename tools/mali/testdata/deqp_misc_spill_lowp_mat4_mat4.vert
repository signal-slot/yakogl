attribute highp vec4 a_position;
varying mediump vec4 v_color;
uniform lowp mat4 u_in0;
uniform lowp mat4 u_in1;

void main (void)
{
    gl_Position = a_position;
    lowp mat4 res = u_in0 * u_in1;
    v_color = vec4(res[0].xyz+res[1].yzw+res[2].zwx+res[3].wxy, 1.0);
}
