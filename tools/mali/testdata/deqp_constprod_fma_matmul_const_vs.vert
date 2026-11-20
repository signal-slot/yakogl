attribute highp vec4 a_position;
varying mediump vec4 v_color;
const highp mat2 in0 = mat2(0.6, 0.7, -1.0, 0.4);
const highp mat2 in1 = mat2(-0.5, 0.7, -0.4, -0.8);

void main (void)
{
    gl_Position = a_position;
    highp mat2 res = in0 * in1;
    v_color = vec4(res[0][0], res[1][0], res[0][1]+res[1][1], 1.0);
}
