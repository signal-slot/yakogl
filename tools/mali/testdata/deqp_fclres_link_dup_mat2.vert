attribute highp vec4 a_position;
varying mediump vec4 v_color;
attribute mediump vec2 a_coords;
const mediump mat2 in1 = mat2(-0.5, 0.7, -0.4, -0.8);

void main (void)
{
    gl_Position = a_position;
    mediump vec2 res = a_coords * in1;
    v_color = vec4(res.x, res.y, res.x, 1.0);
}
