attribute highp vec4 a_position;
varying mediump vec4 v_color;
const highp mat4 in0 = mat4(1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.2, -0.3, 0.2, 1.0);
attribute highp vec4 a_coords;

void main (void)
{
    gl_Position = a_position;
    highp vec4 res = in0 * a_coords;
    v_color = vec4(res.x, res.y, res.z+res.w, 1.0);
}
