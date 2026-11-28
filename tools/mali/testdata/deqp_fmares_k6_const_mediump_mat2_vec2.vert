attribute highp vec4 a_position;
varying mediump vec4 v_color;
const mediump mat2 in0 = mat2(0.6, 0.7, -1.0, 0.4);
attribute mediump vec2 a_coords;

void main (void)
{
    gl_Position = a_position;
    mediump vec2 res = in0 * a_coords;
    v_color = vec4(res.x, res.y, res.x, 1.0);
}
