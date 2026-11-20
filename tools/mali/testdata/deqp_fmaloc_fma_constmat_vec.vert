attribute highp vec4 a_position;
varying mediump vec4 v_color;
const highp mat3 in0 = mat3(1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.2, -0.3, 1.0);
attribute highp vec3 a_coords;

void main (void)
{
    gl_Position = a_position;
    highp vec3 res = in0 * a_coords;
    v_color = vec4(res, 1.0);
}
