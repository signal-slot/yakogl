attribute highp vec4 a_position;
varying mediump vec4 v_color;
attribute mediump mat4 a_mat4;
attribute mediump vec4 a_coords;

void main (void)
{
    gl_Position = a_position;
    mediump vec4 res = a_mat4 * a_coords;
    v_color = vec4(res.x, res.y, res.z+res.w, 1.0);
}
