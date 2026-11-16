attribute highp vec4 a_position;
varying mediump vec4 v_color;
uniform lowp mat3 u_in0;
uniform lowp mat3 u_in1;

void main (void)
{
    gl_Position = a_position;
    lowp mat3 res = u_in0 - u_in1;
    v_color = vec4(res[0]+res[1]+res[2], 1.0);
}
