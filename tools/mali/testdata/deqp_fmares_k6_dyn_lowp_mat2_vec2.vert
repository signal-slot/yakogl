attribute highp vec4 a_position;
varying mediump vec4 v_color;
attribute lowp mat2 a_mat2;
attribute lowp vec2 a_coords;

void main (void)
{
    gl_Position = a_position;
    lowp vec2 res = a_mat2 * a_coords;
    v_color = vec4(res.x, res.y, res.x, 1.0);
}
