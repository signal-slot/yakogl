attribute highp vec4 a_position;
varying mediump vec4 v_color;
uniform highp mat3 u_in0;
attribute highp vec3 a_coords;

void main (void)
{
    gl_Position = a_position;
    highp vec3 res = u_in0 * a_coords;
    v_color = vec4(res, 1.0);
}
