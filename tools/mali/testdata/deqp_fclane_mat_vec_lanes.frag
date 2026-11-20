varying highp vec2 v_coords;
varying highp mat2 v_mat2;

void main (void)
{
    highp vec2 res = v_coords * v_mat2;
    gl_FragColor = vec4(res.x, res.y, res.x, 1.0);
}
