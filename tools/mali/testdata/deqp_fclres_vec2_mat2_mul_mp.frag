varying mediump vec2 v_coords;
varying mediump mat2 v_mat2;

void main (void)
{
    mediump vec2 res = v_coords * v_mat2;
    gl_FragColor = vec4(res.x, res.y, res.x, 1.0);
}
