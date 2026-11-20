attribute highp vec4 a_position;
varying mediump vec4 v_color;
const highp mat3 in0 = mat3(1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.2, -0.3, 1.0);
const highp mat3 in1 = mat3(1.2, 0.1, 0.2, 0.1, 0.9, -0.1, -0.1, 0.2, 0.7);

void main (void)
{
    gl_Position = a_position;
    highp mat3 res = in0 * in1;
    v_color = vec4(res[0]+res[1]+res[2], 1.0);
}
