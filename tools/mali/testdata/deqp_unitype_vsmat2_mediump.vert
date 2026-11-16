attribute highp vec4 a_position;
varying mediump vec4 v_color;
uniform mediump mat2 u_in0;
attribute mediump float a_coords;

void main (void)
{
    gl_Position = a_position;
    mediump mat2 res = u_in0 + a_coords;
    v_color = vec4(res[0][0], res[1][0], res[0][1]+res[1][1], 1.0);
}
