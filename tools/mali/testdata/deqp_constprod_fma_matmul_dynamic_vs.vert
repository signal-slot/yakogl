attribute highp vec4 a_position;
varying mediump vec4 v_color;
attribute highp mat3 a_mat3;
attribute highp vec3 a_coords;

void main (void)
{
    gl_Position = a_position;
    highp vec3 res = a_mat3 * a_coords;
    v_color = vec4(res, 1.0);
}
